// 省流模式序列化/反序列化：与 Android SyncDataSerializer.kt 对齐
// backup.bin: Base64(GZIP(ProtoBuf))

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use prost::Message;
use std::io::{Read, Write};

use crate::error::{AppError, AppResult};
use super::models::*;
use super::proto_models::*;

const MAX_DECOMPRESSED_BYTES: usize = 16 * 1024 * 1024;

/// 返回省流模式使用的文件名
pub fn get_filename(data_saver: bool) -> &'static str {
    if data_saver { "backup.bin" } else { "backup.json" }
}

/// 序列化 SyncData（省流模式: ProtoBuf + GZIP + Base64）
pub fn serialize_compressed(data: &SyncData) -> AppResult<String> {
    let normalized = data.normalized_for_sync();
    let proto = sync_data_to_proto(&normalized);
    let proto_bytes = proto.encode_to_vec();

    // GZIP 压缩
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&proto_bytes)
        .map_err(|e| AppError::Other(format!("GZIP compress: {}", e)))?;
    let compressed = encoder.finish()
        .map_err(|e| AppError::Other(format!("GZIP finish: {}", e)))?;

    // Base64 编码
    Ok(BASE64.encode(&compressed))
}

/// 反序列化省流模式数据（Base64 -> GZIP -> ProtoBuf -> SyncData）
pub fn deserialize_compressed(content: &str) -> AppResult<SyncData> {
    // Base64 解码
    let compressed = BASE64.decode(content.trim())
        .map_err(|e| AppError::Other(format!("Base64 decode: {}", e)))?;

    // GZIP 解压
    let mut decoder = GzDecoder::new(&compressed[..]);
    let mut proto_bytes = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let read = decoder
            .read(&mut buffer)
            .map_err(|e| AppError::Other(format!("GZIP decompress: {}", e)))?;
        if read == 0 {
            break;
        }
        if read > MAX_DECOMPRESSED_BYTES - proto_bytes.len() {
            return Err(AppError::Other(
                "Decompressed sync data is too large".into(),
            ));
        }
        proto_bytes.extend_from_slice(&buffer[..read]);
    }

    let current = ProtoSyncData::decode(&proto_bytes[..]).map(|proto| proto_to_sync_data(&proto));
    match current {
        Ok(data) => {
            if let Ok(legacy_proto) = LegacyProtoSyncData::decode(&proto_bytes[..]) {
                let legacy_data = legacy_proto_to_sync_data(&legacy_proto);
                if should_use_legacy_decode(&data, &legacy_data) {
                    return Ok(legacy_data.normalized_for_sync());
                }
            }
            Ok(data.normalized_for_sync())
        }
        Err(current_err) => {
            let legacy_proto = LegacyProtoSyncData::decode(&proto_bytes[..]).map_err(|legacy_err| {
                AppError::Other(format!(
                    "ProtoBuf decode: {}; legacy decode: {}",
                    current_err, legacy_err
                ))
            })?;
            Ok(legacy_proto_to_sync_data(&legacy_proto).normalized_for_sync())
        }
    }
}

/// 根据 data_saver 标志选择序列化方式
pub fn serialize(data: &SyncData, data_saver: bool) -> AppResult<String> {
    if data_saver {
        serialize_compressed(data)
    } else {
        serde_json::to_string_pretty(&data.normalized_for_sync())
            .map_err(|e| AppError::Other(format!("JSON serialize: {}", e)))
    }
}

/// 根据文件后缀判断格式并反序列化
pub fn deserialize(content: &str, is_binary: bool) -> AppResult<SyncData> {
    if is_binary {
        deserialize_compressed(content)
    } else {
        serde_json::from_str::<SyncData>(content)
            .map(|data| data.normalized_for_sync())
            .map_err(|e| AppError::Other(format!("JSON parse: {}", e)))
    }
}

// Proto <-> SyncData 转换
fn sync_data_to_proto(data: &SyncData) -> ProtoSyncData {
    ProtoSyncData {
        version: data.version.clone(),
        device_id: data.device_id.clone(),
        device_name: data.device_name.clone(),
        last_modified: data.last_modified,
        playlists: data.playlists.iter().map(sync_playlist_to_proto).collect(),
        favorite_playlists: data.favorite_playlists.iter().map(fav_playlist_to_proto).collect(),
        recent_plays: data.recent_plays.iter().map(recent_play_to_proto).collect(),
        sync_log: data.sync_log.iter().map(log_entry_to_proto).collect(),
        recent_play_deletions: data.recent_play_deletions.iter().map(deletion_to_proto).collect(),
        playback_stats: data.playback_stats.iter().map(track_stat_to_proto).collect(),
        playback_stats_cleared_at: data.playback_stats_cleared_at,
        playback_stat_buckets: data.playback_stat_buckets.iter().map(stat_bucket_to_proto).collect(),
        playlist_song_deletions: data.playlist_song_deletions.iter().map(playlist_song_deletion_to_proto).collect(),
    }
}

