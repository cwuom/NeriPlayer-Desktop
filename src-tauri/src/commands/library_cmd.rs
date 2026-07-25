use crate::error::{AppError, AppResult};
use crate::library::{
    playlist::{acquire_playlist_io_lock, PlaylistStore},
    scanner,
};
use crate::state::TrackInfo;
use crate::sync::manager;
use crate::sync::models::SyncCausalToken;
use crate::sync::models::SyncFavoritePlaylist;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tauri::{AppHandle, Emitter};

#[tauri::command]
pub async fn scan_music_directory(
    dir: String,
    name_template: Option<String>,
) -> AppResult<Vec<TrackInfo>> {
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

const SYSTEM_FAVORITES_PLAYLIST_ID: i64 = -1001;
const SYSTEM_LOCAL_FILES_PLAYLIST_ID: i64 = -1002;

fn is_system_playlist_id(id: i64) -> bool {
    matches!(
        id,
        SYSTEM_FAVORITES_PLAYLIST_ID | SYSTEM_LOCAL_FILES_PLAYLIST_ID
    )
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
    let _playlist_guard = acquire_playlist_io_lock()?;
    let mut store = PlaylistStore::load(&path);

    // 自动清理重复歌单（同名只保留歌曲最多的）
    let mut needs_save = false;
    let mut seen_names: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut to_remove = Vec::new();
    for (i, pl) in store.playlists.iter().enumerate() {
        let name = pl.name.trim().to_string();
        if let Some(&prev_idx) = seen_names.get(&name) {
            if pl.tracks.len() > store.playlists[prev_idx].tracks.len() {
                to_remove.push(prev_idx);
                seen_names.insert(name, i);
            } else {
                to_remove.push(i);
            }
            needs_save = true;
        } else {
            seen_names.insert(name, i);
        }
    }
    if needs_save {
        to_remove.sort_unstable();
        to_remove.dedup();
        for &idx in to_remove.iter().rev() {
            store.playlists.remove(idx);
        }
        let _ = store.save(&path);
    }

    let list: Vec<PlaylistInfo> = store
        .playlists
        .iter()
        .map(|p| {
            let cover = p.tracks.iter().find_map(|t| {
                t.cover_url
                    .as_ref()
                    .filter(|url| !url.trim().is_empty())
                    .cloned()
            });
            let mut seen = std::collections::HashSet::new();
            let unique_count = p
                .tracks
                .iter()
                .filter(|track| !track.id.is_empty() && seen.insert(playlist_track_key(track)))
                .count();
            PlaylistInfo {
                id: p.id,
                name: p.name.clone(),
                track_count: unique_count,
                modified_at: p.modified_at,
                cover_url: cover,
            }
        })
        .collect();
    Ok(list)
}

#[tauri::command]
pub async fn create_playlist(app: AppHandle, name: String) -> AppResult<PlaylistInfo> {
    let _playlist_guard = acquire_playlist_io_lock()?;
    let path = playlists_path();
    let mut store = PlaylistStore::load(&path);
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
    if is_system_playlist_id(id) {
        return Ok(false);
    }
    let _playlist_guard = acquire_playlist_io_lock()?;
    let path = playlists_path();
    let mut store = PlaylistStore::load(&path);
    let deleted = store.delete(id);
    if deleted {
        store.save(&path)?;
        let _ = app.emit("playlists-changed", ());
    }
    Ok(deleted)
}

#[tauri::command]
pub async fn rename_playlist(app: AppHandle, id: i64, name: String) -> AppResult<bool> {
    if is_system_playlist_id(id) {
        return Ok(false);
    }
    let _playlist_guard = acquire_playlist_io_lock()?;
    let path = playlists_path();
    let mut store = PlaylistStore::load(&path);
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
        let store = PlaylistStore::load(&path);
        let pl = store
            .playlists
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| AppError::NotFound("Playlist not found".into()))?;
        let mut seen = std::collections::HashSet::new();
        let tracks: Vec<TrackInfo> = pl
            .tracks
            .iter()
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
    let _playlist_guard = acquire_playlist_io_lock()?;
    let path = playlists_path();
    let mut store = PlaylistStore::load(&path);
    let added_at = {
        let pl = store
            .playlists
            .iter()
            .find(|p| p.id == playlist_id)
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
    let pl = store
        .playlists
        .iter_mut()
        .find(|p| p.id == playlist_id)
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
pub async fn add_tracks_to_playlist(
    app: AppHandle,
    playlist_id: i64,
    tracks: Vec<TrackInfo>,
) -> AppResult<usize> {
    let _playlist_guard = acquire_playlist_io_lock()?;
    let path = playlists_path();
    let mut store = PlaylistStore::load(&path);

    let mut seen = std::collections::HashSet::new();
    let new_tracks: Vec<TrackInfo> = {
        let pl = store
            .playlists
            .iter()
            .find(|p| p.id == playlist_id)
            .ok_or_else(|| AppError::NotFound("Playlist not found".into()))?;
        tracks
            .into_iter()
            .filter(|track| {
                let key = playlist_track_key(track);
                !track.id.is_empty()
                    && !pl
                        .tracks
                        .iter()
                        .any(|current| playlist_track_key(current) == key)
                    && seen.insert(key)
            })
            .collect()
    };

    let added = new_tracks.len();
    if added > 0 {
        let newest_added_at = {
            let pl = store
                .playlists
                .iter()
                .find(|p| p.id == playlist_id)
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
        let pl = store
            .playlists
            .iter_mut()
            .find(|p| p.id == playlist_id)
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
    let _playlist_guard = acquire_playlist_io_lock()?;
    let path = playlists_path();
    let mut store = PlaylistStore::load(&path);
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
pub async fn remove_from_playlist(
    app: AppHandle,
    playlist_id: i64,
    track_id: String,
) -> AppResult<()> {
    let _playlist_guard = acquire_playlist_io_lock()?;
    let path = playlists_path();
    let mut store = PlaylistStore::load(&path);
    let removed_track = {
        let pl = store
            .playlists
            .iter_mut()
            .find(|p| p.id == playlist_id)
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
pub async fn remove_tracks_from_playlist(
    app: AppHandle,
    playlist_id: i64,
    track_ids: Vec<String>,
) -> AppResult<usize> {
    let _playlist_guard = acquire_playlist_io_lock()?;
    let path = playlists_path();
    let mut store = PlaylistStore::load(&path);
    if !store
        .playlists
        .iter()
        .any(|playlist| playlist.id == playlist_id)
    {
        return Err(AppError::NotFound("Playlist not found".into()));
    }

    let ids: std::collections::HashSet<String> = track_ids.into_iter().collect();
    if ids.is_empty() {
        return Ok(0);
    }

    let (removed_tracks, removed) = {
        let pl = store
            .playlists
            .iter_mut()
            .find(|p| p.id == playlist_id)
            .ok_or_else(|| AppError::NotFound("Playlist not found".into()))?;
        let matches =
            |track: &TrackInfo| ids.contains(&playlist_track_key(track)) || ids.contains(&track.id);
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
            let deletion =
                manager::track_to_playlist_song_deletion_pub(playlist_id, track, device_id.clone());
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
    let _playlist_guard = acquire_playlist_io_lock()?;
    let path = playlists_path();
    let mut store = PlaylistStore::load(&path);

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
    if let Some(key) = track
        .playlist_key
        .as_ref()
        .filter(|key| !key.trim().is_empty())
    {
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

/// 获取收藏歌单列表
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoritePlaylistInput {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub cover_url: String,
    #[serde(default)]
    pub track_count: i32,
    pub source: String,
    #[serde(default)]
    pub tracks: Vec<TrackInfo>,
}

#[tauri::command]
pub async fn add_favorite_playlist(
    app: AppHandle,
    input: FavoritePlaylistInput,
) -> AppResult<SyncFavoritePlaylist> {
    let id = input
        .id
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .map(|id| id.to_string())
        .ok_or_else(|| AppError::Other("Favorite playlist ID must be a positive integer".into()))?;
    let source = input.source.trim().to_ascii_lowercase();
    if source.is_empty() {
        return Err(AppError::Other(
            "Favorite playlist source is required".into(),
        ));
    }
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Other("Favorite playlist name is required".into()));
    }

    let songs = manager::tracks_to_sync_songs_pub(&input.tracks);
    let song_count = i32::try_from(songs.len()).unwrap_or(i32::MAX);
    let now = chrono::Utc::now().timestamp_millis();
    let favorite = manager::mutate_favorite_playlists_pub(move |favorites| {
        let existing_index = favorites.iter().position(|favorite| {
            favorite.id == id && favorite.source.eq_ignore_ascii_case(&source)
        });
        let existing = existing_index
            .and_then(|index| favorites.get(index))
            .cloned();
        let preserve_order = existing
            .as_ref()
            .is_some_and(|favorite| !favorite.is_deleted);
        let added_time = existing
            .as_ref()
            .filter(|_| preserve_order)
            .map(|favorite| favorite.added_time)
            .filter(|value| *value > 0)
            .unwrap_or(now);
        let sort_order = existing
            .as_ref()
            .filter(|_| preserve_order)
            .map(|favorite| favorite.sort_order)
            .filter(|value| *value != 0)
            .unwrap_or(added_time);
        let favorite = SyncFavoritePlaylist {
            id: id.clone(),
            name: name.clone(),
            cover_url: input.cover_url.trim().to_string(),
            track_count: input.track_count.max(song_count).max(0),
            source: source.clone(),
            songs: songs.clone(),
            added_time,
            modified_at: now,
            is_deleted: false,
            sort_order,
            browse_id: existing
                .as_ref()
                .and_then(|favorite| favorite.browse_id.clone()),
            playlist_id: existing
                .as_ref()
                .and_then(|favorite| favorite.playlist_id.clone()),
            subtitle: existing
                .as_ref()
                .and_then(|favorite| favorite.subtitle.clone()),
        }
        .normalized_for_sync();

        if let Some(index) = existing_index {
            favorites[index] = favorite.clone();
        } else {
            favorites.push(favorite.clone());
        }
        Ok(favorite)
    })
    .await?;

    let _ = app.emit("favorite-playlists-changed", ());
    Ok(favorite)
}

#[tauri::command]
pub async fn remove_favorite_playlist(
    app: AppHandle,
    id: String,
    source: String,
) -> AppResult<bool> {
    let id = id
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .map(|id| id.to_string())
        .ok_or_else(|| AppError::Other("Favorite playlist ID must be a positive integer".into()))?;
    let source = source.trim().to_ascii_lowercase();
    if source.is_empty() {
        return Err(AppError::Other(
            "Favorite playlist source is required".into(),
        ));
    }
    let now = chrono::Utc::now().timestamp_millis();
    let changed = manager::mutate_favorite_playlists_pub(move |favorites| {
        if let Some(favorite) = favorites
            .iter_mut()
            .find(|favorite| favorite.id == id && favorite.source.eq_ignore_ascii_case(&source))
        {
            if favorite.is_deleted {
                return Ok(false);
            }
            favorite.is_deleted = true;
            favorite.modified_at = now;
            favorite.track_count = 0;
            favorite.songs.clear();
        } else {
            favorites.push(SyncFavoritePlaylist {
                id,
                name: String::new(),
                cover_url: String::new(),
                track_count: 0,
                source,
                songs: Vec::new(),
                added_time: now,
                modified_at: now,
                is_deleted: true,
                sort_order: now,
                browse_id: None,
                playlist_id: None,
                subtitle: None,
            });
        }
        Ok(true)
    })
    .await?;

    if changed {
        let _ = app.emit("favorite-playlists-changed", ());
    }
    Ok(changed)
}

#[tauri::command]
pub async fn list_favorite_playlists() -> AppResult<Vec<SyncFavoritePlaylist>> {
    Ok(manager::load_favorite_playlists_strict_pub()?
        .into_iter()
        .filter(|favorite| !favorite.is_deleted)
        .collect())
}
