//! 系统托盘
//!
//! Windows：左键恢复主窗口，右键弹出自绘面板（tray-popup 窗口，跟随应用主题色与深浅色）。
//! macOS / Linux：原生菜单（Linux AppIndicator 收不到图标点击事件，只能挂菜单）。
//! 曲目、文案、主题由主窗口发布（publish_tray_snapshot）；播放/暂停由后台 ticker 同步，
//! 主窗口隐藏到托盘、页面停止渲染时也准确。
//!
//! 托盘与菜单句柄的方法会派发到主线程并等待执行完，调用时一律不持有 TRAY 锁，
//! 否则主线程同时处理托盘事件时会互相等待

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent, Wry,
};

use crate::error::{AppError, AppResult};
use crate::webview_args::MainBrowserArgs;

pub const POPUP_LABEL: &str = "tray-popup";
const TRAY_ID: &str = "main-tray";
/// 自绘面板只在 Windows 上用；其余平台走原生菜单
const USE_POPUP: bool = cfg!(target_os = "windows");
/// 面板窗口的逻辑尺寸，四周含 10px 透明边留给阴影，与 TrayPopupView 的布局对应
const POPUP_WIDTH: f64 = 300.0;
const POPUP_HEIGHT: f64 = 316.0;
/// 面板开着时再点托盘图标：失焦隐藏先于点击事件到达，这段时间内的点击不再重新打开
const REOPEN_GUARD: Duration = Duration::from_millis(300);
/// 主窗口没在这段时间内落盘完成就直接退出，避免页面卡死时退不掉
const QUIT_FLUSH_TIMEOUT: Duration = Duration::from_secs(3);
const TOOLTIP_MAX_CHARS: usize = 100;
const MENU_TRACK_MAX_CHARS: usize = 40;
const MAX_COVER_URL_BYTES: usize = 1_000_000;

const STATE_EVENT: &str = "tray-popup:state";
const OPEN_NOW_PLAYING_EVENT: &str = "tray:open-now-playing";
const TOGGLE_DESKTOP_LYRICS_EVENT: &str = "tray:toggle-desktop-lyrics";
const QUIT_EVENT: &str = "tray:quit";

