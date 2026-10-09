#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use neri_player_desktop::audio::analyzer::SharedAudioLevel;
use neri_player_desktop::audio::media_session::{MediaAction, MediaSessionController};
use neri_player_desktop::auth;
use neri_player_desktop::commands::{
    auth_cmd, cache_cmd, debug_cmd, desktop_lyrics_cmd, download_cmd, image_cmd, library_cmd,
    local_files_cmd, listen_together_cmd, lyrics_cmd, player_cmd, playback_fallback_cmd,
    recommend_cmd, search_cmd, settings_cmd, stats_cmd, storage_cmd, sync_cmd, tray_cmd,
    user_data_cmd,
};
use neri_player_desktop::state::AppState;
use std::sync::mpsc;
use std::time::Duration;
use tauri::{Emitter, Manager, WindowEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlaybackFinishState {
    None,
    Ended,
    Stalled,
}

fn classify_playback_finish(
    finished_flag: bool,
    was_playing: bool,
    position_ms: u64,
    duration_ms: u64,
) -> PlaybackFinishState {
    if !finished_flag || !was_playing || position_ms <= 500 {
        return PlaybackFinishState::None;
    }

    // 流式内容可能没有 Content-Length，duration 为 0 时 EOF 就是正常结束。
    // 只有已知时长且明显不在结尾才作为中段断流恢复
    if duration_ms == 0
        || position_ms.saturating_add(5_000) >= duration_ms
        || position_ms.saturating_mul(100) / duration_ms >= 98
    {
        PlaybackFinishState::Ended
    } else {
        PlaybackFinishState::Stalled
    }
}

fn main() {
    // 崩溃收集必须最早安装：之后任何线程 panic 都会把现场落盘
    neri_player_desktop::logging::install_panic_hook(env!("CARGO_PKG_VERSION"));
    // 强制 WebView2 (Chromium) 启用 GPU 硬件加速
    // 仅 Windows 生效：WEBVIEW2_* 对 macOS 的 WKWebView、Linux 的 WebKitGTK 均为 no-op
    #[cfg(target_os = "windows")]
    std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
        "--enable-gpu --enable-gpu-rasterization --enable-zero-copy --enable-features=CanvasOopRasterization");

    // WebKitGTK 的 DMABUF 渲染在 Nvidia 私有驱动与虚拟机上有已知的白屏/
    // WebGL 失效问题，而本应用重度依赖 WebGL 背景。取舍：禁用 DMABUF 牺牲
    // 少量合成性能，换取这类环境下能正常显示。用户已显式设置时尊重用户选择
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    // 在其余插件之前初始化统一日志：启动期直接读取持久化设置决定
    // 是否写文件与日志级别（此时尚无 app handle）
    let log_cfg = neri_player_desktop::logging::load_bootstrap_config();

    tauri::Builder::default()
        // 单实例保护必须最先注册：双实例会共享 deviceId 与 causal counter，
        // 重复发号 token 造成跨设备同步静默数据损坏，playlists/stats 等
        // 落盘文件也会互相覆盖。二次启动改为聚焦已有窗口
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray_cmd::show_main_window(app);
        }))
        .plugin(neri_player_desktop::logging::build_plugin(
            log_cfg.log_to_file,
            log_cfg.level,
        ))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState::new())
        .setup(move |app| {
            let handle = app.handle().clone();
            // 日志插件初始化时把 max_level 放到了 Trace，这里收回到设置的级别
            neri_player_desktop::logging::set_runtime_level(log_cfg.level);

            // 安装包把 FFmpeg 放在资源目录的 ffmpeg/ 下，各平台、各包格式的位置由 Tauri 给出
            match app.path().resource_dir() {
                Ok(resources) => {
                    neri_player_desktop::audio::ffmpeg::set_bundled_directory(resources.join("ffmpeg"));
                }
                Err(error) => log::warn!(
                    target: "audio-decoder",
                    "resource directory unavailable, looking for FFmpeg next to the executable only: {error}",
                ),
            }
            // 后台预加载 FFmpeg：第一首杜比或 Opus 曲目起播时就不用再等动态库加载
            let preload = std::thread::Builder::new()
                .name("ffmpeg-preload".into())
                .spawn(|| {
                    let _ = neri_player_desktop::audio::ffmpeg::runtime();
                });
            if let Err(error) = preload {
                log::warn!(target: "audio-decoder", "could not start FFmpeg preloading: {error}");
            }

            // macOS 使用原生红绿灯（Overlay 标题栏）；Windows/Linux 移除原生装饰，
            // 由前端 TitleBar.vue 自绘窗口控制。配置里 decorations 默认为 true 以
            // 启用 macOS 的 titleBarStyle Overlay，其余平台在此运行时关闭。
            #[cfg(not(target_os = "macos"))]
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_decorations(false);
            }

            #[cfg(windows)]
            if let Some(win) = app.get_webview_window("main") {
                pause_rendering_while_minimized(win);
            }

            // macOS: 挂载空 NSToolbar 并启用 Unified 工具栏样式（macOS 11+），
            // 由 AppKit 将标题栏加高到约 52pt 并把红绿灯垂直居中，与前端
            // 52px 的 CSS 标题栏对齐；全屏进出时按钮位置由系统自动管理，
            // 无需 tao 的 traffic_light_inset 手动平移
            #[cfg(target_os = "macos")]
            if let Some(win) = app.get_webview_window("main") {
                use objc2::{available, MainThreadMarker, MainThreadOnly};
                use objc2_app_kit::{
                    NSTitlebarSeparatorStyle, NSToolbar, NSWindow, NSWindowTitleVisibility,
                    NSWindowToolbarStyle,
                };

                if available!(macos = 11.0) {
                    if let Ok(ns_window_ptr) = win.ns_window() {
                        // Tauri 的 setup 钩子在 macOS 上运行于主线程
                        let mtm = MainThreadMarker::new()
                            .expect("setup 必须在主线程执行");
                        let ns_window = unsafe { &*ns_window_ptr.cast::<NSWindow>() };
                        let toolbar = NSToolbar::init(NSToolbar::alloc(mtm));
                        ns_window.setToolbar(Some(&toolbar));
                        ns_window.setToolbarStyle(NSWindowToolbarStyle::Unified);
                        // 去掉工具栏底部的系统分隔线, 由前端自行绘制标题栏视觉
                        ns_window.setTitlebarSeparatorStyle(NSTitlebarSeparatorStyle::None);
                        // Overlay 已设置透明标题栏与隐藏标题, 此处显式兜底防止被覆盖
                        ns_window.setTitlebarAppearsTransparent(true);
                        ns_window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
                    }
                }
            }

            // 系统托盘：关闭主窗口后收进托盘继续播放
            tray_cmd::setup(app)?;

            // 恢复持久化的登录 Cookie
            {
                let state = handle.state::<AppState>();
                let saved_auth = auth::cookies::load_auth(&handle);
                auth::cookies::inject_all(&state.cookie_jar, &saved_auth);
                *state.auth.lock() = saved_auth;
            }

            // 代理模式与 YouTube 地区偏好在首批请求前就按已保存的设置生效，不等前端水合
            match neri_player_desktop::settings::store::load_settings(&handle) {
                Ok(loaded) => {
                    if !loaded.settings.bypass_proxy {
                        handle.state::<AppState>().rebuild_http(false);
                    }
                    settings_cmd::apply_runtime_settings(&loaded.settings);
                    let handle_prune = handle.clone();
                    let cache_limit = loaded.settings.max_cache_size;
                    tauri::async_runtime::spawn_blocking(move || {
                        player_cmd::prune_media_caches(&handle_prune, cache_limit);
                    });
                }
                Err(error) => log::warn!(target: "settings", "启动时读取设置失败: {error}"),
            }

            // 启动即主动保鲜一次 YouTube 会话, 让长期空闲的登录在首次使用前完成 cookie 轮换
            {
                let handle_yt = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let state = handle_yt.state::<AppState>();
                    auth_cmd::maybe_refresh_youtube_session(&handle_yt, state.inner(), true).await;
                    // 会话保鲜之后再预热：首页配置按登录指纹缓存，Cookie 轮换前拿的会作废
                    let auth = state.auth.lock().youtube.clone();
                    neri_player_desktop::api::youtube::playback::warm_playback(auth, state.bypasses_system_proxy()).await;
                });
            }

            // 进程长时间保持运行时也要定期触发 SIDTS 主动轮换, 不能只依赖启动或下一次页面访问
            {
                let handle_yt = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(300));
                    interval.tick().await;
                    loop {
                        interval.tick().await;
                        let state = handle_yt.state::<AppState>();
                        auth_cmd::maybe_refresh_youtube_session(&handle_yt, state.inner(), false)
                            .await;
                    }
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
                let mut last_stalled = false;
                let mut media_update_counter: u32 = 0;
                // 缓存上次发送给 media session 的元数据 ID，避免重复设置
                let mut last_media_track_id = String::new();
                // 每 300 tick（约 60s）回收一次服务端轮换的 Cookie
                let mut cookie_sync_counter: u32 = 0;

                loop {
                    std::thread::sleep(Duration::from_millis(200));

                    let state = handle_ticker.state::<AppState>();

                    cookie_sync_counter = cookie_sync_counter.wrapping_add(1);
                    if cookie_sync_counter.is_multiple_of(300) {
                        auth_cmd::persist_rotated_cookies(&handle_ticker, state.inner());
                    }

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
                        player.current_path.is_some().then(|| {
                            (
                                player.is_playing,
                                player.position_ms(),
                                player.duration_ms,
                                player.loaded_generation().unwrap_or(0),
                                player.shared_audio_level.clone(),
                            )
                        })
                    }; // <- 锁在此释放

                    // 托盘更新会等主线程执行，必须在 player 锁外调用
                    tray_cmd::sync_playing(
                        &handle_ticker,
                        snapshot.as_ref().is_some_and(|snapshot| snapshot.0),
                    );

                    let Some((snap_playing, snap_pos, snap_dur, snap_generation, shared_level)) =
                        snapshot
                    else {
                        last_ended = false;
                        // 空闲时也更新 media session 状态
                        if !last_media_track_id.is_empty() {
                            if let Some(ref ms) = media_session {
                                ms.stop();
                            }
                            last_media_track_id.clear();
                        }
                        continue;
                    };

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

                        // 元数据来源改为前端镜像: PlayQueue 从不被前端填充, 读它使
                        // SMTC/MPRIS 永远拿不到曲目信息（PB-01）
                        let current_meta = state.media_metadata.lock().clone();
                        if let Some(meta) = current_meta {
                            if !meta.id.is_empty() && meta.id != last_media_track_id {
                                last_media_track_id = meta.id.clone();
                                ms.update_metadata(
                                    &meta.title,
                                    &meta.artist,
                                    &meta.album,
                                    meta.cover_url.as_deref(),
                                    meta.duration_ms,
                                );
                            }
                        }

                        if media_update_counter >= 5 {
                            media_update_counter = 0;
                            ms.update_playback(snap_playing, snap_pos);
                        }
                    }

                    // 慢检测：短锁发起查询，锁外等待结果——旧实现在持锁状态下
                    // recv_timeout(100ms)，音频线程忙（crossfade/prepare/seek）时
                    // 每 tick 持锁满 100ms，阻塞 pause/get_player_state 等命令
                    // 中段解码饿死/虚拟 body 落点失败不能当「播完」——只在接近曲尾才 emit track-ended
                    {
                        let (finished_rx, position_ms, duration_ms, was_playing) = {
                            let player = state.player.lock();
                            (
                                player.begin_finished_query(),
                                player.position_ms(),
                                player.duration_ms,
                                player.is_playing,
                            )
                        }; // <- 锁在此释放，下面的等待不再挡住其他 player 命令
                        let finished_flag = finished_rx
                            .map(|rx| {
                                rx.recv_timeout(Duration::from_millis(100)).unwrap_or(false)
                            })
                            .unwrap_or(false);
                        let finish_state = classify_playback_finish(
                            finished_flag,
                            was_playing,
                            position_ms,
                            duration_ms,
                        );
                        let finished = finish_state == PlaybackFinishState::Ended;
                        // 曲中断流/解码饿死/DeviceLost 重建失败不能当播完静默卡死，
                        // 发独立 stalled 事件让前端从当前位置重试
                        let stalled = finish_state == PlaybackFinishState::Stalled;
                        if stalled && !last_stalled {
                            last_stalled = true;
                            let stalled_generation = {
                                let player = state.player.lock();
                                player.loaded_generation().unwrap_or(0)
                            };
                            let _ = handle_ticker.emit(
                                "player:playback-stalled",
                                serde_json::json!({
                                    "positionMs": position_ms,
                                    "requestGeneration": stalled_generation,
                                }),
                            );
                        } else if !stalled {
                            last_stalled = false;
                        }
                        if finished && !last_ended {
                            last_ended = true;
                            let ended_generation = {
                                let mut player = state.player.lock();
                                let generation = player.loaded_generation().unwrap_or(0);
                                player.mark_ended();
                                generation
                            };
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
        .invoke_handler({
            // 安全护栏：Tauri 无条件向每个 webview（含加载第三方远端页面的登录窗口）注入
            // IPC 脚本，且应用自定义命令不经 ACL 校验。仅放行主窗口调用命令，阻止登录页
            // （music.163.com / passport.bilibili.com / accounts.google.com）上的任意 JS
            // 越权调用 save_file_bytes 等命令写/读任意文件或导出凭据
            // 桌面歌词窗口只允许读取显示快照，播放和账号命令仍只对主窗口开放
            let app_handler: Box<
                dyn Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static,
            > = Box::new(tauri::generate_handler![
            desktop_lyrics_cmd::open_desktop_lyrics,
            desktop_lyrics_cmd::close_desktop_lyrics,
            desktop_lyrics_cmd::publish_desktop_lyrics,
            desktop_lyrics_cmd::get_desktop_lyrics_snapshot,
            desktop_lyrics_cmd::set_desktop_lyrics_lock,
            desktop_lyrics_cmd::desktop_lyrics_hit_region,
            desktop_lyrics_cmd::desktop_lyrics_action,
            player_cmd::trace_playback_ui,
            player_cmd::begin_playback_request,
            player_cmd::play_file,
            player_cmd::play_cached_audio,
            player_cmd::play_cached_audio_candidates,
            player_cmd::has_cached_audio,
            player_cmd::play_url,
            player_cmd::play_url_fast,
            player_cmd::play_url_streaming,
            player_cmd::prewarm_remote_audio,
            player_cmd::pause,
            player_cmd::resume,
            player_cmd::toggle_play_pause,
            player_cmd::set_volume,
            player_cmd::list_audio_output_devices,
            player_cmd::set_audio_output_device,
            player_cmd::seek,
            player_cmd::stop,
            player_cmd::set_speed,
            player_cmd::set_loudness_gain,
            player_cmd::set_normalize_volume,
            player_cmd::set_volume_balance,
            player_cmd::set_equalizer,
            player_cmd::reset_audio_effects,
            player_cmd::pause_with_fade,
            player_cmd::resume_with_fade,
            player_cmd::crossfade_url,
            player_cmd::crossfade_url_fast,
            player_cmd::crossfade_url_streaming,
            player_cmd::crossfade_file,
            player_cmd::get_player_state,
            player_cmd::update_media_metadata,
            player_cmd::next_track,
            player_cmd::prev_track,
            player_cmd::set_queue,
            player_cmd::toggle_shuffle,
            player_cmd::cycle_repeat,
            library_cmd::scan_music_directory,
            local_files_cmd::scan_local_files,
            local_files_cmd::cancel_local_scan,
            local_files_cmd::get_local_playlist_tracks,
            local_files_cmd::edit_local_file_tags,
            local_files_cmd::get_local_audio_info,
            player_cmd::release_audio_file,
            player_cmd::get_playback_audio_info,
            player_cmd::get_decoder_capabilities,
            player_cmd::set_multichannel_drc,
            library_cmd::list_playlists,
            library_cmd::get_playlist_usage_stats,
            library_cmd::record_playlist_open,
            library_cmd::get_home_local_playlists,
            library_cmd::create_playlist,
            library_cmd::ensure_favorites_playlist,
            library_cmd::delete_playlist,
            library_cmd::rename_playlist,
            library_cmd::get_playlist_tracks,
            library_cmd::add_to_playlist,
            library_cmd::add_tracks_to_playlist,
            library_cmd::remove_from_playlist,
            library_cmd::remove_tracks_from_playlist,
            library_cmd::reorder_playlist_tracks,
            library_cmd::reorder_playlists,
            library_cmd::update_playlist_track,
            library_cmd::record_lyric_override,
            library_cmd::list_favorite_playlists,
            library_cmd::set_artist_favorite,
            library_cmd::import_followed_artists,
            library_cmd::get_bili_artist_detail,
            library_cmd::get_bili_artist_contents,
            library_cmd::get_bili_artist_collection,
            library_cmd::get_youtube_artist_detail,
            library_cmd::get_youtube_artist_items,
            playback_fallback_cmd::find_netease_local_sources,
            playback_fallback_cmd::find_netease_bili_sources,
            search_cmd::search,
            image_cmd::fetch_bilibili_cover,
            lyrics_cmd::parse_lrc_content,
            lyrics_cmd::load_lyrics_file,
            lyrics_cmd::fetch_lyrics,
            lyrics_cmd::fetch_word_timed_lyrics,
            lyrics_cmd::fetch_netease_romanized_lyric,
            lyrics_cmd::match_lyrics,
            settings_cmd::get_settings,
            settings_cmd::save_settings,
            settings_cmd::get_app_data_dir,
            settings_cmd::import_background_image,
            settings_cmd::clear_background_images,
            settings_cmd::get_log_dir,
            settings_cmd::get_netease_song_url,
            settings_cmd::get_qq_song_url,
            settings_cmd::get_bili_audio_url,
            settings_cmd::get_youtube_audio_url,
            settings_cmd::save_file_bytes,
            settings_cmd::set_bypass_proxy,
            settings_cmd::get_build_info,
            settings_cmd::get_system_accent_color,
            settings_cmd::probe_platform_connectivity,
            debug_cmd::get_recent_logs,
            debug_cmd::audio_engine_stats,
            debug_cmd::export_debug_report,
            debug_cmd::reveal_in_file_manager,
            debug_cmd::list_crash_reports,
            debug_cmd::read_crash_report,
            debug_cmd::clear_crash_reports,
            debug_cmd::debug_trigger_crash,
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
            recommend_cmd::get_netease_home_section,
            recommend_cmd::get_user_playlists,
            recommend_cmd::get_user_account,
            recommend_cmd::get_home_feed,
            recommend_cmd::get_high_quality_playlists,
            recommend_cmd::get_high_quality_tags,
            recommend_cmd::like_song,
            recommend_cmd::get_liked_song_ids,
            recommend_cmd::get_album_detail,
            recommend_cmd::get_netease_artist_detail,
            recommend_cmd::get_netease_artist_albums,
            recommend_cmd::get_netease_artist_songs,
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
            sync_cmd::approve_sync_protocol_upgrade,
            sync_cmd::disconnect_github_sync,
            sync_cmd::update_github_sync_settings,
            sync_cmd::update_sync_preferences,
            sync_cmd::update_webdav_sync_settings,
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
            listen_together_cmd::lt_leave_room,
            listen_together_cmd::lt_send_event,
            listen_together_cmd::lt_send_control,
            listen_together_cmd::lt_send_ping,
            stats_cmd::record_playback_session,
            stats_cmd::record_playback_sessions,
            stats_cmd::get_playback_stats,
            stats_cmd::get_playback_stats_overview,
            stats_cmd::clear_playback_stats,
            stats_cmd::remove_playback_stats,
            stats_cmd::playback_stats_identity_key,
            user_data_cmd::load_user_data_snapshot,
            user_data_cmd::import_legacy_user_data,
            user_data_cmd::save_playback_state,
            user_data_cmd::record_play_history,
            user_data_cmd::remove_play_history,
            user_data_cmd::clear_play_history,
            user_data_cmd::replace_play_history,
            user_data_cmd::set_lyric_offset,
            user_data_cmd::replace_lyric_offsets,
            cache_cmd::cache_get,
            cache_cmd::cache_put,
            cache_cmd::cache_remove,
            tray_cmd::publish_tray_snapshot,
            tray_cmd::get_tray_popup_state,
            tray_cmd::tray_popup_ready,
            tray_cmd::tray_popup_action,
            tray_cmd::quit_app,
            ]);
            move |invoke: tauri::ipc::Invoke<tauri::Wry>| {
                let label = invoke.message.webview().label().to_string();
                if !window_command_allowed(&label, invoke.message.command()) {
                    let command = invoke.message.command().to_string();
                    log::warn!(
                        target: "security",
                        "已拦截非主窗口 '{label}' 的 IPC 调用: {command}"
                    );
                    invoke
                        .resolver
                        .reject(format!("IPC not allowed from window '{label}'"));
                    return true;
                }
                app_handler(invoke)
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| match event {
            tauri::RunEvent::ExitRequested { .. } => {
                // 退出前 flush 一次轮转 Cookie：60s 定时器之外，退出前最后一窗口的
                // Set-Cookie 轮换令牌若不落盘，下次启动会重放旧令牌导致偶发掉登录（AU-05）
                let state = app_handle.state::<AppState>();
                auth_cmd::persist_rotated_cookies(app_handle, state.inner());
            }
            tauri::RunEvent::WindowEvent {
                label,
                event: WindowEvent::CloseRequested { api, .. },
                ..
            } if label == "main" => {
                // 关闭主窗口 = 收进托盘，播放继续；是否改为退出由前端按设置决定（quit_app）。
                // prevent_close 在 GTK 层真正取消 delete-event，不会重发；前端若在 JS 侧
                // hide()/close() 回退会与平台关闭状态互扰，在 WebKitGTK 下造成
                // CloseRequested 死循环并拖垮 GPU 上下文。桌面歌词、托盘面板照常关闭销毁
                api.prevent_close();
                tray_cmd::hide_main_window(app_handle);
            }
            tauri::RunEvent::WindowEvent {
                label,
                event: WindowEvent::Focused(true),
                ..
            } if label == "main" => tray_cmd::main_window_focused(app_handle),
            tauri::RunEvent::WindowEvent {
                label,
                event: WindowEvent::Destroyed,
                ..
            } if label == "main" => {
                // 主窗口销毁时一并关闭桌面歌词窗口与托盘面板
                for child in [desktop_lyrics_cmd::WINDOW_LABEL, tray_cmd::POPUP_LABEL] {
                    if let Some(window) = app_handle.get_webview_window(child) {
                        let _ = window.close();
                    }
                }
            }
            // 主窗口收进托盘后点 Dock 图标恢复
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => tray_cmd::show_main_window(app_handle),
            _ => {}
        });
}

/// 窗口最小化时告诉 WebView2 页面不可见，恢复时再设回可见
///
/// WebView2 不会自己察觉窗口最小化：页面仍报告可见，并按没有 vsync 的 300 多帧每秒驱动 rAF、
/// 动画与合成，只放着听歌也要吃掉半个核。设为不可见后 Chromium 暂停这些渲染；计时器节流已在
/// 启动参数里关掉，播放、桌面歌词等后台逻辑照常运行。
#[cfg(windows)]
fn pause_rendering_while_minimized(window: tauri::WebviewWindow) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let visible = Arc::new(AtomicBool::new(true));
    let handle = window.clone();
    window.on_window_event(move |event| {
        if !matches!(event, tauri::WindowEvent::Resized(_)) {
            return;
        }
        let show = !handle.is_minimized().unwrap_or(false);
        if visible.swap(show, Ordering::AcqRel) == show {
            return;
        }
        let result = handle.with_webview(move |webview| {
            // SAFETY: with_webview 的回调在 UI 线程上执行，控制器在窗口存活期间有效
            if let Err(error) = unsafe { webview.controller().SetIsVisible(show) } {
                log::warn!(target: "window", "WebView2 visibility not set to {show}: {error}");
            }
        });
        match result {
            Ok(()) => log::info!(
                target: "window",
                "webview rendering {}",
                if show { "resumed" } else { "paused while minimized" },
            ),
            Err(error) => log::warn!(
                target: "window",
                "could not reach the webview to set visibility {show}: {error}",
            ),
        }
    });
}

/// 桌面歌词窗口只能读自己的快照、报告解锁按钮位置、把工具栏操作转给主窗口；
/// 托盘面板只能读自己的状态、报告就绪、发出菜单动作
fn window_command_allowed(label: &str, command: &str) -> bool {
    label == "main"
        || (label == desktop_lyrics_cmd::WINDOW_LABEL
            && matches!(
                command,
                "get_desktop_lyrics_snapshot" | "desktop_lyrics_action" | "desktop_lyrics_hit_region"
            ))
        || (label == tray_cmd::POPUP_LABEL
            && matches!(
                command,
                "get_tray_popup_state" | "tray_popup_ready" | "tray_popup_action"
            ))
}

#[cfg(test)]
mod tests {
    use super::{classify_playback_finish, PlaybackFinishState};

    #[test]
    fn desktop_lyrics_window_only_reads_its_snapshot() {
        assert!(super::window_command_allowed(
            "main", "publish_desktop_lyrics",
        ));
        assert!(super::window_command_allowed(
            "desktop-lyrics", "get_desktop_lyrics_snapshot",
        ));
        assert!(super::window_command_allowed("desktop-lyrics", "desktop_lyrics_action"));
        assert!(super::window_command_allowed("desktop-lyrics", "desktop_lyrics_hit_region"));
        assert!(!super::window_command_allowed("desktop-lyrics", "set_desktop_lyrics_lock"));
        assert!(!super::window_command_allowed("desktop-lyrics", "play_url"));
        assert!(!super::window_command_allowed(
            "desktop-lyrics", "publish_desktop_lyrics",
        ));
        assert!(!super::window_command_allowed(
            "youtube-login", "get_desktop_lyrics_snapshot",
        ));
    }

    #[test]
    fn tray_popup_only_reads_its_state_and_sends_actions() {
        assert!(super::window_command_allowed("tray-popup", "get_tray_popup_state"));
        assert!(super::window_command_allowed("tray-popup", "tray_popup_ready"));
        assert!(super::window_command_allowed("tray-popup", "tray_popup_action"));
        assert!(!super::window_command_allowed("tray-popup", "publish_tray_snapshot"));
        assert!(!super::window_command_allowed("tray-popup", "quit_app"));
        assert!(!super::window_command_allowed("tray-popup", "play_url"));
        assert!(!super::window_command_allowed("desktop-lyrics", "tray_popup_action"));
    }

    #[test]
    fn unknown_duration_eof_ends_track() {
        assert_eq!(
            classify_playback_finish(true, true, 10_000, 0),
            PlaybackFinishState::Ended,
        );
    }

    #[test]
    fn known_duration_near_end_eof_ends_track() {
        assert_eq!(
            classify_playback_finish(true, true, 97_000, 100_000),
            PlaybackFinishState::Ended,
        );
    }

    #[test]
    fn known_duration_middle_eof_is_stalled() {
        assert_eq!(
            classify_playback_finish(true, true, 30_000, 100_000),
            PlaybackFinishState::Stalled,
        );
    }
}
