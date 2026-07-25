// 同步管理器：协调 GitHub/WebDAV 同步流程
use super::github_api::GitHubApiClient;
use super::merge;
use super::models::*;
use super::serializer;
use super::webdav_api::WebDavApiClient;
use crate::error::{AppError, AppResult};
use crate::library::playlist::{acquire_playlist_io_lock, Playlist, PlaylistStore};
use crate::state::{TrackInfo, TrackSource};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::OnceLock;
use tauri::AppHandle;
use tokio::sync::{Mutex as TokioMutex, MutexGuard};

static SYNC_LOCK: OnceLock<TokioMutex<()>> = OnceLock::new();
static SYNC_CAUSAL_TOKEN_LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
static FAVORITES_IO_LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
const MAX_GITHUB_UPLOAD_CONFLICT_RETRIES: usize = 3;

async fn acquire_sync_lock() -> MutexGuard<'static, ()> {
    SYNC_LOCK.get_or_init(|| TokioMutex::new(())).lock().await
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

/// GitHub 同步（支持省流模式 backup.bin / 普通模式 backup.json）
pub async fn sync_github(
    http: &reqwest::Client,
    config: &mut GitHubSyncConfig,
    app: &AppHandle,
    history_entries: Option<&[SyncHistoryEntry]>,
    history_deletions: Option<&[SyncHistoryDeletion]>,
) -> AppResult<SyncResult> {
    let _sync_guard = acquire_sync_lock().await;
    let local_data = build_local_sync_data(app, history_entries, history_deletions);
    let local_data = &local_data;
    let api = GitHubApiClient::new(http, &config.token);
    let data_saver = config.data_saver;
    let primary_file = serializer::get_filename(data_saver);

    let remote_snapshot = fetch_remote_snapshot(&api, config, primary_file, data_saver).await?;
    let is_first_sync = config.last_remote_sha.is_empty();
    let initial_remote_missing = remote_snapshot.is_none();
    let mut remote_changed_during_sync = remote_snapshot.as_ref().is_some_and(|snapshot| {
        !config.last_remote_sha.is_empty()
            && config.last_remote_sha != snapshot.version.sha.as_deref().unwrap_or_default()
    });
    let base_snapshot = load_base_snapshot("github");
    let mut remote_data = remote_snapshot
        .as_ref()
        .map(|snapshot| snapshot.data.clone());
    let mut remote_version = remote_snapshot
        .map(|snapshot| snapshot.version)
        .unwrap_or_else(|| GitHubRemoteVersion {
            sha: None,
            file_name: primary_file.to_string(),
        });
    let mut final_merged = None;
    let mut final_remote_data = None;
    let mut upload_performed = false;

    for attempt in 0..=MAX_GITHUB_UPLOAD_CONFLICT_RETRIES {
        let merged = match remote_data.as_ref() {
            Some(remote) => {
                merge::three_way_merge(local_data, remote, config.last_sync_time, &base_snapshot)
            }
            None => {
                let mut initial = local_data.normalized_for_sync();
                initial.last_modified = chrono::Utc::now().timestamp_millis();
                initial
            }
        };

        let has_meaningful_change = match remote_data.as_ref() {
            Some(remote) => merge::has_data_changed(remote, &merged),
            None => true,
        };
        if !has_meaningful_change {
            final_remote_data = remote_data.clone();
            final_merged = Some(merged);
            break;
        }

        let use_binary_format = remote_version.file_name.ends_with(".bin");
        let content = serializer::serialize(&merged, use_binary_format)?;
        match api
            .update_file_content(
                &config.owner,
                &config.repo,
                &remote_version.file_name,
                &content,
                remote_version.sha.as_deref().unwrap_or_default(),
                "Sync from NeriPlayer Desktop",
            )
            .await
        {
            Ok(new_sha) => {
                remote_version.sha = Some(new_sha);
                final_remote_data = remote_data.clone();
                final_merged = Some(merged);
                upload_performed = true;
                break;
            }
            Err(error)
                if error.is_content_conflict() && attempt < MAX_GITHUB_UPLOAD_CONFLICT_RETRIES =>
            {
                let refreshed = fetch_remote_snapshot(
                    &api,
                    config,
                    &remote_version.file_name,
                    use_binary_format,
                )
                .await?;
                remote_data = refreshed.as_ref().map(|snapshot| snapshot.data.clone());
                remote_version = refreshed
                    .map(|snapshot| snapshot.version)
                    .unwrap_or_else(|| GitHubRemoteVersion {
                        sha: None,
                        file_name: remote_version.file_name.clone(),
                    });
                remote_changed_during_sync = true;
            }
            Err(error) => return Err(error.into()),
        }
    }

    let merged = final_merged
        .ok_or_else(|| AppError::Api("GitHub upload conflict retry budget exhausted".into()))?;

    save_synced_playlists(&merged)?;
    save_recent_play_history(&merged);
    save_base_snapshot(&merged, "github");

    let (remote_playlist_count, remote_song_count) = final_remote_data
        .as_ref()
        .map(|remote| {
            (
                remote.playlists.len(),
                remote
                    .playlists
                    .iter()
                    .map(|playlist| playlist.songs.len())
                    .sum::<usize>(),
            )
        })
        .unwrap_or_default();
    let playlists_added = merged.playlists.len() as i32 - remote_playlist_count as i32;
    let songs_added = merged
        .playlists
        .iter()
        .map(|playlist| playlist.songs.len())
        .sum::<usize>() as i32
        - remote_song_count as i32;

    config.last_remote_sha = remote_version.sha.unwrap_or_default();
    config.last_sync_time = chrono::Utc::now().timestamp_millis();
    let message = if !upload_performed && !remote_changed_during_sync && !is_first_sync {
        "Already up to date"
    } else if initial_remote_missing && upload_performed && !remote_changed_during_sync {
        "Initial upload complete"
    } else {
        "Sync complete"
    };

    Ok(with_history(
        SyncResult {
            success: true,
            message: message.into(),
            playlists_added: playlists_added.max(0),
            playlists_updated: 0,
            playlists_deleted: 0,
            songs_added: songs_added.max(0),
            songs_removed: 0,
            history: None,
        },
        &merged,
    ))
}