static TRAY: Mutex<Option<TrayRuntime>> = Mutex::new(None);
static QUITTING: AtomicBool = AtomicBool::new(false);
/// 主窗口是否被收进托盘（WebView2 页面已设为不可见，重新显示时要恢复）
static MAIN_HIDDEN: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayTrack {
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub cover_url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayTheme {
    pub dark: bool,
    /// 主窗口当前生效的 --md-* 颜色（含动态取色结果）
    #[serde(default)]
    pub vars: HashMap<String, String>,
}

/// 主窗口发布的托盘内容
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraySnapshot {
    #[serde(default)]
    pub locale: String,
    /// 原生菜单与提示文字（已按当前语言翻译）
    #[serde(default)]
    pub texts: HashMap<String, String>,
    #[serde(default)]
    pub track: Option<TrayTrack>,
    #[serde(default)]
    pub theme: TrayTheme,
    #[serde(default)]
    pub desktop_lyrics_open: bool,
}

/// 发给面板窗口的完整状态
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayPopupState {
    locale: String,
    track: Option<TrayTrack>,
    theme: TrayTheme,
    is_playing: bool,
    desktop_lyrics_open: bool,
}

#[derive(Clone)]
struct NativeMenu {
    track: MenuItem<Wry>,
    previous: MenuItem<Wry>,
    toggle: MenuItem<Wry>,
    next: MenuItem<Wry>,
    show_main: MenuItem<Wry>,
    desktop_lyrics: CheckMenuItem<Wry>,
    quit: MenuItem<Wry>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct MenuLabels {
    track: String,
    has_track: bool,
    previous: String,
    toggle: String,
    next: String,
    show_main: String,
    desktop_lyrics: String,
    desktop_lyrics_open: bool,
    quit: String,
}

#[derive(Default)]
struct TrayRuntime {
    snapshot: TraySnapshot,
    is_playing: bool,
    menu: Option<NativeMenu>,
    /// 面板首次创建、页面还没就绪时记下的锚点，就绪后在这里弹出
    popup_anchor: Option<PhysicalPosition<f64>>,
    popup_hidden_at: Option<Instant>,
    sent_state: Option<TrayPopupState>,
    sent_labels: Option<MenuLabels>,
    sent_tooltip: Option<String>,
}

impl TrayRuntime {
    fn text(&self, key: &str, fallback: &str) -> String {
        self.snapshot
            .texts
            .get(key)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .unwrap_or(fallback)
            .to_string()
    }

    fn popup_state(&self) -> TrayPopupState {
        TrayPopupState {
            locale: self.snapshot.locale.clone(),
            track: self.snapshot.track.clone(),
            theme: self.snapshot.theme.clone(),
            is_playing: self.is_playing,
            desktop_lyrics_open: self.snapshot.desktop_lyrics_open,
        }
    }

    fn menu_labels(&self) -> MenuLabels {
        let track = self.snapshot.track.as_ref();
        MenuLabels {
            track: track
                .map(|track| truncate_chars(&track_line(track), MENU_TRACK_MAX_CHARS))
                .unwrap_or_else(|| self.text("idle", "未在播放")),
            has_track: track.is_some(),
            previous: self.text("previous", "上一首"),
            toggle: if self.is_playing {
                self.text("pause", "暂停")
            } else {
                self.text("play", "播放")
            },
            next: self.text("next", "下一首"),
            show_main: self.text("show_main", "显示主界面"),
            desktop_lyrics: self.text("desktop_lyrics", "桌面歌词"),
            desktop_lyrics_open: self.snapshot.desktop_lyrics_open,
            quit: self.text("quit", "退出 NeriPlayer"),
        }
    }

    fn tooltip(&self) -> String {
        match self.snapshot.track.as_ref() {
            Some(track) => truncate_chars(&track_line(track), TOOLTIP_MAX_CHARS),
            None => "NeriPlayer".into(),
        }
    }
}

fn track_line(track: &TrayTrack) -> String {
    let title = track.title.trim();
    let artist = track.artist.trim();
    if artist.is_empty() {
        title.to_string()
    } else {
        format!("{title} - {artist}")
    }
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(max.saturating_sub(1)).collect();
    truncated.push('…');
    truncated
}

fn sanitize(snapshot: &mut TraySnapshot) {
    snapshot.locale = truncate_chars(snapshot.locale.trim(), 16);
    snapshot
        .texts
        .retain(|key, value| key.len() <= 32 && value.chars().count() <= 80);
    snapshot.theme.vars.retain(|key, value| {
        key.starts_with("--md-") && key.len() <= 48 && value.len() <= 64 && !value.contains(';')
    });
    if let Some(track) = snapshot.track.as_mut() {
        track.title = truncate_chars(track.title.trim(), 200);
        track.artist = truncate_chars(track.artist.trim(), 200);
        if track.cover_url.len() > MAX_COVER_URL_BYTES {
            track.cover_url.clear();
        }
    }
}

/// 面板左上角：默认贴在光标左上方（任务栏在底部、右侧时），放不下就翻到另一侧，最后限制在工作区内
fn popup_origin(anchor: (i32, i32), size: (i32, i32), area: (i32, i32, i32, i32)) -> (i32, i32) {
    let (left, top, right, bottom) = area;
    let (width, height) = size;
    let mut x = anchor.0 - width;
    if x < left {
        x = anchor.0;
    }
    let mut y = anchor.1 - height;
    if y < top {
        y = anchor.1;
    }
    (
        x.clamp(left, (right - width).max(left)),
        y.clamp(top, (bottom - height).max(top)),
    )
}

/// 托盘收起主窗口时告诉 WebView2 页面不可见（同最小化）：否则看不见的页面仍按满帧率跑动画
fn set_page_visible(window: &WebviewWindow, visible: bool) {
    #[cfg(windows)]
    {
        let result = window.with_webview(move |webview| {
            // SAFETY: with_webview 的回调在 UI 线程上执行，控制器在窗口存活期间有效
            if let Err(error) = unsafe { webview.controller().SetIsVisible(visible) } {
                log::warn!(target: "tray", "WebView2 visibility not set to {visible}: {error}");
            }
        });
        if let Err(error) = result {
            log::warn!(target: "tray", "could not reach the webview to set visibility: {error}");
        }
    }
    #[cfg(not(windows))]
    let _ = (window, visible);
}

pub fn show_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    if MAIN_HIDDEN.swap(false, Ordering::AcqRel) {
        set_page_visible(&window, true);
    }
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

/// 关闭主窗口 = 收进托盘，播放继续
pub fn hide_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    let _ = window.hide();
    if !MAIN_HIDDEN.swap(true, Ordering::AcqRel) {
        set_page_visible(&window, false);
    }
}

/// 主窗口被其他途径（前端 show、系统切换）重新显示并获得焦点时，补上恢复页面渲染
pub fn main_window_focused(app: &AppHandle) {
    if !MAIN_HIDDEN.swap(false, Ordering::AcqRel) {
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        set_page_visible(&window, true);
    }
}

fn build_native_menu(app: &AppHandle) -> tauri::Result<(Menu<Wry>, NativeMenu)> {
    let items = NativeMenu {
        track: MenuItem::with_id(app, "now-playing", "未在播放", false, None::<&str>)?,
        previous: MenuItem::with_id(app, "previous", "上一首", true, None::<&str>)?,
        toggle: MenuItem::with_id(app, "toggle", "播放", true, None::<&str>)?,
        next: MenuItem::with_id(app, "next", "下一首", true, None::<&str>)?,
        show_main: MenuItem::with_id(app, "show-main", "显示主界面", true, None::<&str>)?,
        desktop_lyrics: CheckMenuItem::with_id(
            app,
            "desktop-lyrics",
            "桌面歌词",
            true,
            false,
            None::<&str>,
        )?,
        quit: MenuItem::with_id(app, "quit", "退出 NeriPlayer", true, None::<&str>)?,
    };
    let menu = Menu::with_items(
        app,
        &[
            &items.track,
            &PredefinedMenuItem::separator(app)?,
            &items.previous,
            &items.toggle,
            &items.next,
            &PredefinedMenuItem::separator(app)?,
            &items.show_main,
            &items.desktop_lyrics,
            &PredefinedMenuItem::separator(app)?,
            &items.quit,
        ],
    )?;
    Ok((menu, items))
}

pub fn setup(app: &tauri::App) -> tauri::Result<()> {
    let handle = app.handle().clone();
    let native = if USE_POPUP {
        None
    } else {
        Some(build_native_menu(&handle)?)
    };

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("NeriPlayer")
        // macOS 菜单栏图标惯例是单击出菜单；Windows 左键恢复主窗口
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_tray_icon_event(|tray, event| handle_icon_event(tray.app_handle(), event))
        .on_menu_event(|app, event| handle_action(app, event.id().as_ref()));
    if let Some((menu, _)) = native.as_ref() {
        builder = builder.menu(menu);
    }
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;

    *TRAY.lock() = Some(TrayRuntime {
        menu: native.map(|(_, items)| items),
        ..Default::default()
    });
    Ok(())
}

fn handle_icon_event(app: &AppHandle, event: TrayIconEvent) {
    let TrayIconEvent::Click {
        button,
        button_state: MouseButtonState::Up,
        position,
        ..
    } = event
    else {
        return;
    };
    match button {
        MouseButton::Left if !cfg!(target_os = "macos") => {
            hide_popup(app);
            show_main_window(app);
        }
        MouseButton::Right if USE_POPUP => toggle_popup(app, position),
        _ => {}
    }
}

/// 托盘菜单、面板共用的动作分发
fn handle_action(app: &AppHandle, action: &str) {
    match action {
        "previous" => {
            let _ = app.emit_to("main", "media:previous", ());
        }
        "toggle" => {
            let _ = app.emit_to("main", "media:toggle", ());
        }
        "next" => {
            let _ = app.emit_to("main", "media:next", ());
        }
        "now-playing" => {
            show_main_window(app);
            let _ = app.emit_to("main", OPEN_NOW_PLAYING_EVENT, ());
        }
        "show-main" => show_main_window(app),
        "desktop-lyrics" => {
            let _ = app.emit_to("main", TOGGLE_DESKTOP_LYRICS_EVENT, ());
        }
        "quit" => request_quit(app),
        _ => log::warn!(target: "tray", "unknown tray action: {action}"),
    }
}

/// 退出前让主窗口把播放状态、统计落盘，主窗口完成后调用 quit_app；超时则直接退出
pub fn request_quit(app: &AppHandle) {
    if QUITTING.swap(true, Ordering::AcqRel) {
        return;
    }
    hide_popup(app);
    if app.get_webview_window("main").is_none() || app.emit_to("main", QUIT_EVENT, ()).is_err() {
        app.exit(0);
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(QUIT_FLUSH_TIMEOUT);
        log::warn!(target: "tray", "main window did not finish flushing in time, exiting anyway");
        app.exit(0);
    });
}

