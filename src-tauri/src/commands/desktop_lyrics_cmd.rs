use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::error::{AppError, AppResult};
use crate::webview_args::MainBrowserArgs;

pub const WINDOW_LABEL: &str = "desktop-lyrics";
const FRAME_EVENT: &str = "desktop-lyrics:frame";
const CLOSED_EVENT: &str = "desktop-lyrics:closed";
const ACTION_EVENT: &str = "desktop-lyrics:action";
const BOUNDS_EVENT: &str = "desktop-lyrics:bounds";
const HOVER_EVENT: &str = "desktop-lyrics:hover";
const LOCK_EVENT: &str = "desktop-lyrics:lock";

const MAX_FRAME_LINES: usize = 6;
/// 5 行 × 128 字时帧仍远小于 MAX_FRAME_BYTES；逐字歌词一行通常几十个字
const MAX_LINE_WORDS: usize = 128;
const MAX_TEXT_BYTES: usize = 1024;
const MAX_WORD_BYTES: usize = 64;
const MAX_STYLE_BYTES: usize = 8 * 1024;
const MAX_FRAME_BYTES: usize = 128 * 1024;
/// 逻辑像素，与前端 DESKTOP_LYRICS_WINDOW 一致
const MIN_WIDTH: f64 = 360.0;
const MIN_HEIGHT: f64 = 110.0;
const MAX_WIDTH: f64 = 4096.0;
const MAX_HEIGHT: f64 = 1200.0;
const DEFAULT_WIDTH: f64 = 860.0;
const DEFAULT_HEIGHT: f64 = 180.0;
/// 默认位置离屏幕工作区底边的距离
const DEFAULT_BOTTOM_MARGIN: f64 = 72.0;
/// 锁定时检测光标的间隔
const LOCK_POLL: Duration = Duration::from_millis(80);
/// 拖动、缩放停下这么久后才把位置告诉主窗口保存
const BOUNDS_SETTLE: Duration = Duration::from_millis(400);

