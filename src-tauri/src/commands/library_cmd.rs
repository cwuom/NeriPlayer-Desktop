use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, TrackInfo, TrackSource};
use crate::library::{scanner, playlist::PlaylistStore};
use crate::sync::models::SyncFavoritePlaylist;
use crate::sync::models::SyncCausalToken;
use crate::sync::manager;

#[tauri::command]
pub async fn scan_music_directory(
    dir: String,
    name_template: Option<String>,
) -> AppResult<scanner::ScanResult> {
    tokio::task::spawn_blocking(move || scanner::scan_directory(&dir, name_template.as_deref()))
        .await
        .map_err(|e| AppError::Other(e.to_string()))?
}

// 播放列表路径
fn playlists_path() -> std::path::PathBuf {
    let mut path = dirs_next::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    path.push("NeriPlayer");
    path.push("playlists.json");
    path
}

#[derive(Serialize)]
pub struct PlaylistInfo {
    pub id: i64,
    pub name: String,
    pub track_count: usize,
    pub modified_at: u64,
    pub cover_url: Option<String>,
}

#[tauri::command]
pub async fn list_playlists() -> AppResult<Vec<PlaylistInfo>> {
    let started = Instant::now();
    log::info!(target: "playlist-io", "list begin");
    let queued_at = Instant::now();
    let playlists = tokio::task::spawn_blocking(move || {
        let worker_started = Instant::now();
        log::info!(
            target: "playlist-io",
            "list worker started queued_ms={}",
            queued_at.elapsed().as_millis(),
        );
        let result = list_playlists_blocking(playlists_path());
        log::info!(
            target: "playlist-io",
            "list worker finished ok={}, worker_ms={}",
            result.is_ok(),
            worker_started.elapsed().as_millis(),
        );
        result
    })
    .await
    .map_err(|error| AppError::Other(error.to_string()))??;
    log::info!(
        target: "playlist-io",
        "list end count={}, elapsed_ms={}",
        playlists.len(),
        started.elapsed().as_millis(),
    );
    Ok(playlists)
}

fn list_playlists_blocking(path: std::path::PathBuf) -> AppResult<Vec<PlaylistInfo>> {
    let store = PlaylistStore::load_strict(&path)?;

    // 同名歌单不再自动删除：旧逻辑「同名只保留曲目最多的一个并落盘」会在
    // 多设备同步/导入并存的场景下静默销毁用户数据（曲目数不是新旧的可靠
    // 判据）。现在仅记录警告并全部保留，由用户自行处置
    {
        let mut seen_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        for pl in &store.playlists {
            let name = pl.name.trim().to_string();
            if !seen_names.insert(name.clone()) {
                log::warn!(
                    target: "playlist-io",
                    "发现同名歌单未合并: name={:?}, id={}, tracks={} (全部保留, 不自动删除)",
                    name,
                    pl.id,
                    pl.tracks.len(),
                );
            }
        }
    }

    let list: Vec<PlaylistInfo> = store.playlists.iter().map(|p| {
        let cover = p.tracks.iter().find_map(|t| {
            t.cover_url
                .as_ref()
                .filter(|url| !url.trim().is_empty())
                .cloned()
        });
        let mut seen = std::collections::HashSet::new();
        let unique_count = p.tracks.iter()
            .filter(|track| {
                !track.id.is_empty() && seen.insert(playlist_track_key(track))
            })
            .count();
        PlaylistInfo {
            id: p.id,
            name: p.name.clone(),
            track_count: unique_count,
            modified_at: p.modified_at,
            cover_url: cover,
        }
    }).collect();
    Ok(list)
}

#[tauri::command]
pub async fn create_playlist(app: AppHandle, name: String) -> AppResult<PlaylistInfo> {
    let path = playlists_path();
    let mut store = PlaylistStore::load_strict(&path)?;
    let pl = store.create(name);
    let info = PlaylistInfo {
        id: pl.id,
        name: pl.name.clone(),
        track_count: 0,
        modified_at: pl.modified_at,
        cover_url: None,
    };
    store.save(&path)?;
    let _ = app.emit("playlists-changed", ());
    Ok(info)
}

#[tauri::command]
pub async fn delete_playlist(app: AppHandle, id: i64) -> AppResult<bool> {
    let path = playlists_path();
    let mut store = PlaylistStore::load_strict(&path)?;
    let deleted = store.delete(id);
    if deleted {
        store.save(&path)?;
        let _ = app.emit("playlists-changed", ());
    }
    Ok(deleted)
}