fn hide_popup(app: &AppHandle) {
    let Some(window) = app.get_webview_window(POPUP_LABEL) else { return };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        if let Some(runtime) = TRAY.lock().as_mut() {
            runtime.popup_hidden_at = Some(Instant::now());
        }
    }
}

fn toggle_popup(app: &AppHandle, anchor: PhysicalPosition<f64>) {
    let Some(window) = app.get_webview_window(POPUP_LABEL) else {
        create_popup(app, anchor);
        return;
    };
    if window.is_visible().unwrap_or(false) {
        hide_popup(app);
        return;
    }
    let just_hidden = TRAY
        .lock()
        .as_ref()
        .and_then(|runtime| runtime.popup_hidden_at)
        .is_some_and(|at| at.elapsed() < REOPEN_GUARD);
    if !just_hidden {
        show_popup(app, &window, anchor);
    }
}

fn show_popup(app: &AppHandle, window: &WebviewWindow, anchor: PhysicalPosition<f64>) {
    let monitor = app
        .monitor_from_point(anchor.x, anchor.y)
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten());
    if let Some(monitor) = monitor {
        let scale = monitor.scale_factor().max(0.1);
        let area = monitor.work_area();
        let size = (
            (POPUP_WIDTH * scale).round() as i32,
            (POPUP_HEIGHT * scale).round() as i32,
        );
        let (x, y) = popup_origin(
            (anchor.x.round() as i32, anchor.y.round() as i32),
            size,
            (
                area.position.x,
                area.position.y,
                area.position.x + area.size.width as i32,
                area.position.y + area.size.height as i32,
            ),
        );
        // 先移到目标屏幕再定尺寸：跨 DPI 屏幕移动时系统会按新缩放重设一次尺寸
        let _ = window.set_position(PhysicalPosition::new(x, y));
        let _ = window.set_size(PhysicalSize::new(size.0 as u32, size.1 as u32));
    }
    push_state(app, true);
    let _ = window.show();
    let _ = window.set_focus();
}

