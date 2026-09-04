#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use neri_player_desktop::audio::analyzer::SharedAudioLevel;
use neri_player_desktop::audio::media_session::{MediaAction, MediaSessionController};
use neri_player_desktop::auth;
use neri_player_desktop::commands::{
    auth_cmd, download_cmd, image_cmd, library_cmd, listen_together_cmd, lyrics_cmd, player_cmd,
    recommend_cmd, search_cmd, settings_cmd, stats_cmd, storage_cmd, sync_cmd, tray_cmd,
    debug_cmd,};
use neri_player_desktop::state::AppState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
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

    // 托盘「退出」标记：窗口关闭只隐藏到托盘，仅托盘退出允许结束进程
    let quitting = Arc::new(AtomicBool::new(false));
    let quitting_tray = quitting.clone();

    tauri::Builder::default()
        // 单实例保护必须最先注册：双实例会共享 deviceId 与 causal counter，
        // 重复发号 token 造成跨设备同步静默数据损坏，playlists/stats 等
        // 落盘文件也会互相覆盖。二次启动改为聚焦已有窗口
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.unminimize();
                let _ = win.show();
                let _ = win.set_focus();
            }
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

            // macOS 使用原生红绿灯（Overlay 标题栏）；Windows/Linux 移除原生装饰，
            // 由前端 TitleBar.vue 自绘窗口控制。配置里 decorations 默认为 true 以
            // 启用 macOS 的 titleBarStyle Overlay，其余平台在此运行时关闭。
            #[cfg(not(target_os = "macos"))]
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_decorations(false);
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

            // 系统托盘：窗口关闭后隐藏到托盘继续播放；左键单击恢复主窗口
            // 播放控制复用 media:* 事件（与系统媒体键同一套前端处理链路）
            // 「正在播放」菜单项由后台 ticker 在曲目切换时更新标题
            let tray_now_playing_item = MenuItem::with_id(
                app, "tray-now-playing", "正在播放", true, None::<&str>,
            )?;
            {
                let quit_flag = quitting_tray.clone();
                fn show_main_window(app: &tauri::AppHandle) {
                    if let Some(win) = app.get_webview_window("main") {
                        let _ = win.unminimize();
                        let _ = win.show();
                        let _ = win.set_focus();
                    }
                }

                let tray_menu = Menu::new(app)?;
                let tray_prev_item =
                    MenuItem::with_id(app, "tray-prev", "上一首", true, None::<&str>)?;
                let tray_toggle_item =
                    MenuItem::with_id(app, "tray-toggle", "暂停/播放", true, None::<&str>)?;
                let tray_next_item =
                    MenuItem::with_id(app, "tray-next", "下一首", true, None::<&str>)?;
                let tray_home_item =
                    MenuItem::with_id(app, "tray-home", "打开主页面", true, None::<&str>)?;
                let tray_quit_item =
                    MenuItem::with_id(app, "tray-quit", "退出", true, None::<&str>)?;
                tray_menu.append(&tray_prev_item)?;
                tray_menu.append(&tray_toggle_item)?;
                tray_menu.append(&tray_next_item)?;
                tray_menu.append(&PredefinedMenuItem::separator(app)?)?;
                tray_menu.append(&tray_now_playing_item)?;
                tray_menu.append(&tray_home_item)?;
                tray_menu.append(&PredefinedMenuItem::separator(app)?)?;
                tray_menu.append(&tray_quit_item)?;

                // 注册句柄供 set_tray_texts / update_now_playing 使用（多语言 + 曲目名）
                tray_cmd::register_tray_handles(app.handle(), tray_cmd::TrayMenuHandles {
                        prev: tray_prev_item,
                        toggle: tray_toggle_item,
                        next: tray_next_item,
                        now_playing: tray_now_playing_item,
                        home: tray_home_item,
                        quit: tray_quit_item,
                    },
                );

                let mut builder = TrayIconBuilder::with_id("main-tray")
                    .menu(&tray_menu)
                    .tooltip("NeriPlayer")
                    // Windows 上左键直接恢复窗口、右键弹菜单
                    .show_menu_on_left_click(false)
                    .on_menu_event(move |app, event| match event.id().as_ref() {
                        "tray-prev" => {
                            let _ = app.emit("media:previous", ());
                        }
                        "tray-toggle" => {
                            let _ = app.emit("media:toggle", ());
                        }
                        "tray-next" => {
                            let _ = app.emit("media:next", ());
                        }
                        "tray-now-playing" => {
                            show_main_window(app);
                            let _ = app.emit("tray:open-now-playing", ());
                        }
                        "tray-home" => {
                            show_main_window(app);
                            let _ = app.emit("tray:open-home", ());
                        }
                        "tray-quit" => {
                            quit_flag.store(true, Ordering::Relaxed);
                            app.exit(0);
                        }
                        _ => {}
                    })
                    .on_tray_icon_event(move |tray, event| {
                        // 左键单击恢复主窗口
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            show_main_window(tray.app_handle());
                        }
                    });

                // 托盘图标默认取应用的 default_window_icon（bundle.icon 嵌入）
                if let Some(icon) = app.default_window_icon() {
                    builder = builder.icon(icon.clone());
                }
                builder.build(app)?;
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

                    // 媒体会话同步（每 1s = 每 5 个 tick）与托盘曲目名同步。
                    // 托盘更新不依赖 media_session：MPRIS/SMTC 不可用时曲目名也要更新
                    let current_meta = state.media_metadata.lock().clone();
                    if let Some(meta) = current_meta {
                        if !meta.id.is_empty() && meta.id != last_media_track_id {
                            last_media_track_id = meta.id.clone();
                            // 托盘菜单同步当前曲目名（仅在曲目切换时更新）
                            tray_cmd::update_now_playing(&meta.title);
                            if let Some(ref ms) = media_session {
                                ms.update_metadata(
                                    &meta.title,
                                    &meta.artist,
                                    &meta.album,
                                    meta.cover_url.as_deref(),
                                    meta.duration_ms,
                                );
                            }
                        }
                    } else if !last_media_track_id.is_empty() {
                        last_media_track_id.clear();
                        tray_cmd::update_now_playing("");
                    }

                    if let Some(ref ms) = media_session {
                        media_update_counter += 1;
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
            let app_handler: Box<
                dyn Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static,
            > = Box::new(tauri::generate_handler![
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
            player_cmd::set_normalize_volume,
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
            search_cmd::search,
            image_cmd::fetch_bilibili_cover,
            lyrics_cmd::parse_lrc_content,
            lyrics_cmd::load_lyrics_file,
            lyrics_cmd::fetch_lyrics,
            settings_cmd::get_settings,
            settings_cmd::save_settings,
            settings_cmd::get_app_data_dir,
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
            stats_cmd::record_playback_session,
            stats_cmd::record_playback_sessions,
            stats_cmd::get_playback_stats,
            stats_cmd::get_playback_stats_overview,
            stats_cmd::clear_playback_stats,
            stats_cmd::remove_playback_stats,
            stats_cmd::playback_stats_identity_key,
            tray_cmd::set_tray_texts,
            ]);
            move |invoke: tauri::ipc::Invoke<tauri::Wry>| {
                let label = invoke.message.webview().label().to_string();
                if label != "main" {
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
        .run({
            let quitting = quitting.clone();
            move |app_handle, event| match event {
                tauri::RunEvent::ExitRequested { api, .. } => {
                    if !quitting.load(Ordering::Relaxed) && cfg!(not(target_os = "macos")) {
                        // 窗口关闭只是隐藏到托盘；仅托盘「退出」允许结束进程。
                        // macOS 保留原生 Cmd+Q 语义（关窗本就不退出）
                        api.prevent_exit();
                        for win in app_handle.webview_windows().values() {
                            let _ = win.hide();
                        }
                    } else {
                        // 退出前 flush 一次轮转 Cookie：60s 定时器之外，退出前最后一窗口的
                        // Set-Cookie 轮换令牌若不落盘，下次启动会重放旧令牌导致偶发掉登录（AU-05）
                        let state = app_handle.state::<AppState>();
                        auth_cmd::persist_rotated_cookies(app_handle, state.inner());
                    }
                }
                tauri::RunEvent::WindowEvent {
                    label,
                    event: WindowEvent::CloseRequested { api, .. },
                    ..
                } => {
                    // 关闭 = 隐藏到托盘。prevent_close 在 GTK 层真正取消
                    // delete-event，不会重发；前端若在 JS 侧 hide()/close()
                    // 回退会与平台关闭状态互扰，在 WebKitGTK 下造成
                    // CloseRequested 死循环（flush 刷屏）并拖垮 GPU 上下文。
                    // 已隐藏时忽略重复请求，避免重发循环
                    api.prevent_close();
                    if let Some(win) = app_handle.get_webview_window(&label) {
                        if win.is_visible().unwrap_or(false) {
                            let _ = win.hide();
                        }
                    }
                }
                _ => {}
            }
        });
}

#[cfg(test)]
mod tests {
    use super::{classify_playback_finish, PlaybackFinishState};

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
