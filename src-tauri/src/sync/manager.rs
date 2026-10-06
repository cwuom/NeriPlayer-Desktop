// 同步管理器：协调 GitHub/WebDAV 同步流程
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tokio::sync::{Mutex as TokioMutex, MutexGuard};
use crate::error::{AppError, AppResult};
use crate::state::{TrackInfo, TrackSource};
use crate::library::playlist::{self, Playlist, PlaylistStore};
use super::models::*;
#[cfg(test)]
use super::serializer;
#[cfg(test)]
use super::github_api::GitHubApiClient;
use super::merge;

static SYNC_LOCK: OnceLock<TokioMutex<()>> = OnceLock::new();
static SYNC_CAUSAL_TOKEN_LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();

async fn acquire_sync_lock() -> MutexGuard<'static, ()> {
    SYNC_LOCK
        .get_or_init(|| TokioMutex::new(()))
        .lock()
        .await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncHistoryEntry {
    pub track: TrackInfo,
    pub played_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncHistoryDeletion {
    pub track: TrackInfo,
    pub deleted_at: i64,
}

/// 本地播放统计快照，由 stats 模块产出后注入同步信封
#[derive(Debug, Clone, Default)]
pub struct SyncStatsPayload {
    pub stats: Vec<SyncTrackStat>,
    pub buckets: Vec<SyncPlaybackStatBucket>,
    pub cleared_at: i64,
}

/// 同步产物：调用方需要合并结果才能把统计回写本地
pub struct SyncOutcome {
    pub result: SyncResult,
    pub merged: SyncData,
    /// 合并结果相对本地数据是否有实际内容变化
    ///
    /// 命令层据此决定要不要 emit `playlists-changed`：无条件 emit 会被
    /// 前端的"事件 → 防抖自动同步"监听放大成 5s 自激同步环
    pub local_changed: bool,
}

/// GitHub 归档以单个分支 CAS 发布完整对象闭包
pub(crate) async fn sync_github(
    http: &reqwest::Client,
    config: &GitHubSyncConfig,
    local_data: &SyncData,
    playlist_epoch: u64,
    finish: impl FnOnce(super::cloud::Completed) -> AppResult<SyncOutcome>,
) -> AppResult<SyncOutcome> {
    let _guard = acquire_sync_lock().await;
    let completed = super::cloud::github(http, config, local_data, playlist_epoch).await?;
    finish(completed)
}

/// 严格读取远端快照，仅当两种格式都 404 时才视为首次同步
/// 远端正文是否为空
///
/// 不能对字节直接 `trim`：省流备份是 GZIP 二进制。只判「空或全是 ASCII 空白」，
/// 二进制正文里出现的 0x20 之类字节不会让整份被误判成空。
#[cfg(test)]
fn is_blank_payload(content: &[u8]) -> bool {
    content
        .iter()
        .all(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
}

#[cfg(test)]
async fn fetch_remote_snapshot(
    api: &GitHubApiClient,
    config: &GitHubSyncConfig,
    preferred_file: &str,
    data_saver: bool,
) -> AppResult<Option<SyncData>> {
    let alternative_file = serializer::get_filename(!data_saver);
    let (content, _sha, _actual_file) = match api
        .get_file_content(&config.owner, &config.repo, preferred_file)
        .await
        .map_err(AppError::from)?
    {
        Some((content, sha)) => (content, sha, preferred_file.to_string()),
        None => match api
            .get_file_content(&config.owner, &config.repo, alternative_file)
            .await
            .map_err(AppError::from)?
        {
            Some((content, sha)) => (content, sha, alternative_file.to_string()),
            None => return Ok(None),
        },
    };

    if is_blank_payload(&content) {
        return Err(AppError::Other("Remote backup file is empty".into()));
    }

    let data = serializer::deserialize(&content)?;
    Ok(Some(data))
}

/// WebDAV 在条件清单发布和租约释放确认后推进本地状态
pub(crate) async fn sync_webdav(
    http: &reqwest::Client,
    config: &WebDavSyncConfig,
    local_data: &SyncData,
    playlist_epoch: u64,
    finish: impl FnOnce(super::cloud::Completed) -> AppResult<SyncOutcome>,
) -> AppResult<SyncOutcome> {
    let _guard = acquire_sync_lock().await;
    let completed = super::cloud::webdav(http, config, local_data, playlist_epoch).await?;
    finish(completed)
}

pub(crate) fn complete_cloud_sync(
    completed: &super::cloud::Completed,
    local: &SyncData,
    epoch: u64,
) -> AppResult<SyncOutcome> {
    ensure_local_playlist_epoch(epoch)?;
    save_archive_metadata(&completed.merged)?;
    save_synced_playlists_if_epoch(&completed.merged, epoch)?;
    save_recent_play_history(&completed.merged)?;
    save_base_snapshot(&completed.merged, &completed.scope)?;
    let previous_playlists = completed
        .remote
        .as_ref()
        .map(|data| data.playlists.len())
        .unwrap_or(0);
    let previous_songs = completed
        .remote
        .as_ref()
        .map(|data| {
            data.playlists
                .iter()
                .map(|playlist| playlist.songs.len())
                .sum::<usize>()
        })
        .unwrap_or(0);
    let result = with_history(
        SyncResult {
            success: true,
            message: if !completed.uploaded {
                "Already up to date"
            } else if completed.remote.is_none() {
                "Initial upload complete"
            } else {
                "Sync complete"
            }
            .into(),
            playlists_added: completed
                .merged
                .playlists
                .len()
                .saturating_sub(previous_playlists) as i32,
            songs_added: completed
                .merged
                .playlists
                .iter()
                .map(|playlist| playlist.songs.len())
                .sum::<usize>()
                .saturating_sub(previous_songs) as i32,
            ..Default::default()
        },
        &completed.merged,
    );
    Ok(SyncOutcome {
        result,
        merged: completed.merged.clone(),
        local_changed: merge::has_data_changed(local, &completed.merged),
    })
}

pub(crate) fn sync_extensions_path() -> std::path::PathBuf {
    dirs_next::data_dir().unwrap_or_else(|| std::path::PathBuf::from(".")).join("NeriPlayer").join("sync-android-metadata.json")
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all="camelCase")]
struct PersistedArchiveMetadata {
    #[serde(default)] extensions: serde_json::Map<String, serde_json::Value>,
    #[serde(default)] playback_stats: Vec<SyncTrackStat>,
    #[serde(default)] playback_stat_buckets: Vec<SyncPlaybackStatBucket>,
    #[serde(default)] playback_stats_cleared_at: i64,
}

pub(crate) fn load_saved_stats_provenance(path: &std::path::Path) -> AppResult<SyncStatsPayload> {
    let metadata: PersistedArchiveMetadata =
        read_optional_json(path, "Android sync metadata")?.unwrap_or_default();
    Ok(SyncStatsPayload {
        stats: metadata.playback_stats,
        buckets: metadata.playback_stat_buckets,
        cleared_at: metadata.playback_stats_cleared_at,
    })
}

fn save_archive_metadata(data: &SyncData) -> AppResult<()> {
    let metadata=PersistedArchiveMetadata {extensions:data.extensions.clone(),playback_stats:data.playback_stats.clone(),playback_stat_buckets:data.playback_stat_buckets.clone(),playback_stats_cleared_at:data.playback_stats_cleared_at};
    crate::fsutil::atomic_write(sync_extensions_path(), serde_json::to_vec(&metadata)?)?;
    Ok(())
}

/// 构建本地同步数据（从 tauri-plugin-store 读取歌单等）
pub fn build_local_sync_data(
    app: &AppHandle,
    history_entries: Option<&[SyncHistoryEntry]>,
    history_deletions: Option<&[SyncHistoryDeletion]>,
    stats: Option<SyncStatsPayload>,
) -> AppResult<SyncData> {
    // 从 store 读取本地歌单数据
    // 当前歌单系统使用文件存储，构建 SyncData
    let device_id = get_or_create_device_id(app);
    let hostname = whoami::fallible::hostname().unwrap_or_else(|_| "Desktop".into());

    let stored_history = Some(load_recent_play_history()?);
    let mut recent_plays = history_entries
        .map(|entries| history_entries_to_sync(entries, &device_id))
        .unwrap_or_else(|| {
            stored_history
                .as_ref()
                .map(|history| history.recent_plays.clone())
                .unwrap_or_default()
        });
    let mut recent_play_deletions = history_deletions
        .map(|deletions| history_deletions_to_sync(deletions, &device_id))
        .unwrap_or_else(|| {
            stored_history
                .as_ref()
                .map(|history| history.recent_play_deletions.clone())
                .unwrap_or_default()
        });
    let stored = stored_history.as_ref().unwrap();
    preserve_recent_progress(&mut recent_plays, &stored.recent_plays);
    recent_play_deletions = merge::merge_recent_play_deletions(&recent_play_deletions, &stored.recent_play_deletions);
    recent_plays = merge::merge_recent_plays(&recent_plays, &stored.recent_plays, &recent_play_deletions);
    let stats = stats.unwrap_or_default();
    let metadata:PersistedArchiveMetadata=read_optional_json(&sync_extensions_path(),"Android sync metadata")?.unwrap_or_default();
    let cleared_at=stats.cleared_at.max(metadata.playback_stats_cleared_at);
    let buckets=merge::merge_stat_buckets(&stats.buckets,&metadata.playback_stat_buckets,cleared_at);
    let merged_stats=merge::merge_playback_stats(&stats.stats,&metadata.playback_stats,cleared_at);

    Ok(SyncData {
        version: "2.0".into(),
        device_id,
        device_name: format!("NeriPlayer Desktop ({})", hostname),
        last_modified: chrono::Utc::now().timestamp_millis(),
        playlists: load_local_playlists(app)?,
        favorite_playlists: load_favorites_at(&favorites_path(), true)?,
        recent_plays,
        sync_log: Vec::new(),
        recent_play_deletions,
        playback_stats: merge::lift_stats_to_bucket_totals(&merged_stats,&buckets),
        playback_stats_cleared_at: cleared_at,
        playback_stat_buckets: buckets,
        playlist_song_deletions: load_local_playlist_song_deletions()?,
        extensions: metadata.extensions,
    })
}

fn history_entries_to_sync(entries: &[SyncHistoryEntry], device_id: &str) -> Vec<SyncRecentPlay> {
    entries
        .iter()
        .filter(|entry| entry.track.source != TrackSource::Local && !entry.track.id.is_empty())
        .map(|entry| {
            let song = track_to_sync_song(&entry.track);
            SyncRecentPlay {
                song_id: song.id.clone(),
                song,
                played_at: entry.played_at.max(0),
                device_id: device_id.to_string(),
                resume_position_ms: 0,
            }
        })
        .collect()
}

fn preserve_recent_progress(entries: &mut [SyncRecentPlay], stored: &[SyncRecentPlay]) {
    for entry in entries {
        if let Some(previous) = stored.iter().find(|previous| previous.played_at == entry.played_at && previous.song.identity().stable_key() == entry.song.identity().stable_key()) {
            entry.resume_position_ms = previous.resume_position_ms;
            entry.device_id = previous.device_id.clone();
        }
    }
}

fn history_deletions_to_sync(
    deletions: &[SyncHistoryDeletion],
    device_id: &str,
) -> Vec<SyncRecentPlayDeletion> {
    deletions
        .iter()
        .filter(|deletion| deletion.track.source != TrackSource::Local && !deletion.track.id.is_empty())
        .map(|deletion| {
            let song = track_to_sync_song(&deletion.track);
            SyncRecentPlayDeletion {
                song_id: song.id,
                album: song.album,
                media_uri: song.media_uri,
                deleted_at: deletion.deleted_at.max(0),
                device_id: device_id.to_string(),
            }
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PersistedRecentPlayHistory {
    #[serde(default)]
    recent_plays: Vec<SyncRecentPlay>,
    #[serde(default)]
    recent_play_deletions: Vec<SyncRecentPlayDeletion>,
}

fn recent_play_history_path() -> std::path::PathBuf {
    let mut path = dirs_next::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    path.push("NeriPlayer");
    path.push("recent-play-history.json");
    path
}

fn load_recent_play_history() -> AppResult<PersistedRecentPlayHistory> {
    read_optional_json(&recent_play_history_path(), "recent-play-history.json")
        .map(|history| history.unwrap_or_default())
}

fn save_recent_play_history(data: &SyncData) -> AppResult<()> {
    save_recent_history_at(&recent_play_history_path(), data)
}

fn save_recent_history_at(path: &std::path::Path, data: &SyncData) -> AppResult<()> {
    let history = PersistedRecentPlayHistory {
        recent_plays: data.recent_plays.clone(),
        recent_play_deletions: data.recent_play_deletions.clone(),
    };
    crate::fsutil::atomic_write(path, serde_json::to_vec_pretty(&history)?)?;
    Ok(())
}

fn with_history(mut result: SyncResult, data: &SyncData) -> SyncResult {
    let entries: Vec<SyncHistoryEntry> = data
        .recent_plays
        .iter()
        .map(|entry| SyncHistoryEntry {
            track: sync_song_to_track(&entry.song),
            played_at: entry.played_at,
        })
        .collect();
    result.history = Some(serde_json::json!({
        "entries": entries,
        "deletions": &data.recent_play_deletions,
    }));
    result
}

/// 获取或创建设备 ID
fn get_or_create_device_id(app: &AppHandle) -> String {
    use tauri_plugin_store::StoreExt;
    let store = app.store("sync-state.json").ok();

    if let Some(ref s) = store {
        if let Some(id) = s.get("deviceId").and_then(|v| v.as_str().map(String::from)) {
            return id;
        }
    }

    let id = uuid::Uuid::new_v4().to_string();
    if let Some(s) = store {
        s.set("deviceId", serde_json::json!(id));
    }
    id
}

pub fn get_or_create_device_id_pub(app: &AppHandle) -> String {
    get_or_create_device_id(app)
}

pub fn next_sync_causal_tokens_pub(
    app: &AppHandle,
    count: usize,
) -> AppResult<Vec<SyncCausalToken>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let _counter_guard = SYNC_CAUSAL_TOKEN_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .map_err(|_| AppError::Other("Sync causal counter lock is poisoned".into()))?;

    use tauri_plugin_store::StoreExt;
    let store = app
        .store("sync-state.json")
        .map_err(|error| AppError::Other(format!("Failed to open sync state: {}", error)))?;
    let device_id = get_or_create_device_id(app);
    let current = store
        .get("syncCausalCounter")
        .and_then(|value| value.as_i64())
        .unwrap_or_default()
        .max(0);
    let count = i64::try_from(count)
        .map_err(|_| AppError::Other("Sync causal token count is too large".into()))?;
    let next = current
        .checked_add(count)
        .ok_or_else(|| AppError::Other("Sync causal counter overflow".into()))?;
    store.set("syncCausalCounter", serde_json::json!(next));
    store
        .save()
        .map_err(|error| AppError::Other(format!("Failed to persist sync state: {}", error)))?;

    Ok((1..=count)
        .map(|offset| SyncCausalToken {
            device_id: device_id.clone(),
            counter: current + offset,
        })
        .collect())
}

pub fn attach_sync_membership_token_pub(
    track: &mut TrackInfo,
    token: SyncCausalToken,
) {
    let mut payload = track_to_sync_song(track);
    payload.added_at = track.added_at.max(0);
    payload.sync_membership_tokens = vec![token];
    payload.sync_metadata_version = CURRENT_SYNC_METADATA_VERSION;
    track.playlist_key = Some(payload.identity().stable_key());
    track.sync_payload = Some(payload);
}

/// 歌单文件路径（与 library_cmd 保持一致）
fn playlists_path() -> std::path::PathBuf {
    let mut path = dirs_next::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    path.push("NeriPlayer");
    path.push("playlists.json");
    path
}

pub fn tracks_to_sync_songs_pub(tracks: &[TrackInfo]) -> Vec<SyncSong> {
    tracks_to_sync_songs(tracks)
}

/// 播放统计身份键：与 Android SongItem.stableKey() 同源
///
/// 本地文件也要计入统计，所以不能像同步那样直接过滤掉 Local 源。
pub fn playback_stats_identity_key_pub(track: &TrackInfo) -> String {
    if track.source == TrackSource::Local {
        let identity = SongIdentity {
            id: numeric_id_or_zero(&track.id).to_string(),
            album: track.album.clone(),
            media_uri: track.id.clone(),
        };
        return identity.stable_key();
    }
    track_to_sync_song(track).identity().stable_key()
}

pub fn playlist_track_identity_key_pub(track: &TrackInfo) -> String {
    if track.source == TrackSource::Local {
        return track
            .playlist_key
            .clone()
            .filter(|key| !key.trim().is_empty())
            .unwrap_or_else(|| track.id.clone());
    }
    track_to_sync_song(track).identity().stable_key()
}

fn tracks_to_sync_songs(tracks: &[TrackInfo]) -> Vec<SyncSong> {
    tracks.iter()
        .filter(|track| track.source != TrackSource::Local)
        .map(track_to_sync_song)
        .collect()
}

pub fn track_to_playlist_song_deletion_pub(
    playlist_id: i64,
    track: &TrackInfo,
    device_id: String,
) -> SyncPlaylistSongDeletion {
    let song = track_to_sync_song(track);
    SyncPlaylistSongDeletion {
        playlist_id: playlist_id.to_string(),
        song_id: song.id,
        album: song.album,
        media_uri: if song.media_uri.is_empty() { None } else { Some(song.media_uri) },
        deleted_at: chrono::Utc::now().timestamp_millis(),
        device_id,
        removed_membership_tokens: song.sync_membership_tokens,
    }
}

/// TrackInfo -> SyncSong 转换（内部使用）
fn track_to_sync_song(track: &TrackInfo) -> SyncSong {
    if let Some(payload) = &track.sync_payload {
        let mut preserved = payload.normalized_for_sync();
        preserved.added_at = track.added_at.max(0);
        // 有完整载荷时标记为 CURRENT, 对齐 Android SyncSong.fromSongItem
        if preserved.sync_metadata_version < CURRENT_SYNC_METADATA_VERSION {
            preserved.sync_metadata_version = CURRENT_SYNC_METADATA_VERSION;
        }
        return preserved;
    }

    let platform = sync_platform_identity(track);

    // 本地/未知来源无 payload 时的兜底身份：把唯一路径写进 media_uri，
    // 否则同专辑本地曲目 stable_key 全部坍缩为 "0|album|"（SC-3/SC-4：
    // 视图去重只剩 1 首、按 key 删除会连带删同专辑全部）
    let media_uri = match platform.media_uri {
        Some(uri) => uri,
        None if platform.channel_id.is_none() => track.id.clone(),
        None => String::new(),
    };

    SyncSong {
        id: platform.id,
        name: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        album_id: String::new(),
        duration_ms: track.duration_ms as i64,
        cover_url: track.cover_url.clone().unwrap_or_default(),
        media_uri,
        added_at: track.added_at.max(0),
        matched_lyric: None,
        matched_translated_lyric: None,
        matched_lyric_source: None,
        matched_song_id: None,
        user_lyric_offset_ms: 0,
        custom_cover_url: None,
        custom_name: None,
        custom_artist: None,
        original_cover_url: None,
        original_name: None,
        original_artist: None,
        original_lyric: None,
        original_translated_lyric: None,
        channel_id: platform.channel_id,
        audio_id: platform.audio_id,
        sub_audio_id: platform.sub_audio_id,
        playlist_context_id: None,
        sync_membership_tokens: Vec::new(),
        // 无历史载荷时仍用 LEGACY, 让 merge fill-missing 可补齐云端歌词
        sync_metadata_version: LEGACY_SYNC_METADATA_VERSION,
        legacy_added_at: None,
        ..Default::default()
    }
}

struct SyncPlatformIdentity {
    id: String,
    media_uri: Option<String>,
    channel_id: Option<String>,
    audio_id: Option<String>,
    sub_audio_id: Option<String>,
}

fn sync_platform_identity(track: &TrackInfo) -> SyncPlatformIdentity {
    if let Some(nid) = track.id.strip_prefix("netease:") {
        return SyncPlatformIdentity {
            id: nid.to_string(),
            media_uri: None,
            channel_id: Some("netease".into()),
            audio_id: Some(nid.to_string()),
            sub_audio_id: None,
        };
    }

    if let Some(mid) = track.id.strip_prefix("qq:") {
        return SyncPlatformIdentity {
            id: stable_remote_android_id("qq", mid, "").to_string(),
            media_uri: None,
            channel_id: Some("qq".into()),
            audio_id: Some(mid.to_string()),
            sub_audio_id: None,
        };
    }

    if let Some(vid) = track.id.strip_prefix("youtube:") {
        return SyncPlatformIdentity {
            id: stable_sync_identity_id(vid).to_string(),
            media_uri: Some(build_youtube_music_media_uri(vid)),
            channel_id: Some("youtube_music".into()),
            audio_id: Some(vid.to_string()),
            sub_audio_id: None,
        };
    }

    if let Some(bili_id) = track.id.strip_prefix("bilibili:") {
        let sub_audio_id = bilibili_cid_from_album(&track.album);
        let sub_audio = sub_audio_id.as_deref().unwrap_or("");
        return SyncPlatformIdentity {
            id: stable_remote_android_id("bilibili", bili_id, sub_audio).to_string(),
            media_uri: None,
            channel_id: Some("bilibili".into()),
            audio_id: Some(bili_id.to_string()),
            sub_audio_id,
        };
    }

    SyncPlatformIdentity {
        id: numeric_id_or_zero(&track.id).to_string(),
        media_uri: None,
        channel_id: None,
        audio_id: None,
        sub_audio_id: None,
    }
}

fn stable_remote_android_id(channel: &str, audio: &str, sub_audio: &str) -> i64 {
    if channel == "netease" {
        return audio.parse::<i64>().unwrap_or_else(|_| stable_sync_identity_id(&format!("{channel}|{audio}")));
    }
    stable_sync_identity_id(&format!("{channel}|{audio}|{sub_audio}"))
}

fn numeric_id_or_zero(value: &str) -> i64 {
    value.parse::<i64>().unwrap_or(0)
}

fn bilibili_cid_from_album(album: &str) -> Option<String> {
    album
        .strip_prefix("Bilibili|")
        .filter(|cid| !cid.is_empty())
        .map(String::from)
}

/// SyncSong -> TrackInfo 转换
/// Android 端格式：
///   - 网易云: mediaUri 为空，id 为纯数字
///   - YouTube: mediaUri = "ytmusic://video/{videoId}"
///   - B站: album 以 "Bilibili" 开头，id 可能含 channelId 信息
///   - 本地: mediaUri 在同步时被清除
pub fn sync_song_to_track_pub(song: &SyncSong) -> TrackInfo {
    sync_song_to_track(song)
}

fn sync_song_to_track(song: &SyncSong) -> TrackInfo {
    use crate::state::TrackSource;

    let channel = song.channel_id.as_deref().unwrap_or("").to_ascii_lowercase();
    let is_youtube = channel == "youtube_music"
        || channel == "youtubemusic"
        || channel == "youtube"
        || song.media_uri.starts_with("ytmusic://");
    let is_bilibili = channel == "bilibili" || song.album.starts_with("Bilibili");
    let is_qq = channel == "qq";
    let is_netease = channel == "netease" || (!song.id.is_empty() && song.media_uri.is_empty());

    let (full_id, source) = if is_youtube {
        // ytmusic://video/{videoId}?playlistId=... -> 提取 videoId
        let video_id = song.audio_id.as_deref()
            .or_else(|| song.media_uri.strip_prefix("ytmusic://video/"))
            .unwrap_or(&song.id)
            .split('?')
            .next()
            .unwrap_or(&song.id);
        (format!("youtube:{}", video_id), TrackSource::Youtube)
    } else if is_bilibili {
        let bili_id = song.audio_id.as_deref()
            .filter(|id| !id.is_empty())
            .unwrap_or(&song.id);
        (format!("bilibili:{}", bili_id), TrackSource::Bilibili)
    } else if is_qq {
        let qq_id = song.audio_id.as_deref()
            .filter(|id| !id.is_empty())
            .unwrap_or(&song.id);
        (format!("qq:{}", qq_id), TrackSource::Qq)
    } else if is_netease {
        let netease_id = song.audio_id.as_deref()
            .filter(|id| !id.is_empty())
            .unwrap_or(&song.id);
        (format!("netease:{}", netease_id), TrackSource::Netease)
    } else {
        // 无法识别来源
        (song.id.clone(), TrackSource::Local)
    };

    let display_cover = song
        .custom_cover_url
        .as_ref()
        .filter(|url| !url.trim().is_empty())
        .cloned()
        .or_else(|| (!song.cover_url.is_empty()).then(|| song.cover_url.clone()));
    let playback_album = if is_bilibili {
        song.sub_audio_id
            .as_deref()
            .filter(|cid| !cid.trim().is_empty())
            .map(|cid| format!("Bilibili|{}", cid.trim()))
            .unwrap_or_else(|| song.album.clone())
    } else {
        song.album.clone()
    };
    let playlist_key = song.identity().stable_key();
    TrackInfo {
        id: full_id,
        title: song.custom_name.clone().unwrap_or_else(|| song.name.clone()),
        artist: song.custom_artist.clone().unwrap_or_else(|| song.artist.clone()),
        album: playback_album,
        duration_ms: song.duration_ms.max(0) as u64,
        source,
        url: String::new(), // URL 在播放时动态获取
        cover_url: display_cover,
        added_at: song.added_at.max(0),
        sync_payload: Some(song.normalized_for_sync()),
        playlist_key: Some(playlist_key),
    }
}

/// 从本地歌单存储加载，转换为同步格式
/// 歌单文件损坏时必须中止同步：以空库继续会把"空态"推上云端，
/// 经 base-snapshot 删除检测放大为全设备数据丢失
fn load_local_playlists(_app: &AppHandle) -> AppResult<Vec<SyncPlaylist>> {
    let path = playlists_path();
    let store = PlaylistStore::load_strict(&path)?;

    let mut playlists: Vec<SyncPlaylist> = store.playlists.iter().map(|pl| {
        let sync_id = sync_playlist_id(pl.id, &pl.name);
        SyncPlaylist {
            id: sync_id,
            name: pl.name.clone(),
            songs: tracks_to_sync_songs(&pl.tracks),
            created_at: pl.id,
            modified_at: pl.modified_at as i64,
            is_deleted: false,
            song_order_version: DISPLAY_ORDER_SONG_ORDER_VERSION,
        }
    }).collect();

    let existing_ids: HashSet<String> = playlists.iter().map(|playlist| playlist.id.clone()).collect();
    for deleted_id in store.deleted_playlist_ids {
        let id = deleted_id.to_string();
        if existing_ids.contains(&id) {
            continue;
        }
        playlists.push(SyncPlaylist {
            id,
            name: String::new(),
            songs: Vec::new(),
            created_at: deleted_id,
            modified_at: chrono::Utc::now().timestamp_millis(),
            is_deleted: true,
            song_order_version: DISPLAY_ORDER_SONG_ORDER_VERSION,
        });
    }
    Ok(playlists)
}

fn sync_playlist_id(id: i64, name: &str) -> String {
    if is_favorites_name(name) {
        return SYSTEM_FAVORITES_ID.to_string();
    }
    if is_local_name(name) {
        return SYSTEM_LOCAL_ID.to_string();
    }
    id.to_string()
}

/// 系统歌单 ID（对齐 Android FavoritesPlaylist / LocalFilesPlaylist）
const SYSTEM_FAVORITES_ID: i64 = -1001;
const SYSTEM_LOCAL_ID: i64 = -1002;

/// 识别系统歌单的候选名称
const FAVORITES_NAMES: &[&str] = &["我喜欢的音乐", "我喜歡的音樂", "お気に入りの曲", "Liked Songs", "My Favorite Music"];
const LOCAL_NAMES: &[&str] = &["本地音乐", "本機音樂", "ローカル音楽", "Local Music"];

fn is_favorites_name(name: &str) -> bool { FAVORITES_NAMES.contains(&name) }
fn is_local_name(name: &str) -> bool { LOCAL_NAMES.contains(&name) }

/// 解析 SyncPlaylist ID，识别系统歌单
fn resolve_system_id(sp_id: &str, sp_name: &str) -> i64 {
    if let Ok(id) = sp_id.parse::<i64>() {
        if id == SYSTEM_FAVORITES_ID { return SYSTEM_FAVORITES_ID; }
        if id == SYSTEM_LOCAL_ID { return SYSTEM_LOCAL_ID; }
        if id > 0 { return id; }
    }
    if is_favorites_name(sp_name) { return SYSTEM_FAVORITES_ID; }
    if is_local_name(sp_name) { return SYSTEM_LOCAL_ID; }
    0 // 需要分配新 ID
}

/// 将同步合并后的歌单回写到本地存储（对齐 Android applyMergedDataToLocal）
pub fn save_synced_playlists(merged: &SyncData) -> AppResult<()> {
    let _guard = playlist::lock_io();
    save_synced_playlists_locked(merged)
}

/// 仅在同步期间没有本地歌单写入时应用合并结果
///
/// 网络请求可能持续数秒，期间用户仍可编辑歌单。epoch 变化时拒绝回写，
/// 保留用户刚写入的文件，下一轮同步再合并远端结果，避免静默覆盖本地编辑
pub fn save_synced_playlists_if_epoch(merged: &SyncData, expected_epoch: u64) -> AppResult<()> {
    let _guard = playlist::lock_io();
    ensure_local_playlist_epoch(expected_epoch)?;
    save_synced_playlists_locked(merged)
}

pub(super) fn ensure_local_playlist_epoch(expected_epoch: u64) -> AppResult<()> {
    if playlist::io_epoch() != expected_epoch {
        return Err(AppError::Other(
            "Local playlists changed during sync; remote result was not applied".into(),
        ));
    }
    Ok(())
}

fn save_synced_playlists_locked(merged: &SyncData) -> AppResult<()> {
    let path = playlists_path();
    // 损坏时中止回写：在空库上重建会把用户本地独有的歌单 ID 映射全部丢弃
    let mut store = PlaylistStore::load_strict(&path)?;
    let existing_playlists = store.playlists.clone();

    let mut new_playlists: Vec<Playlist> = Vec::new();
    let mut max_id: i64 = existing_playlists.iter().map(|p| p.id).filter(|&id| id > 0).max().unwrap_or(0);
    let mut active_ids = HashSet::new();
    let mut deleted_ids = HashSet::new();

    for sp in &merged.playlists {
        if sp.is_deleted {
            if let Ok(id) = sp.id.parse::<i64>() {
                deleted_ids.insert(id);
            }
            continue;
        }
        let playlist = sp.normalized_for_display_order();

        let mut local_id = resolve_system_id(&playlist.id, &playlist.name);
        if local_id == 0 {
            local_id = playlist.id.parse::<i64>().ok()
                .filter(|&id| id > 0)
                .or_else(|| existing_playlists.iter().find(|p| p.name == playlist.name).map(|p| p.id))
                .unwrap_or_else(|| { max_id += 1; max_id });
        }

        // 检查 ID 冲突
        if new_playlists.iter().any(|p| p.id == local_id) {
            max_id += 1;
            local_id = max_id;
        }
        active_ids.insert(local_id);

        let current_playlist = existing_playlists.iter().find(|current| {
            current.id == local_id || sync_playlist_id(current.id, &current.name) == playlist.id
        });
        let mut seen_song_keys = HashSet::new();
        let mut tracks: Vec<TrackInfo> = playlist.songs.iter()
            .filter(|song| {
                let keys = song.identity_keys();
                if keys.iter().any(|key| seen_song_keys.contains(key)) {
                    return false;
                }
                seen_song_keys.extend(keys);
                true
            })
            .map(sync_song_to_track)
            .filter(|track| !track.id.is_empty())
            .collect();
        let mut local_track_ids: HashSet<String> = tracks.iter().map(|track| track.id.clone()).collect();
        if let Some(current) = current_playlist {
            for track in &current.tracks {
                if track.source == TrackSource::Local && local_track_ids.insert(track.id.clone()) {
                    tracks.push(track.clone());
                }
            }
        }

        new_playlists.push(Playlist {
            id: local_id,
            name: playlist.name,
            tracks,
            modified_at: playlist.modified_at.max(0) as u64,
        });
    }

    for id in deleted_ids {
        if active_ids.contains(&id) {
            continue;
        }
        if !store.deleted_playlist_ids.contains(&id) {
            store.deleted_playlist_ids.push(id);
        }
    }
    store.deleted_playlist_ids.retain(|id| !active_ids.contains(id));

    // 排序：我喜欢的音乐始终第一，本地文件始终最后，其余保持原序
    new_playlists.sort_by(|a, b| {
        let rank = |p: &Playlist| -> i32 {
            if p.id == SYSTEM_FAVORITES_ID { -1 }
            else if p.id == SYSTEM_LOCAL_ID { i32::MAX }
            else { 0 }
        };
        rank(a).cmp(&rank(b))
    });

    store.playlists = new_playlists;
    store.playlist_song_deletions = merged
        .playlist_song_deletions
        .iter()
        .map(|deletion| {
            let mut normalized = deletion.clone();
            normalized.removed_membership_tokens = normalize_sync_causal_tokens(
                &deletion.removed_membership_tokens,
            );
            normalized
        })
        .collect();
    store.fix_next_id();
    // 歌单库是同步的最终落点，写失败必须上抛，静默吞掉会让用户以为已同步
    store.save_locked(&path)?;

    // 保存收藏歌单到独立文件
    save_favorite_playlists(merged)?;
    Ok(())
}

/// 收藏歌单存储路径
fn favorites_path() -> std::path::PathBuf {
    let mut path = dirs_next::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    path.push("NeriPlayer");
    path.push("favorites.json");
    path
}

/// 保存收藏歌单（FavoritePlaylist）
fn save_favorite_playlists(merged: &SyncData) -> AppResult<()> {
    save_favorites_at(&favorites_path(), merged)
}

fn save_favorites_at(path: &std::path::Path, merged: &SyncData) -> AppResult<()> {
    // 删除记录必须持久化，下一轮同步才不会被旧端的收藏重新恢复
    crate::fsutil::atomic_write(path, serde_json::to_vec_pretty(&merged.favorite_playlists)?)?;
    Ok(())
}

/// 读取收藏歌单（供 list 命令调用）
pub fn load_favorite_playlists() -> AppResult<Vec<SyncFavoritePlaylist>> {
    load_favorites_at(&favorites_path(), false)
}

pub fn update_favorite_playlists<T>(
    update: impl FnOnce(&mut Vec<SyncFavoritePlaylist>) -> AppResult<T>,
) -> AppResult<T> {
    let _guard = playlist::lock_io();
    update_favorites_at(&favorites_path(), update)
}

fn update_favorites_at<T>(
    path: &std::path::Path,
    update: impl FnOnce(&mut Vec<SyncFavoritePlaylist>) -> AppResult<T>,
) -> AppResult<T> {
    let mut favorites = load_favorites_at(path, true)?;
    let result = update(&mut favorites)?;
    let data = SyncData { favorite_playlists: favorites, ..Default::default() };
    save_favorites_at(path, &data)?;
    // 收藏也是同步快照的一部分，网络窗口中的修改必须让旧快照失效
    playlist::mark_io_changed();
    Ok(result)
}

fn load_favorites_at(path: &std::path::Path, include_deleted: bool) -> AppResult<Vec<SyncFavoritePlaylist>> {
    read_optional_json::<Vec<SyncFavoritePlaylist>>(path, "favorites.json")
        .map(|favorites| {
            favorites
                .unwrap_or_default()
                .into_iter()
                .filter(|favorite| include_deleted || !favorite.is_deleted)
                .map(|favorite| favorite.normalized_for_sync())
                .collect()
        })
}

fn load_local_playlist_song_deletions() -> AppResult<Vec<SyncPlaylistSongDeletion>> {
    let store = PlaylistStore::load_strict(&playlists_path())?;
    Ok(store
        .playlist_song_deletions
        .into_iter()
        .map(|mut deletion| {
            deletion.removed_membership_tokens = normalize_sync_causal_tokens(
                &deletion.removed_membership_tokens,
            );
            deletion
        })
        .collect())
}

// Base Snapshot：用于三方歌曲合并的删除检测
/// snapshot 文件路径
fn base_snapshot_path(scope: &str) -> std::path::PathBuf {
    let mut path = dirs_next::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    path.push("NeriPlayer");
    path.push(format!("sync-base-snapshot-{}.json", scope));
    path
}

/// 加载上次同步后每个歌单的歌曲 stable_key 集合
/// 格式: { "playlist_id": ["key1", "key2", ...], ... }
pub fn load_base_snapshot(scope: &str) -> AppResult<HashMap<String, HashSet<String>>> {
    let path = base_snapshot_path(scope);
    let raw: HashMap<String, Vec<String>> =
        read_optional_json(&path, &format!("base snapshot {scope}"))?.unwrap_or_default();
    Ok(raw
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect())
}

/// 读取可选 JSON 文件：不存在表示首次运行，损坏则隔离现场并失败。
/// 把解析错误当空数据会在下一轮同步中覆盖原文件并扩散错误状态（SY-4）。
fn read_optional_json<T>(path: &std::path::Path, label: &str) -> AppResult<Option<T>>
where
    T: serde::de::DeserializeOwned,
{
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(AppError::Other(format!("读取 {label} 失败: {error}"))),
    };

    match serde_json::from_str::<T>(&content) {
        Ok(value) => Ok(Some(value)),
        Err(error) => {
            let quarantined = crate::fsutil::quarantine_corrupt_file(path);
            log::error!(
                target: "sync",
                "{label} 解析失败, 现场已隔离到 {:?}: {error}",
                quarantined
            );
            Err(AppError::Other(format!(
                "{label} 已损坏, 原文件已隔离到 {:?}: {error}",
                quarantined
            )))
        }
    }
}

/// 保存当前合并结果作为下次同步的 base snapshot
fn save_base_snapshot(merged: &SyncData, scope: &str) -> AppResult<()> {
    save_base_snapshot_at(&base_snapshot_path(scope), merged)
}

fn save_base_snapshot_at(path: &std::path::Path, merged: &SyncData) -> AppResult<()> {
    let snapshot: HashMap<String, Vec<String>> = merged.playlists.iter()
        .filter(|p| !p.is_deleted)
        .map(|p| {
            let keys: Vec<String> = p.songs.iter()
                .map(|s| s.identity().stable_key())
                .collect();
            (p.id.clone(), keys)
        })
        .collect();

    crate::fsutil::atomic_write(path, serde_json::to_vec(&snapshot)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{TrackInfo, TrackSource};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn rebuilding_frontend_history_preserves_remote_resume_and_device_for_same_play() {
        let remote=SyncRecentPlay{song_id:"1".into(),song:SyncSong{id:"1".into(),..Default::default()},played_at:100,device_id:"android".into(),resume_position_ms:2345};
        let mut entries=vec![SyncRecentPlay{device_id:"desktop".into(),resume_position_ms:0,..remote.clone()}];
        preserve_recent_progress(&mut entries,std::slice::from_ref(&remote));
        assert_eq!(entries[0].resume_position_ms,2345);assert_eq!(entries[0].device_id,"android");
        entries[0].played_at=101;entries[0].resume_position_ms=0;preserve_recent_progress(&mut entries,&[remote]);assert_eq!(entries[0].resume_position_ms,0);
    }

    async fn mock_github_server(
        responses: Vec<String>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0_u8; 4096];
                let _ = socket.read(&mut request).await.unwrap();
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
            }
        });
        (format!("http://{}", address), handle)
    }

    /// 同 github_api 测试：mock server 在回环地址，必须绕开系统代理
    fn loopback_client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("failed to build loopback test client")
    }

    fn github_response(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            status,
            body.len(),
            body
        )
    }

    fn github_config() -> GitHubSyncConfig {
        GitHubSyncConfig {
            owner: "owner".into(),
            repo: "repo".into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn remote_api_failure_is_not_treated_as_initial_sync() {
        let (base, server) = mock_github_server(vec![github_response(
            "500 Internal Server Error",
            r#"{"message":"temporary failure"}"#,
        )])
        .await;
        let api = GitHubApiClient::new_with_api_base(
            &loopback_client(),
            "token",
            &base,
        );

        let error = fetch_remote_snapshot(&api, &github_config(), "backup.bin", true)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("500"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn initial_sync_requires_both_remote_formats_to_be_missing() {
        let (base, server) = mock_github_server(vec![
            github_response("404 Not Found", "{}"),
            github_response("404 Not Found", "{}"),
        ])
        .await;
        let api = GitHubApiClient::new_with_api_base(
            &loopback_client(),
            "token",
            &base,
        );

        let snapshot = fetch_remote_snapshot(&api, &github_config(), "backup.bin", true)
            .await
            .unwrap();

        assert!(snapshot.is_none());
        server.await.unwrap();
    }

    #[test]
    fn stale_playlist_epoch_rejects_sync_before_remote_upload() {
        let expected_epoch = playlist::io_epoch().wrapping_add(1);
        let error = ensure_local_playlist_epoch(expected_epoch)
            .expect_err("a stale local epoch must reject before remote upload");

        assert!(error
            .to_string()
            .contains("Local playlists changed during sync"));
    }

    #[test]
    fn corrupt_optional_json_is_quarantined_and_returns_error() {
        let dir = std::env::temp_dir().join(format!(
            "neri-sync-optional-json-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("favorites.json");
        std::fs::write(&path, b"{not-json").unwrap();

        let error = read_optional_json::<Vec<SyncFavoritePlaylist>>(&path, "favorites.json")
            .expect_err("corrupt optional JSON must stop the read");

        assert!(error.to_string().contains("favorites.json"));
        assert!(!path.exists());
        let quarantined: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("favorites.json.corrupt-")
            })
            .collect();
        assert_eq!(quarantined.len(), 1);
        assert_eq!(std::fs::read(quarantined[0].path()).unwrap(), b"{not-json");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn favorite_updates_preserve_tombstones_and_invalidate_sync_snapshots() {
        let directory = std::env::temp_dir().join(format!("neri-favorite-update-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("favorites.json");
        let deleted: SyncFavoritePlaylist = serde_json::from_value(serde_json::json!({"id":42,"name":"deleted artist","source":"neteaseArtist","modifiedAt":20,"isDeleted":true})).unwrap();
        save_favorites_at(&path, &SyncData { favorite_playlists: vec![deleted], ..Default::default() }).unwrap();
        let epoch = playlist::io_epoch();
        update_favorites_at(&path, |favorites| {
            assert!(favorites[0].is_deleted);
            favorites.push(serde_json::from_value(serde_json::json!({"id":43,"name":"new artist","source":"biliArtist","modifiedAt":21})).unwrap());
            Ok(())
        }).unwrap();
        assert!(playlist::io_epoch() > epoch);
        assert_eq!(load_favorites_at(&path, true).unwrap().len(), 2);
        assert_eq!(load_favorites_at(&path, false).unwrap().len(), 1);
        let before = std::fs::read(&path).unwrap();
        assert!(update_favorites_at(&path, |favorites| { favorites.clear(); Err::<(), _>(AppError::Other("fixture error".into())) }).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn favorite_tombstone_survives_two_sync_rounds_and_is_hidden_from_listing() {
        let directory=std::env::temp_dir().join(format!("neri-favorite-persistence-{}",uuid::Uuid::new_v4()));std::fs::create_dir_all(&directory).unwrap();let path=directory.join("favorites.json");
        let active:SyncFavoritePlaylist=serde_json::from_value(serde_json::json!({"id":42,"name":"phone favorite","source":"netease","modifiedAt":10})).unwrap();
        let mut deleted=active.clone();deleted.is_deleted=true;deleted.modified_at=20;
        let data=SyncData{favorite_playlists:vec![deleted],..Default::default()};
        save_favorites_at(&path,&data).unwrap();let first=std::fs::read(&path).unwrap();save_favorites_at(&path,&data).unwrap();assert_eq!(std::fs::read(&path).unwrap(),first);
        assert!(load_favorites_at(&path,false).unwrap().is_empty());
        let local=SyncData{favorite_playlists:load_favorites_at(&path,true).unwrap(),..Default::default()};
        let remote=SyncData{favorite_playlists:vec![active],..Default::default()};
        let next=merge::three_way_merge(&local,&remote,0,&HashMap::new());assert!(next.favorite_playlists[0].is_deleted);
        save_favorites_at(&path,&next).unwrap();assert!(load_favorites_at(&path,true).unwrap()[0].is_deleted);assert!(load_favorites_at(&path,false).unwrap().is_empty());
        std::fs::remove_file(path).unwrap();std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn failed_history_favorites_and_base_writes_propagate_and_retry_idempotently() {
        let directory=std::env::temp_dir().join(format!("neri-sync-write-failure-{}",uuid::Uuid::new_v4()));std::fs::create_dir_all(&directory).unwrap();let blocker=directory.join("blocked");std::fs::write(&blocker,b"file").unwrap();let blocked=blocker.join("state.json");let data=SyncData::default();
        assert!(save_recent_history_at(&blocked,&data).is_err());assert!(save_favorites_at(&blocked,&data).is_err());assert!(save_base_snapshot_at(&blocked,&data).is_err());
        std::fs::remove_file(&blocker).unwrap();
        for (name,write) in [("history",save_recent_history_at as fn(&std::path::Path,&SyncData)->AppResult<()>),("favorites",save_favorites_at),("base",save_base_snapshot_at)] {
            let path=directory.join(name);write(&path,&data).unwrap();let first=std::fs::read(&path).unwrap();write(&path,&data).unwrap();assert_eq!(std::fs::read(&path).unwrap(),first);std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn sync_song_conversion_preserves_playlist_added_at() {
        let songs = tracks_to_sync_songs_pub(&[
            track("netease:1", 123),
            track("netease:2", 0),
        ]);

        assert_eq!(songs.iter().map(|song| song.added_at).collect::<Vec<_>>(), vec![123, 0]);

        let imported = sync_song_to_track(&SyncSong {
            id: "42".into(),
            name: "Remote".into(),
            album: "netease".into(),
            added_at: 777,
            channel_id: Some("netease".into()),
            audio_id: Some("42".into()),
            ..Default::default()
        });
        let roundtrip = tracks_to_sync_songs_pub(&[imported]);

        assert_eq!(roundtrip[0].added_at, 777);
    }

    #[test]
    fn sync_song_conversion_preserves_complete_metadata_payload() {
        let original = SyncSong {
            id: "42".into(),
            name: "Original".into(),
            artist: "Artist".into(),
            album: "netease".into(),
            cover_url: "https://example.com/base.jpg".into(),
            added_at: 777,
            matched_lyric: Some("[00:01.00]lyric".into()),
            matched_translated_lyric: Some("[00:01.00]translation".into()),
            custom_cover_url: Some("https://example.com/custom.jpg".into()),
            custom_name: Some("Custom title".into()),
            original_name: Some("Original".into()),
            channel_id: Some("netease".into()),
            audio_id: Some("42".into()),
            playlist_context_id: Some("context".into()),
            sync_membership_tokens: vec![SyncCausalToken {
                device_id: "android".into(),
                counter: 1,
            }],
            sync_metadata_version: CURRENT_SYNC_METADATA_VERSION,
            ..Default::default()
        };

        let imported = sync_song_to_track(&original);
        let expected_playlist_key = original.identity().stable_key();
        assert_eq!(
            imported.cover_url.as_deref(),
            Some("https://example.com/custom.jpg")
        );
        assert_eq!(
            imported.playlist_key.as_deref(),
            Some(expected_playlist_key.as_str())
        );
        let roundtrip = tracks_to_sync_songs_pub(&[imported]);

        assert_eq!(roundtrip[0].matched_lyric, original.matched_lyric);
        assert_eq!(
            roundtrip[0].matched_translated_lyric,
            original.matched_translated_lyric
        );
        assert_eq!(roundtrip[0].custom_name, original.custom_name);
        assert_eq!(roundtrip[0].custom_cover_url, original.custom_cover_url);
        assert_eq!(roundtrip[0].original_name, original.original_name);
        assert_eq!(roundtrip[0].playlist_context_id, original.playlist_context_id);
        assert_eq!(
            roundtrip[0].sync_membership_tokens,
            original.sync_membership_tokens
        );
        assert_eq!(
            roundtrip[0].sync_metadata_version,
            CURRENT_SYNC_METADATA_VERSION
        );
    }

    #[test]
    fn imported_and_fresh_tracks_share_playlist_identity_key() {
        let imported = sync_song_to_track(&SyncSong {
            id: "42".into(),
            name: "Song".into(),
            album: "netease".into(),
            channel_id: Some("netease".into()),
            audio_id: Some("42".into()),
            ..Default::default()
        });
        let fresh = track("netease:42", 0);

        assert_eq!(
            playlist_track_identity_key_pub(&imported),
            playlist_track_identity_key_pub(&fresh)
        );
    }

    #[test]
    fn sync_song_conversion_restores_bilibili_cid_for_playback() {
        let imported = sync_song_to_track(&SyncSong {
            id: "-123456".into(),
            name: "Bilibili song".into(),
            album: "Synced album".into(),
            channel_id: Some("bilibili".into()),
            audio_id: Some("BV1sync".into()),
            sub_audio_id: Some("987654".into()),
            ..Default::default()
        });

        assert_eq!(imported.id, "bilibili:BV1sync");
        assert_eq!(imported.album, "Bilibili|987654");
        assert_eq!(
            imported
                .sync_payload
                .as_ref()
                .and_then(|payload| payload.sub_audio_id.as_deref()),
            Some("987654")
        );
    }

    #[test]
    fn sync_song_conversion_accepts_backup_source_aliases_without_optional_cid() {
        let bilibili = sync_song_to_track(&SyncSong {
            id: "-1".into(),
            name: "Bilibili song".into(),
            album: "Bilibili".into(),
            channel_id: Some("bilibili".into()),
            audio_id: Some("1252950228".into()),
            ..Default::default()
        });
        let youtube = sync_song_to_track(&SyncSong {
            id: "-2".into(),
            name: "YouTube song".into(),
            channel_id: Some("youtubeMusic".into()),
            audio_id: Some("video-id".into()),
            ..Default::default()
        });

        assert_eq!(bilibili.id, "bilibili:1252950228");
        assert_eq!(bilibili.album, "Bilibili");
        assert_eq!(bilibili.source, TrackSource::Bilibili);
        assert_eq!(youtube.id, "youtube:video-id");
        assert_eq!(youtube.source, TrackSource::Youtube);
    }

    #[test]
    fn legacy_track_is_upgraded_only_after_complete_payload_is_attached() {
        let mut existing = track("netease:42", 100);
        assert_eq!(
            track_to_sync_song(&existing).sync_metadata_version,
            LEGACY_SYNC_METADATA_VERSION
        );

        let token = SyncCausalToken {
            device_id: "desktop".into(),
            counter: 1,
        };
        attach_sync_membership_token_pub(&mut existing, token.clone());
        let upgraded = track_to_sync_song(&existing);

        assert_eq!(upgraded.sync_metadata_version, CURRENT_SYNC_METADATA_VERSION);
        assert_eq!(upgraded.sync_membership_tokens, vec![token]);
    }

    fn track(id: &str, added_at: i64) -> TrackInfo {
        TrackInfo {
            id: id.into(),
            title: "Song".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            duration_ms: 1_000,
            source: TrackSource::Netease,
            url: String::new(),
            cover_url: None,
            added_at,
            sync_payload: None,
            playlist_key: None,
        }
    }
}
