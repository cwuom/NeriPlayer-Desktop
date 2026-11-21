//! 跨模块共享的数据模型：IPC 曲目类型、云同步 JSON / ProtoBuf 模型与 B 站跳过规则。
//! 同步模型的 ProtoBuf 字段编号与 Android `SyncDataModels.kt` 手工对齐，修改会破坏跨端同步

pub mod bili_video_skip;
pub mod sync;
pub mod track;

pub use track::{RepeatMode, TrackInfo, TrackSource};