fn create_popup(app: &AppHandle, anchor: PhysicalPosition<f64>) {
    {
        let mut guard = TRAY.lock();
        let Some(runtime) = guard.as_mut() else { return };
        // 页面就绪前再点一次只更新锚点，不重复建窗
        let creating = runtime.popup_anchor.replace(anchor).is_some();
        if creating {
            return;
        }
    }
    let app = app.clone();
    // WebView2 窗口在事件回调里同步创建会卡住主线程，放到异步运行时里建
    tauri::async_runtime::spawn(async move {
        let builder = WebviewWindowBuilder::new(
            &app,
            POPUP_LABEL,
            WebviewUrl::App("index.html?window=tray-popup".into()),
        );
        // macOS 透明 WebView 需要额外私有 API feature；面板目前只在 Windows 上使用
        #[cfg(not(target_os = "macos"))]
        let builder = builder.transparent(true);
        let built = builder
            .title("NeriPlayer")
            .inner_size(POPUP_WIDTH, POPUP_HEIGHT)
            .decorations(false)
            .shadow(false)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .main_browser_args(&app)
            .build();
        let window = match built {
            Ok(window) => window,
            Err(error) => {
                log::warn!(target: "tray", "tray popup window not created: {error}");
                if let Some(runtime) = TRAY.lock().as_mut() {
                    runtime.popup_anchor = None;
                }
                return;
            }
        };
        let event_app = app.clone();
        window.on_window_event(move |event| match event {
            WindowEvent::Focused(false) => hide_popup(&event_app),
            WindowEvent::Destroyed => {
                if let Some(runtime) = TRAY.lock().as_mut() {
                    runtime.popup_anchor = None;
                    runtime.sent_state = None;
                }
            }
            _ => {}
        });
    });
}

