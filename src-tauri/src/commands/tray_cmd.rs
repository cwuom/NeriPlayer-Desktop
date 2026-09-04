// 托盘菜单多语言：文案统一存放在前端 i18n 资源（src/i18n/*.json），
// 前端 setLocale 时把当前语言已翻译的文案通过 set_tray_texts 下发，
// Rust 侧不维护任何语言数据，只负责应用与存模板
use std::sync::Mutex;
use tauri::menu::MenuItem;
use tauri::AppHandle;

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

struct TrayState {
    app: AppHandle,
    handles: TrayMenuHandles,
    texts: Option<TrayTexts>,
    /// 最近一次同步的曲目名：文案更新（语言切换）后重绘用
    current_title: String,
}

// 单一锁：句柄/文案/曲目名同锁保护，避免多锁嵌套顺序不一致导致的死锁
static TRAY: Mutex<Option<TrayState>> = Mutex::new(None);

/// GTK（Linux）菜单项必须在主线程变更，跨线程 set_text 会静默失效；
/// 统一经 run_on_main_thread 派发（Windows/macOS 同样安全）
fn apply_item_text(app: &AppHandle, item: MenuItem<tauri::Wry>, text: String) {
    let _ = app.run_on_main_thread(move || {
        let _ = item.set_text(text);
    });
}

/// 渲染「正在播放」项文案：空标题用裸标签，否则按模板替换（超长截断 40 字）
fn format_now_playing(title: &str, now_playing: &str, title_template: &str) -> String {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return now_playing.to_string();
    }
    let truncated: String = trimmed.chars().take(40).collect();
    let truncated = if truncated.chars().count() < trimmed.chars().count() {
        format!("{truncated}…")
    } else {
        truncated
    };
    title_template.replace("{title}", &truncated)
}

/// 根据当前状态计算「正在播放」项文案（无文案下发时返回 None，暂不更新）
fn render_now_playing(state: &TrayState) -> Option<String> {
    let texts = state.texts.as_ref()?;
    Some(format_now_playing(
        &state.current_title,
        &texts.now_playing,
        &texts.now_playing_title,
    ))
}

/// 托盘菜单创建完成后注册句柄与 AppHandle（main.rs setup 调用一次）
pub fn register_tray_handles(app: &AppHandle, handles: TrayMenuHandles) {
    *TRAY.lock().unwrap() = Some(TrayState {
        app: app.clone(),
        handles,
        texts: None,
        current_title: String::new(),
    });
}

/// 应用语言变化时同步托盘菜单文案（前端 setLocale 调用，文案由 i18n 资源翻译）。
/// 更新后按当前曲目名重绘「正在播放：xxx」，避免语言切换把标题刷掉
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
    let mut guard = TRAY.lock().unwrap();
    let Some(state) = guard.as_mut() else {
        return;
    };
    log::info!(target: "tray", "set_tray_texts 已接收文案: nowPlaying={now_playing} template={now_playing_title}");
    let app = state.app.clone();
    let prev_item = state.handles.prev.clone();
    let toggle_item = state.handles.toggle.clone();
    let next_item = state.handles.next.clone();
    let now_item = state.handles.now_playing.clone();
    let home_item = state.handles.home.clone();
    let quit_item = state.handles.quit.clone();
    state.texts = Some(TrayTexts {
        now_playing,
        now_playing_title,
    });
    drop(guard);
    // 主线程统一应用文案（含按当前曲目名重绘的「正在播放」项）
    apply_item_text(&app, prev_item, prev);
    apply_item_text(&app, toggle_item, toggle);
    apply_item_text(&app, next_item, next);
    apply_item_text(&app, home_item, open_home);
    apply_item_text(&app, quit_item, quit);
    {
        let guard = TRAY.lock().unwrap();
        if let Some(text) = guard.as_ref().and_then(render_now_playing) {
            apply_item_text(&app, now_item, text);
        }
    }
}

/// 同步托盘「正在播放」项为当前曲目名（title 为空时恢复「正在播放」）。
/// 由后台 ticker 在曲目切换时调用；文案模板来自前端 i18n 资源
pub fn update_now_playing(title: &str) {
    let mut guard = TRAY.lock().unwrap();
    let Some(state) = guard.as_mut() else {
        return;
    };
    state.current_title = title.to_string();
    let Some(text) = render_now_playing(state) else {
        // 文案尚未下发（set_tray_texts 未到），先记住标题，下发后自动重绘
        return;
    };
    log::info!(target: "tray", "update_now_playing: title={title:?}");
    let app = state.app.clone();
    let item = state.handles.now_playing.clone();
    drop(guard);
    apply_item_text(&app, item, text);
}

#[cfg(test)]
mod tests {
    use super::format_now_playing;

    #[test]
    fn empty_title_uses_bare_label() {
        assert_eq!(
            format_now_playing("", "正在播放", "正在播放：{title}"),
            "正在播放"
        );
        assert_eq!(
            format_now_playing("  ", "Now Playing", "Now Playing: {title}"),
            "Now Playing"
        );
    }

    #[test]
    fn title_is_embedded_in_template() {
        assert_eq!(
            format_now_playing("风又音理", "正在播放", "正在播放：{title}"),
            "正在播放：风又音理"
        );
        assert_eq!(
            format_now_playing("Song", "Now Playing", "Now Playing: {title}"),
            "Now Playing: Song"
        );
    }

    #[test]
    fn long_title_is_truncated_to_40_chars() {
        let long = "很长的歌曲标题，超过四十个字的时候应该被截断并加省略号，多出来的这些字不应该显示出来";
        let rendered = format_now_playing(long, "正在播放", "正在播放：{title}");
        let tail = rendered.trim_start_matches("正在播放：");
        assert_eq!(tail.chars().count(), 41); // 40 字 + 省略号
        assert!(tail.ends_with('…'));
    }

    #[test]
    fn template_without_placeholder_keeps_text() {
        assert_eq!(
            format_now_playing("Song", "Now Playing", "Playing {title} now"),
            "Playing Song now"
        );
    }
}
