use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

/// 领域层向宿主投递事件的出口（Tauri 中转发为前端事件）。
/// 领域 crate 只描述事件名与 JSON 负载，不依赖具体 UI 框架，测试中可用记录型实现替代
pub trait EventSink: Send + Sync {
    fn emit_json(&self, event: &str, payload: Value);
}

pub type SharedEventSink = Arc<dyn EventSink>;

impl dyn EventSink {
    /// 序列化后投递；序列化失败时不投递并把错误交给调用方
    pub fn emit<T: Serialize>(&self, event: &str, payload: T) -> serde_json::Result<()> {
        self.emit_json(event, serde_json::to_value(payload)?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<(String, Value)>>);

    impl EventSink for Recorder {
        fn emit_json(&self, event: &str, payload: Value) {
            self.0.lock().unwrap().push((event.to_owned(), payload));
        }
    }

    #[test]
    fn emit_serializes_payload_before_forwarding() {
        let recorder = Arc::new(Recorder::default());
        let sink: SharedEventSink = recorder.clone();
        sink.emit("ready", serde_json::json!({ "id": 1 })).unwrap();
        sink.emit("unit", ()).unwrap();
        let events = recorder.0.lock().unwrap();
        assert_eq!(events[0], ("ready".into(), serde_json::json!({ "id": 1 })));
        assert_eq!(events[1], ("unit".into(), Value::Null));
    }
}
