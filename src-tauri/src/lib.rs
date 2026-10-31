pub mod api;
pub mod audio;
/// Linux WebKitGTK/NVIDIA 图形兼容（启动前设置环境变量）。
/// Linux WebKitGTK/NVIDIA graphics compatibility (sets env vars before startup).
#[cfg(target_os = "linux")]
pub mod linux_graphics;
pub mod auth;
pub mod commands;
pub mod config_backup;
pub mod db;
pub mod error;
pub mod fsutil;
pub mod library;
pub mod listen_together;
pub mod logging;
pub mod lyrics;
pub mod security;
pub mod settings;
pub mod state;
pub mod stats;
pub mod sync;
pub mod webview_args;
