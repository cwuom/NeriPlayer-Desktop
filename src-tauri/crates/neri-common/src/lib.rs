//! 全项目共用的基础设施：统一错误类型、同步失败分类、原子文件写入、本地凭据存储、
//! 配置备份编解码，以及跨层的事件出口。不依赖任何项目内 crate，也不依赖 Tauri

pub mod config_backup;
pub mod error;
pub mod events;
pub mod fsutil;
pub mod security;
pub mod sync_failure;
