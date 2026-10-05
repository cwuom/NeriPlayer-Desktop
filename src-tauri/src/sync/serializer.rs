// 省流模式序列化/反序列化：与 Android SyncDataSerializer.kt 对齐
//
// 传输产物统一为「原始字节」，不再叠加 Base64：
// - 省流（backup.bin）: 原始 GZIP(ProtoBuf) 字节
// - 普通（backup.json）: UTF-8 JSON 字节
//
// 读取按内容自动识别（read-both），三种在野格式都要能读：
// 1. GZIP 魔数 0x1F 0x8B 开头 -> 直接解压 -> ProtoBuf（新 raw 格式）
// 2. 首个有效字节为 '{' -> JSON（旧 backup.json / 旧 WebDAV JSON）
// 3. 其余文本 -> 旧 Base64(GZIP(ProtoBuf)) -> 解码 -> 解压（旧 backup.bin）
//
// 对端未更新时读到新格式会安全失败（同步空转，不覆盖本地、不回流），
// 而新版永远读得懂旧备份。详见 docs/SYNC-MODEL-CONTRACT.md

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use prost::Message;
use std::io::{Read, Write};

use crate::error::{AppError, AppResult};
use super::models::*;
use super::proto_models::*;

/// 压缩正文上限，与 Android `MAX_COMPRESSED_BYTES` 一致
const MAX_COMPRESSED_BYTES: usize = 12 * 1024 * 1024;

/// JSON 正文上限，与 Android `MAX_JSON_BYTES` 一致
const MAX_JSON_BYTES: usize = 8 * 1024 * 1024;

/// 解压后上限，与 Android `MAX_DECOMPRESSED_BYTES` 一致
///
/// 没有这个护栏，损坏或恶意构造的 GZIP 能把几百 KB 膨胀成几 GB，
/// 一次同步就把内存吃光。`read_to_end` 本身不设上限，必须靠 `take` 截断。
const MAX_DECOMPRESSED_BYTES: u64 = 16 * 1024 * 1024;

/// 返回省流模式使用的文件名
pub fn get_filename(data_saver: bool) -> &'static str {
    if data_saver { "backup.bin" } else { "backup.json" }
}

/// 序列化 SyncData（省流模式: ProtoBuf + GZIP，原始字节不再叠 Base64）
pub fn serialize_compressed(data: &SyncData) -> AppResult<Vec<u8>> {
    let normalized = data.normalized_for_sync();
    let proto = sync_data_to_proto(&normalized);
    let mut projected = proto_to_sync_data(&proto);
    projected.extensions = normalized.extensions;
    let proto_bytes = super::archive::encode_legacy_proto(&projected)?;

    // GZIP 压缩
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&proto_bytes)
        .map_err(|e| AppError::Other(format!("GZIP compress: {}", e)))?;
    let compressed = encoder.finish()
        .map_err(|e| AppError::Other(format!("GZIP finish: {}", e)))?;

    Ok(compressed)
}

/// 远端正文体积护栏，按内容类型选上限（对齐 Android `ensureRemoteContentSize`）
///
/// 放在 deserialize 入口而不是各传输层：GitHub、WebDAV 以及以后新增的通道
/// 都会自动受保护，不必每处各写一遍、也就不会漏掉某一处。
pub fn ensure_remote_content_size(content: &[u8]) -> AppResult<()> {
    let max = if looks_like_json(content) {
        MAX_JSON_BYTES
    } else {
        MAX_COMPRESSED_BYTES
    };
    if content.len() > max {
        return Err(AppError::Other(format!(
            "remote sync data is too large: {} bytes (limit {})",
            content.len(),
            max
        )));
    }
    Ok(())
}

/// 原始 GZIP 字节以魔数 0x1F 0x8B 开头，用于区分新 raw 格式与历史文本格式
fn looks_like_gzip(bytes: &[u8]) -> bool {
    matches!(bytes, [0x1F, 0x8B, ..])
}

/// 跳过前导 UTF-8 BOM 与空白后，首个有效字节为 '{' 即视为 JSON 对象
fn looks_like_json(bytes: &[u8]) -> bool {
    let rest = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    matches!(
        rest.iter().find(|byte| !matches!(byte, b' ' | b'\n' | b'\r' | b'\t')),
        Some(b'{')
    )
}