#[derive(Debug, Clone)]
struct GitHubRemoteVersion {
    sha: Option<String>,
    file_name: String,
}

#[derive(Debug, Clone)]
struct GitHubRemoteSnapshot {
    data: SyncData,
    version: GitHubRemoteVersion,
}

/// 严格读取远端快照，仅当两种格式都 404 时才视为首次同步
async fn fetch_remote_snapshot(
    api: &GitHubApiClient,
    config: &GitHubSyncConfig,
    preferred_file: &str,
    data_saver: bool,
) -> AppResult<Option<GitHubRemoteSnapshot>> {
    let alternative_file = serializer::get_filename(!data_saver);
    let (content, sha, mut actual_file) = match api
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

    if content.trim().is_empty() {
        return Err(AppError::Other("Remote backup file is empty".into()));
    }

    let mut actual_sha = sha;
    let data = match serializer::deserialize(&content, actual_file.ends_with(".bin")) {
        Ok(data) => data,
        Err(primary_error) => {
            let fallback_file = serializer::get_filename(!actual_file.ends_with(".bin"));
            let fallback = api
                .get_file_content(&config.owner, &config.repo, fallback_file)
                .await
                .map_err(AppError::from)?;
            let Some((fallback_content, fallback_sha)) = fallback else {
                return Err(primary_error);
            };
            if fallback_content.trim().is_empty() {
                return Err(primary_error);
            }
            let fallback_data =
                serializer::deserialize(&fallback_content, fallback_file.ends_with(".bin"))
                    .map_err(|_| primary_error)?;
            actual_file = fallback_file.to_string();
            actual_sha = fallback_sha;
            fallback_data
        }
    };

    Ok(Some(GitHubRemoteSnapshot {
        data,
        version: GitHubRemoteVersion {
            sha: Some(actual_sha),
            file_name: actual_file,
        },
    }))
}

/// WebDAV 同步
pub async fn sync_webdav(
    http: &reqwest::Client,
    config: &mut WebDavSyncConfig,
    app: &AppHandle,
    history_entries: Option<&[SyncHistoryEntry]>,
    history_deletions: Option<&[SyncHistoryDeletion]>,
) -> AppResult<SyncResult> {
    let _sync_guard = acquire_sync_lock().await;
    let local_data = build_local_sync_data(app, history_entries, history_deletions);
    let local_data = &local_data;
    let api = WebDavApiClient::new(
        http,
        &config.server_url,
        &config.username,
        &config.password,
        &config.base_path,
    );

    // 验证连接
    api.validate_connection().await?;

    // 拉取远程文件
    let (remote_content, remote_fingerprint) = match api.get_file_content().await? {
        Some((content, fp)) if !content.trim().is_empty() => (content, fp),
        _ => {
            let content = serde_json::to_string_pretty(local_data)?;
            let fp = api.update_file_content(&content).await?;
            save_base_snapshot(local_data, "webdav");
            save_recent_play_history(local_data);
            config.last_remote_fingerprint = fp;
            config.last_sync_time = chrono::Utc::now().timestamp_millis();
            return Ok(with_history(
                SyncResult {
                    success: true,
                    message: "Initial upload complete".into(),
                    ..Default::default()
                },
                local_data,
            ));
        }
    };

    // 解析远程数据（WebDAV 始终 JSON）
    let remote_data: SyncData = serde_json::from_str(&remote_content)
        .map_err(|e| AppError::Other(format!("Failed to parse remote sync data: {}", e)))?;

    let remote_changed = remote_fingerprint != config.last_remote_fingerprint;
    let base_snapshot = load_base_snapshot("webdav");
    let merged = merge::three_way_merge(
        local_data,
        &remote_data,
        config.last_sync_time,
        &base_snapshot,
    );
    save_synced_playlists(&merged)?;
    save_recent_play_history(&merged);
    save_base_snapshot(&merged, "webdav");

    if !remote_changed && !merge::has_data_changed(&remote_data, &merged) {
        config.last_remote_fingerprint = remote_fingerprint;
        config.last_sync_time = chrono::Utc::now().timestamp_millis();
        return Ok(with_history(
            SyncResult {
                success: true,
                message: "Already up to date".into(),
                ..Default::default()
            },
            &merged,
        ));
    }

    let content = serde_json::to_string_pretty(&merged)?;
    let fp = api.update_file_content(&content).await?;

    let playlists_added = merged.playlists.len() as i32 - remote_data.playlists.len() as i32;
    let songs_added = merged
        .playlists
        .iter()
        .map(|p| p.songs.len())
        .sum::<usize>() as i32
        - remote_data
            .playlists
            .iter()
            .map(|p| p.songs.len())
            .sum::<usize>() as i32;

    config.last_remote_fingerprint = fp;
    config.last_sync_time = chrono::Utc::now().timestamp_millis();

    Ok(with_history(
        SyncResult {
            success: true,
            message: "Sync complete".into(),
            playlists_added: playlists_added.max(0),
            playlists_updated: 0,
            playlists_deleted: 0,
            songs_added: songs_added.max(0),
            songs_removed: 0,
            history: None,
        },
        &merged,
    ))
}

