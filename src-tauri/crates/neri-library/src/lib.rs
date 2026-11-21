//! 用户数据与同步：SQLite 用户库、本地音乐与歌单、下载文件元数据、歌词获取与匹配、
//! 播放统计，以及基于基线快照的三路合并同步引擎（GitHub / WebDAV）。
//! 歌词与下载元数据同在本 crate：歌词需要读写已下载文件的内嵌与旁挂歌词。
//! 宿主能力（事件投递、同步状态存储）通过接口注入，本 crate 不依赖 Tauri

pub mod db;
pub mod download_metadata;
pub mod downloads;
pub mod library;
pub mod lyrics;
pub mod stats;
pub mod sync;