/// 歌词窗口工具栏能发给主窗口的操作
const ACTIONS: &[&str] = &[
    "toggle-play",
    "previous",
    "next",
    "font-smaller",
    "font-larger",
    "cycle-layout",
    "lock",
    "unlock",
    "open-settings",
];

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopLyricsWord {
    start_ms: f64,
    duration_ms: f64,
    text: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopLyricsLine {
    start_ms: f64,
    duration_ms: f64,
    text: String,
    translation: String,
    roman: String,
    words: Vec<DesktopLyricsWord>,
}

/// 当前行附近的几行 + 播放时钟锚点 + 外观；歌词窗口按锚点本地插值
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopLyricsFrame {
    track_id: String,
    title: String,
    artist: String,
    is_playing: bool,
    position_ms: f64,
    anchor_at: f64,
    rate: f64,
    offset_ms: f64,
    first_index: u32,
    lines: Vec<DesktopLyricsLine>,
    accent: String,
    /// 外观由前端规整，这里只限制大小和类型；歌词窗口收到后会再规整一遍
    style: serde_json::Value,
}

fn invalid(message: &str) -> AppError {
    AppError::Other(format!("Desktop lyrics frame {message}"))
}

impl DesktopLyricsFrame {
    fn validate(&self) -> AppResult<()> {
        for (value, maximum) in [
            (&self.track_id, 512),
            (&self.title, MAX_TEXT_BYTES),
            (&self.artist, MAX_TEXT_BYTES),
        ] {
            if value.len() > maximum {
                return Err(invalid("exceeds text limit"));
            }
        }
        if !(self.accent.is_empty()
            || (self.accent.len() == 7
                && self.accent.starts_with('#')
                && self.accent[1..].chars().all(|character| character.is_ascii_hexdigit())))
        {
            return Err(invalid("has an invalid accent color"));
        }
        if !(self.rate > 0.0 && self.rate <= 8.0) {
            return Err(invalid("has an invalid playback rate"));
        }
        if self.lines.len() > MAX_FRAME_LINES {
            return Err(invalid("has too many lines"));
        }
        for line in &self.lines {
            if [&line.text, &line.translation, &line.roman]
                .iter()
                .any(|text| text.len() > MAX_TEXT_BYTES)
                || line.words.len() > MAX_LINE_WORDS
                || line.words.iter().any(|word| word.text.len() > MAX_WORD_BYTES)
            {
                return Err(invalid("exceeds text limit"));
            }
            let times = std::iter::once((line.start_ms, line.duration_ms))
                .chain(line.words.iter().map(|word| (word.start_ms, word.duration_ms)));
            if times.into_iter().any(|(start, duration)| !(0.0..=1e9).contains(&start) || !(0.0..=1e9).contains(&duration)) {
                return Err(invalid("has an invalid time"));
            }
        }
        if !(self.style.is_object() || self.style.is_null())
            || serde_json::to_vec(&self.style)?.len() > MAX_STYLE_BYTES
        {
            return Err(invalid("has an invalid style"));
        }
        if serde_json::to_vec(self)?.len() > MAX_FRAME_BYTES {
            return Err(invalid("exceeds size limit"));
        }
        Ok(())
    }
}

/// 窗口位置和大小，逻辑像素
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogicalBounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// 显示器工作区（去掉任务栏），逻辑像素
#[derive(Clone, Copy, Debug, PartialEq)]
struct WorkArea {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// 锁定时只有这块区域（解锁按钮）可点，CSS 像素，相对窗口左上角
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HitRegion {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl HitRegion {
    fn valid(&self) -> bool {
        [self.x, self.y, self.width, self.height].iter().all(|value| value.is_finite() && value.abs() <= 10_000.0)
            && self.width >= 0.0
            && self.height >= 0.0
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x <= self.x + self.width && y <= self.y + self.height
    }
}

#[derive(Default)]
struct LockState {
    locked: bool,
    /// 每次加锁加一，旧的轮询任务据此退出
    generation: u64,
    hit_region: Option<HitRegion>,
}

#[derive(Default)]
struct DesktopLyricsState {
    session_id: String,
    window_instance: String,
    frame: DesktopLyricsFrame,
    lock: LockState,
}

impl DesktopLyricsState {
    fn adopt(&mut self, session_id: String) {
        if self.session_id != session_id {
            self.frame = DesktopLyricsFrame::default();
            self.session_id = session_id;
        }
    }

    fn owns(&self, session_id: &str) -> bool {
        !self.window_instance.is_empty() && self.session_id == session_id
    }

    fn retire(&mut self, instance: &str) -> Option<String> {
        if self.window_instance != instance {
            return None;
        }
        let session = self.session_id.clone();
        let generation = self.lock.generation + 1;
        *self = Self::default();
        self.lock.generation = generation;
        Some(session)
    }
}

fn snapshot() -> &'static Mutex<DesktopLyricsState> {
    static SNAPSHOT: OnceLock<Mutex<DesktopLyricsState>> = OnceLock::new();
    SNAPSHOT.get_or_init(|| Mutex::new(DesktopLyricsState::default()))
}

fn window_operation() -> &'static Mutex<()> {
    static OPERATION: OnceLock<Mutex<()>> = OnceLock::new();
    OPERATION.get_or_init(|| Mutex::new(()))
}

fn bounds_generation() -> &'static AtomicU64 {
    static GENERATION: AtomicU64 = AtomicU64::new(0);
    &GENERATION
}

fn validate_session(session_id: &str) -> AppResult<()> {
    uuid::Uuid::parse_str(session_id)
        .map(|_| ())
        .map_err(|_| AppError::Other("Invalid desktop lyrics window session".into()))
}

fn require_main(label: &str) -> AppResult<()> {
    if label != "main" {
        return Err(AppError::Other(
            "Desktop lyrics can only be controlled by the main window".into(),
        ));
    }
    Ok(())
}

fn require_lyrics(label: &str) -> AppResult<()> {
    if label != WINDOW_LABEL {
        return Err(AppError::Other(
            "Only the desktop lyrics window can send this".into(),
        ));
    }
    Ok(())
}

