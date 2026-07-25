#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use neri_player_desktop::audio::analyzer::SharedAudioLevel;
use neri_player_desktop::audio::media_session::{MediaAction, MediaSessionController};
use neri_player_desktop::auth;
use neri_player_desktop::commands::{
    auth_cmd, download_cmd, image_cmd, library_cmd, listen_together_cmd, lyrics_cmd, player_cmd,
    recommend_cmd, search_cmd, settings_cmd, storage_cmd, sync_cmd,
};
use neri_player_desktop::state::AppState;
use std::sync::mpsc;
use std::time::Duration;
use tauri::{Emitter, Manager};

fn main() {
    // 强制 WebView2 (Chromium) 启用 GPU 硬件加速
    // 仅 Windows 生效：WEBVIEW2_* 对 macOS 的 WKWebView、Linux 的 WebKitGTK 均为 no-op
    #[cfg(target_os = "windows")]
    std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
        "--enable-gpu --enable-gpu-rasterization --enable-zero-copy --enable-features=CanvasOopRasterization");

    // 在其余插件之前初始化统一日志：启动期直接读取持久化设置决定
    // 是否写文件与日志级别（此时尚无 app handle）
    let log_cfg = neri_player_desktop::logging::load_bootstrap_config();

    tauri::Builder::default()
        .plugin(neri_player_desktop::logging::build_plugin(
            log_cfg.log_to_file,
            log_cfg.level,
        ))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState::new())
        .setup(|app| {
            let handle = app.handle().clone();

            // macOS 使用原生红绿灯（Overlay 标题栏）；Windows/Linux 移除原生装饰，
            // 由前端 TitleBar.vue 自绘窗口控制。配置里 decorations 默认为 true 以
            // 启用 macOS 的 titleBarStyle Overlay，其余平台在此运行时关闭。
            #[cfg(not(target_os = "macos"))]
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_decorations(false);
            }

            // 恢复持久化的登录 Cookie
            {
                let state = handle.state::<AppState>();
                let saved_auth = auth::cookies::load_auth(&handle);
                auth::cookies::inject_all(&state.cookie_jar, &saved_auth);
                *state.auth.lock() = saved_auth;
            }

            // 启动即主动保鲜一次 YouTube 会话, 让长期空闲的登录在首次使用前完成 cookie 轮换
            {
                let handle_yt = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let state = handle_yt.state::<AppState>();
                    auth_cmd::maybe_refresh_youtube_session(&handle_yt, state.inner(), true).await;
                });
            }

            // 启动时迁移同步 Token 和 WebDAV 密码到当前构建的凭据存储
            sync_cmd::initialize_secure_storage(&handle);

            // 初始化系统媒体会话 (SMTC / MPRIS)
            let (media_action_tx, media_action_rx) = mpsc::channel::<MediaAction>();

            // 获取 HWND（Windows 必需）
            let hwnd: Option<*mut std::ffi::c_void> = {
                #[cfg(target_os = "windows")]
                {
                    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                    let result = app.get_webview_window("main").and_then(|w| {
                        let handle = w.window_handle().ok()?;
                        if let RawWindowHandle::Win32(h) = handle.as_raw() {
                            Some(h.hwnd.get() as *mut std::ffi::c_void)
                        } else {
                            None
                        }
                    });
                    result
                }
                #[cfg(not(target_os = "windows"))]
                {
                    None
                }
            };

            let media_session = MediaSessionController::new(hwnd, media_action_tx);
            if media_session.is_none() {
                log::warn!(target: "media_session", "系统媒体会话不可用（非致命）");
            }

            // 后台定时器：每 200ms 推送播放位置 + 媒体会话同步
            let handle_ticker = handle.clone();
            std::thread::spawn(move || {
                let mut last_ended = false;
                let mut media_update_counter: u32 = 0;
                // 缓存上次发送给 media session 的元数据 ID，避免重复设置
                let mut last_media_track_id = String::new();

                loop {
                    std::thread::sleep(Duration::from_millis(200));

                    let state = handle_ticker.state::<AppState>();

                    // 处理媒体键事件
                    while let Ok(action) = media_action_rx.try_recv() {
                        match action {
                            MediaAction::Play => {
                                let _ = handle_ticker.emit("media:play", ());
                            }
                            MediaAction::Pause => {
                                let _ = handle_ticker.emit("media:pause", ());
                            }
                            MediaAction::Toggle => {
                                let _ = handle_ticker.emit("media:toggle", ());
                            }
                            MediaAction::Next => {
                                let _ = handle_ticker.emit("media:next", ());
                            }
                            MediaAction::Previous => {
                                let _ = handle_ticker.emit("media:previous", ());
                            }
                            MediaAction::SeekTo(ms) => {
                                let _ = handle_ticker.emit(
                                    "media:seek-requested",
                                    serde_json::json!({ "positionMs": ms }),
                                );
                            }
                        }
                    }

                    // 快速快照（锁持有 <1μs）
                    let snapshot = {
                        let player = state.player.lock();
                        if player.current_path.is_none() {
                            last_ended = false;
                            // 空闲时也更新 media session 状态
                            if !last_media_track_id.is_empty() {
                                if let Some(ref ms) = media_session {
                                    ms.stop();
                                }
                                last_media_track_id.clear();
                            }
                            continue;
                        }
                        (
                            player.is_playing,
                            player.position_ms(),
                            player.duration_ms,
                            player.loaded_generation().unwrap_or(0),
                            player.shared_audio_level.clone(),
                        )
                    }; // <- 锁在此释放

                    let (snap_playing, snap_pos, snap_dur, snap_generation, shared_level) =
                        snapshot;

                    // 发射事件（无锁）
                    if snap_playing || snap_pos > 0 {
                        let _ = handle_ticker.emit(
                            "player:position",
                            serde_json::json!({
                                "positionMs": snap_pos,
                                "durationMs": snap_dur,
                                "isPlaying": snap_playing,
                                "requestGeneration": snap_generation,
                            }),
                        );
                    }

                    if snap_playing {
                        if let Some((level, beat)) = SharedAudioLevel::try_snapshot(&shared_level) {
                            let _ = handle_ticker.emit(
                                "player:audio-level",
                                serde_json::json!({
                                    "level": level,
                                    "beat": beat,
                                }),
                            );
                        }
                    }

                    // 媒体会话同步（每 1s = 每 5 个 tick）
                    if let Some(ref ms) = media_session {
                        media_update_counter += 1;

                        // 元数据更新：检查当前曲目是否变化
                        let current_track_id = {
                            let q = state.queue.lock();
                            q.current().map(|t| t.id.clone()).unwrap_or_default()
                        };
                        if !current_track_id.is_empty() && current_track_id != last_media_track_id {
                            last_media_track_id = current_track_id;
                            let q = state.queue.lock();
                            if let Some(track) = q.current() {
                                ms.update_metadata(
                                    &track.title,
                                    &track.artist,
                                    &track.album,
                                    track.cover_url.as_deref(),
                                    track.duration_ms,
                                );
                            }
                        }

                        if media_update_counter >= 5 {
                            media_update_counter = 0;
                            ms.update_playback(snap_playing, snap_pos);
                        }
                    }

                    // 慢检测：重新获取锁（is_finished 内部有 200ms recv）
                    // 中段解码饿死/虚拟 body 落点失败不能当「播完」——只在接近曲尾才 emit track-ended
                    {
                        let mut player = state.player.lock();
                        let position_ms = player.position_ms();
                        let duration_ms = player.duration_ms;
                        let near_end = duration_ms > 0
                            && (position_ms.saturating_add(5_000) >= duration_ms
                                || position_ms.saturating_mul(100) / duration_ms.max(1) >= 98);
                        let finished = player.is_finished()
                            && player.is_playing
                            && position_ms > 500
                            && near_end;
                        if finished && !last_ended {
                            last_ended = true;
                            let ended_generation = player.loaded_generation().unwrap_or(0);
                            player.mark_ended();
                            drop(player);
                            let _ = handle_ticker.emit(
                                "player:track-ended",
                                serde_json::json!({
                                    "requestGeneration": ended_generation,
                                }),
                            );
                        } else if !finished {
                            last_ended = false;
                        }
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            player_cmd::trace_playback_ui,
            player_cmd::begin_playback_request,
            player_cmd::play_file,
            player_cmd::play_cached_audio,
            player_cmd::play_cached_audio_candidates,
            player_cmd::play_url,
            player_cmd::play_url_fast,
            player_cmd::play_url_streaming,
            player_cmd::pause,
            player_cmd::resume,
            player_cmd::toggle_play_pause,
            player_cmd::set_volume,
            player_cmd::seek,
            player_cmd::stop,
            player_cmd::set_speed,
            player_cmd::set_loudness_gain,
            player_cmd::set_equalizer,
            player_cmd::reset_audio_effects,
            player_cmd::pause_with_fade,
            player_cmd::resume_with_fade,
            player_cmd::crossfade_url,
            player_cmd::crossfade_url_fast,
            player_cmd::crossfade_url_streaming,
            player_cmd::crossfade_file,
            player_cmd::get_player_state,
            player_cmd::next_track,
            player_cmd::prev_track,
            player_cmd::set_queue,
            player_cmd::toggle_shuffle,
            player_cmd::cycle_repeat,
            library_cmd::scan_music_directory,
            library_cmd::list_playlists,
            library_cmd::create_playlist,
            library_cmd::delete_playlist,
            library_cmd::rename_playlist,
            library_cmd::get_playlist_tracks,
            library_cmd::add_to_playlist,
            library_cmd::add_tracks_to_playlist,
            library_cmd::remove_from_playlist,
            library_cmd::remove_tracks_from_playlist,
            library_cmd::reorder_playlist_tracks,
            library_cmd::update_playlist_track,
            library_cmd::list_favorite_playlists,
            library_cmd::add_favorite_playlist,
            library_cmd::remove_favorite_playlist,
            search_cmd::search,
            image_cmd::fetch_bilibili_cover,
            lyrics_cmd::parse_lrc_content,
            lyrics_cmd::load_lyrics_file,
            lyrics_cmd::fetch_lyrics,
            settings_cmd::get_settings,
            settings_cmd::save_settings,
            settings_cmd::get_app_data_dir,
            settings_cmd::get_log_dir,
            settings_cmd::open_log_dir,
            settings_cmd::get_netease_song_url,
            settings_cmd::get_netease_song_download_url,
            settings_cmd::get_qq_song_url,
            settings_cmd::get_bili_video_pages,
            settings_cmd::get_bili_audio_url,
            settings_cmd::get_youtube_audio_url,
            settings_cmd::save_file_bytes,
            settings_cmd::set_bypass_proxy,
            settings_cmd::get_build_info,
            storage_cmd::get_storage_usage,
            storage_cmd::clear_storage_cache,
            auth_cmd::login_netease,
            auth_cmd::login_bilibili,
            auth_cmd::login_youtube,
            auth_cmd::login_with_cookies,
            auth_cmd::refresh_youtube_profile,
            auth_cmd::check_auth_status,
            auth_cmd::get_debug_cookie_storage_status,
            auth_cmd::clear_debug_cookie_storage,
            auth_cmd::logout,
            recommend_cmd::get_recommended_playlists,
            recommend_cmd::get_recommended_songs,
            recommend_cmd::get_user_playlists,
            recommend_cmd::get_user_account,
            recommend_cmd::get_home_feed,
            recommend_cmd::get_high_quality_playlists,
            recommend_cmd::get_high_quality_tags,
            recommend_cmd::like_song,
            recommend_cmd::get_liked_song_ids,
            recommend_cmd::get_album_detail,
            recommend_cmd::get_netease_song_detail,
            recommend_cmd::get_user_stared_albums,
            recommend_cmd::get_bili_fav_folder_info,
            recommend_cmd::get_bili_favorite_items,
            recommend_cmd::validate_auth,
            recommend_cmd::get_netease_playlist_detail,
            recommend_cmd::get_youtube_playlist_detail,
            sync_cmd::get_github_sync_config,
            sync_cmd::get_sync_preferences,
            sync_cmd::validate_github_token,
            sync_cmd::create_github_repo,
            sync_cmd::use_existing_github_repo,
            sync_cmd::configure_github_sync,
            sync_cmd::sync_github,
            sync_cmd::disconnect_github_sync,
            sync_cmd::update_github_sync_settings,
            sync_cmd::update_sync_preferences,
            sync_cmd::update_webdav_sync_settings,
            sync_cmd::clear_app_cache,
            sync_cmd::export_playlists,
            sync_cmd::import_playlists,
            sync_cmd::export_config,
            sync_cmd::import_config,
            sync_cmd::get_webdav_sync_config,
            sync_cmd::configure_webdav_sync,
            sync_cmd::sync_webdav,
            sync_cmd::disconnect_webdav_sync,
            download_cmd::download_track,
            download_cmd::list_downloads,
            download_cmd::validate_downloads,
            download_cmd::delete_download,
            download_cmd::cancel_download,
            download_cmd::cancel_all_downloads,
            download_cmd::set_download_dir,
            download_cmd::get_default_download_dir,
            download_cmd::reveal_file,
            listen_together_cmd::lt_create_room,
            listen_together_cmd::lt_join_room,
            listen_together_cmd::lt_get_room_state,
            listen_together_cmd::lt_connect_ws,
            listen_together_cmd::lt_disconnect_ws,
            listen_together_cmd::lt_send_event,
            listen_together_cmd::lt_send_ping,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