/// 反序列化省流模式数据（GZIP -> ProtoBuf -> SyncData）
pub fn deserialize_compressed(compressed: &[u8]) -> AppResult<SyncData> {
    if compressed.len() > MAX_COMPRESSED_BYTES {
        return Err(AppError::Other(format!(
            "compressed backup is too large: {} bytes",
            compressed.len()
        )));
    }

    // GZIP 解压；多读 1 字节用于判定是否越界，避免把超限数据当成正常内容
    let mut proto_bytes = Vec::new();
    GzDecoder::new(compressed)
        .take(MAX_DECOMPRESSED_BYTES + 1)
        .read_to_end(&mut proto_bytes)
        .map_err(|e| AppError::Other(format!("GZIP decompress: {}", e)))?;
    if proto_bytes.len() as u64 > MAX_DECOMPRESSED_BYTES {
        return Err(AppError::Other(format!(
            "decompressed backup exceeds {} bytes",
            MAX_DECOMPRESSED_BYTES
        )));
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
            super::archive::decode_legacy_proto(&proto_bytes).map(|data| data.normalized_for_sync())
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

/// 上传体积护栏，与下载侧 `ensure_remote_content_size` 同一组上限
///
/// 下载有 8/12MB 护栏而上传没有时，超限备份会被成功推上云端，
/// 之后所有设备（含本机）拉取一律失败——同步从此自锁。必须在
/// 上传前用同一标准拒绝，并明确告知实际体积与上限。
fn ensure_upload_content_size(content: &[u8], data_saver: bool) -> AppResult<()> {
    let (max, kind) = if data_saver {
        (MAX_COMPRESSED_BYTES, "compressed")
    } else {
        (MAX_JSON_BYTES, "JSON")
    };
    if content.len() > max {
        return Err(AppError::Other(format!(
            "sync backup is too large to upload: {} bytes ({} limit is {} bytes); \
             remote data is left untouched",
            content.len(),
            kind,
            max
        )));
    }
    Ok(())
}

/// 根据 data_saver 标志选择序列化方式，产物一律是原始字节
pub fn serialize(data: &SyncData, data_saver: bool) -> AppResult<Vec<u8>> {
    let content = if data_saver {
        serialize_compressed(data)?
    } else {
        serde_json::to_string_pretty(&data.normalized_for_sync())
            .map(String::into_bytes)
            .map_err(|e| AppError::Other(format!("JSON serialize: {}", e)))?
    };
    ensure_upload_content_size(&content, data_saver)?;
    Ok(content)
}

/// 按内容自动识别格式并反序列化（read-both）
///
/// 不看文件后缀：后缀只说明「本端打算写成什么」，而远端那份可能是对端
/// 旧版本写的，甚至是用户手动放上去的。按内容判别才能真正做到读旧读新都不炸。
pub fn deserialize(content: &[u8]) -> AppResult<SyncData> {
    ensure_remote_content_size(content)?;
    if looks_like_gzip(content) {
        return deserialize_compressed(content);
    }
    if looks_like_json(content) {
        // 认出来之后必须把 BOM 去掉再解析：serde_json 不接受前导 U+FEFF
        let body = content.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(content);
        let text = std::str::from_utf8(body)
            .map_err(|e| AppError::Other(format!("JSON is not UTF-8: {}", e)))?;
        return serde_json::from_str::<serde_json::Value>(text)
            .map_err(AppError::from)
            .and_then(super::archive::decode_legacy_json)
            .map(|data| data.normalized_for_sync())
            .map_err(|e| AppError::Other(format!("JSON parse: {}", e)));
    }
    // 旧 backup.bin：Base64(GZIP(ProtoBuf))
    let text = std::str::from_utf8(content)
        .map_err(|e| AppError::Other(format!("legacy payload is not UTF-8: {}", e)))?;
    let cleaned: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let compressed = BASE64
        .decode(&cleaned)
        .map_err(|e| AppError::Other(format!("Base64 decode: {}", e)))?;
    deserialize_compressed(&compressed)
}

// Android 端 kotlinx.serialization 把「无默认值的属性」视为必填, 缺字段直接抛
// MissingFieldException; 而 prost 遵循 proto3 语义, 标量等于默认值时不写入报文。
// 两者叠加会让「桌面写出的合法报文对端解不开」, 因此编码前必须保证这些字段非零,
// 无法保证的记录只能整条丢弃。详见 docs/SYNC-MODEL-CONTRACT.md §3.1
const FALLBACK_DEVICE_ID: &str = "neriplayer-desktop";
const FALLBACK_DEVICE_NAME: &str = "NeriPlayer Desktop";

fn non_empty_or(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

/// 日志条目的 timestamp / deviceId / action 在 Android 侧均为必填,
/// 其中 action 的 CREATE_PLAYLIST 序数为 0, prost 会省略该 tag。
/// syncLog 只是排障辅助数据, 丢弃安全, 优先保证对端能解码。
fn log_entry_is_proto_safe(entry: &SyncLogEntry) -> bool {
    entry.timestamp > 0
        && !entry.device_id.trim().is_empty()
        && sync_action_to_code(&entry.action) != 0
}

// Proto <-> SyncData 转换
fn sync_data_to_proto(data: &SyncData) -> ProtoSyncData {
    ProtoSyncData {
        version: non_empty_or(&data.version, "2.0"),
        device_id: non_empty_or(&data.device_id, FALLBACK_DEVICE_ID),
        device_name: non_empty_or(&data.device_name, FALLBACK_DEVICE_NAME),
        last_modified: data.last_modified,
        playlists: data
            .playlists
            .iter()
            .filter(|playlist| sync_i64_from_string(&playlist.id) != 0)
            .map(sync_playlist_to_proto)
            .collect(),
        favorite_playlists: data
            .favorite_playlists
            .iter()
            .filter(|playlist| sync_i64_from_string(&playlist.id) != 0)
            .map(fav_playlist_to_proto)
            .collect(),
        recent_plays: data.recent_plays.iter().map(recent_play_to_proto).collect(),
        sync_log: data
            .sync_log
            .iter()
            .filter(|entry| log_entry_is_proto_safe(entry))
            .map(log_entry_to_proto)
            .collect(),
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
        extensions: Default::default(),
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
        extensions: Default::default(),
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
        legacy_added_at: None,
        ..Default::default()
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
        resume_position_ms: 0,
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
        legacy_added_at: s.legacy_added_at,
        lyric_sync_revision: s.lyric_sync_revision,
        lyric_sync_edited: s.lyric_sync_edited,
        matched_romanized_lyric: s.matched_romanized_lyric.clone(),
        original_romanized_lyric: s.original_romanized_lyric.clone(),
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
        legacy_added_at: p.legacy_added_at,
        lyric_sync_revision: p.lyric_sync_revision,
        lyric_sync_edited: p.lyric_sync_edited,
        matched_romanized_lyric: p.matched_romanized_lyric.clone(),
        original_romanized_lyric: p.original_romanized_lyric.clone(),
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
        resume_position_ms: r.resume_position_ms,
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
        resume_position_ms: p.resume_position_ms,
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

    fn sample_sync_data() -> SyncData {
        let mut data = SyncData {
            device_id: "desktop-test".into(),
            device_name: "Desktop".into(),
            last_modified: 1_700_000_000_000,
            ..Default::default()
        };
        data.playlists.push(SyncPlaylist {
            id: "101".into(),
            name: "跨端歌单".into(),
            songs: Vec::new(),
            created_at: 1_600_000_000_000,
            modified_at: 1_700_000_000_000,
            is_deleted: false,
            song_order_version: DISPLAY_ORDER_SONG_ORDER_VERSION,
        });
        data
    }

    /// read-both：三种在野格式都必须读得懂
    ///
    /// Android 89dcc8f1 起 backup.bin 改成原始 GZIP(ProtoBuf)，不再叠 Base64。
    /// 但云端可能还躺着任意一端旧版本写的备份，任何一种读不了都会让同步中断，
    /// 甚至被误判成「远端损坏」。
    #[test]
    fn deserialize_reads_raw_gzip_json_and_legacy_base64() {
        let data = sample_sync_data();

        // 1. 新格式：原始 GZIP(ProtoBuf) 字节
        let raw = serialize(&data, true).unwrap();
        assert_eq!(&raw[..2], &[0x1F, 0x8B], "省流产物必须是裸 GZIP，不能再套 Base64");
        assert_eq!(deserialize(&raw).unwrap().playlists[0].name, "跨端歌单");

        // 2. JSON 文本（旧 backup.json / 旧 WebDAV）
        let json = serialize(&data, false).unwrap();
        assert_eq!(json[0], b'{');
        assert_eq!(deserialize(&json).unwrap().playlists[0].name, "跨端歌单");

        // 3. 旧 backup.bin：Base64(GZIP(ProtoBuf))
        let legacy = BASE64.encode(&raw);
        assert_eq!(
            deserialize(legacy.as_bytes()).unwrap().playlists[0].name,
            "跨端歌单",
        );
    }

    /// 上传前必须套用与下载相同的体积护栏
    ///
    /// 下载有护栏而上传没有时，超限备份会被推上云端，
    /// 之后所有设备拉取一律失败，同步永久自锁。
    #[test]
    fn serialize_rejects_payloads_exceeding_download_guardrails() {
        let mut data = sample_sync_data();
        // 用一段远超 8MB 的不可压缩前歌词字段撑爆 JSON 上限
        let huge = "字".repeat(4 * 1024 * 1024);
        data.playlists[0].songs.push(SyncSong {
            id: "42".into(),
            name: "Huge".into(),
            album: "netease".into(),
            matched_lyric: Some(huge),
            ..Default::default()
        });

        let error = serialize(&data, false).unwrap_err().to_string();

        assert!(error.contains("too large to upload"), "unexpected error: {error}");
        assert!(error.contains("8388608"), "error should carry the limit: {error}");
    }

    /// 带 BOM 与前导空白的 JSON 仍要认出来
    ///
    /// 认错会落进 Base64 兜底分支，报出与真实原因无关的解码错误。
    #[test]
    fn json_detection_tolerates_bom_and_leading_whitespace() {
        let json = serialize(&sample_sync_data(), false).unwrap();

        let mut with_bom = vec![0xEF, 0xBB, 0xBF];
        with_bom.extend_from_slice(&json);
        assert!(deserialize(&with_bom).is_ok());

        let mut padded = b"\n\r\t  ".to_vec();
        padded.extend_from_slice(&json);
        assert!(deserialize(&padded).is_ok());
    }

    /// 损坏的远端报文必须报错，绝不能静默返回空数据
    ///
    /// 一旦空数据被当成「远端就是空的」参与合并，本地内容会被判成
    /// 「本端新增」反复上传，甚至在对端把已删除的条目复活——即回流。
    #[test]
    fn corrupt_payload_fails_instead_of_yielding_empty_data() {
        assert!(deserialize(&[0x1F, 0x8B, 0x08, 0x00, 0xDE, 0xAD]).is_err(), "坏 GZIP");
        assert!(deserialize(b"{not json at all").is_err(), "坏 JSON");
        assert!(deserialize(b"!!!not base64!!!").is_err(), "坏 Base64");
    }

    /// 压缩炸弹必须被挡住，而不是把内存吃光
    ///
    /// Android 有 MAX_DECOMPRESSED_BYTES，桌面此前没有等价护栏：
    /// read_to_end 不设上限，几百 KB 的 GZIP 能膨胀到几 GB。
    #[test]
    fn decompression_bomb_is_rejected_by_the_size_guard() {
        // 17 MB 的零字节压完只有几十 KB，正好越过 16 MB 上限
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&vec![0_u8; 17 * 1024 * 1024]).unwrap();
        let bomb = encoder.finish().unwrap();
        assert!(bomb.len() < 128 * 1024, "构造前提：压缩后应远小于解压后");

        let error = deserialize(&bomb).unwrap_err().to_string();
        assert!(error.contains("exceeds"), "{error}");
    }

    /// 超大正文按内容类型分别设限（对齐 Android ensureRemoteContentSize）
    #[test]
    fn oversized_payloads_are_rejected_by_content_type() {
        // JSON 上限 8 MB
        let mut huge_json = b"{\"playlists\":[".to_vec();
        huge_json.resize(MAX_JSON_BYTES + 1, b' ');
        let error = deserialize(&huge_json).unwrap_err().to_string();
        assert!(error.contains("too large"), "{error}");

        // 非 JSON 走压缩档，上限 12 MB
        let mut huge_binary = vec![0x1F, 0x8B];
        huge_binary.resize(MAX_COMPRESSED_BYTES + 1, 0);
        let error = deserialize(&huge_binary).unwrap_err().to_string();
        assert!(error.contains("too large"), "{error}");

        // 8~12 MB 之间的二进制不该被 JSON 档误伤
        let mut mid_binary = vec![0x1F, 0x8B];
        mid_binary.resize(MAX_JSON_BYTES + 1, 0);
        let error = deserialize(&mid_binary).unwrap_err().to_string();
        assert!(!error.contains("too large"), "不应按 JSON 档拒收: {error}");
    }

    /// 去掉 Base64 层后体积必须真的变小
    #[test]
    fn raw_payload_is_smaller_than_the_legacy_base64_form() {
        let raw = serialize(&sample_sync_data(), true).unwrap();
        let legacy_len = BASE64.encode(&raw).len();
        assert!(raw.len() < legacy_len, "raw={} legacy={}", raw.len(), legacy_len);
    }

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
            legacy_added_at: Some(1_234),
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
        assert_eq!(decoded_song.legacy_added_at, Some(1_234));
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
    fn compressed_encode_drops_records_that_would_omit_android_required_fields() {
        let data = SyncData {
            version: String::new(),
            device_id: "  ".into(),
            device_name: String::new(),
            playlists: vec![
                SyncPlaylist {
                    id: "not-a-number".into(),
                    name: "Dropped".into(),
                    songs: Vec::new(),
                    created_at: 1,
                    modified_at: 2,
                    is_deleted: false,
                    song_order_version: 1,
                },
                SyncPlaylist {
                    id: "7".into(),
                    name: "Kept".into(),
                    songs: Vec::new(),
                    created_at: 1,
                    modified_at: 2,
                    is_deleted: false,
                    song_order_version: 1,
                },
            ],
            favorite_playlists: vec![SyncFavoritePlaylist {
                id: "0".into(),
                name: "Dropped".into(),
                cover_url: String::new(),
                track_count: 0,
                source: "netease".into(),
                songs: Vec::new(),
                added_time: 1,
                modified_at: 1,
                is_deleted: false,
                sort_order: 1,
                browse_id: None,
                playlist_id: None,
                subtitle: None,
            }],
            sync_log: vec![
                // action 序数为 0, prost 会省略 tag 3 -> Android 必填校验失败
                SyncLogEntry {
                    timestamp: 10,
                    device_id: "desktop".into(),
                    action: "CREATE_PLAYLIST".into(),
                    playlist_id: None,
                    song_id: None,
                    details: None,
                },
                SyncLogEntry {
                    timestamp: 0,
                    device_id: "desktop".into(),
                    action: "ADD_SONG".into(),
                    playlist_id: None,
                    song_id: None,
                    details: None,
                },
                SyncLogEntry {
                    timestamp: 20,
                    device_id: "  ".into(),
                    action: "ADD_SONG".into(),
                    playlist_id: None,
                    song_id: None,
                    details: None,
                },
                SyncLogEntry {
                    timestamp: 30,
                    device_id: "desktop".into(),
                    action: "REMOVE_SONG".into(),
                    playlist_id: None,
                    song_id: None,
                    details: None,
                },
            ],
            ..Default::default()
        };

        let encoded = serialize_compressed(&data).unwrap();
        let decoded = deserialize_compressed(&encoded).unwrap();

        assert_eq!(decoded.version, "2.0");
        assert_eq!(decoded.device_id, "neriplayer-desktop");
        assert_eq!(decoded.device_name, "NeriPlayer Desktop");
        assert_eq!(decoded.playlists.len(), 1);
        assert_eq!(decoded.playlists[0].id, "7");
        assert!(decoded.favorite_playlists.is_empty());
        assert_eq!(decoded.sync_log.len(), 1);
        assert_eq!(decoded.sync_log[0].action, "REMOVE_SONG");
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
        // 走旧 backup.bin 的 Base64 文本形态，验证 read-both 的兜底分支
        let encoded = BASE64.encode(encoder.finish().unwrap());

        let decoded = deserialize(encoded.as_bytes()).unwrap();
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
}