fn require_reader(label: &str) -> AppResult<()> {
    if label != "main" && label != WINDOW_LABEL {
        return Err(AppError::Other(
            "Desktop lyrics snapshot is not available to this window".into(),
        ));
    }
    Ok(())
}

fn clamp_size(bounds: LogicalBounds) -> LogicalBounds {
    LogicalBounds {
        width: bounds.width.clamp(MIN_WIDTH, MAX_WIDTH),
        height: bounds.height.clamp(MIN_HEIGHT, MAX_HEIGHT),
        ..bounds
    }
}

fn overlap(bounds: LogicalBounds, area: WorkArea) -> f64 {
    let width = (bounds.x + bounds.width).min(area.x + area.width) - bounds.x.max(area.x);
    let height = (bounds.y + bounds.height).min(area.y + area.height) - bounds.y.max(area.y);
    width.max(0.0) * height.max(0.0)
}

/// 打开时放在哪：上次的位置还有三分之一以上落在某块屏幕的工作区内就沿用（拔掉显示器后
/// 不会跑到屏幕外），否则放在主屏幕底部居中
fn placement(saved: Option<LogicalBounds>, areas: &[WorkArea], primary: Option<WorkArea>) -> LogicalBounds {
    if let Some(saved) = saved.filter(|bounds| {
        [bounds.x, bounds.y, bounds.width, bounds.height].iter().all(|value| value.is_finite())
    }) {
        let saved = clamp_size(saved);
        let visible = areas.iter().map(|area| overlap(saved, *area)).fold(0.0, f64::max);
        if visible >= saved.width * saved.height / 3.0 {
            return saved;
        }
    }
    let area = primary.or_else(|| areas.first().copied()).unwrap_or(WorkArea {
        x: 0.0,
        y: 0.0,
        width: 1280.0,
        height: 720.0,
    });
    let width = DEFAULT_WIDTH.min(area.width * 0.8).max(MIN_WIDTH);
    let height = DEFAULT_HEIGHT;
    LogicalBounds {
        x: (area.x + (area.width - width) / 2.0).round(),
        y: (area.y + area.height - height - DEFAULT_BOTTOM_MARGIN).max(area.y).round(),
        width,
        height,
    }
}

fn work_area(monitor: &tauri::Monitor) -> WorkArea {
    let scale = monitor.scale_factor().max(0.1);
    let area = monitor.work_area();
    WorkArea {
        x: f64::from(area.position.x) / scale,
        y: f64::from(area.position.y) / scale,
        width: f64::from(area.size.width) / scale,
        height: f64::from(area.size.height) / scale,
    }
}

fn window_bounds(window: &WebviewWindow) -> Option<LogicalBounds> {
    let scale = window.scale_factor().ok()?;
    let position = window.outer_position().ok()?.to_logical::<f64>(scale);
    let size = window.outer_size().ok()?.to_logical::<f64>(scale);
    Some(LogicalBounds {
        x: position.x.round(),
        y: position.y.round(),
        width: size.width.round(),
        height: size.height.round(),
    })
}

/// 移动、缩放停下后把位置发给主窗口保存
fn report_bounds_when_settled(app: &AppHandle) {
    let generation = bounds_generation().fetch_add(1, Ordering::AcqRel) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(BOUNDS_SETTLE).await;
        if bounds_generation().load(Ordering::Acquire) != generation {
            return;
        }
        let Some(window) = app.get_webview_window(WINDOW_LABEL) else { return };
        if window.is_minimized().unwrap_or(false) {
            return;
        }
        if let Some(bounds) = window_bounds(&window) {
            let _ = app.emit_to("main", BOUNDS_EVENT, bounds);
        }
    });
}

