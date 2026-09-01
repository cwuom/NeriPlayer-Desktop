// 托盘菜单多语言：文案统一存放在前端 i18n 资源（src/i18n/*.json），
// 前端 setLocale 时把当前语言已翻译的文案通过 set_tray_texts 下发，
// Rust 侧不维护任何语言数据，只负责应用与存模板
use std::sync::Mutex;
use tauri::menu::MenuItem;

pub struct TrayMenuHandles {
    pub prev: MenuItem<tauri::Wry>,
    pub toggle: MenuItem<tauri::Wry>,
    pub next: MenuItem<tauri::Wry>,
    pub now_playing: MenuItem<tauri::Wry>,
    pub home: MenuItem<tauri::Wry>,
    pub quit: MenuItem<tauri::Wry>,
}

/// 前端下发的最新托盘文案（含「正在播放」裸标签与「正在播放：{title}」模板）
struct TrayTexts {
    now_playing: String,
    now_playing_title: String,
}

static TRAY_HANDLES: Mutex<Option<TrayMenuHandles>> = Mutex::new(None);
static TRAY_TEXTS: Mutex<Option<TrayTexts>> = Mutex::new(None);

/// 托盘菜单创建完成后注册句柄（main.rs setup 调用一次）
pub fn register_tray_handles(handles: TrayMenuHandles) {
    *TRAY_HANDLES.lock().unwrap() = Some(handles);
}

/// 应用语言变化时同步托盘菜单文案（前端 setLocale 调用，文案由 i18n 资源翻译）
#[tauri::command]
pub fn set_tray_texts(
    prev: String,
    toggle: String,
    next: String,
    now_playing: String,
    now_playing_title: String,
    open_home: String,
    quit: String,
) {
    *TRAY_TEXTS.lock().unwrap() = Some(TrayTexts {
        now_playing: now_playing.clone(),
        now_playing_title,
    });
    let guard = TRAY_HANDLES.lock().unwrap();
    let Some(handles) = guard.as_ref() else {
        return;
    };
    let _ = handles.prev.set_text(prev);
    let _ = handles.toggle.set_text(toggle);
    let _ = handles.next.set_text(next);
    let _ = handles.now_playing.set_text(now_playing);
    let _ = handles.home.set_text(open_home);
    let _ = handles.quit.set_text(quit);
}

/// 同步托盘「正在播放」项为当前曲目名（title 为空时恢复「正在播放」）。
/// 由后台 ticker 在曲目切换时调用；文案模板来自前端 i18n 资源
pub fn update_now_playing(title: &str) {
    let guard = TRAY_HANDLES.lock().unwrap();
    let Some(handles) = guard.as_ref() else {
        return;
    };
    let texts = TRAY_TEXTS.lock().unwrap();
    let trimmed = title.trim();
    let text = if trimmed.is_empty() {
        texts.as_ref().map(|t| t.now_playing.clone())
    } else {
        // 超长标题截断，避免菜单被撑爆
        let truncated: String = trimmed.chars().take(40).collect();
        let truncated = if truncated.chars().count() < trimmed.chars().count() {
            format!("{truncated}…")
        } else {
            truncated
        };
        texts
            .as_ref()
            .map(|t| t.now_playing_title.replace("{title}", &truncated))
    };
    if let Some(text) = text {
        let _ = handles.now_playing.set_text(text);
    }
}
