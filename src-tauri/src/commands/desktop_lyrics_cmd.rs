use std::sync::OnceLock;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

use crate::error::{AppError, AppResult};

pub const WINDOW_LABEL: &str = "desktop-lyrics";
const FRAME_EVENT: &str = "desktop-lyrics:frame";
const CLOSED_EVENT: &str = "desktop-lyrics:closed";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopLyricsFrame {
    track_id: String,
    title: String,
    artist: String,
    previous: String,
    current: String,
    next: String,
    translation: String,
    is_playing: bool,
}

impl DesktopLyricsFrame {
    fn validate(&self) -> AppResult<()> {
        for (value, maximum) in [
            (&self.track_id, 512),
            (&self.title, 1024),
            (&self.artist, 1024),
            (&self.previous, 4096),
            (&self.current, 4096),
            (&self.next, 4096),
            (&self.translation, 4096),
        ] {
            if value.len() > maximum {
                return Err(AppError::Other(
                    "Desktop lyrics frame exceeds text limit".into(),
                ));
            }
        }
        if serde_json::to_vec(self)?.len() > 24 * 1024 {
            return Err(AppError::Other(
                "Desktop lyrics frame exceeds size limit".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Default)]
struct DesktopLyricsState {
    session_id: String,
    window_instance: String,
    frame: DesktopLyricsFrame,
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
        *self = Self::default();
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

fn require_reader(label: &str) -> AppResult<()> {
    if label != "main" && label != WINDOW_LABEL {
        return Err(AppError::Other(
            "Desktop lyrics snapshot is not available to this window".into(),
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn open_desktop_lyrics(
    app: AppHandle,
    window: WebviewWindow,
    session_id: String,
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
    .inner_size(620.0, 190.0)
    .min_inner_size(360.0, 170.0)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .visible(false);
    // macOS 透明 WebView 需要额外私有 API feature，保留普通背景兼容默认构建
    #[cfg(target_os = "windows")]
    let builder = builder.transparent(true);
    let lyrics_window = builder.build().map_err(|error| {
        snapshot().lock().retire(&instance);
        AppError::Other(error.to_string())
    })?;
    let event_app = app.clone();
    lyrics_window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            let session = snapshot().lock().retire(&instance);
            if let Some(session_id) = session {
                let _ = event_app.emit_to(
                    "main",
                    CLOSED_EVENT,
                    serde_json::json!({"sessionId":session_id}),
                );
            }
        }
    });
    if app.get_webview_window("main").is_none() {
        let _ = lyrics_window.close();
        return Err(AppError::Other(
            "Main window closed while opening desktop lyrics".into(),
        ));
    }
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
    fn ipc_sessions_require_a_bounded_uuid() {
        assert!(validate_session(&uuid::Uuid::new_v4().to_string()).is_ok());
        assert!(validate_session("").is_err());
        assert!(validate_session(&"x".repeat(2048)).is_err());
    }

    #[test]
    fn frame_is_bounded_by_utf8_bytes_and_serialized_size() {
        let mut frame = DesktopLyricsFrame {
            current: "词".repeat(1365),
            ..Default::default()
        };
        assert!(frame.validate().is_ok());
        frame.current.push('词');
        assert!(frame.validate().is_err());
        frame.current = "\u{0001}".repeat(4096);
        assert!(frame.validate().is_err());
    }

    #[test]
    fn child_can_only_read_and_unrelated_windows_cannot_read() {
        assert!(require_main("main").is_ok());
        assert!(require_main(WINDOW_LABEL).is_err());
        assert!(require_main("youtube-login").is_err());
        assert!(require_reader(WINDOW_LABEL).is_ok());
        assert!(require_reader("main").is_ok());
        assert!(require_reader("youtube-login").is_err());
    }

    #[test]
    fn frame_rejects_unknown_fields_and_uses_the_frontend_names() {
        let value = serde_json::to_value(DesktopLyricsFrame::default()).unwrap();
        assert_eq!(value["trackId"], "");
        assert_eq!(value["isPlaying"], false);
        let mut changed = value;
        changed["audioUrl"] = serde_json::Value::String("unexpected".into());
        assert!(serde_json::from_value::<DesktopLyricsFrame>(changed).is_err());
    }
}
