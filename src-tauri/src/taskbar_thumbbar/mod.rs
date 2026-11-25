//! Windows 任务栏缩略图工具栏：悬停任务栏图标时预览图下方的「上一首 / 播放暂停 / 下一首」。
//!
//! 状态跟托盘同源：`tray_cmd::refresh` 在主窗口发布快照或后台 ticker 发现播放状态变化时调用
//! [`update`]；点击沿用托盘的 `media:*` 事件交给主窗口处理，播放逻辑只有一处。
//! 按钮模型、图标绘制与平台无关，便于单测；`windows.rs` 只依赖 windows crate，不碰 Tauri

pub mod glyph;
mod model;
#[cfg(windows)]
mod windows;

pub use model::*;

#[cfg(windows)]
static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// 给主窗口挂上缩略图工具栏；须在主线程调用（Tauri setup 里）。非 Windows 平台什么也不做
#[cfg(windows)]
pub fn install(app: &tauri::AppHandle, window: &tauri::WebviewWindow) {
    let _ = APP.set(app.clone());
    match window.hwnd() {
        Ok(hwnd) => windows::install(hwnd.0 as isize, emit_action),
        Err(error) => log::warn!(target: "thumbbar", "main window handle unavailable: {error}"),
    }
}

#[cfg(not(windows))]
pub fn install(_app: &tauri::AppHandle, _window: &tauri::WebviewWindow) {}

#[cfg(windows)]
fn emit_action(action: Action) {
    use tauri::Emitter;
    if let Some(app) = APP.get() {
        let _ = app.emit_to("main", action.event(), ());
    }
}

/// 同步按钮状态；COM 对象只能在窗口所在的主线程上用，这里转发过去，状态没变时不做系统调用
#[cfg(windows)]
pub fn update(app: &tauri::AppHandle, state: ThumbbarState) {
    if let Err(error) = app.run_on_main_thread(move || windows::apply(state)) {
        log::warn!(target: "thumbbar", "thumbnail toolbar not updated: {error}");
    }
}

#[cfg(not(windows))]
pub fn update(_app: &tauri::AppHandle, _state: ThumbbarState) {}