/// 构建本地同步数据（从 tauri-plugin-store 读取歌单等）
pub fn build_local_sync_data(
    app: &AppHandle,
    history_entries: Option<&[SyncHistoryEntry]>,
    history_deletions: Option<&[SyncHistoryDeletion]>,
) -> SyncData {
    // 从 store 读取本地歌单数据
    // 当前歌单系统使用文件存储，构建 SyncData
    let device_id = get_or_create_device_id(app);
    let hostname = whoami::fallible::hostname().unwrap_or_else(|_| "Desktop".into());

    let recent_plays = history_entries
        .map(|entries| history_entries_to_sync(entries, &device_id))
        .unwrap_or_else(load_recent_plays);
    let recent_play_deletions = history_deletions
        .map(|deletions| history_deletions_to_sync(deletions, &device_id))
        .unwrap_or_else(load_recent_play_deletions);

    SyncData {
        version: "2.0".into(),
        device_id,
        device_name: format!("NeriPlayer Desktop ({})", hostname),
        last_modified: chrono::Utc::now().timestamp_millis(),
        playlists: load_local_playlists(app),
        favorite_playlists: load_favorite_playlists(),
        recent_plays,
        sync_log: Vec::new(),
        recent_play_deletions,
        playback_stats: Vec::new(),
        playback_stats_cleared_at: 0,
        playback_stat_buckets: Vec::new(),
        playlist_song_deletions: load_local_playlist_song_deletions(),
    }
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
            }
        })
        .collect()
}

