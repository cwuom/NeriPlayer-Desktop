//! 一起听：与 Android 对齐的房间协议模型、会话状态与 WebSocket 客户端。
//! 连接事件经 [`neri_common::events::EventSink`] 交给宿主投递，本 crate 不依赖 Tauri

pub mod protocol;
pub mod session;
pub mod ws_client;
