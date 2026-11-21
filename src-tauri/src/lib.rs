//! 桌面端宿主：Tauri 命令、窗口与托盘、设置持久化、日志，以及把领域 crate 需要的
//! 宿主能力（事件出口、WebView 令牌、同步状态存储、异步运行时）接到 Tauri 上。
//! 业务逻辑位于 crates/ 下的 neri-* crate

pub mod auth;
pub mod commands;
pub mod events;
/// Linux WebKitGTK/NVIDIA 图形兼容（启动前设置环境变量）。
/// Linux WebKitGTK/NVIDIA graphics compatibility (sets env vars before startup).
#[cfg(target_os = "linux")]
pub mod linux_graphics;
pub mod logging;
pub mod platform_webview;
pub mod settings;
pub mod state;
pub mod sync_state;
pub mod webview_args;