fn history_deletions_to_sync(
    deletions: &[SyncHistoryDeletion],
    device_id: &str,
) -> Vec<SyncRecentPlayDeletion> {
    deletions
        .iter()
        .filter(|deletion| {
            deletion.track.source != TrackSource::Local && !deletion.track.id.is_empty()
        })
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

fn load_recent_plays() -> Vec<SyncRecentPlay> {
    let path = recent_play_history_path();
    let content = std::fs::read_to_string(path).unwrap_or_default();
    serde_json::from_str::<PersistedRecentPlayHistory>(&content)
        .map(|history| history.recent_plays)
        .unwrap_or_default()
}

fn load_recent_play_deletions() -> Vec<SyncRecentPlayDeletion> {
    let path = recent_play_history_path();
    let content = std::fs::read_to_string(path).unwrap_or_default();
    serde_json::from_str::<PersistedRecentPlayHistory>(&content)
        .map(|history| history.recent_play_deletions)
        .unwrap_or_default()
}

fn save_recent_play_history(data: &SyncData) {
    let path = recent_play_history_path();
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let history = PersistedRecentPlayHistory {
        recent_plays: data.recent_plays.clone(),
        recent_play_deletions: data.recent_play_deletions.clone(),
    };
    if let Ok(content) = serde_json::to_string_pretty(&history) {
        let _ = std::fs::write(path, content);
    }
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
        let _ = s.set("deviceId", serde_json::json!(id));
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

pub fn attach_sync_membership_token_pub(track: &mut TrackInfo, token: SyncCausalToken) {
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
    tracks
        .iter()
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
        media_uri: if song.media_uri.is_empty() {
            None
        } else {
            Some(song.media_uri)
        },
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

    SyncSong {
        id: platform.id,
        name: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        album_id: String::new(),
        duration_ms: track.duration_ms as i64,
        cover_url: track.cover_url.clone().unwrap_or_default(),
        media_uri: platform.media_uri.unwrap_or_default(),
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
        return audio
            .parse::<i64>()
            .unwrap_or_else(|_| stable_sync_identity_id(&format!("{channel}|{audio}")));
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
fn sync_song_to_track(song: &SyncSong) -> TrackInfo {
    use crate::state::TrackSource;

    let channel = song
        .channel_id
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_youtube = channel == "youtube_music"
        || channel == "youtubemusic"
        || channel == "youtube"
        || song.media_uri.starts_with("ytmusic://");
    let is_bilibili = channel == "bilibili" || song.album.starts_with("Bilibili");
    let is_qq = channel == "qq";
    let is_netease = channel == "netease" || (!song.id.is_empty() && song.media_uri.is_empty());

    let (full_id, source) = if is_youtube {
        // ytmusic://video/{videoId}?playlistId=... -> 提取 videoId
        let video_id = song
            .audio_id
            .as_deref()
            .or_else(|| song.media_uri.strip_prefix("ytmusic://video/"))
            .unwrap_or(&song.id)
            .split('?')
            .next()
            .unwrap_or(&song.id);
        (format!("youtube:{}", video_id), TrackSource::Youtube)
    } else if is_bilibili {
        let bili_id = song
            .audio_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .unwrap_or(&song.id);
        (format!("bilibili:{}", bili_id), TrackSource::Bilibili)
    } else if is_qq {
        let qq_id = song
            .audio_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .unwrap_or(&song.id);
        (format!("qq:{}", qq_id), TrackSource::Qq)
    } else if is_netease {
        let netease_id = song
            .audio_id
            .as_deref()
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
        title: song
            .custom_name
            .clone()
            .unwrap_or_else(|| song.name.clone()),
        artist: song
            .custom_artist
            .clone()
            .unwrap_or_else(|| song.artist.clone()),
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
fn load_local_playlists(_app: &AppHandle) -> Vec<SyncPlaylist> {
    let path = playlists_path();
    let store = PlaylistStore::load(&path);

    let mut playlists: Vec<SyncPlaylist> = store
        .playlists
        .iter()
        .map(|pl| {
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
        })
        .collect();

    let existing_ids: HashSet<String> = playlists
        .iter()
        .map(|playlist| playlist.id.clone())
        .collect();
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
    playlists
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
const FAVORITES_NAMES: &[&str] = &[
    "我喜欢的音乐",
    "我喜歡的音樂",
    "お気に入りの曲",
    "Liked Songs",
    "My Favorite Music",
];
const LOCAL_NAMES: &[&str] = &[
    "本地文件",
    "本地音乐",
    "本機音樂",
    "ローカルファイル",
    "ローカル音楽",
    "Local Files",
    "Local Music",
];

fn is_favorites_name(name: &str) -> bool {
    FAVORITES_NAMES.iter().any(|n| *n == name)
}
fn is_local_name(name: &str) -> bool {
    LOCAL_NAMES.iter().any(|n| *n == name)
}

/// 解析 SyncPlaylist ID，识别系统歌单
fn resolve_system_id(sp_id: &str, sp_name: &str) -> i64 {
    if let Ok(id) = sp_id.parse::<i64>() {
        if id == SYSTEM_FAVORITES_ID {
            return SYSTEM_FAVORITES_ID;
        }
        if id == SYSTEM_LOCAL_ID {
            return SYSTEM_LOCAL_ID;
        }
        if id > 0 {
            return id;
        }
    }
    if is_favorites_name(sp_name) {
        return SYSTEM_FAVORITES_ID;
    }
    if is_local_name(sp_name) {
        return SYSTEM_LOCAL_ID;
    }
    0 // 需要分配新 ID
}

/// 将同步数据中的普通歌单回写到本地存储（对齐 Android applyMergedDataToLocal）
fn save_playlists(merged: &SyncData) -> AppResult<()> {
    let _playlist_guard = acquire_playlist_io_lock()?;
    save_playlists_unlocked(merged)
}

fn save_playlists_unlocked(merged: &SyncData) -> AppResult<()> {
    let path = playlists_path();
    let mut store = PlaylistStore::load(&path);
    let existing_playlists = store.playlists.clone();

    let mut new_playlists: Vec<Playlist> = Vec::new();
    let mut max_id: i64 = existing_playlists
        .iter()
        .map(|p| p.id)
        .filter(|&id| id > 0)
        .max()
        .unwrap_or(0);
    let mut active_ids = HashSet::new();
    let mut deleted_ids = HashSet::new();
    let now = chrono::Utc::now().timestamp_millis();

    for sp in &merged.playlists {
        if sp.is_deleted {
            if let Ok(id) = sp.id.parse::<i64>() {
                deleted_ids.insert(id);
            }
            continue;
        }
        let playlist = sp.normalized_for_display_order(now);

        let mut local_id = resolve_system_id(&playlist.id, &playlist.name);
        if local_id == 0 {
            local_id = playlist
                .id
                .parse::<i64>()
                .ok()
                .filter(|&id| id > 0)
                .or_else(|| {
                    existing_playlists
                        .iter()
                        .find(|p| p.name == playlist.name)
                        .map(|p| p.id)
                })
                .unwrap_or_else(|| {
                    max_id += 1;
                    max_id
                });
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
        let mut tracks: Vec<TrackInfo> = playlist
            .songs
            .iter()
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
        let mut local_track_ids: HashSet<String> =
            tracks.iter().map(|track| track.id.clone()).collect();
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
    store
        .deleted_playlist_ids
        .retain(|id| !active_ids.contains(id));

    // 排序：我喜欢的音乐始终第一，本地文件始终最后，其余保持原序
    new_playlists.sort_by(|a, b| {
        let rank = |p: &Playlist| -> i32 {
            if p.id == SYSTEM_FAVORITES_ID {
                -1
            } else if p.id == SYSTEM_LOCAL_ID {
                i32::MAX
            } else {
                0
            }
        };
        rank(a).cmp(&rank(b))
    });

    store.playlists = new_playlists;
    store.playlist_song_deletions = merged
        .playlist_song_deletions
        .iter()
        .map(|deletion| {
            let mut normalized = deletion.clone();
            normalized.removed_membership_tokens =
                normalize_sync_causal_tokens(&deletion.removed_membership_tokens);
            normalized
        })
        .collect();
    store.fix_next_id();
    store.save(&path)
}

fn merge_imported_playlists_into_store(store: &mut PlaylistStore, imported: &SyncData) {
    let now = chrono::Utc::now().timestamp_millis();
    let mut max_id = store
        .playlists
        .iter()
        .map(|playlist| playlist.id)
        .filter(|id| *id > 0)
        .max()
        .unwrap_or(0);

    for imported_playlist in imported
        .playlists
        .iter()
        .filter(|playlist| !playlist.is_deleted)
    {
        let playlist = imported_playlist.normalized_for_display_order(now);
        let resolved_id = resolve_system_id(&playlist.id, &playlist.name);
        let existing_index = store.playlists.iter().position(|current| {
            (resolved_id != 0 && current.id == resolved_id)
                || sync_playlist_id(current.id, &current.name) == playlist.id
                || current.name == playlist.name
        });

        let mut seen_song_keys = HashSet::new();
        let imported_tracks: Vec<TrackInfo> = playlist
            .songs
            .iter()
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

        if let Some(index) = existing_index {
            let current = &mut store.playlists[index];
            let current_id = current.id;
            let mut known_keys = HashSet::new();
            for track in &current.tracks {
                if track.source == TrackSource::Local {
                    known_keys.insert(playlist_track_identity_key_pub(track));
                } else {
                    known_keys.extend(track_to_sync_song(track).identity_keys());
                }
            }

            let mut missing = Vec::new();
            for track in imported_tracks {
                let keys = if track.source == TrackSource::Local {
                    vec![playlist_track_identity_key_pub(&track)]
                } else {
                    track_to_sync_song(&track).identity_keys()
                };
                if keys.iter().any(|key| known_keys.contains(key)) {
                    continue;
                }
                known_keys.extend(keys);
                missing.push(track);
            }

            if !missing.is_empty() {
                missing.append(&mut current.tracks);
                current.tracks = missing;
                current.modified_at = now.max(0) as u64;
            }
            store.deleted_playlist_ids.retain(|id| *id != current_id);
            continue;
        }

        let local_id = if resolved_id != 0 {
            resolved_id
        } else {
            max_id += 1;
            max_id
        };
        max_id = max_id.max(local_id);
        store.deleted_playlist_ids.retain(|id| *id != local_id);
        store.playlists.push(Playlist {
            id: local_id,
            name: playlist.name,
            tracks: imported_tracks,
            modified_at: now.max(0) as u64,
        });
    }

    store.playlists.sort_by_key(|playlist| {
        if playlist.id == SYSTEM_FAVORITES_ID {
            -1
        } else if playlist.id == SYSTEM_LOCAL_ID {
            1
        } else {
            0
        }
    });
    store.fix_next_id();
}

fn merge_imported_playlists_unlocked(imported: &SyncData) -> AppResult<()> {
    let path = playlists_path();
    let mut store = PlaylistStore::load_strict(&path)?;
    merge_imported_playlists_into_store(&mut store, imported);
    store.save(&path)
}

/// 导入普通歌单时逐项合并，不删除本机独有歌单，也不改动独立存储的在线收藏。
pub async fn save_imported_playlists(imported: &SyncData) -> AppResult<()> {
    let _sync_guard = acquire_sync_lock().await;
    let _playlist_guard = acquire_playlist_io_lock()?;
    merge_imported_playlists_unlocked(imported)
}

/// 合并 Desktop 旧格式歌单；损坏的现有库会报错而不是被空库覆盖。
pub async fn save_imported_desktop_playlists(imported: Vec<Playlist>) -> AppResult<()> {
    let _sync_guard = acquire_sync_lock().await;
    let _playlist_guard = acquire_playlist_io_lock()?;
    let path = playlists_path();
    let mut store = PlaylistStore::load_strict(&path)?;
    for playlist in imported {
        if !store
            .playlists
            .iter()
            .any(|saved| saved.name == playlist.name)
        {
            store.playlists.push(playlist);
        }
    }
    store.fix_next_id();
    store.save(&path)
}

fn merge_imported_favorite_playlists(
    local: Vec<SyncFavoritePlaylist>,
    imported: Vec<SyncFavoritePlaylist>,
) -> (Vec<SyncFavoritePlaylist>, usize) {
    let mut imported: Vec<_> = imported
        .into_iter()
        .filter(|favorite| !favorite.is_deleted)
        .map(|favorite| favorite.normalized_for_sync())
        .collect();
    let imported_count = imported.len();

    // 手工“导入”采用加法恢复语义：备份墓碑不删除桌面收藏，活动条目可恢复本地墓碑。
    for favorite in &mut imported {
        if let Some(deleted_local) = local
            .iter()
            .find(|saved| saved.group_key() == favorite.group_key() && saved.is_deleted)
        {
            favorite.modified_at = favorite
                .modified_at
                .max(deleted_local.modified_at.saturating_add(1));
            favorite.is_deleted = false;
        }
    }

    let local = SyncData {
        favorite_playlists: local,
        ..Default::default()
    };
    let remote = SyncData {
        favorite_playlists: imported,
        ..Default::default()
    };
    (
        merge::three_way_merge(&local, &remote, 0, &HashMap::new()).favorite_playlists,
        imported_count,
    )
}

fn merge_imported_favorites_unlocked(imported: Vec<SyncFavoritePlaylist>) -> AppResult<usize> {
    let local = load_favorite_playlists_unlocked()?;
    let (merged, imported_count) = merge_imported_favorite_playlists(local, imported);
    save_favorite_playlists_unlocked(merged)?;
    Ok(imported_count)
}

/// 合并导入文件中的在线收藏歌单，保留本机未出现在备份中的收藏。
pub async fn save_imported_favorite_playlists(
    imported: Vec<SyncFavoritePlaylist>,
) -> AppResult<usize> {
    let _sync_guard = acquire_sync_lock().await;
    let _favorites_guard = FAVORITES_IO_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .map_err(|_| AppError::Other("Favorites storage lock poisoned".into()))?;
    merge_imported_favorites_unlocked(imported)
}

/// 完整备份的普通歌单与在线收藏在同一个同步临界区内落盘。
pub async fn save_imported_playlist_backup(imported: &SyncData) -> AppResult<usize> {
    let _sync_guard = acquire_sync_lock().await;
    let _playlist_guard = acquire_playlist_io_lock()?;
    let _favorites_guard = FAVORITES_IO_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .map_err(|_| AppError::Other("Favorites storage lock poisoned".into()))?;

    let path = playlists_path();
    let original_store = PlaylistStore::load_strict(&path)?;
    let mut merged_store = original_store.clone();
    merge_imported_playlists_into_store(&mut merged_store, imported);

    let original_favorites = load_favorite_playlists_unlocked()?;
    let (merged_favorites, imported_count) = merge_imported_favorite_playlists(
        original_favorites.clone(),
        imported.favorite_playlists.clone(),
    );

    merged_store.save(&path)?;
    if let Err(error) = save_favorite_playlists_unlocked(merged_favorites) {
        let favorites_rollback = save_favorite_playlists_unlocked(original_favorites);
        let playlists_rollback = original_store.save(&path);
        return match (playlists_rollback, favorites_rollback) {
            (Ok(()), Ok(())) => Err(error),
            (Err(playlist_error), Ok(())) => Err(AppError::Other(format!(
                "Favorite import failed: {error}; playlist rollback failed: {playlist_error}"
            ))),
            (Ok(()), Err(favorites_error)) => Err(AppError::Other(format!(
                "Favorite import failed: {error}; favorites rollback failed: {favorites_error}"
            ))),
            (Err(playlist_error), Err(favorites_error)) => Err(AppError::Other(format!(
                "Favorite import failed: {error}; playlist rollback failed: {playlist_error}; favorites rollback failed: {favorites_error}"
            ))),
        };
    }

    Ok(imported_count)
}

/// 将完整同步结果回写到本地存储。
pub fn save_synced_playlists(merged: &SyncData) -> AppResult<()> {
    save_playlists(merged)?;

    // 保存收藏歌单到独立文件
    save_favorite_playlists_pub(merged.favorite_playlists.clone())?;
    Ok(())
}

/// 收藏歌单存储路径
fn favorites_path() -> std::path::PathBuf {
    let mut path = dirs_next::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    path.push("NeriPlayer");
    path.push("favorites.json");
    path
}

fn atomic_write_favorites_file(path: &std::path::Path, content: &[u8]) -> std::io::Result<()> {
    atomic_write_favorites_file_with_publish(path, content, |temporary, path| {
        temporary
            .persist(path)
            .map(|_| ())
            .map_err(|error| error.error)
    })
}

fn atomic_write_favorites_file_with_publish(
    path: &std::path::Path,
    content: &[u8],
    publish: impl FnOnce(tempfile::NamedTempFile, &std::path::Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "favorites path has no parent directory",
        )
    })?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".favorites-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    temporary.write_all(content)?;
    temporary.as_file().sync_all()?;

    publish(temporary, path)?;
    sync_favorites_parent_directory(path)
}

#[cfg(unix)]
fn sync_favorites_parent_directory(path: &std::path::Path) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "favorites path has no parent directory",
        )
    })?;
    std::fs::File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_favorites_parent_directory(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

fn load_favorite_playlists_unlocked() -> AppResult<Vec<SyncFavoritePlaylist>> {
    let path = favorites_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|error| AppError::Other(format!("Failed to read favorites: {}", error)))?;
    serde_json::from_str::<Vec<SyncFavoritePlaylist>>(&content)
        .map(|favorites| {
            favorites
                .into_iter()
                .map(|favorite| favorite.normalized_for_sync())
                .collect()
        })
        .map_err(|error| AppError::Other(format!("Failed to parse favorites: {}", error)))
}

fn save_favorite_playlists_unlocked(favorites: Vec<SyncFavoritePlaylist>) -> AppResult<()> {
    let path = favorites_path();
    let favorites: Vec<SyncFavoritePlaylist> = favorites
        .into_iter()
        .map(|favorite| favorite.normalized_for_sync())
        .collect();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            AppError::Other(format!("Failed to create favorites directory: {}", error))
        })?;
    }
    let content = serde_json::to_string_pretty(&favorites)
        .map_err(|error| AppError::Other(format!("Failed to serialize favorites: {}", error)))?;
    atomic_write_favorites_file(&path, content.as_bytes())
        .map_err(|error| AppError::Other(format!("Failed to persist favorites: {}", error)))
}

pub fn save_favorite_playlists_pub(favorites: Vec<SyncFavoritePlaylist>) -> AppResult<()> {
    let _guard = FAVORITES_IO_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .map_err(|_| AppError::Other("Favorites storage lock poisoned".into()))?;
    save_favorite_playlists_unlocked(favorites)
}

pub async fn mutate_favorite_playlists_pub<T>(
    mutation: impl FnOnce(&mut Vec<SyncFavoritePlaylist>) -> AppResult<T>,
) -> AppResult<T> {
    let _sync_guard = acquire_sync_lock().await;
    let _guard = FAVORITES_IO_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .map_err(|_| AppError::Other("Favorites storage lock poisoned".into()))?;
    let mut favorites = load_favorite_playlists_unlocked()?;
    let result = mutation(&mut favorites)?;
    save_favorite_playlists_unlocked(favorites)?;
    Ok(result)
}

pub fn load_favorite_playlists_strict_pub() -> AppResult<Vec<SyncFavoritePlaylist>> {
    let _guard = FAVORITES_IO_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .map_err(|_| AppError::Other("Favorites storage lock poisoned".into()))?;
    load_favorite_playlists_unlocked()
}

/// 读取收藏歌单（同步快照需要保留删除墓碑）
pub fn load_favorite_playlists() -> Vec<SyncFavoritePlaylist> {
    match load_favorite_playlists_strict_pub() {
        Ok(favorites) => favorites,
        Err(error) => {
            log::error!(target: "sync", "failed to load favorite playlists: {}", error);
            Vec::new()
        }
    }
}

fn load_local_playlist_song_deletions() -> Vec<SyncPlaylistSongDeletion> {
    let store = PlaylistStore::load(&playlists_path());
    store
        .playlist_song_deletions
        .into_iter()
        .map(|mut deletion| {
            deletion.removed_membership_tokens =
                normalize_sync_causal_tokens(&deletion.removed_membership_tokens);
            deletion
        })
        .collect()
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
pub fn load_base_snapshot(scope: &str) -> HashMap<String, HashSet<String>> {
    let path = base_snapshot_path(scope);
    if !path.exists() {
        return HashMap::new();
    }
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let raw: HashMap<String, Vec<String>> = serde_json::from_str(&content).unwrap_or_default();
    raw.into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect()
}

/// 保存当前合并结果作为下次同步的 base snapshot
fn save_base_snapshot(merged: &SyncData, scope: &str) {
    let snapshot: HashMap<String, Vec<String>> = merged
        .playlists
        .iter()
        .filter(|p| !p.is_deleted)
        .map(|p| {
            let keys: Vec<String> = p.songs.iter().map(|s| s.identity().stable_key()).collect();
            (p.id.clone(), keys)
        })
        .collect();

    let path = base_snapshot_path(scope);
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = std::fs::write(&path, serde_json::to_string(&snapshot).unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{TrackInfo, TrackSource};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn mock_github_server(responses: Vec<String>) -> (String, tokio::task::JoinHandle<()>) {
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
        let api = GitHubApiClient::new_with_api_base(&reqwest::Client::new(), "token", &base);

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
        let api = GitHubApiClient::new_with_api_base(&reqwest::Client::new(), "token", &base);

        let snapshot = fetch_remote_snapshot(&api, &github_config(), "backup.bin", true)
            .await
            .unwrap();

        assert!(snapshot.is_none());
        server.await.unwrap();
    }

    #[test]
    fn sync_song_conversion_preserves_playlist_added_at() {
        let songs = tracks_to_sync_songs_pub(&[track("netease:1", 123), track("netease:2", 0)]);

        assert_eq!(
            songs.iter().map(|song| song.added_at).collect::<Vec<_>>(),
            vec![123, 0]
        );

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
        assert_eq!(
            roundtrip[0].playlist_context_id,
            original.playlist_context_id
        );
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

        assert_eq!(
            upgraded.sync_metadata_version,
            CURRENT_SYNC_METADATA_VERSION
        );
        assert_eq!(upgraded.sync_membership_tokens, vec![token]);
    }

    #[test]
    fn favorite_atomic_write_stages_sibling_and_replaces_existing_file() {
        let root = tempfile::tempdir().expect("favorites root");
        let path = root.path().join("favorites.json");
        let previous = br#"[{"id":"previous"}]"#;
        let replacement = br#"[{"id":"replacement"}]"#;
        std::fs::write(&path, previous).expect("seed favorites");
        let mut staging_path = None;

        atomic_write_favorites_file_with_publish(&path, replacement, |temporary, destination| {
            assert_eq!(temporary.path().parent(), destination.parent());
            assert_eq!(
                std::fs::read(temporary.path()).expect("read staging"),
                replacement
            );
            assert_eq!(std::fs::read(destination).expect("read previous"), previous);
            staging_path = Some(temporary.path().to_path_buf());
            temporary
                .persist(destination)
                .map(|_| ())
                .map_err(|error| error.error)
        })
        .expect("publish favorites");

        assert_eq!(std::fs::read(&path).expect("read favorites"), replacement);
        assert!(!staging_path.expect("observed staging path").exists());
    }

    #[test]
    fn favorite_atomic_write_preserves_existing_file_when_publish_fails() {
        let root = tempfile::tempdir().expect("favorites root");
        let path = root.path().join("favorites.json");
        let previous = br#"[{"id":"previous"}]"#;
        let replacement = br#"[{"id":"replacement"}]"#;
        std::fs::write(&path, previous).expect("seed favorites");
        let mut staging_path = None;

        let error = atomic_write_favorites_file_with_publish(
            &path,
            replacement,
            |temporary, destination| {
                assert_eq!(temporary.path().parent(), destination.parent());
                assert_eq!(
                    std::fs::read(temporary.path()).expect("read staging"),
                    replacement
                );
                assert_eq!(std::fs::read(destination).expect("read previous"), previous);
                staging_path = Some(temporary.path().to_path_buf());
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected publication failure",
                ))
            },
        )
        .expect_err("publication must fail");

        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read(&path).expect("read favorites"), previous);
        assert!(!staging_path.expect("observed staging path").exists());
    }

    #[test]
    fn playlist_import_merges_without_deleting_desktop_only_playlists() {
        let mut store = PlaylistStore::default();
        store.playlists = vec![
            Playlist {
                id: 1,
                name: "Desktop only".into(),
                tracks: vec![track("netease:1", 100)],
                modified_at: 100,
            },
            Playlist {
                id: 2,
                name: "Shared".into(),
                tracks: vec![track("netease:2", 200)],
                modified_at: 200,
            },
        ];
        let imported = SyncData {
            playlists: vec![
                SyncPlaylist {
                    id: "2".into(),
                    name: "Shared".into(),
                    songs: vec![SyncSong {
                        id: "3".into(),
                        name: "Imported".into(),
                        channel_id: Some("netease".into()),
                        audio_id: Some("3".into()),
                        ..Default::default()
                    }],
                    created_at: 2,
                    modified_at: 300,
                    is_deleted: false,
                    song_order_version: DISPLAY_ORDER_SONG_ORDER_VERSION,
                },
                SyncPlaylist {
                    id: "3".into(),
                    name: "Phone only".into(),
                    songs: Vec::new(),
                    created_at: 3,
                    modified_at: 300,
                    is_deleted: false,
                    song_order_version: DISPLAY_ORDER_SONG_ORDER_VERSION,
                },
            ],
            ..Default::default()
        };

        merge_imported_playlists_into_store(&mut store, &imported);

        assert_eq!(store.playlists.len(), 3);
        assert!(store
            .playlists
            .iter()
            .any(|playlist| playlist.name == "Desktop only"));
        let shared = store
            .playlists
            .iter()
            .find(|playlist| playlist.id == 2)
            .expect("shared playlist");
        assert_eq!(
            shared
                .tracks
                .iter()
                .map(|track| track.id.as_str())
                .collect::<Vec<_>>(),
            vec!["netease:3", "netease:2"]
        );
        assert!(store
            .playlists
            .iter()
            .any(|playlist| playlist.name == "Phone only"));
    }

    #[test]
    fn playlist_import_preserves_android_system_playlist_ids() {
        let mut store = PlaylistStore::default();
        let imported = SyncData {
            playlists: vec![
                SyncPlaylist {
                    id: SYSTEM_LOCAL_ID.to_string(),
                    name: "Local Files".into(),
                    ..Default::default()
                },
                SyncPlaylist {
                    id: SYSTEM_FAVORITES_ID.to_string(),
                    name: "My Favorite Music".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        merge_imported_playlists_into_store(&mut store, &imported);

        assert_eq!(store.playlists.len(), 2);
        assert_eq!(store.playlists.first().unwrap().id, SYSTEM_FAVORITES_ID);
        assert_eq!(store.playlists.last().unwrap().id, SYSTEM_LOCAL_ID);
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