fn proto_to_sync_data(p: &ProtoSyncData) -> SyncData {
    SyncData {
        version: if p.version.is_empty() { "2.0".into() } else { p.version.clone() },
        device_id: p.device_id.clone(),
        device_name: p.device_name.clone(),
        last_modified: p.last_modified,
        playlists: p.playlists.iter().map(proto_to_sync_playlist).collect(),
        favorite_playlists: p.favorite_playlists.iter().map(proto_to_fav_playlist).collect(),
        recent_plays: p.recent_plays.iter().map(proto_to_recent_play).collect(),
        sync_log: p.sync_log.iter().map(proto_to_log_entry).collect(),
        recent_play_deletions: p.recent_play_deletions.iter().map(proto_to_deletion).collect(),
        playback_stats: p.playback_stats.iter().map(proto_to_track_stat).collect(),
        playback_stats_cleared_at: p.playback_stats_cleared_at,
        playback_stat_buckets: p.playback_stat_buckets.iter().map(proto_to_stat_bucket).collect(),
        playlist_song_deletions: p.playlist_song_deletions.iter().map(proto_to_playlist_song_deletion).collect(),
    }
}

#[derive(Clone, PartialEq, prost::Message)]
struct LegacyProtoSyncData {
    #[prost(string, tag = "1")]
    version: String,
    #[prost(string, tag = "2")]
    device_id: String,
    #[prost(string, tag = "3")]
    device_name: String,
    #[prost(int64, tag = "4")]
    last_modified: i64,
    #[prost(message, repeated, tag = "5")]
    playlists: Vec<LegacyProtoSyncPlaylist>,
    #[prost(message, repeated, tag = "6")]
    favorite_playlists: Vec<LegacyProtoSyncFavoritePlaylist>,
    #[prost(message, repeated, tag = "7")]
    recent_plays: Vec<LegacyProtoSyncRecentPlay>,
    #[prost(message, repeated, tag = "8")]
    sync_log: Vec<LegacyProtoSyncLogEntry>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct LegacyProtoSyncPlaylist {
    #[prost(int64, tag = "1")]
    id: i64,
    #[prost(string, tag = "2")]
    name: String,
    #[prost(message, repeated, tag = "3")]
    songs: Vec<LegacyProtoSyncSong>,
    #[prost(int64, tag = "4")]
    created_at: i64,
    #[prost(int64, tag = "5")]
    modified_at: i64,
    #[prost(bool, tag = "6")]
    is_deleted: bool,
}

#[derive(Clone, PartialEq, prost::Message)]
struct LegacyProtoSyncSong {
    #[prost(int64, tag = "1")]
    id: i64,
    #[prost(string, tag = "2")]
    name: String,
    #[prost(string, tag = "3")]
    artist: String,
    #[prost(string, tag = "4")]
    album: String,
    #[prost(int64, tag = "5")]
    album_id: i64,
    #[prost(int64, tag = "6")]
    duration_ms: i64,
    #[prost(string, optional, tag = "7")]
    cover_url: Option<String>,
    #[prost(int64, tag = "8")]
    added_at: i64,
    #[prost(string, optional, tag = "9")]
    matched_lyric: Option<String>,
    #[prost(string, optional, tag = "10")]
    matched_translated_lyric: Option<String>,
    #[prost(string, optional, tag = "11")]
    matched_lyric_source: Option<String>,
    #[prost(string, optional, tag = "12")]
    matched_song_id: Option<String>,
    #[prost(int64, tag = "13")]
    user_lyric_offset_ms: i64,
    #[prost(string, optional, tag = "14")]
    custom_cover_url: Option<String>,
    #[prost(string, optional, tag = "15")]
    custom_name: Option<String>,
    #[prost(string, optional, tag = "16")]
    custom_artist: Option<String>,
    #[prost(string, optional, tag = "17")]
    original_name: Option<String>,
    #[prost(string, optional, tag = "18")]
    original_artist: Option<String>,
    #[prost(string, optional, tag = "19")]
    original_cover_url: Option<String>,
    #[prost(string, optional, tag = "20")]
    original_lyric: Option<String>,
    #[prost(string, optional, tag = "21")]
    original_translated_lyric: Option<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct LegacyProtoSyncRecentPlay {
    #[prost(int64, tag = "1")]
    song_id: i64,
    #[prost(message, optional, tag = "2")]
    song: Option<LegacyProtoSyncSong>,
    #[prost(int64, tag = "3")]
    played_at: i64,
    #[prost(string, tag = "4")]
    device_id: String,
}

#[derive(Clone, PartialEq, prost::Message)]
struct LegacyProtoSyncFavoritePlaylist {
    #[prost(int64, tag = "1")]
    id: i64,
    #[prost(string, tag = "2")]
    name: String,
    #[prost(string, optional, tag = "3")]
    cover_url: Option<String>,
    #[prost(int32, tag = "4")]
    track_count: i32,
    #[prost(string, tag = "5")]
    source: String,
    #[prost(message, repeated, tag = "6")]
    songs: Vec<LegacyProtoSyncSong>,
    #[prost(int64, tag = "7")]
    added_time: i64,
}

#[derive(Clone, PartialEq, prost::Message)]
struct LegacyProtoSyncLogEntry {
    #[prost(int64, tag = "1")]
    timestamp: i64,
    #[prost(string, tag = "2")]
    device_id: String,
    #[prost(int32, tag = "3")]
    action: i32,
    #[prost(int64, optional, tag = "4")]
    playlist_id: Option<i64>,
    #[prost(int64, optional, tag = "5")]
    song_id: Option<i64>,
    #[prost(string, optional, tag = "6")]
    details: Option<String>,
}

fn should_use_legacy_decode(current: &SyncData, legacy: &SyncData) -> bool {
    let current_added_at = non_zero_song_added_at_count(current);
    let legacy_added_at = non_zero_song_added_at_count(legacy);
    legacy_added_at > 0 && current_added_at == 0
}

fn non_zero_song_added_at_count(data: &SyncData) -> usize {
    let playlist_songs = data.playlists.iter().flat_map(|playlist| playlist.songs.iter());
    let favorite_songs = data.favorite_playlists.iter().flat_map(|playlist| playlist.songs.iter());
    let recent_songs = data.recent_plays.iter().map(|play| &play.song);
    playlist_songs
        .chain(favorite_songs)
        .chain(recent_songs)
        .filter(|song| song.added_at != 0)
        .count()
}

fn legacy_proto_to_sync_data(p: &LegacyProtoSyncData) -> SyncData {
    SyncData {
        version: if p.version.is_empty() { "2.0".into() } else { p.version.clone() },
        device_id: p.device_id.clone(),
        device_name: p.device_name.clone(),
        last_modified: p.last_modified,
        playlists: p.playlists.iter().map(legacy_proto_to_sync_playlist).collect(),
        favorite_playlists: p.favorite_playlists.iter().map(legacy_proto_to_fav_playlist).collect(),
        recent_plays: p.recent_plays.iter().map(legacy_proto_to_recent_play).collect(),
        sync_log: p.sync_log.iter().map(legacy_proto_to_log_entry).collect(),
        recent_play_deletions: Vec::new(),
        playback_stats: Vec::new(),
        playback_stats_cleared_at: 0,
        playback_stat_buckets: Vec::new(),
        playlist_song_deletions: Vec::new(),
    }
}

fn legacy_proto_to_sync_playlist(p: &LegacyProtoSyncPlaylist) -> SyncPlaylist {
    SyncPlaylist {
        id: p.id.to_string(),
        name: p.name.clone(),
        songs: p.songs.iter().map(legacy_proto_to_sync_song).collect(),
        created_at: p.created_at,
        modified_at: p.modified_at,
        is_deleted: p.is_deleted,
        song_order_version: 0,
    }
}

fn legacy_proto_to_sync_song(p: &LegacyProtoSyncSong) -> SyncSong {
    SyncSong {
        id: p.id.to_string(),
        name: p.name.clone(),
        artist: p.artist.clone(),
        album: p.album.clone(),
        album_id: p.album_id.to_string(),
        duration_ms: p.duration_ms,
        cover_url: p.cover_url.clone().unwrap_or_default(),
        media_uri: String::new(),
        added_at: p.added_at,
        matched_lyric: p.matched_lyric.clone(),
        matched_translated_lyric: p.matched_translated_lyric.clone(),
        matched_lyric_source: p.matched_lyric_source.clone(),
        matched_song_id: p.matched_song_id.clone(),
        user_lyric_offset_ms: p.user_lyric_offset_ms,
        custom_cover_url: p.custom_cover_url.clone(),
        custom_name: p.custom_name.clone(),
        custom_artist: p.custom_artist.clone(),
        original_name: p.original_name.clone(),
        original_artist: p.original_artist.clone(),
        original_cover_url: p.original_cover_url.clone(),
        original_lyric: p.original_lyric.clone(),
        original_translated_lyric: p.original_translated_lyric.clone(),
        channel_id: None,
        audio_id: None,
        sub_audio_id: None,
        playlist_context_id: None,
        sync_membership_tokens: Vec::new(),
        sync_metadata_version: LEGACY_SYNC_METADATA_VERSION,
    }
}

fn legacy_proto_to_recent_play(p: &LegacyProtoSyncRecentPlay) -> SyncRecentPlay {
    SyncRecentPlay {
        song_id: p.song_id.to_string(),
        song: p.song.as_ref().map(legacy_proto_to_sync_song).unwrap_or_else(|| SyncSong {
            id: p.song_id.to_string(),
            ..Default::default()
        }),
        played_at: p.played_at,
        device_id: p.device_id.clone(),
    }
}

fn legacy_proto_to_fav_playlist(p: &LegacyProtoSyncFavoritePlaylist) -> SyncFavoritePlaylist {
    SyncFavoritePlaylist {
        id: p.id.to_string(),
        name: p.name.clone(),
        cover_url: p.cover_url.clone().unwrap_or_default(),
        track_count: p.track_count,
        source: p.source.clone(),
        songs: p.songs.iter().map(legacy_proto_to_sync_song).collect(),
        added_time: p.added_time,
        modified_at: p.added_time,
        is_deleted: false,
        sort_order: p.added_time,
        browse_id: None,
        playlist_id: None,
        subtitle: None,
    }
}

fn legacy_proto_to_log_entry(p: &LegacyProtoSyncLogEntry) -> SyncLogEntry {
    SyncLogEntry {
        timestamp: p.timestamp,
        device_id: p.device_id.clone(),
        action: sync_action_from_code(p.action),
        playlist_id: p.playlist_id.map(|v| v.to_string()),
        song_id: p.song_id.map(|v| v.to_string()),
        details: p.details.clone(),
    }
}

/// 空白 optional 文本编码为 None, 对齐 Android null/omit 语义, 避免 Some("") 上云
fn option_text_to_proto(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn required_text_to_proto(value: &str) -> Option<String> {
    option_text_to_proto(Some(value))
}

fn sync_song_to_proto(s: &SyncSong) -> ProtoSyncSong {
    ProtoSyncSong {
        id: sync_i64_from_string(&s.id),
        name: s.name.clone(),
        artist: s.artist.clone(),
        album: s.album.clone(),
        album_id: sync_i64_from_string(&s.album_id),
        duration_ms: s.duration_ms,
        cover_url: required_text_to_proto(&s.cover_url),
        media_uri: required_text_to_proto(&s.media_uri),
        added_at: s.added_at,
        matched_lyric: option_text_to_proto(s.matched_lyric.as_deref()),
        matched_translated_lyric: option_text_to_proto(s.matched_translated_lyric.as_deref()),
        matched_lyric_source: option_text_to_proto(s.matched_lyric_source.as_deref()),
        matched_song_id: option_text_to_proto(s.matched_song_id.as_deref()),
        user_lyric_offset_ms: s.user_lyric_offset_ms,
        custom_cover_url: option_text_to_proto(s.custom_cover_url.as_deref()),
        custom_name: option_text_to_proto(s.custom_name.as_deref()),
        custom_artist: option_text_to_proto(s.custom_artist.as_deref()),
        original_name: option_text_to_proto(s.original_name.as_deref()),
        original_artist: option_text_to_proto(s.original_artist.as_deref()),
        original_cover_url: option_text_to_proto(s.original_cover_url.as_deref()),
        original_lyric: option_text_to_proto(s.original_lyric.as_deref()),
        original_translated_lyric: option_text_to_proto(s.original_translated_lyric.as_deref()),
        channel_id: option_text_to_proto(s.channel_id.as_deref()),
        audio_id: option_text_to_proto(s.audio_id.as_deref()),
        sub_audio_id: option_text_to_proto(s.sub_audio_id.as_deref()),
        playlist_context_id: option_text_to_proto(s.playlist_context_id.as_deref()),
        sync_membership_tokens: s.sync_membership_tokens.iter().map(causal_token_to_proto).collect(),
        sync_metadata_version: s.sync_metadata_version,
    }
}

fn proto_to_sync_song(p: &ProtoSyncSong) -> SyncSong {
    SyncSong {
        id: p.id.to_string(),
        name: p.name.clone(),
        artist: p.artist.clone(),
        album: p.album.clone(),
        album_id: p.album_id.to_string(),
        duration_ms: p.duration_ms,
        cover_url: p.cover_url.clone().unwrap_or_default(),
        media_uri: p.media_uri.clone().unwrap_or_default(),
        added_at: p.added_at,
        matched_lyric: p.matched_lyric.clone(),
        matched_translated_lyric: p.matched_translated_lyric.clone(),
        matched_lyric_source: p.matched_lyric_source.clone(),
        matched_song_id: p.matched_song_id.clone(),
        user_lyric_offset_ms: p.user_lyric_offset_ms,
        custom_cover_url: p.custom_cover_url.clone(),
        custom_name: p.custom_name.clone(),
        custom_artist: p.custom_artist.clone(),
        original_cover_url: p.original_cover_url.clone(),
        original_name: p.original_name.clone(),
        original_artist: p.original_artist.clone(),
        original_lyric: p.original_lyric.clone(),
        original_translated_lyric: p.original_translated_lyric.clone(),
        channel_id: p.channel_id.clone(),
        audio_id: p.audio_id.clone(),
        sub_audio_id: p.sub_audio_id.clone(),
        playlist_context_id: p.playlist_context_id.clone(),
        sync_membership_tokens: p.sync_membership_tokens.iter().map(proto_to_causal_token).collect(),
        sync_metadata_version: p.sync_metadata_version,
    }
}

fn sync_playlist_to_proto(p: &SyncPlaylist) -> ProtoSyncPlaylist {
    ProtoSyncPlaylist {
        id: sync_i64_from_string(&p.id),
        name: p.name.clone(),
        songs: p.songs.iter().map(sync_song_to_proto).collect(),
        created_at: p.created_at,
        modified_at: p.modified_at,
        is_deleted: p.is_deleted,
        song_order_version: p.song_order_version,
    }
}

fn proto_to_sync_playlist(p: &ProtoSyncPlaylist) -> SyncPlaylist {
    SyncPlaylist {
        id: p.id.to_string(),
        name: p.name.clone(),
        songs: p.songs.iter().map(proto_to_sync_song).collect(),
        created_at: p.created_at,
        modified_at: p.modified_at,
        is_deleted: p.is_deleted,
        song_order_version: p.song_order_version,
    }
}

fn fav_playlist_to_proto(f: &SyncFavoritePlaylist) -> ProtoSyncFavoritePlaylist {
    ProtoSyncFavoritePlaylist {
        id: sync_i64_from_string(&f.id),
        name: f.name.clone(),
        cover_url: required_text_to_proto(&f.cover_url),
        track_count: f.track_count,
        source: f.source.clone(),
        songs: f.songs.iter().map(sync_song_to_proto).collect(),
        added_time: f.added_time,
        modified_at: f.modified_at,
        is_deleted: f.is_deleted,
        sort_order: f.sort_order,
        browse_id: option_text_to_proto(f.browse_id.as_deref()),
        playlist_id: option_text_to_proto(f.playlist_id.as_deref()),
        subtitle: option_text_to_proto(f.subtitle.as_deref()),
    }
}

fn proto_to_fav_playlist(p: &ProtoSyncFavoritePlaylist) -> SyncFavoritePlaylist {
    SyncFavoritePlaylist {
        id: p.id.to_string(),
        name: p.name.clone(),
        cover_url: p.cover_url.clone().unwrap_or_default(),
        track_count: p.track_count,
        source: p.source.clone(),
        songs: p.songs.iter().map(proto_to_sync_song).collect(),
        added_time: p.added_time,
        modified_at: p.modified_at,
        is_deleted: p.is_deleted,
        sort_order: p.sort_order,
        browse_id: p.browse_id.clone(),
        playlist_id: p.playlist_id.clone(),
        subtitle: p.subtitle.clone(),
    }
}

fn recent_play_to_proto(r: &SyncRecentPlay) -> ProtoSyncRecentPlay {
    ProtoSyncRecentPlay {
        song_id: sync_i64_from_string(&r.song_id),
        song: Some(sync_song_to_proto(&r.song)),
        played_at: r.played_at,
        device_id: r.device_id.clone(),
    }
}

fn proto_to_recent_play(p: &ProtoSyncRecentPlay) -> SyncRecentPlay {
    SyncRecentPlay {
        song_id: p.song_id.to_string(),
        song: p.song.as_ref().map(proto_to_sync_song).unwrap_or_else(|| SyncSong {
            id: p.song_id.to_string(),
            ..Default::default()
        }),
        played_at: p.played_at,
        device_id: p.device_id.clone(),
    }
}

fn log_entry_to_proto(e: &SyncLogEntry) -> ProtoSyncLogEntry {
    ProtoSyncLogEntry {
        timestamp: e.timestamp,
        device_id: e.device_id.clone(),
        action: sync_action_to_code(&e.action),
        playlist_id: e.playlist_id.as_ref().and_then(|s| s.parse::<i64>().ok()),
        song_id: e.song_id.as_ref().and_then(|s| s.parse::<i64>().ok()),
        details: e.details.clone(),
    }
}

fn proto_to_log_entry(p: &ProtoSyncLogEntry) -> SyncLogEntry {
    SyncLogEntry {
        timestamp: p.timestamp,
        device_id: p.device_id.clone(),
        action: sync_action_from_code(p.action),
        playlist_id: p.playlist_id.map(|v| v.to_string()),
        song_id: p.song_id.map(|v| v.to_string()),
        details: p.details.clone(),
    }
}

fn deletion_to_proto(d: &SyncRecentPlayDeletion) -> ProtoSyncRecentPlayDeletion {
    ProtoSyncRecentPlayDeletion {
        song_id: sync_i64_from_string(&d.song_id),
        album: d.album.clone(),
        media_uri: required_text_to_proto(&d.media_uri),
        deleted_at: d.deleted_at,
        device_id: d.device_id.clone(),
    }
}

fn proto_to_deletion(p: &ProtoSyncRecentPlayDeletion) -> SyncRecentPlayDeletion {
    SyncRecentPlayDeletion {
        song_id: p.song_id.to_string(),
        album: p.album.clone(),
        media_uri: p.media_uri.clone().unwrap_or_default(),
        deleted_at: p.deleted_at,
        device_id: p.device_id.clone(),
    }
}

// 新增：Android 对齐的转换函数
fn causal_token_to_proto(t: &SyncCausalToken) -> ProtoSyncCausalToken {
    ProtoSyncCausalToken {
        device_id: t.device_id.clone(),
        counter: t.counter,
    }
}

fn proto_to_causal_token(p: &ProtoSyncCausalToken) -> SyncCausalToken {
    SyncCausalToken {
        device_id: p.device_id.clone(),
        counter: p.counter,
    }
}

fn counter_shard_to_proto(s: &SyncPlaybackCounterShard) -> ProtoSyncPlaybackCounterShard {
    ProtoSyncPlaybackCounterShard {
        device_id: s.device_id.clone(),
        epoch_started_at: s.epoch_started_at,
        total_listen_ms: s.total_listen_ms,
        play_count: s.play_count,
        first_played_at: s.first_played_at,
        last_played_at: s.last_played_at,
    }
}

fn proto_to_counter_shard(p: &ProtoSyncPlaybackCounterShard) -> SyncPlaybackCounterShard {
    SyncPlaybackCounterShard {
        device_id: p.device_id.clone(),
        epoch_started_at: p.epoch_started_at,
        total_listen_ms: p.total_listen_ms,
        play_count: p.play_count,
        first_played_at: p.first_played_at,
        last_played_at: p.last_played_at,
    }
}

fn track_stat_to_proto(s: &SyncTrackStat) -> ProtoSyncTrackStat {
    ProtoSyncTrackStat {
        identity_key: s.identity_key.clone(),
        name: s.name.clone(),
        artist: s.artist.clone(),
        album: s.album.clone(),
        total_listen_ms: s.total_listen_ms,
        play_count: s.play_count,
        last_played_at: s.last_played_at,
        first_played_at: s.first_played_at,
        cover_url: option_text_to_proto(s.cover_url.as_deref()),
        duration_ms: s.duration_ms,
        media_uri: option_text_to_proto(s.media_uri.as_deref()),
        id: sync_i64_from_string(&s.id),
        album_id: sync_i64_from_string(&s.album_id),
        counter_base_listen_ms: s.counter_base_listen_ms,
        counter_base_play_count: s.counter_base_play_count,
        counter_shards: s.counter_shards.iter().map(counter_shard_to_proto).collect(),
    }
}

fn proto_to_track_stat(p: &ProtoSyncTrackStat) -> SyncTrackStat {
    SyncTrackStat {
        identity_key: p.identity_key.clone(),
        name: p.name.clone(),
        artist: p.artist.clone(),
        album: p.album.clone(),
        total_listen_ms: p.total_listen_ms,
        play_count: p.play_count,
        last_played_at: p.last_played_at,
        first_played_at: p.first_played_at,
        cover_url: p.cover_url.clone(),
        duration_ms: p.duration_ms,
        media_uri: p.media_uri.clone(),
        id: p.id.to_string(),
        album_id: p.album_id.to_string(),
        counter_base_listen_ms: p.counter_base_listen_ms,
        counter_base_play_count: p.counter_base_play_count,
        counter_shards: p.counter_shards.iter().map(proto_to_counter_shard).collect(),
    }
}

fn stat_bucket_to_proto(b: &SyncPlaybackStatBucket) -> ProtoSyncPlaybackStatBucket {
    ProtoSyncPlaybackStatBucket {
        day_start_at: b.day_start_at,
        identity_key: b.identity_key.clone(),
        name: b.name.clone(),
        artist: b.artist.clone(),
        album: b.album.clone(),
        total_listen_ms: b.total_listen_ms,
        play_count: b.play_count,
        last_played_at: b.last_played_at,
        first_played_at: b.first_played_at,
        cover_url: option_text_to_proto(b.cover_url.as_deref()),
        duration_ms: b.duration_ms,
        media_uri: option_text_to_proto(b.media_uri.as_deref()),
        id: sync_i64_from_string(&b.id),
        album_id: sync_i64_from_string(&b.album_id),
        counter_base_listen_ms: b.counter_base_listen_ms,
        counter_base_play_count: b.counter_base_play_count,
        counter_shards: b.counter_shards.iter().map(counter_shard_to_proto).collect(),
    }
}

fn proto_to_stat_bucket(p: &ProtoSyncPlaybackStatBucket) -> SyncPlaybackStatBucket {
    SyncPlaybackStatBucket {
        day_start_at: p.day_start_at,
        identity_key: p.identity_key.clone(),
        name: p.name.clone(),
        artist: p.artist.clone(),
        album: p.album.clone(),
        total_listen_ms: p.total_listen_ms,
        play_count: p.play_count,
        last_played_at: p.last_played_at,
        first_played_at: p.first_played_at,
        cover_url: p.cover_url.clone(),
        duration_ms: p.duration_ms,
        media_uri: p.media_uri.clone(),
        id: p.id.to_string(),
        album_id: p.album_id.to_string(),
        counter_base_listen_ms: p.counter_base_listen_ms,
        counter_base_play_count: p.counter_base_play_count,
        counter_shards: p.counter_shards.iter().map(proto_to_counter_shard).collect(),
    }
}

fn playlist_song_deletion_to_proto(d: &SyncPlaylistSongDeletion) -> ProtoSyncPlaylistSongDeletion {
    ProtoSyncPlaylistSongDeletion {
        playlist_id: sync_i64_from_string(&d.playlist_id),
        song_id: sync_i64_from_string(&d.song_id),
        album: d.album.clone(),
        media_uri: option_text_to_proto(d.media_uri.as_deref()),
        deleted_at: d.deleted_at,
        device_id: d.device_id.clone(),
        removed_membership_tokens: d.removed_membership_tokens.iter().map(causal_token_to_proto).collect(),
    }
}

fn proto_to_playlist_song_deletion(p: &ProtoSyncPlaylistSongDeletion) -> SyncPlaylistSongDeletion {
    SyncPlaylistSongDeletion {
        playlist_id: p.playlist_id.to_string(),
        song_id: p.song_id.to_string(),
        album: p.album.clone(),
        media_uri: if p.media_uri.as_deref() == Some("") { None } else { p.media_uri.clone() },
        deleted_at: p.deleted_at,
        device_id: p.device_id.clone(),
        removed_membership_tokens: p.removed_membership_tokens.iter().map(proto_to_causal_token).collect(),
    }
}

#[cfg(test)]
mod compressed_contract_tests {
    use super::*;

    #[test]
    fn compressed_roundtrip_preserves_android_only_song_fields_and_action() {
        let song = SyncSong {
            id: "42".into(),
            name: "Song".into(),
            album_id: "9".into(),
            matched_lyric: Some("matched".into()),
            matched_translated_lyric: Some("translated".into()),
            matched_lyric_source: Some("NETEASE".into()),
            matched_song_id: Some("song-key".into()),
            original_lyric: Some("original".into()),
            original_translated_lyric: Some("original translated".into()),
            sync_metadata_version: CURRENT_SYNC_METADATA_VERSION,
            ..Default::default()
        };
        let data = SyncData {
            version: "2.0".into(),
            device_id: "desktop".into(),
            device_name: "Desktop".into(),
            playlists: vec![SyncPlaylist {
                id: "1".into(),
                name: "Playlist".into(),
                songs: vec![song],
                created_at: 10,
                modified_at: 20,
                is_deleted: false,
                song_order_version: 1,
            }],
            sync_log: vec![SyncLogEntry {
                timestamp: 30,
                device_id: "desktop".into(),
                action: "REMOVE_SONG".into(),
                playlist_id: Some("1".into()),
                song_id: Some("42".into()),
                details: Some("removed".into()),
            }],
            ..Default::default()
        };

        let encoded = serialize_compressed(&data).unwrap();
        let decoded = deserialize_compressed(&encoded).unwrap();
        let decoded_song = &decoded.playlists[0].songs[0];

        assert_eq!(decoded_song.matched_lyric.as_deref(), Some("matched"));
        assert_eq!(decoded_song.matched_translated_lyric.as_deref(), Some("translated"));
        assert_eq!(decoded_song.matched_lyric_source.as_deref(), Some("NETEASE"));
        assert_eq!(decoded_song.matched_song_id.as_deref(), Some("song-key"));
        assert_eq!(decoded_song.original_lyric.as_deref(), Some("original"));
        assert_eq!(
            decoded_song.original_translated_lyric.as_deref(),
            Some("original translated")
        );
        assert_eq!(
            decoded_song.sync_metadata_version,
            CURRENT_SYNC_METADATA_VERSION
        );
        assert_eq!(decoded.sync_log[0].action, "REMOVE_SONG");
    }

    #[test]
    fn compressed_encode_omits_blank_optional_song_fields() {
        let song = SyncSong {
            id: "42".into(),
            name: "Song".into(),
            cover_url: "   ".into(),
            media_uri: "".into(),
            matched_lyric: Some("   ".into()),
            matched_translated_lyric: Some("".into()),
            custom_name: Some("\t".into()),
            channel_id: Some(" netease ".into()),
            audio_id: Some("42".into()),
            sync_metadata_version: CURRENT_SYNC_METADATA_VERSION,
            ..Default::default()
        };
        let data = SyncData {
            version: "2.0".into(),
            device_id: "desktop".into(),
            device_name: "Desktop".into(),
            playlists: vec![SyncPlaylist {
                id: "1".into(),
                name: "Playlist".into(),
                songs: vec![song],
                created_at: 10,
                modified_at: 20,
                is_deleted: false,
                song_order_version: 1,
            }],
            playback_stats: vec![SyncTrackStat {
                identity_key: "k".into(),
                cover_url: Some("".into()),
                media_uri: Some("   ".into()),
                ..Default::default()
            }],
            playlist_song_deletions: vec![SyncPlaylistSongDeletion {
                playlist_id: "1".into(),
                song_id: "42".into(),
                media_uri: Some("".into()),
                deleted_at: 1,
                device_id: "desktop".into(),
                ..Default::default()
            }],
            ..Default::default()
        };

        let encoded = serialize_compressed(&data).unwrap();
        let decoded = deserialize_compressed(&encoded).unwrap();
        let decoded_song = &decoded.playlists[0].songs[0];

        assert_eq!(decoded_song.cover_url, "");
        assert_eq!(decoded_song.media_uri, "");
        assert!(decoded_song.matched_lyric.is_none());
        assert!(decoded_song.matched_translated_lyric.is_none());
        assert!(decoded_song.custom_name.is_none());
        assert_eq!(decoded_song.channel_id.as_deref(), Some("netease"));
        assert_eq!(decoded_song.audio_id.as_deref(), Some("42"));
        assert!(decoded.playback_stats[0].cover_url.is_none());
        assert!(decoded.playback_stats[0].media_uri.is_none());
        assert!(decoded.playlist_song_deletions[0].media_uri.is_none());
    }

    #[test]
    fn compressed_decode_accepts_legacy_android_song_field_numbers() {
        let legacy = LegacyProtoSyncData {
            version: "2.0".into(),
            device_id: "android".into(),
            device_name: "Android".into(),
            last_modified: 100,
            playlists: vec![LegacyProtoSyncPlaylist {
                id: 7,
                name: "Legacy".into(),
                songs: vec![LegacyProtoSyncSong {
                    id: 42,
                    name: "Song".into(),
                    artist: "Artist".into(),
                    album: "Album".into(),
                    album_id: 9,
                    duration_ms: 1234,
                    cover_url: Some("https://cover".into()),
                    added_at: 88,
                    matched_lyric: Some("matched".into()),
                    matched_translated_lyric: Some("translated".into()),
                    matched_lyric_source: Some("NETEASE".into()),
                    matched_song_id: Some("song-key".into()),
                    user_lyric_offset_ms: 12,
                    custom_cover_url: Some("https://custom".into()),
                    custom_name: Some("Custom".into()),
                    custom_artist: Some("Custom Artist".into()),
                    original_name: Some("Original".into()),
                    original_artist: Some("Original Artist".into()),
                    original_cover_url: Some("https://original".into()),
                    original_lyric: Some("original lyric".into()),
                    original_translated_lyric: Some("original translated".into()),
                }],
                created_at: 10,
                modified_at: 20,
                is_deleted: false,
            }],
            favorite_playlists: Vec::new(),
            recent_plays: Vec::new(),
            sync_log: vec![LegacyProtoSyncLogEntry {
                timestamp: 30,
                device_id: "android".into(),
                action: sync_action_to_code("ADD_SONG"),
                playlist_id: Some(7),
                song_id: Some(42),
                details: Some("legacy".into()),
            }],
        };
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&legacy.encode_to_vec()).unwrap();
        let encoded = BASE64.encode(encoder.finish().unwrap());

        let decoded = deserialize_compressed(&encoded).unwrap();
        let decoded_song = &decoded.playlists[0].songs[0];

        assert_eq!(decoded.playlists[0].song_order_version, 0);
        assert_eq!(decoded_song.id, "42");
        assert_eq!(decoded_song.added_at, 88);
        assert_eq!(decoded_song.media_uri, "");
        assert_eq!(decoded_song.matched_lyric.as_deref(), Some("matched"));
        assert_eq!(decoded_song.matched_translated_lyric.as_deref(), Some("translated"));
        assert_eq!(decoded_song.matched_lyric_source.as_deref(), Some("NETEASE"));
        assert_eq!(decoded_song.matched_song_id.as_deref(), Some("song-key"));
        assert_eq!(decoded_song.user_lyric_offset_ms, 12);
        assert_eq!(
            decoded_song.sync_metadata_version,
            LEGACY_SYNC_METADATA_VERSION
        );
        assert_eq!(decoded_song.original_lyric.as_deref(), Some("original lyric"));
        assert_eq!(
            decoded_song.original_translated_lyric.as_deref(),
            Some("original translated")
        );
        assert_eq!(decoded.sync_log[0].action, "ADD_SONG");
    }

    #[test]
    fn compressed_decode_rejects_payload_over_android_size_limit() {
        let oversized = vec![0_u8; MAX_DECOMPRESSED_BYTES + 1];
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&oversized).unwrap();
        let encoded = BASE64.encode(encoder.finish().unwrap());

        let error = deserialize_compressed(&encoded).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("Decompressed sync data is too large"),
            "unexpected error: {error}"
        );
    }
}