/// 把最新状态推给面板窗口；force 时不管有没有变化都推（面板弹出前）
fn push_state(app: &AppHandle, force: bool) {
    let state = {
        let mut guard = TRAY.lock();
        let Some(runtime) = guard.as_mut() else { return };
        let state = runtime.popup_state();
        if !force && runtime.sent_state.as_ref() == Some(&state) {
            return;
        }
        runtime.sent_state = Some(state.clone());
        state
    };
    if app.get_webview_window(POPUP_LABEL).is_some() {
        let _ = app.emit_to(POPUP_LABEL, STATE_EVENT, state);
    }
}

/// 状态变化后同步面板、原生菜单与托盘提示
fn refresh(app: &AppHandle) {
    push_state(app, false);
    let (menu, labels, tooltip) = {
        let mut guard = TRAY.lock();
        let Some(runtime) = guard.as_mut() else { return };
        let labels = runtime.menu.as_ref().map(|_| runtime.menu_labels());
        let labels = labels.filter(|labels| runtime.sent_labels.as_ref() != Some(labels));
        if let Some(labels) = labels.as_ref() {
            runtime.sent_labels = Some(labels.clone());
        }
        let tooltip = Some(runtime.tooltip())
            .filter(|tooltip| runtime.sent_tooltip.as_ref() != Some(tooltip));
        if let Some(tooltip) = tooltip.as_ref() {
            runtime.sent_tooltip = Some(tooltip.clone());
        }
        (runtime.menu.clone(), labels, tooltip)
    };
    if let (Some(menu), Some(labels)) = (menu, labels) {
        let _ = menu.track.set_text(&labels.track);
        let _ = menu.track.set_enabled(labels.has_track);
        let _ = menu.previous.set_text(&labels.previous);
        let _ = menu.toggle.set_text(&labels.toggle);
        let _ = menu.next.set_text(&labels.next);
        let _ = menu.show_main.set_text(&labels.show_main);
        let _ = menu.desktop_lyrics.set_text(&labels.desktop_lyrics);
        let _ = menu.desktop_lyrics.set_checked(labels.desktop_lyrics_open);
        let _ = menu.quit.set_text(&labels.quit);
    }
    if let Some(tooltip) = tooltip {
        if let Some(tray) = app.tray_by_id(TRAY_ID) {
            let _ = tray.set_tooltip(Some(tooltip));
        }
    }
}

/// 后台 ticker 每 200ms 调用；只在播放状态变化时才真正刷新
pub fn sync_playing(app: &AppHandle, is_playing: bool) {
    {
        let mut guard = TRAY.lock();
        let Some(runtime) = guard.as_mut() else { return };
        if runtime.is_playing == is_playing {
            return;
        }
        runtime.is_playing = is_playing;
    }
    refresh(app);
}

fn require_window(window: &WebviewWindow, label: &str) -> AppResult<()> {
    if window.label() != label {
        return Err(AppError::Other(format!(
            "Tray command is not available to window '{}'",
            window.label()
        )));
    }
    Ok(())
}

/// 主窗口发布托盘内容：曲目、文案、主题色、桌面歌词开关
#[tauri::command]
pub fn publish_tray_snapshot(
    app: AppHandle,
    window: WebviewWindow,
    mut snapshot: TraySnapshot,
) -> AppResult<()> {
    require_window(&window, "main")?;
    sanitize(&mut snapshot);
    {
        let mut guard = TRAY.lock();
        let Some(runtime) = guard.as_mut() else { return Ok(()) };
        runtime.snapshot = snapshot;
    }
    refresh(&app);
    Ok(())
}

#[tauri::command]
pub fn get_tray_popup_state(window: WebviewWindow) -> AppResult<TrayPopupState> {
    require_window(&window, POPUP_LABEL)?;
    Ok(TRAY
        .lock()
        .as_ref()
        .map(TrayRuntime::popup_state)
        .unwrap_or_default())
}

/// 面板页面首次加载完成：在记下的锚点处弹出
#[tauri::command]
pub fn tray_popup_ready(app: AppHandle, window: WebviewWindow) -> AppResult<()> {
    require_window(&window, POPUP_LABEL)?;
    let anchor = TRAY
        .lock()
        .as_mut()
        .and_then(|runtime| runtime.popup_anchor.take());
    if let Some(anchor) = anchor {
        show_popup(&app, &window, anchor);
    }
    Ok(())
}

