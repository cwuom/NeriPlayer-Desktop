// ProtoBuf 消息定义：与 Android 端 SyncDataModels.kt 的 @ProtoNumber 对齐
// 用于省流模式 (backup.bin: ProtoBuf + GZIP + Base64)

/// SyncData 根容器
#[derive(Clone, PartialEq, prost::Message)]
pub struct ProtoSyncData {
    #[prost(string, tag = "1")]
    pub version: String,
    #[prost(string, tag = "2")]
    pub device_id: String,
    #[prost(string, tag = "3")]
    pub device_name: String,
    #[prost(int64, tag = "4")]
    pub last_modified: i64,
    #[prost(message, repeated, tag = "5")]
    pub playlists: Vec<ProtoSyncPlaylist>,
    #[prost(message, repeated, tag = "6")]
    pub favorite_playlists: Vec<ProtoSyncFavoritePlaylist>,
    #[prost(message, repeated, tag = "7")]
    pub recent_plays: Vec<ProtoSyncRecentPlay>,
    #[prost(message, repeated, tag = "8")]
    pub sync_log: Vec<ProtoSyncLogEntry>,
    #[prost(message, repeated, tag = "9")]
    pub recent_play_deletions: Vec<ProtoSyncRecentPlayDeletion>,
    #[prost(message, repeated, tag = "10")]
    pub playback_stats: Vec<ProtoSyncTrackStat>,
    #[prost(int64, tag = "11")]
    pub playback_stats_cleared_at: i64,
    #[prost(message, repeated, tag = "12")]
    pub playback_stat_buckets: Vec<ProtoSyncPlaybackStatBucket>,
    #[prost(message, repeated, tag = "13")]
    pub playlist_song_deletions: Vec<ProtoSyncPlaylistSongDeletion>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ProtoSyncPlaylist {
    #[prost(int64, tag = "1")]
    pub id: i64,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(message, repeated, tag = "3")]
    pub songs: Vec<ProtoSyncSong>,
    #[prost(int64, tag = "4")]
    pub created_at: i64,
    #[prost(int64, tag = "5")]
    pub modified_at: i64,
    #[prost(bool, tag = "6")]
    pub is_deleted: bool,
    #[prost(int32, tag = "7")]
    pub song_order_version: i32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ProtoSyncSong {
    #[prost(int64, tag = "1")]
    pub id: i64,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(string, tag = "3")]
    pub artist: String,
    #[prost(string, tag = "4")]
    pub album: String,
    #[prost(int64, tag = "5")]
    pub album_id: i64,
    #[prost(int64, tag = "6")]
    pub duration_ms: i64,
    #[prost(string, optional, tag = "7")]
    pub cover_url: Option<String>,
    #[prost(string, optional, tag = "8")]
    pub media_uri: Option<String>,
    #[prost(int64, tag = "9")]
    pub added_at: i64,
    #[prost(string, optional, tag = "10")]
    pub matched_lyric: Option<String>,
    #[prost(string, optional, tag = "11")]
    pub matched_translated_lyric: Option<String>,
    #[prost(string, optional, tag = "12")]
    pub matched_lyric_source: Option<String>,
    #[prost(string, optional, tag = "13")]
    pub matched_song_id: Option<String>,
    #[prost(int64, tag = "14")]
    pub user_lyric_offset_ms: i64,
    #[prost(string, optional, tag = "15")]
    pub custom_cover_url: Option<String>,
    #[prost(string, optional, tag = "16")]
    pub custom_name: Option<String>,
    #[prost(string, optional, tag = "17")]
    pub custom_artist: Option<String>,
    #[prost(string, optional, tag = "18")]
    pub original_name: Option<String>,
    #[prost(string, optional, tag = "19")]
    pub original_artist: Option<String>,
    #[prost(string, optional, tag = "20")]
    pub original_cover_url: Option<String>,
    #[prost(string, optional, tag = "21")]
    pub original_lyric: Option<String>,
    #[prost(string, optional, tag = "22")]
    pub original_translated_lyric: Option<String>,
    #[prost(string, optional, tag = "23")]
    pub channel_id: Option<String>,
    #[prost(string, optional, tag = "24")]
    pub audio_id: Option<String>,
    #[prost(string, optional, tag = "25")]
    pub sub_audio_id: Option<String>,
    #[prost(string, optional, tag = "26")]
    pub playlist_context_id: Option<String>,
    #[prost(message, repeated, tag = "27")]
    pub sync_membership_tokens: Vec<ProtoSyncCausalToken>,
    #[prost(int32, tag = "28")]
    pub sync_metadata_version: i32,
    #[prost(int64, optional, tag = "29")]
    pub legacy_added_at: Option<i64>,
    #[prost(int64, tag = "30")]
    pub lyric_sync_revision: i64,
    #[prost(bool, optional, tag = "31")]
    pub lyric_sync_edited: Option<bool>,
    #[prost(string, optional, tag = "32")]
    pub matched_romanized_lyric: Option<String>,
    #[prost(string, optional, tag = "33")]
    pub original_romanized_lyric: Option<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ProtoSyncRecentPlay {
    #[prost(int64, tag = "1")]
    pub song_id: i64,
    #[prost(message, optional, tag = "2")]
    pub song: Option<ProtoSyncSong>,
    #[prost(int64, tag = "3")]
    pub played_at: i64,
    #[prost(string, tag = "4")]
    pub device_id: String,
    #[prost(int64, tag = "5")]
    pub resume_position_ms: i64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ProtoSyncRecentPlayDeletion {
    #[prost(int64, tag = "1")]
    pub song_id: i64,
    #[prost(string, tag = "2")]
    pub album: String,
    #[prost(string, optional, tag = "3")]
    pub media_uri: Option<String>,
    #[prost(int64, tag = "4")]
    pub deleted_at: i64,
    #[prost(string, tag = "5")]
    pub device_id: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ProtoSyncFavoritePlaylist {
    #[prost(int64, tag = "1")]
    pub id: i64,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(string, optional, tag = "3")]
    pub cover_url: Option<String>,
    #[prost(int32, tag = "4")]
    pub track_count: i32,
    #[prost(string, tag = "5")]
    pub source: String,
    #[prost(message, repeated, tag = "6")]
    pub songs: Vec<ProtoSyncSong>,
    #[prost(int64, tag = "7")]
    pub added_time: i64,
    #[prost(int64, tag = "8")]
    pub modified_at: i64,
    #[prost(bool, tag = "9")]
    pub is_deleted: bool,
    #[prost(int64, tag = "10")]
    pub sort_order: i64,
    #[prost(string, optional, tag = "11")]
    pub browse_id: Option<String>,
    #[prost(string, optional, tag = "12")]
    pub playlist_id: Option<String>,
    #[prost(string, optional, tag = "13")]
    pub subtitle: Option<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ProtoSyncLogEntry {
    #[prost(int64, tag = "1")]
    pub timestamp: i64,
    #[prost(string, tag = "2")]
    pub device_id: String,
    #[prost(int32, tag = "3")]
    pub action: i32,
    #[prost(int64, optional, tag = "4")]
    pub playlist_id: Option<i64>,
    #[prost(int64, optional, tag = "5")]
    pub song_id: Option<i64>,
    #[prost(string, optional, tag = "6")]
    pub details: Option<String>,
}

// 以下为 Android 对齐新增的结构体
/// 因果一致性令牌：用于 CRDT 风格的歌曲成员关系追踪
#[derive(Clone, PartialEq, prost::Message)]
pub struct ProtoSyncCausalToken {
    #[prost(string, tag = "1")]
    pub device_id: String,
    #[prost(int64, tag = "2")]
    pub counter: i64,
}

/// 单曲播放统计
#[derive(Clone, PartialEq, prost::Message)]
pub struct ProtoSyncTrackStat {
    #[prost(string, tag = "1")]
    pub identity_key: String,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(string, tag = "3")]
    pub artist: String,
    #[prost(string, tag = "4")]
    pub album: String,
    #[prost(int64, tag = "5")]
    pub total_listen_ms: i64,
    #[prost(int32, tag = "6")]
    pub play_count: i32,
    #[prost(int64, tag = "7")]
    pub last_played_at: i64,
    #[prost(int64, tag = "8")]
    pub first_played_at: i64,
    #[prost(string, optional, tag = "9")]
    pub cover_url: Option<String>,
    #[prost(int64, tag = "10")]
    pub duration_ms: i64,
    #[prost(string, optional, tag = "11")]
    pub media_uri: Option<String>,
    #[prost(int64, tag = "12")]
    pub id: i64,
    #[prost(int64, tag = "13")]
    pub album_id: i64,
    #[prost(int64, tag = "14")]
    pub counter_base_listen_ms: i64,
    #[prost(int32, tag = "15")]
    pub counter_base_play_count: i32,
    #[prost(message, repeated, tag = "16")]
    pub counter_shards: Vec<ProtoSyncPlaybackCounterShard>,
}

/// 播放计数分片：每设备独立计数，避免并发冲突
#[derive(Clone, PartialEq, prost::Message)]
pub struct ProtoSyncPlaybackCounterShard {
    #[prost(string, tag = "1")]
    pub device_id: String,
    #[prost(int64, tag = "2")]
    pub epoch_started_at: i64,
    #[prost(int64, tag = "3")]
    pub total_listen_ms: i64,
    #[prost(int32, tag = "4")]
    pub play_count: i32,
    #[prost(int64, tag = "5")]
    pub first_played_at: i64,
    #[prost(int64, tag = "6")]
    pub last_played_at: i64,
}

/// 按天分桶的播放统计
#[derive(Clone, PartialEq, prost::Message)]
pub struct ProtoSyncPlaybackStatBucket {
    #[prost(int64, tag = "1")]
    pub day_start_at: i64,
    #[prost(string, tag = "2")]
    pub identity_key: String,
    #[prost(string, tag = "3")]
    pub name: String,
    #[prost(string, tag = "4")]
    pub artist: String,
    #[prost(string, tag = "5")]
    pub album: String,
    #[prost(int64, tag = "6")]
    pub total_listen_ms: i64,
    #[prost(int32, tag = "7")]
    pub play_count: i32,
    #[prost(int64, tag = "8")]
    pub last_played_at: i64,
    #[prost(int64, tag = "9")]
    pub first_played_at: i64,
    #[prost(string, optional, tag = "10")]
    pub cover_url: Option<String>,
    #[prost(int64, tag = "11")]
    pub duration_ms: i64,
    #[prost(string, optional, tag = "12")]
    pub media_uri: Option<String>,
    #[prost(int64, tag = "13")]
    pub id: i64,
    #[prost(int64, tag = "14")]
    pub album_id: i64,
    #[prost(int64, tag = "15")]
    pub counter_base_listen_ms: i64,
    #[prost(int32, tag = "16")]
    pub counter_base_play_count: i32,
    #[prost(message, repeated, tag = "17")]
    pub counter_shards: Vec<ProtoSyncPlaybackCounterShard>,
}

/// 歌单内歌曲删除记录：区别于 RecentPlayDeletion（最近播放的删除）
#[derive(Clone, PartialEq, prost::Message)]
pub struct ProtoSyncPlaylistSongDeletion {
    #[prost(int64, tag = "1")]
    pub playlist_id: i64,
    #[prost(int64, tag = "2")]
    pub song_id: i64,
    #[prost(string, tag = "3")]
    pub album: String,
    #[prost(string, optional, tag = "4")]
    pub media_uri: Option<String>,
    #[prost(int64, tag = "5")]
    pub deleted_at: i64,
    #[prost(string, tag = "6")]
    pub device_id: String,
    #[prost(message, repeated, tag = "7")]
    pub removed_membership_tokens: Vec<ProtoSyncCausalToken>,
}
