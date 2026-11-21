//! 把领域 crate 的事件出口接到 Tauri 前端事件

use std::sync::Arc;

use neri_common::events::{EventSink, SharedEventSink};
use serde_json::Value;
use tauri::{AppHandle, Emitter};

struct TauriEventSink(AppHandle);

impl EventSink for TauriEventSink {
    fn emit_json(&self, event: &str, payload: Value) {
        if let Err(error) = self.0.emit(event, payload) {
            log::warn!(target: "events", "emit {event} failed: {error}");
        }
    }
}

pub fn event_sink(app: &AppHandle) -> SharedEventSink {
    Arc::new(TauriEventSink(app.clone()))
}
