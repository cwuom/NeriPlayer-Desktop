//! 前后端与各领域 crate 共享的曲目模型。字段名与 serde 别名属于 IPC 契约，
//! 同时被播放队列、本地歌单和云同步快照读写，修改时须保持向后兼容

use serde::{Deserialize, Serialize};

use crate::sync::models::SyncSong;

/// 曲目信息（前后端共享）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackInfo {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub source: TrackSource,
    /// 本地文件路径或远程 URL
    pub url: String,
    pub cover_url: Option<String>,
    #[serde(default, alias = "addedAt")]
    pub added_at: i64,
    #[serde(
        default,
        alias = "syncPayload",
        skip_serializing_if = "Option::is_none"
    )]
    pub sync_payload: Option<SyncSong>,
    #[serde(
        default,
        alias = "playlistKey",
        skip_serializing_if = "Option::is_none"
    )]
    pub playlist_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TrackSource {
    Local,
    Netease,
    Qq,
    Bilibili,
    Youtube,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RepeatMode {
    Off,
    All,
    One,
}