#[tauri::command]
pub async fn open_desktop_lyrics(
    app: AppHandle,
    window: WebviewWindow,
    session_id: String,
    bounds: Option<LogicalBounds>,
) -> AppResult<()> {
    require_main(window.label())?;
    validate_session(&session_id)?;
    let _operation = window_operation().lock();
    if app.get_webview_window("main").is_none() {
        return Err(AppError::Other("Main window is no longer available".into()));
    }
    if let Some(existing) = app.get_webview_window(WINDOW_LABEL) {
        snapshot().lock().adopt(session_id);
        return existing
            .show()
            .map_err(|error| AppError::Other(error.to_string()));
    }
    let areas = app
        .available_monitors()
        .map(|monitors| monitors.iter().map(work_area).collect::<Vec<_>>())
        .unwrap_or_default();
    let primary = app.primary_monitor().ok().flatten().map(|monitor| work_area(&monitor));
    let place = placement(bounds, &areas, primary);
    let instance = uuid::Uuid::new_v4().to_string();
    {
        let mut state = snapshot().lock();
        state.adopt(session_id);
        state.window_instance = instance.clone();
    }
    let builder = WebviewWindowBuilder::new(
        &app,
        WINDOW_LABEL,
        WebviewUrl::App("index.html?window=desktop-lyrics".into()),
    )
    .title("NeriPlayer")
    .inner_size(place.width, place.height)
    .position(place.x, place.y)
    .min_inner_size(MIN_WIDTH, MIN_HEIGHT)
    .max_inner_size(MAX_WIDTH, MAX_HEIGHT)
    .decorations(false)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .visible(false);
    // macOS 透明 WebView 需要额外私有 API feature，保留普通背景兼容默认构建
    #[cfg(target_os = "windows")]
    let builder = builder.transparent(true);
    let lyrics_window = builder.main_browser_args(&app).build().map_err(|error| {
        snapshot().lock().retire(&instance);
        AppError::Other(error.to_string())
    })?;
    let event_app = app.clone();
    lyrics_window.on_window_event(move |event| match event {
        WindowEvent::Destroyed => {
            let session = snapshot().lock().retire(&instance);
            if let Some(session_id) = session {
                let _ = event_app.emit_to(
                    "main",
                    CLOSED_EVENT,
                    serde_json::json!({"sessionId":session_id}),
                );
            }
        }
        WindowEvent::Moved(_) | WindowEvent::Resized(_) => report_bounds_when_settled(&event_app),
        _ => {}
    });
    if app.get_webview_window("main").is_none() {
        let _ = lyrics_window.close();
        return Err(AppError::Other(
            "Main window closed while opening desktop lyrics".into(),
        ));
    }
    log::info!(
        target: "desktop-lyrics",
        "opened at x={} y={} {}x{} saved={}",
        place.x,
        place.y,
        place.width,
        place.height,
        bounds.is_some(),
    );
    Ok(())
}