#[tauri::command]
pub fn tray_popup_action(app: AppHandle, window: WebviewWindow, action: String) -> AppResult<()> {
    require_window(&window, POPUP_LABEL)?;
    match action.as_str() {
        // 切歌、播放暂停后面板保持打开，方便连续操作
        "previous" | "toggle" | "next" => {}
        _ => hide_popup(&app),
    }
    if action != "dismiss" {
        handle_action(&app, &action);
    }
    Ok(())
}

/// 主窗口落盘完成后真正退出
#[tauri::command]
pub fn quit_app(app: AppHandle, window: WebviewWindow) -> AppResult<()> {
    require_window(&window, "main")?;
    QUITTING.store(true, Ordering::Release);
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_with(track: Option<TrayTrack>, is_playing: bool) -> TrayRuntime {
        TrayRuntime {
            snapshot: TraySnapshot {
                locale: "en".into(),
                texts: [
                    ("play".to_string(), "Play".to_string()),
                    ("pause".to_string(), "Pause".to_string()),
                    ("idle".to_string(), "Not playing".to_string()),
                ]
                .into_iter()
                .collect(),
                track,
                ..Default::default()
            },
            is_playing,
            ..Default::default()
        }
    }

    #[test]
    fn menu_labels_follow_track_and_playback() {
        let idle = runtime_with(None, false).menu_labels();
        assert_eq!(idle.track, "Not playing");
        assert!(!idle.has_track);
        assert_eq!(idle.toggle, "Play");
        // 未下发的文案回退到内置中文
        assert_eq!(idle.next, "下一首");

        let playing = runtime_with(
            Some(TrayTrack {
                title: "Song".into(),
                artist: "Artist".into(),
                cover_url: String::new(),
            }),
            true,
        )
        .menu_labels();
        assert_eq!(playing.track, "Song - Artist");
        assert!(playing.has_track);
        assert_eq!(playing.toggle, "Pause");
    }

    #[test]
    fn tooltip_shows_track_or_app_name() {
        assert_eq!(runtime_with(None, false).tooltip(), "NeriPlayer");
        let runtime = runtime_with(
            Some(TrayTrack {
                title: "Song".into(),
                artist: String::new(),
                cover_url: String::new(),
            }),
            false,
        );
        assert_eq!(runtime.tooltip(), "Song");
    }

    #[test]
    fn long_text_is_truncated_with_ellipsis() {
        let long = "很".repeat(50);
        let truncated = truncate_chars(&long, MENU_TRACK_MAX_CHARS);
        assert_eq!(truncated.chars().count(), MENU_TRACK_MAX_CHARS);
        assert!(truncated.ends_with('…'));
        assert_eq!(truncate_chars("short", 10), "short");
    }

    #[test]
    fn sanitize_drops_unexpected_theme_vars_and_huge_covers() {
        let mut snapshot = TraySnapshot {
            track: Some(TrayTrack {
                title: "t".into(),
                artist: "a".into(),
                cover_url: "x".repeat(MAX_COVER_URL_BYTES + 1),
            }),
            theme: TrayTheme {
                dark: true,
                vars: [
                    ("--md-primary".to_string(), "rgb(1, 2, 3)".to_string()),
                    ("color".to_string(), "red".to_string()),
                    ("--md-on-primary".to_string(), "red; background: url(x)".to_string()),
                ]
                .into_iter()
                .collect(),
            },
            ..Default::default()
        };
        sanitize(&mut snapshot);
        assert_eq!(snapshot.theme.vars.len(), 1);
        assert!(snapshot.theme.vars.contains_key("--md-primary"));
        assert!(snapshot.track.unwrap().cover_url.is_empty());
    }

    #[test]
    fn popup_opens_above_left_of_cursor_on_bottom_taskbar() {
        // 1920x1040 工作区，光标在右下角托盘区
        let origin = popup_origin((1800, 1060), (300, 316), (0, 0, 1920, 1040));
        assert_eq!(origin, (1500, 724));
    }

    #[test]
    fn popup_flips_when_taskbar_is_on_top_or_left() {
        // 任务栏在顶部：工作区从 y=40 开始，面板翻到光标下方
        assert_eq!(
            popup_origin((1800, 20), (300, 316), (0, 40, 1920, 1080)),
            (1500, 40)
        );
        // 任务栏在左侧：面板翻到光标右侧
        assert_eq!(
            popup_origin((20, 1000), (300, 316), (60, 0, 1920, 1080)),
            (60, 684)
        );
    }
}