#[tauri::command]
pub async fn rename_playlist(app: AppHandle, id: i64, name: String) -> AppResult<bool> {
    let path = playlists_path();
    let mut store = PlaylistStore::load_strict(&path)?;
    if let Some(pl) = store.playlists.iter_mut().find(|p| p.id == id) {
        pl.name = name;
        pl.modified_at = chrono::Utc::now().timestamp_millis() as u64;
        store.save(&path)?;
        let _ = app.emit("playlists-changed", ());
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
pub async fn get_playlist_tracks(id: i64) -> AppResult<Vec<TrackInfo>> {
    let started = Instant::now();
    log::info!(target: "playlist-io", "tracks begin playlist_id={}", id);
    let queued_at = Instant::now();
    let tracks = tokio::task::spawn_blocking(move || {
        let worker_started = Instant::now();
        log::info!(
            target: "playlist-io",
            "tracks worker started playlist_id={}, queued_ms={}",
            id,
            queued_at.elapsed().as_millis(),
        );
        let path = playlists_path();
        let store = PlaylistStore::load_strict(&path)?;
        let pl = store.playlists.iter().find(|p| p.id == id)
            .ok_or_else(|| AppError::NotFound("Playlist not found".into()))?;
        let mut seen = std::collections::HashSet::new();
        let tracks: Vec<TrackInfo> = pl.tracks.iter()
            .filter(|track| !track.id.is_empty() && seen.insert(playlist_track_key(track)))
            .cloned()
            .map(|mut track| {
                track.playlist_key = Some(playlist_track_key(&track));
                track
            })
            .collect();
        log::info!(
            target: "playlist-io",
            "tracks worker finished playlist_id={}, count={}, worker_ms={}",
            id,
            tracks.len(),
            worker_started.elapsed().as_millis(),
        );
        Ok::<Vec<TrackInfo>, AppError>(tracks)
    })
    .await
    .map_err(|e| AppError::Other(e.to_string()))??;
    log::info!(
        target: "playlist-io",
        "tracks end playlist_id={}, count={}, elapsed_ms={}",
        id,
        tracks.len(),
        started.elapsed().as_millis(),
    );
    Ok(tracks)
}

#[tauri::command]
pub async fn add_to_playlist(app: AppHandle, playlist_id: i64, track: TrackInfo) -> AppResult<()> {
    let path = playlists_path();
    let mut store = PlaylistStore::load_strict(&path)?;
    let added_at = {
        let pl = store.playlists.iter().find(|p| p.id == playlist_id)
            .ok_or_else(|| AppError::NotFound("Playlist not found".into()))?;
        if pl
            .tracks
            .iter()
            .any(|current| playlist_track_key(current) == playlist_track_key(&track))
        {
            return Ok(());
        }
        next_track_added_at(&pl.tracks, 1)
    };
    let token = manager::next_sync_causal_tokens_pub(&app, 1)?
        .into_iter()
        .next()
        .expect("one causal token must be allocated");
    let stamped = stamp_track_for_playlist_insert(track, added_at, token);
    let sync_song = manager::tracks_to_sync_songs_pub(std::slice::from_ref(&stamped))
        .into_iter()
        .next();
    let pl = store.playlists.iter_mut().find(|p| p.id == playlist_id)
        .ok_or_else(|| AppError::NotFound("Playlist not found".into()))?;
    pl.tracks.insert(0, stamped);
    pl.modified_at = added_at as u64;
    if let Some(sync_song) = sync_song {
        store.clear_playlist_song_deletion(&playlist_id.to_string(), &sync_song);
    }
    store.save(&path)?;
    let _ = app.emit("playlists-changed", ());
    Ok(())
}


#[tauri::command]
pub async fn add_tracks_to_playlist(app: AppHandle, playlist_id: i64, tracks: Vec<TrackInfo>) -> AppResult<usize> {
    let path = playlists_path();
    let mut store = PlaylistStore::load_strict(&path)?;

    let mut seen = std::collections::HashSet::new();
    let new_tracks: Vec<TrackInfo> = {
        let pl = store.playlists.iter().find(|p| p.id == playlist_id)
            .ok_or_else(|| AppError::NotFound("Playlist not found".into()))?;
        tracks
            .into_iter()
            .filter(|track| {
                let key = playlist_track_key(track);
                !track.id.is_empty()
                    && !pl.tracks.iter().any(|current| playlist_track_key(current) == key)
                    && seen.insert(key)
            })
            .collect()
    };

    let added = new_tracks.len();
    if added > 0 {
        let newest_added_at = {
            let pl = store.playlists.iter().find(|p| p.id == playlist_id)
                .ok_or_else(|| AppError::NotFound("Playlist not found".into()))?;
            next_track_added_at(&pl.tracks, added)
        };
        let tokens = manager::next_sync_causal_tokens_pub(&app, added)?;
        let stamped_tracks: Vec<TrackInfo> = new_tracks
            .into_iter()
            .zip(tokens)
            .enumerate()
            .map(|(index, (track, token))| {
                let added_at = newest_added_at.saturating_sub(index as i64).max(1);
                stamp_track_for_playlist_insert(track, added_at, token)
            })
            .collect();
        for track in &stamped_tracks {
            if let Some(sync_song) = manager::tracks_to_sync_songs_pub(std::slice::from_ref(track))
                .into_iter()
                .next()
            {
                store.clear_playlist_song_deletion(&playlist_id.to_string(), &sync_song);
            }
        }
        let pl = store.playlists.iter_mut().find(|p| p.id == playlist_id)
            .ok_or_else(|| AppError::NotFound("Playlist not found".into()))?;
        for track in stamped_tracks.into_iter().rev() {
            pl.tracks.insert(0, track);
        }
        pl.modified_at = newest_added_at as u64;
        store.save(&path)?;
        let _ = app.emit("playlists-changed", ());
    }
    Ok(added)
}

fn stamp_track_for_playlist_insert(
    mut track: TrackInfo,
    added_at: i64,
    token: SyncCausalToken,
) -> TrackInfo {
    track.added_at = added_at.max(1);
    manager::attach_sync_membership_token_pub(&mut track, token);
    track
}

fn playlist_track_key(track: &TrackInfo) -> String {
    manager::playlist_track_identity_key_pub(track)
}

fn next_track_added_at(tracks: &[TrackInfo], count: usize) -> i64 {
    let now = chrono::Utc::now().timestamp_millis();
    let existing_max = tracks.iter().map(|track| track.added_at).max().unwrap_or(0);
    now.max(existing_max.saturating_add(count as i64)).max(1)
}

/// 按 track id / playlist_key 更新本地歌单中的曲目元数据(含 sync_payload)
/// 用于歌词编辑、偏移写入等需要回写 Android 对齐字段的场景
#[tauri::command]
pub async fn update_playlist_track(
    app: AppHandle,
    playlist_id: Option<i64>,
    track: TrackInfo,
) -> AppResult<usize> {
    let path = playlists_path();
    let mut store = PlaylistStore::load_strict(&path)?;
    let target_key = playlist_track_key(&track);
    let target_id = track.id.clone();
    let mut updated = 0usize;
    let now = chrono::Utc::now().timestamp_millis().max(1) as u64;

    for pl in store.playlists.iter_mut() {
        if let Some(pid) = playlist_id {
            if pl.id != pid {
                continue;
            }
        }
        let mut touched = false;
        for existing in pl.tracks.iter_mut() {
            let same = playlist_track_key(existing) == target_key
                || (!target_id.is_empty() && existing.id == target_id);
            if !same {
                continue;
            }
            // 保留歌单内加入时间与 playlist_key, 只覆盖展示字段与 sync_payload
            let added_at = existing.added_at;
            let playlist_key = existing.playlist_key.clone();
            *existing = track.clone();
            existing.added_at = added_at;
            if existing.playlist_key.is_none() {
                existing.playlist_key = playlist_key.or_else(|| Some(playlist_track_key(existing)));
            }
            // 确保上传载荷使用 CURRENT metadata version (对齐 Android fromSongItem)
            // 注意: 此处不能调用 normalized_for_sync, 它会把有意清空的 "" 歌词剥成 None,
            // 本地会丢失 CLEARED 语义并在下次播放时重新在线拉取
            if let Some(payload) = existing.sync_payload.as_mut() {
                if payload.sync_metadata_version
                    < crate::sync::models::CURRENT_SYNC_METADATA_VERSION
                {
                    payload.sync_metadata_version =
                        crate::sync::models::CURRENT_SYNC_METADATA_VERSION;
                }
            }
            touched = true;
            updated += 1;
        }
        if touched {
            pl.modified_at = now;
        }
    }

    if updated > 0 {
        store.save(&path)?;
        let _ = app.emit("playlists-changed", ());
    }
    Ok(updated)
}

#[tauri::command]
pub async fn remove_from_playlist(app: AppHandle, playlist_id: i64, track_id: String) -> AppResult<()> {
    let path = playlists_path();
    let mut store = PlaylistStore::load_strict(&path)?;
    let removed_track = {
        let pl = store.playlists.iter_mut().find(|p| p.id == playlist_id)
            .ok_or_else(|| AppError::NotFound("Playlist not found".into()))?;
        let uses_playlist_key = pl
            .tracks
            .iter()
            .any(|track| playlist_track_key(track) == track_id);
        let matches = |track: &TrackInfo| {
            if uses_playlist_key {
                playlist_track_key(track) == track_id
            } else {
                track.id == track_id
            }
        };
        let removed_track = pl.tracks.iter().find(|track| matches(track)).cloned();
        if removed_track.is_some() {
            pl.tracks.retain(|current| !matches(current));
            pl.modified_at = chrono::Utc::now().timestamp_millis() as u64;
        }
        removed_track
    };
    if let Some(track) = removed_track {
        let deletion = manager::track_to_playlist_song_deletion_pub(
            playlist_id,
            &track,
            manager::get_or_create_device_id_pub(&app),
        );
        store.record_playlist_song_deletion(deletion);
        store.save(&path)?;
        let _ = app.emit("playlists-changed", ());
    }
    Ok(())
}


#[tauri::command]
pub async fn remove_tracks_from_playlist(app: AppHandle, playlist_id: i64, track_ids: Vec<String>) -> AppResult<usize> {
    let path = playlists_path();
    let mut store = PlaylistStore::load_strict(&path)?;
    if !store.playlists.iter().any(|playlist| playlist.id == playlist_id) {
        return Err(AppError::NotFound("Playlist not found".into()));
    }

    let ids: std::collections::HashSet<String> = track_ids.into_iter().collect();
    if ids.is_empty() {
        return Ok(0);
    }

    let (removed_tracks, removed) = {
        let pl = store.playlists.iter_mut().find(|p| p.id == playlist_id)
            .ok_or_else(|| AppError::NotFound("Playlist not found".into()))?;
        let matches = |track: &TrackInfo| {
            ids.contains(&playlist_track_key(track)) || ids.contains(&track.id)
        };
        let removed_tracks: Vec<TrackInfo> = pl
            .tracks
            .iter()
            .filter(|track| matches(track))
            .cloned()
            .collect();
        let before = pl.tracks.len();
        pl.tracks.retain(|track| !matches(track));
        let removed = before.saturating_sub(pl.tracks.len());
        if removed > 0 {
            pl.modified_at = chrono::Utc::now().timestamp_millis() as u64;
        }
        (removed_tracks, removed)
    };

    if removed > 0 {
        let device_id = manager::get_or_create_device_id_pub(&app);
        for track in &removed_tracks {
            let deletion = manager::track_to_playlist_song_deletion_pub(
                playlist_id,
                track,
                device_id.clone(),
            );
            store.record_playlist_song_deletion(deletion);
        }
        store.save(&path)?;
        let _ = app.emit("playlists-changed", ());
    }
    Ok(removed)
}

/// 重排本地歌单内歌曲顺序（仅本地歌单支持）
///
/// 前端传入按新顺序排列的 playlist_key 列表；未包含的歌曲保持相对顺序追加到末尾。
/// 重排后按递减序列重新戳 added_at（对齐 Android stampSongsForDisplayOrder；桌面同步
/// 按 added_at 排序，这样自定义顺序才能跨端持久化）
#[tauri::command]
pub async fn reorder_playlist_tracks(
    app: AppHandle,
    playlist_id: i64,
    ordered_keys: Vec<String>,
) -> AppResult<usize> {
    let path = playlists_path();
    let mut store = PlaylistStore::load_strict(&path)?;

    let count = {
        let pl = store
            .playlists
            .iter_mut()
            .find(|p| p.id == playlist_id)
            .ok_or_else(|| AppError::NotFound("Playlist not found".into()))?;

        let order_index: std::collections::HashMap<String, usize> = ordered_keys
            .iter()
            .enumerate()
            .map(|(idx, key)| (key.clone(), idx))
            .collect();

        // 稳定排序：命中 key 用目标位次，未命中用末位并保留原相对序，保证不丢歌
        let fallback = ordered_keys.len();
        let mut indexed: Vec<(usize, usize, TrackInfo)> = std::mem::take(&mut pl.tracks)
            .into_iter()
            .enumerate()
            .map(|(orig_idx, track)| {
                let rank = playlist_track_order_aliases(&track)
                    .iter()
                    .find_map(|key| order_index.get(key).copied())
                    .unwrap_or(fallback);
                (rank, orig_idx, track)
            })
            .collect();
        indexed.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

        let now = chrono::Utc::now().timestamp_millis();
        let count = indexed.len();
        pl.tracks = indexed
            .into_iter()
            .enumerate()
            .map(|(idx, (_, _, mut track))| {
                track.added_at = (now - idx as i64).max(1);
                track
            })
            .collect();
        pl.modified_at = now as u64;
        count
    };

    store.save(&path)?;
    let _ = app.emit("playlists-changed", ());
    log::info!(
        target: "playlist-io",
        "reorder end playlist_id={}, count={}",
        playlist_id,
        count,
    );
    Ok(count)
}

fn playlist_track_order_aliases(track: &TrackInfo) -> Vec<String> {
    let mut aliases = vec![playlist_track_key(track)];
    if let Some(key) = track.playlist_key.as_ref().filter(|key| !key.trim().is_empty()) {
        aliases.push(key.clone());
    }
    if !track.id.is_empty() {
        aliases.push(track.id.clone());
        aliases.push(format!("{}|{}", track.id, track.album));
        aliases.push(format!("{}|{}|", track.id, track.album));
    }
    aliases
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::TrackSource;

    fn track(id: &str, album: &str, playlist_key: Option<&str>) -> TrackInfo {
        TrackInfo {
            id: id.to_string(),
            title: "Title".to_string(),
            artist: "Artist".to_string(),
            album: album.to_string(),
            duration_ms: 0,
            source: TrackSource::Local,
            url: String::new(),
            cover_url: None,
            added_at: 0,
            sync_payload: None,
            playlist_key: playlist_key.map(str::to_string),
        }
    }

    #[test]
    fn playlist_track_order_aliases_cover_current_and_legacy_keys() {
        let track = track("local-file-id", "Album", Some("stable|playlist|key"));
        let aliases = playlist_track_order_aliases(&track);

        assert!(aliases.iter().any(|key| key == "stable|playlist|key"));
        assert!(aliases.iter().any(|key| key == "local-file-id"));
        assert!(aliases.iter().any(|key| key == "local-file-id|Album"));
        assert!(aliases.iter().any(|key| key == "local-file-id|Album|"));
    }
}

/// 收藏歌单 + 已转换为播放格式的曲目
/// songs 是同步内部模型 (camelCase), 前端无法直接播放; tracks 走统一的 SyncSong -> TrackInfo 转换
#[derive(serde::Serialize)]
pub struct FavoritePlaylistWithTracks {
    #[serde(flatten)]
    pub favorite: SyncFavoritePlaylist,
    pub tracks: Vec<crate::state::TrackInfo>,
}

/// 获取收藏歌单列表
#[tauri::command]
pub async fn list_favorite_playlists() -> AppResult<Vec<FavoritePlaylistWithTracks>> {
    Ok(crate::sync::manager::load_favorite_playlists()?
        .into_iter()
        .map(|favorite| {
            let tracks = favorite
                .songs
                .iter()
                .map(crate::sync::manager::sync_song_to_track_pub)
                .collect();
            FavoritePlaylistWithTracks { favorite, tracks }
        })
        .collect())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistFavoriteInput {
    pub source: String,
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub cover_url: String,
    #[serde(default)]
    pub track_count: i32,
    #[serde(default)]
    pub browse_id: Option<String>,
    #[serde(default)]
    pub subtitle: Option<String>,
}

impl ArtistFavoriteInput {
    fn normalized(mut self) -> AppResult<Self> {
        self.name = self.name.trim().to_string();
        if self.name.is_empty() {
            return Err(AppError::Other("Artist name must not be empty".into()));
        }
        match self.source.as_str() {
            "neteaseArtist" | "biliArtist" => {
                let id = self.id.parse::<i64>().ok().filter(|id| *id > 0)
                    .ok_or_else(|| AppError::Other("Artist id must be positive".into()))?;
                self.id = id.to_string();
            }
            "youtubeMusicArtist" => {
                let browse_id = self.browse_id.as_deref().unwrap_or_default().trim();
                if browse_id.is_empty() {
                    return Err(AppError::Other("YouTube artist browseId is required".into()));
                }
                let digest = Sha256::digest(browse_id.as_bytes());
                let id = i64::from_be_bytes(digest[..8].try_into().expect("SHA-256 prefix"));
                self.id = if id == 0 { 1 } else { id }.to_string();
                self.browse_id = Some(browse_id.to_string());
            }
            _ => return Err(AppError::Other("Unsupported artist source".into())),
        }
        self.track_count = self.track_count.max(0);
        Ok(self)
    }

    fn matches(&self, favorite: &SyncFavoritePlaylist) -> bool {
        favorite.source == self.source && (favorite.id == self.id ||
            (self.source == "youtubeMusicArtist" && favorite.browse_id == self.browse_id))
    }

    fn into_favorite(self, now: i64) -> SyncFavoritePlaylist {
        SyncFavoritePlaylist {
            id: self.id, name: self.name, source: self.source, cover_url: self.cover_url,
            track_count: self.track_count, browse_id: self.browse_id, subtitle: self.subtitle,
            songs: Vec::new(), added_time: now, modified_at: now, sort_order: now,
            is_deleted: false, playlist_id: None,
        }
    }
}

#[tauri::command]
pub async fn set_artist_favorite(app: AppHandle, artist: ArtistFavoriteInput, following: bool) -> AppResult<bool> {
    let artist = artist.normalized()?;
    let now = chrono::Utc::now().timestamp_millis();
    manager::update_favorite_playlists(|favorites| {
        if let Some(existing) = favorites.iter_mut().find(|favorite| artist.matches(favorite)) {
            let was_deleted = existing.is_deleted;
            existing.is_deleted = !following;
            existing.modified_at = now.max(existing.modified_at.saturating_add(1));
            if following {
                if was_deleted {
                    existing.added_time = existing.modified_at;
                    existing.sort_order = existing.modified_at;
                }
                existing.name = artist.name;
                if !artist.cover_url.is_empty() { existing.cover_url = artist.cover_url; }
                existing.subtitle = artist.subtitle.or_else(|| existing.subtitle.clone());
                existing.browse_id = artist.browse_id.or_else(|| existing.browse_id.clone());
                existing.track_count = existing.track_count.max(artist.track_count);
            } else {
                existing.songs.clear();
                existing.track_count = 0;
            }
        } else {
            let mut favorite = artist.into_favorite(now);
            favorite.is_deleted = !following;
            favorites.push(favorite);
        }
        Ok(())
    })?;
    let _ = app.emit("playlists-changed", ());
    Ok(following)
}

fn merge_followed_artists(
    favorites: &mut Vec<SyncFavoritePlaylist>, artists: Vec<ArtistFavoriteInput>, started_at: i64, now: i64,
) -> AppResult<usize> {
    let mut imported = 0;
    let mut seen = std::collections::HashSet::new();
    for artist in artists {
        let artist = artist.normalized()?;
        if !seen.insert(format!("{}:{}", artist.source, artist.id)) { continue; }
        if let Some(existing) = favorites.iter_mut().find(|favorite| artist.matches(favorite)) {
            // 远端拉取只能添加，不能覆盖当前关注或拉取期间发生的取消关注
            if !existing.is_deleted || existing.modified_at >= started_at { continue; }
            *existing = artist.into_favorite(now.max(existing.modified_at.saturating_add(1)));
        } else {
            favorites.push(artist.into_favorite(now));
        }
        imported += 1;
    }
    Ok(imported)
}

fn artist_account_fingerprint(state: &AppState, source: &str) -> Option<Vec<(String, String)>> {
    let auth = state.auth.lock();
    let cookies = match source {
        "neteaseArtist" => &auth.netease.as_ref().filter(|auth| auth.has_login())?.cookies,
        "youtubeMusicArtist" => &auth.youtube.as_ref().filter(|auth| auth.has_login())?.cookies,
        _ => return None,
    };
    let mut identity: Vec<_> = cookies.iter().filter(|cookie| match source {
        "neteaseArtist" => cookie.name == "MUSIC_U",
        _ => matches!(cookie.name.as_str(), "SAPISID" | "__Secure-3PAPISID" | "SID" | "LOGIN_INFO" | "authuser"),
    }).map(|cookie| (cookie.name.clone(), cookie.value.clone())).collect();
    identity.sort();
    Some(identity)
}

#[tauri::command]
pub async fn import_followed_artists(app: AppHandle, source: String, state: State<'_, AppState>) -> AppResult<usize> {
    let started_at = chrono::Utc::now().timestamp_millis();
    let account = artist_account_fingerprint(&state, &source)
        .ok_or_else(|| AppError::Other("请先登录该平台".into()))?;
    let artists = match source.as_str() {
        "neteaseArtist" => load_netease_followed_artists(&state).await?,
        "youtubeMusicArtist" => {
            let auth = state.auth.lock().youtube.clone()
                .ok_or_else(|| AppError::Other("请先登录 YouTube".into()))?;
            state.youtube().get_followed_artists(&auth).await?.into_iter().map(|artist| ArtistFavoriteInput {
                source: source.clone(), id: String::new(), name: artist.name,
                cover_url: artist.cover_url.unwrap_or_default(), track_count: 0,
                browse_id: Some(artist.browse_id), subtitle: Some(artist.subtitle),
            }).collect()
        }
        _ => return Err(AppError::Other("Unsupported artist source".into())),
    };
    let count = manager::update_favorite_playlists(|favorites| {
        if artist_account_fingerprint(&state, &source).as_ref() != Some(&account) {
            return Err(AppError::Other("拉取期间账号已变更，请重试".into()));
        }
        merge_followed_artists(favorites, artists, started_at, chrono::Utc::now().timestamp_millis())
    })?;
    if count > 0 { let _ = app.emit("playlists-changed", ()); }
    Ok(count)
}

async fn load_netease_followed_artists(state: &AppState) -> AppResult<Vec<ArtistFavoriteInput>> {
    let client = state.netease();
    let mut offset = 0_u32;
    let mut visited = std::collections::HashSet::new();
    let mut artists = Vec::new();
    for _ in 0..200 {
        let page = client.get_followed_artists(offset, 50).await?;
        let (items, has_more, raw_count) = parse_netease_followed_artist_page(&page)?;
        let page_ids: Vec<_> = items.iter().map(|artist| artist.id.clone()).collect();
        if raw_count > 0 && !visited.insert(page_ids) {
            return Err(AppError::Api("Repeated artist subscription page".into()));
        }
        artists.extend(items);
        if !has_more { return Ok(artists); }
        offset = offset.checked_add(raw_count).ok_or_else(|| AppError::Api("Artist pagination overflow".into()))?;
    }
    Err(AppError::Api("Artist pagination exceeded budget".into()))
}

fn parse_netease_followed_artist_page(page: &Value) -> AppResult<(Vec<ArtistFavoriteInput>, bool, u32)> {
    if page["code"].as_i64() != Some(200) {
        return Err(AppError::Api("Artist subscriptions request failed".into()));
    }
    let raw = page["data"].as_array().ok_or_else(|| AppError::Api("Artist subscriptions have no data array".into()))?;
    let has_more = page["hasMore"].as_bool().ok_or_else(|| AppError::Api("Artist subscriptions have no pagination state".into()))?;
    if has_more && raw.is_empty() {
        return Err(AppError::Api("Artist subscription pagination did not advance".into()));
    }
    let artists = raw.iter().filter_map(|artist| {
        let id = artist["id"].as_i64().filter(|id| *id > 0)?;
        let name = artist["name"].as_str()?.trim();
        if name.is_empty() { return None; }
        let cover_url = ["picUrl", "img1v1Url", "cover"].iter().find_map(|key|
            artist[key].as_str().filter(|url| !url.trim().is_empty())).unwrap_or_default();
        let aliases = artist["alias"].as_array().map(|aliases| aliases.iter().filter_map(Value::as_str)
            .filter(|alias| !alias.trim().is_empty()).collect::<Vec<_>>().join(" / "));
        Some(ArtistFavoriteInput {
            source: "neteaseArtist".into(), id: id.to_string(), name: name.into(),
            cover_url: normalize_artist_cover(cover_url), track_count: artist["musicSize"].as_i64()
                .unwrap_or(0).clamp(0, i32::MAX as i64) as i32,
            browse_id: None, subtitle: aliases,
        })
    }).collect();
    Ok((artists, has_more, raw.len().try_into().map_err(|_| AppError::Api("Artist page exceeds budget".into()))?))
}

fn normalize_artist_cover(value: &str) -> String {
    if value.starts_with("//") { format!("https:{value}") }
    else if let Some(path) = value.strip_prefix("http://") { format!("https://{path}") }
    else { value.to_string() }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BiliArtistHeader {
    pub name: String,
    pub cover_url: String,
    pub banner_url: String,
    pub description: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BiliArtistDetail {
    pub header: BiliArtistHeader,
    pub tracks: Vec<TrackInfo>,
    pub page: u32,
    pub total: u64,
    pub has_more: bool,
}

#[tauri::command]
pub async fn get_bili_artist_detail(mid: u64, page: Option<u32>, state: State<'_, AppState>) -> AppResult<BiliArtistDetail> {
    if mid == 0 { return Err(AppError::Other("Uploader mid must be positive".into())); }
    let page = page.unwrap_or(1).max(1);
    let client = state.bilibili();
    let (profile, videos) = tokio::try_join!(client.get_uploader_profile(mid), client.get_uploader_videos(mid, page))?;
    Ok(parse_bili_artist_detail(&profile, &videos, page))
}

fn parse_bili_artist_detail(profile: &Value, videos: &Value, page: u32) -> BiliArtistDetail {
    let profile = &profile["data"];
    let header = BiliArtistHeader {
        name: profile["name"].as_str().unwrap_or_default().into(),
        cover_url: normalize_artist_cover(profile["face"].as_str().unwrap_or_default()),
        banner_url: normalize_artist_cover(profile["top_photo"].as_str().unwrap_or_default()),
        description: profile["sign"].as_str().unwrap_or_default().into(),
    };
    let items = videos["data"]["list"]["vlist"].as_array();
    let tracks = items.into_iter().flatten().filter_map(|video| {
        let bvid = video["bvid"].as_str().filter(|bvid| !bvid.is_empty())?;
        let title = video["title"].as_str().filter(|title| !title.is_empty())?;
        let duration = video["length"].as_str().unwrap_or_default().split(':')
            .try_fold(0_u64, |total, part| part.parse::<u64>().ok().and_then(|part| total.checked_mul(60)?.checked_add(part))).unwrap_or(0);
        Some(TrackInfo {
            id: format!("bilibili:{bvid}"), title: title.into(),
            artist: video["author"].as_str().filter(|name| !name.is_empty()).unwrap_or(&header.name).into(),
            album: String::new(), duration_ms: duration.saturating_mul(1000), source: TrackSource::Bilibili,
            url: String::new(), cover_url: Some(normalize_artist_cover(video["pic"].as_str().unwrap_or_default())),
            added_at: 0, sync_payload: None, playlist_key: None,
        })
    }).collect::<Vec<_>>();
    let total = videos["data"]["page"]["count"].as_u64().unwrap_or(tracks.len() as u64);
    BiliArtistDetail { header, tracks, page, total, has_more: u64::from(page).saturating_mul(30) < total }
}

#[tauri::command]
pub async fn get_youtube_artist_detail(browse_id: String, state: State<'_, AppState>) -> AppResult<Value> {
    let auth = state.auth.lock().youtube.clone();
    state.youtube().get_creator_detail(&browse_id, auth.as_ref()).await
}

#[tauri::command]
pub async fn get_youtube_artist_items(browse_id: String, params: Option<String>, continuation: Option<String>, state: State<'_, AppState>) -> AppResult<Value> {
    let auth = state.auth.lock().youtube.clone();
    state.youtube().get_creator_items(&browse_id, params.as_deref(), continuation.as_deref(), auth.as_ref()).await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BiliArtistContent {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub cover_url: String,
    pub description: String,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BiliArtistContents {
    pub collections: Vec<BiliArtistContent>,
    pub series: Vec<BiliArtistContent>,
    pub page: u32,
    pub has_more: bool,
}

#[tauri::command]
pub async fn get_bili_artist_contents(mid: u64, page: Option<u32>, state: State<'_, AppState>) -> AppResult<BiliArtistContents> {
    if mid == 0 { return Err(AppError::Other("Uploader mid must be positive".into())); }
    let page = page.unwrap_or(1).max(1);
    let raw = state.bilibili().get_uploader_contents(mid, page).await?;
    Ok(parse_bili_artist_contents(&raw, page))
}

fn parse_bili_artist_contents(raw: &Value, page: u32) -> BiliArtistContents {
    let lists = &raw["data"]["items_lists"];
    let parse = |key: &str, kind: &str, id_key: &str| {
        lists[key].as_array().into_iter().flatten().filter_map(|value| {
            let meta = value.get("meta").unwrap_or(value);
            let id = meta[id_key].as_u64().filter(|id| *id > 0)?;
            let name = meta["name"].as_str().or_else(|| meta["title"].as_str()).filter(|name| !name.is_empty())?;
            Some(BiliArtistContent {
                id: id.to_string(), kind: kind.into(), name: name.into(),
                cover_url: normalize_artist_cover(meta["cover"].as_str().unwrap_or_default()),
                description: meta["description"].as_str().unwrap_or_default().into(),
                total: meta["total"].as_u64().unwrap_or_default(),
            })
        }).collect::<Vec<_>>()
    };
    let collections = parse("seasons_list", "collection", "season_id");
    let series = parse("series_list", "series", "series_id");
    let total = lists["page"]["total"].as_u64().unwrap_or((collections.len() + series.len()) as u64);
    BiliArtistContents { collections, series, page, has_more: u64::from(page).saturating_mul(20) < total }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BiliArtistCollection {
    pub tracks: Vec<TrackInfo>,
    pub page: u32,
    pub total: u64,
    pub has_more: bool,
}

#[tauri::command]
pub async fn get_bili_artist_collection(mid: u64, content_id: u64, kind: String, page: Option<u32>, state: State<'_, AppState>) -> AppResult<BiliArtistCollection> {
    if mid == 0 || content_id == 0 { return Err(AppError::Other("Uploader content id must be positive".into())); }
    let page = page.unwrap_or(1).max(1);
    let raw = state.bilibili().get_uploader_collection(mid, content_id, &kind, page).await?;
    Ok(parse_bili_artist_collection(&raw, page))
}

fn parse_bili_artist_collection(raw: &Value, page: u32) -> BiliArtistCollection {
    let tracks = raw["data"]["archives"].as_array().into_iter().flatten().filter_map(|video| {
        let bvid = video["bvid"].as_str().filter(|bvid| !bvid.is_empty())?;
        let title = video["title"].as_str().filter(|title| !title.is_empty())?;
        Some(TrackInfo {
            id: format!("bilibili:{bvid}"), title: title.into(), artist: String::new(), album: String::new(),
            duration_ms: video["duration"].as_u64().unwrap_or_default().saturating_mul(1000),
            source: TrackSource::Bilibili, url: String::new(),
            cover_url: Some(normalize_artist_cover(video["pic"].as_str().unwrap_or_default())),
            added_at: 0, sync_payload: None, playlist_key: None,
        })
    }).collect::<Vec<_>>();
    let total = raw["data"]["page"]["total"].as_u64().or_else(|| raw["data"]["meta"]["total"].as_u64()).unwrap_or(tracks.len() as u64);
    BiliArtistCollection { tracks, page, total, has_more: u64::from(page).saturating_mul(30) < total }
}

#[cfg(test)]
mod artist_tests {
    use super::*;
    use serde_json::json;

    fn artist(id: &str) -> ArtistFavoriteInput {
        ArtistFavoriteInput {
            source: "neteaseArtist".into(), id: id.into(), name: "New remote name".into(),
            cover_url: String::new(), track_count: 0, browse_id: None, subtitle: None,
        }
    }

    #[test]
    fn followed_artist_merge_preserves_active_and_concurrent_deleted_favorites() {
        let mut active = artist("1").into_favorite(10);
        active.name = "Local display name".into();
        let mut recent_delete = artist("2").into_favorite(150);
        recent_delete.is_deleted = true;
        let mut old_delete = artist("3").into_favorite(50);
        old_delete.is_deleted = true;
        let mut favorites = vec![active, recent_delete, old_delete];
        let count = merge_followed_artists(&mut favorites, vec![artist("1"), artist("2"), artist("3"), artist("4"), artist("4")], 100, 200).unwrap();
        assert_eq!(count, 2);
        assert_eq!(favorites[0].name, "Local display name");
        assert_eq!(favorites[0].modified_at, 10);
        assert!(favorites[1].is_deleted);
        assert!(!favorites[2].is_deleted);
        assert_eq!(favorites.len(), 4);
    }

    #[test]
    fn followed_artist_page_requires_real_pagination_and_preserves_aliases() {
        let (artists, more, raw) = parse_netease_followed_artist_page(&json!({
            "code": 200, "hasMore": false, "data": [{"id": 12, "name": " Artist ", "picUrl": "http://cover/a", "musicSize": 8, "alias": ["Alias"]}, {"id": 0}]
        })).unwrap();
        assert_eq!(artists.len(), 1);
        assert_eq!(artists[0].cover_url, "https://cover/a");
        assert_eq!(artists[0].subtitle.as_deref(), Some("Alias"));
        assert_eq!(raw, 2);
        assert!(!more);
        for invalid in [json!({"code": 301}), json!({"code":200,"data":[]}), json!({"code":200,"data":[],"hasMore":true})] {
            assert!(parse_netease_followed_artist_page(&invalid).is_err());
        }
    }

    #[test]
    fn youtube_artist_id_uses_android_signed_sha256_long() {
        let mut input = artist("unsafe JS id");
        input.source = "youtubeMusicArtist".into();
        input.browse_id = Some("UCdemoCreator".into());
        let normalized = input.normalized().unwrap();
        let expected = i64::from_be_bytes(Sha256::digest(b"UCdemoCreator")[..8].try_into().unwrap());
        assert_eq!(normalized.id, expected.to_string());
        assert!(artist("0").normalized().is_err());
    }

    #[test]
    fn bili_artist_detail_preserves_video_ids_duration_and_pagination() {
        let detail = parse_bili_artist_detail(&json!({"data":{"name":"UP","face":"//avatar/a","top_photo":"http://banner/a","sign":"About"}}),
            &json!({"data":{"page":{"count":31},"list":{"vlist":[{"bvid":"BV1demo","title":"Demo","length":"1:02:03","pic":"//cover/a"},{"bvid":"","title":"invalid"}]}}}), 1);
        assert_eq!(detail.tracks.len(), 1);
        assert_eq!(detail.tracks[0].id, "bilibili:BV1demo");
        assert_eq!(detail.tracks[0].duration_ms, 3_723_000);
        assert_eq!(detail.tracks[0].artist, "UP");
        assert_eq!(detail.header.cover_url, "https://avatar/a");
        assert!(detail.has_more);
    }

    #[test]
    fn bili_artist_contents_preserve_collection_series_kind_and_archive_pagination() {
        let contents = parse_bili_artist_contents(&json!({"data":{"items_lists":{
            "page":{"total":21},"seasons_list":[{"meta":{"season_id":12,"name":"Collection","cover":"//cover/collection"}}],
            "series_list":[{"meta":{"series_id":13,"name":"Series"}}]
        }}}), 1);
        assert_eq!(contents.collections[0].kind, "collection");
        assert_eq!(contents.series[0].kind, "series");
        assert!(contents.has_more);
        let archive = parse_bili_artist_collection(&json!({"data":{"page":{"total":31},"archives":[
            {"bvid":"BVcollection","title":"Episode","duration":201,"pic":"http://cover/episode"}
        ]}}), 1);
        assert_eq!(archive.tracks[0].id, "bilibili:BVcollection");
        assert_eq!(archive.tracks[0].duration_ms, 201000);
        assert!(archive.has_more);
    }
}