#[tauri::command]
pub async fn close_desktop_lyrics(
    app: AppHandle,
    window: WebviewWindow,
    session_id: String,
) -> AppResult<()> {
    require_main(window.label())?;
    validate_session(&session_id)?;
    let _operation = window_operation().lock();
    if !snapshot().lock().owns(&session_id) {
        return Ok(());
    }
    if let Some(existing) = app.get_webview_window(WINDOW_LABEL) {
        existing
            .close()
            .map_err(|error| AppError::Other(error.to_string()))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn publish_desktop_lyrics(
    app: AppHandle,
    window: WebviewWindow,
    frame: DesktopLyricsFrame,
    session_id: String,
) -> AppResult<()> {
    require_main(window.label())?;
    validate_session(&session_id)?;
    frame.validate()?;
    let _operation = window_operation().lock();
    {
        let mut state = snapshot().lock();
        if !state.owns(&session_id) {
            return Err(AppError::Other(
                "Desktop lyrics window session changed".into(),
            ));
        }
        state.frame = frame.clone();
    }
    if app.get_webview_window(WINDOW_LABEL).is_some() {
        app.emit_to(WINDOW_LABEL, FRAME_EVENT, frame)
            .map_err(|error| AppError::Other(error.to_string()))?;
    }
    Ok(())
}

#[tauri::command]
pub fn get_desktop_lyrics_snapshot(window: WebviewWindow) -> AppResult<DesktopLyricsFrame> {
    require_reader(window.label())?;
    Ok(snapshot().lock().frame.clone())
}

/// 锁定：窗口点击穿透；光标移进窗口时通知歌词窗口显示解锁按钮，落在按钮上时才接收点击
#[tauri::command]
pub async fn set_desktop_lyrics_lock(
    app: AppHandle,
    window: WebviewWindow,
    session_id: String,
    locked: bool,
) -> AppResult<()> {
    require_main(window.label())?;
    validate_session(&session_id)?;
    let generation = {
        let mut state = snapshot().lock();
        if !state.owns(&session_id) {
            return Ok(());
        }
        if state.lock.locked == locked {
            return Ok(());
        }
        state.lock.locked = locked;
        state.lock.generation += 1;
        state.lock.generation
    };
    let Some(lyrics) = app.get_webview_window(WINDOW_LABEL) else { return Ok(()) };
    lyrics
        .set_ignore_cursor_events(locked)
        .map_err(|error| AppError::Other(error.to_string()))?;
    let _ = app.emit_to(WINDOW_LABEL, LOCK_EVENT, serde_json::json!({"locked":locked}));
    log::info!(target: "desktop-lyrics", "lock={locked}");
    if locked {
        tauri::async_runtime::spawn(poll_locked_cursor(app, generation));
    }
    Ok(())
}

async fn poll_locked_cursor(app: AppHandle, generation: u64) {
    let mut inside = false;
    let mut passthrough = true;
    loop {
        tokio::time::sleep(LOCK_POLL).await;
        let region = {
            let state = snapshot().lock();
            if !state.lock.locked || state.lock.generation != generation {
                break;
            }
            state.lock.hit_region
        };
        let Some(window) = app.get_webview_window(WINDOW_LABEL) else { break };
        let (Ok(cursor), Ok(origin), Ok(size), Ok(scale)) = (
            app.cursor_position(),
            window.outer_position(),
            window.outer_size(),
            window.scale_factor(),
        ) else {
            continue;
        };
        let x = cursor.x - f64::from(origin.x);
        let y = cursor.y - f64::from(origin.y);
        let now_inside = x >= 0.0 && y >= 0.0 && x < f64::from(size.width) && y < f64::from(size.height);
        if now_inside != inside {
            inside = now_inside;
            let _ = app.emit_to(WINDOW_LABEL, HOVER_EVENT, serde_json::json!({"inside":inside}));
        }
        let scale = scale.max(0.1);
        let on_button = now_inside && region.is_some_and(|region| region.contains(x / scale, y / scale));
        if on_button == passthrough {
            passthrough = !on_button;
            let _ = window.set_ignore_cursor_events(passthrough);
        }
    }
}

/// 歌词窗口报告锁定时可点的解锁按钮位置
#[tauri::command]
pub fn desktop_lyrics_hit_region(window: WebviewWindow, region: Option<HitRegion>) -> AppResult<()> {
    require_lyrics(window.label())?;
    if region.is_some_and(|region| !region.valid()) {
        return Err(AppError::Other("Invalid desktop lyrics hit region".into()));
    }
    snapshot().lock().lock.hit_region = region;
    Ok(())
}

/// 歌词窗口工具栏的操作转给主窗口执行；歌词窗口自己不碰播放状态
#[tauri::command]
pub fn desktop_lyrics_action(app: AppHandle, window: WebviewWindow, action: String) -> AppResult<()> {
    require_lyrics(window.label())?;
    if !ACTIONS.contains(&action.as_str()) {
        return Err(AppError::Other("Unknown desktop lyrics action".into()));
    }
    if action == "open-settings" {
        // 主窗口可能最小化或在托盘里，前端没有恢复窗口的权限，这里直接拉起来
        if let Some(main) = app.get_webview_window("main") {
            let _ = main.unminimize();
            let _ = main.show();
            let _ = main.set_focus();
        }
    }
    app.emit_to("main", ACTION_EVENT, serde_json::json!({"action":action}))
        .map_err(|error| AppError::Other(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_sessions_and_old_native_destroy_events_cannot_retire_a_new_window() {
        let mut state = DesktopLyricsState {
            window_instance: "native-old".into(),
            ..Default::default()
        };
        state.adopt("session-old".into());
        assert!(state.owns("session-old"));
        state.adopt("session-new".into());
        assert!(!state.owns("session-old"));
        assert!(state.owns("session-new"));
        state.window_instance = "native-new".into();
        assert!(state.retire("native-old").is_none());
        assert!(state.owns("session-new"));
        assert_eq!(state.retire("native-new"), Some("session-new".into()));
        assert!(!state.owns("session-new"));
    }

    #[test]
    fn closing_the_window_stops_the_lock_poll() {
        let mut state = DesktopLyricsState { window_instance: "native".into(), ..Default::default() };
        state.lock.locked = true;
        state.lock.generation = 4;
        state.retire("native");
        assert!(!state.lock.locked);
        assert_eq!(state.lock.generation, 5, "旧的轮询任务看到代数变化后退出");
    }

    #[test]
    fn ipc_sessions_require_a_bounded_uuid() {
        assert!(validate_session(&uuid::Uuid::new_v4().to_string()).is_ok());
        assert!(validate_session("").is_err());
        assert!(validate_session(&"x".repeat(2048)).is_err());
    }

    fn frame_with_line(line: DesktopLyricsLine) -> DesktopLyricsFrame {
        DesktopLyricsFrame { rate: 1.0, lines: vec![line], ..Default::default() }
    }

    #[test]
    fn frame_is_bounded_by_utf8_bytes_counts_and_serialized_size() {
        let line = DesktopLyricsLine { text: "词".repeat(341), ..Default::default() };
        assert!(frame_with_line(line.clone()).validate().is_ok());
        assert!(frame_with_line(DesktopLyricsLine { text: "词".repeat(342), ..line.clone() }).validate().is_err());
        let words = vec![DesktopLyricsWord { text: "字".into(), ..Default::default() }; MAX_LINE_WORDS + 1];
        assert!(frame_with_line(DesktopLyricsLine { words, ..Default::default() }).validate().is_err());
        let long_word = vec![DesktopLyricsWord { text: "x".repeat(MAX_WORD_BYTES + 1), ..Default::default() }];
        assert!(frame_with_line(DesktopLyricsLine { words: long_word, ..Default::default() }).validate().is_err());
        let mut too_many = frame_with_line(DesktopLyricsLine::default());
        too_many.lines = vec![DesktopLyricsLine::default(); MAX_FRAME_LINES + 1];
        assert!(too_many.validate().is_err());
    }

    #[test]
    fn frame_rejects_bad_times_rates_accents_and_styles() {
        let negative = DesktopLyricsLine { start_ms: -1.0, ..Default::default() };
        assert!(frame_with_line(negative).validate().is_err());
        let mut frame = frame_with_line(DesktopLyricsLine::default());
        frame.rate = 0.0;
        assert!(frame.validate().is_err());
        frame.rate = 1.25;
        frame.accent = "#d0bcff".into();
        assert!(frame.validate().is_ok());
        frame.accent = "red; x: y".into();
        assert!(frame.validate().is_err());
        frame.accent.clear();
        frame.style = serde_json::json!("not an object");
        assert!(frame.validate().is_err());
        frame.style = serde_json::json!({"fontFamily": "x".repeat(MAX_STYLE_BYTES)});
        assert!(frame.validate().is_err());
        frame.style = serde_json::json!({"layout": "double"});
        assert!(frame.validate().is_ok());
    }

    #[test]
    fn frame_rejects_unknown_fields_and_uses_the_frontend_names() {
        let value = serde_json::to_value(frame_with_line(DesktopLyricsLine {
            words: vec![DesktopLyricsWord::default()],
            ..Default::default()
        }))
        .unwrap();
        assert_eq!(value["trackId"], "");
        assert_eq!(value["isPlaying"], false);
        assert_eq!(value["anchorAt"], 0.0);
        assert_eq!(value["lines"][0]["words"][0]["startMs"], 0.0);
        let mut changed = value.clone();
        changed["audioUrl"] = serde_json::Value::String("unexpected".into());
        assert!(serde_json::from_value::<DesktopLyricsFrame>(changed).is_err());
        let mut nested = value;
        nested["lines"][0]["html"] = serde_json::Value::String("<b>".into());
        assert!(serde_json::from_value::<DesktopLyricsFrame>(nested).is_err());
    }

    #[test]
    fn child_can_only_read_and_unrelated_windows_cannot_read() {
        assert!(require_main("main").is_ok());
        assert!(require_main(WINDOW_LABEL).is_err());
        assert!(require_main("youtube-login").is_err());
        assert!(require_reader(WINDOW_LABEL).is_ok());
        assert!(require_reader("main").is_ok());
        assert!(require_reader("youtube-login").is_err());
        assert!(require_lyrics(WINDOW_LABEL).is_ok());
        assert!(require_lyrics("main").is_err(), "工具栏操作只认歌词窗口");
    }

    #[test]
    fn toolbar_actions_are_a_closed_list() {
        for action in ["toggle-play", "next", "lock", "unlock", "open-settings"] {
            assert!(ACTIONS.contains(&action));
        }
        assert!(!ACTIONS.contains(&"set-volume"));
    }

    const SCREEN: WorkArea = WorkArea { x: 0.0, y: 0.0, width: 1920.0, height: 1032.0 };
    const RIGHT_SCREEN: WorkArea = WorkArea { x: 1920.0, y: 0.0, width: 1280.0, height: 984.0 };

    #[test]
    fn a_saved_position_on_a_connected_screen_is_kept() {
        let saved = LogicalBounds { x: 2100.0, y: 700.0, width: 900.0, height: 200.0 };
        assert_eq!(placement(Some(saved), &[SCREEN, RIGHT_SCREEN], Some(SCREEN)), saved);
        let tiny = LogicalBounds { x: 100.0, y: 100.0, width: 50.0, height: 20.0 };
        assert_eq!(
            placement(Some(tiny), &[SCREEN], Some(SCREEN)),
            LogicalBounds { x: 100.0, y: 100.0, width: MIN_WIDTH, height: MIN_HEIGHT },
            "过小的尺寸放大到下限",
        );
    }

    #[test]
    fn an_off_screen_position_falls_back_to_the_bottom_center_of_the_primary_screen() {
        let unplugged = LogicalBounds { x: 2100.0, y: 700.0, width: 900.0, height: 200.0 };
        let placed = placement(Some(unplugged), &[SCREEN], Some(SCREEN));
        assert_eq!(placed, LogicalBounds { x: 530.0, y: 780.0, width: DEFAULT_WIDTH, height: DEFAULT_HEIGHT });
        let mostly_off = LogicalBounds { x: 1700.0, y: 900.0, width: 900.0, height: 200.0 };
        assert_eq!(placement(Some(mostly_off), &[SCREEN], Some(SCREEN)).x, 530.0, "只露出一小角的也重新摆放");
        assert_eq!(placement(None, &[SCREEN], Some(SCREEN)), placed, "第一次打开放在主屏幕底部居中");
        let narrow = WorkArea { x: 0.0, y: 0.0, width: 800.0, height: 600.0 };
        assert_eq!(placement(None, &[narrow], None).width, 640.0, "窄屏幕上按八成宽度");
    }

    #[test]
    fn hit_regions_are_validated_and_hit_tested_in_css_pixels() {
        let region = HitRegion { x: 10.0, y: 8.0, width: 32.0, height: 32.0 };
        assert!(region.valid());
        assert!(region.contains(20.0, 20.0));
        assert!(!region.contains(50.0, 20.0));
        assert!(!HitRegion { x: f64::NAN, ..region }.valid());
        assert!(!HitRegion { width: -1.0, ..region }.valid());
    }
}
