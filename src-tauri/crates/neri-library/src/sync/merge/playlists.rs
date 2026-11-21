//! 歌单合并：删除墓碑、单歌单三方合并与排序，以及歌单内歌曲删除记录

use super::*;

pub(super) fn merge_playlists(
    local: &[SyncPlaylist],
    remote: &[SyncPlaylist],
    last_sync_time: i64,
    base_snapshot: &HashMap<String, HashSet<String>>,
    deletions: &[SyncPlaylistSongDeletion],
) -> Vec<SyncPlaylist> {
    let local_map: HashMap<&str, &SyncPlaylist> = local
        .iter()
        .map(|playlist| (playlist.id.as_str(), playlist))
        .collect();
    let remote_map: HashMap<&str, &SyncPlaylist> = remote
        .iter()
        .map(|playlist| (playlist.id.as_str(), playlist))
        .collect();
    let all_ids: HashSet<&str> = local_map.keys().chain(remote_map.keys()).copied().collect();
    let mut merged = BTreeMap::new();

    for id in all_ids {
        let result = match (local_map.get(id).copied(), remote_map.get(id).copied()) {
            (Some(local_playlist), None) => apply_deletions_to_playlist(local_playlist, deletions),
            (None, Some(remote_playlist)) => {
                apply_deletions_to_playlist(remote_playlist, deletions)
            }
            (Some(local_playlist), Some(remote_playlist)) => {
                if should_keep_playlist_deleted(local_playlist, remote_playlist) {
                    merge_deleted_playlist(local_playlist, remote_playlist)
                } else if local_playlist.is_deleted || remote_playlist.is_deleted {
                    // 另一端在删除之后又改过（或 Android 撤销了删除），活着的那份赢
                    let active = if local_playlist.is_deleted {
                        remote_playlist
                    } else {
                        local_playlist
                    };
                    apply_deletions_to_playlist(active, deletions)
                } else {
                    merge_single_playlist(
                        local_playlist,
                        remote_playlist,
                        last_sync_time,
                        base_snapshot.get(id).cloned().unwrap_or_default(),
                        deletions,
                    )
                }
            }
            (None, None) => continue,
        };
        merged.insert(id.to_string(), result);
    }

    order_merged_playlists(&merged, local, remote, last_sync_time)
        .into_iter()
        .map(|playlist| playlist.normalized_for_display_order())
        .collect()
}

fn apply_deletions_to_playlist(
    playlist: &SyncPlaylist,
    deletions: &[SyncPlaylistSongDeletion],
) -> SyncPlaylist {
    if playlist.is_deleted {
        return playlist.clone();
    }
    let mut merged = playlist.clone();
    merged.songs = apply_playlist_song_deletions(&playlist.id, &playlist.songs, deletions);
    merged
}

/// 对齐 Android SyncPlaylistDeletionPolicy.shouldKeepPlaylistDeleted：
/// 一端删除、另一端还在时，只有删除不早于那份歌单的最后修改才保留墓碑
fn should_keep_playlist_deleted(left: &SyncPlaylist, right: &SyncPlaylist) -> bool {
    match (left.is_deleted, right.is_deleted) {
        (false, false) => false,
        (true, true) => true,
        _ => {
            let (deleted, active) = if left.is_deleted {
                (left, right)
            } else {
                (right, left)
            };
            deleted.modified_at >= active.modified_at
        }
    }
}

fn merge_deleted_playlist(local: &SyncPlaylist, remote: &SyncPlaylist) -> SyncPlaylist {
    SyncPlaylist {
        id: local.id.clone(),
        name: if !local.name.trim().is_empty() {
            local.name.clone()
        } else {
            remote.name.clone()
        },
        songs: Vec::new(),
        created_at: min_positive(local.created_at, remote.created_at),
        modified_at: local.modified_at.max(remote.modified_at),
        is_deleted: true,
        song_order_version: DISPLAY_ORDER_SONG_ORDER_VERSION,
    }
}

fn merge_single_playlist(
    local: &SyncPlaylist,
    remote: &SyncPlaylist,
    last_sync_time: i64,
    base_songs: HashSet<String>,
    deletions: &[SyncPlaylistSongDeletion],
) -> SyncPlaylist {
    let local_changed = local.modified_at > last_sync_time;
    let remote_changed = remote.modified_at > last_sync_time;
    // 仅远端单侧改名时取远端；双方都改（或都未改但名字不同）固定 local-wins，
    // 对齐 Android GitHubSyncManager 的 PLAYLIST_RENAMED_BOTH_SIDES 裁决。
    // 按 modified_at 较新者裁决会让快钟设备永远赢且双端结果不同，名字乒乓。
    let name = if local.name == remote.name {
        local.name.clone()
    } else if remote_changed && !local_changed {
        remote.name.clone()
    } else {
        local.name.clone()
    };

    SyncPlaylist {
        id: local.id.clone(),
        name,
        songs: merge_songs(
            &local.songs,
            &remote.songs,
            local.modified_at,
            remote.modified_at,
            local_changed,
            remote_changed,
            last_sync_time,
            local.id == "-1001" || remote.id == "-1001",
            &base_songs,
            &local.id,
            deletions,
        ),
        created_at: min_positive(local.created_at, remote.created_at),
        modified_at: local.modified_at.max(remote.modified_at),
        is_deleted: false,
        song_order_version: DISPLAY_ORDER_SONG_ORDER_VERSION,
    }
}

fn order_merged_playlists(
    merged: &BTreeMap<String, SyncPlaylist>,
    local: &[SyncPlaylist],
    remote: &[SyncPlaylist],
    last_sync_time: i64,
) -> Vec<SyncPlaylist> {
    let local_changed = local
        .iter()
        .any(|playlist| playlist.modified_at > last_sync_time);
    let remote_changed = remote
        .iter()
        .any(|playlist| playlist.modified_at > last_sync_time);
    let (primary, secondary) = if remote_changed && !local_changed {
        (remote, local)
    } else {
        (local, remote)
    };
    let mut ordered_ids = Vec::new();
    let mut seen = HashSet::new();
    // 与 Android orderMergedPlaylists 的 filterNot(isDeleted) 对齐：
    // 已删歌单不参与来源顺序，统一由下方兜底循环按 key 序补在末尾，
    // 否则双端排序结果不同会被误判为数据变更而回声上传
    for playlist in primary.iter().chain(secondary.iter()) {
        if playlist.is_deleted {
            continue;
        }
        if merged.contains_key(&playlist.id) && seen.insert(playlist.id.clone()) {
            ordered_ids.push(playlist.id.clone());
        }
    }
    for id in merged.keys() {
        if seen.insert(id.clone()) {
            ordered_ids.push(id.clone());
        }
    }
    ordered_ids
        .into_iter()
        .filter_map(|id| merged.get(&id).cloned())
        .collect()
}

pub(super) fn apply_playlist_song_deletions(
    playlist_id: &str,
    songs: &[SyncSong],
    deletions: &[SyncPlaylistSongDeletion],
) -> Vec<SyncSong> {
    let relevant: Vec<&SyncPlaylistSongDeletion> = deletions
        .iter()
        .filter(|deletion| deletion.playlist_id == playlist_id)
        .collect();
    if relevant.is_empty() {
        return songs.to_vec();
    }
    let causal_tokens: HashSet<SyncCausalToken> = relevant
        .iter()
        .flat_map(|deletion| deletion.removed_membership_tokens.iter().cloned())
        .collect();
    songs
        .iter()
        .filter_map(|song| {
            let identity_deletions: Vec<&SyncPlaylistSongDeletion> = relevant
                .iter()
                .copied()
                .filter(|deletion| deletion.matches_song(playlist_id, song))
                .collect();
            let song_tokens = normalize_sync_causal_tokens(&song.sync_membership_tokens);
            if song_tokens.is_empty() {
                let latest = identity_deletions
                    .iter()
                    .max_by(|left, right| deletion_cmp(left, right));
                return match latest {
                    Some(deletion) if effective_added_at(song) <= deletion.deleted_at => None,
                    _ => Some(song.clone()),
                };
            }
            let remaining: Vec<SyncCausalToken> = song_tokens
                .into_iter()
                .filter(|token| !causal_tokens.contains(token))
                .collect();
            if remaining.is_empty() {
                None
            } else {
                let mut surviving = song.clone();
                surviving.sync_membership_tokens = remaining;
                Some(surviving)
            }
        })
        .collect()
}

fn effective_added_at(song: &SyncSong) -> i64 {
    let timestamp = song.legacy_added_at.unwrap_or(song.added_at);
    if timestamp > 0 {
        timestamp
    } else {
        i64::MIN
    }
}

pub(super) fn merge_playlist_song_deletions(
    local: &[SyncPlaylistSongDeletion],
    remote: &[SyncPlaylistSongDeletion],
) -> Vec<SyncPlaylistSongDeletion> {
    let mut groups: BTreeMap<String, Vec<SyncPlaylistSongDeletion>> = BTreeMap::new();
    for deletion in local.iter().chain(remote.iter()) {
        let mut normalized = deletion.clone();
        normalized.removed_membership_tokens =
            normalize_sync_causal_tokens(&deletion.removed_membership_tokens);
        groups
            .entry(normalized.identity())
            .or_default()
            .push(normalized);
    }
    let mut result = Vec::new();
    for snapshots in groups.into_values() {
        let legacy = snapshots
            .iter()
            .filter(|deletion| deletion.removed_membership_tokens.is_empty())
            .max_by(|left, right| deletion_cmp(left, right))
            .cloned();
        let causal_snapshots: Vec<&SyncPlaylistSongDeletion> = snapshots
            .iter()
            .filter(|deletion| !deletion.removed_membership_tokens.is_empty())
            .collect();
        let causal = causal_snapshots
            .iter()
            .max_by(|left, right| deletion_cmp(left, right))
            .map(|deletion| {
                let mut merged = (*deletion).clone();
                let tokens: Vec<SyncCausalToken> = causal_snapshots
                    .iter()
                    .flat_map(|snapshot| snapshot.removed_membership_tokens.iter().cloned())
                    .collect();
                merged.removed_membership_tokens = normalize_sync_causal_tokens(&tokens);
                merged
            });
        if let Some(legacy) = legacy {
            result.push(legacy);
        }
        if let Some(causal) = causal {
            result.push(causal);
        }
    }
    result.sort_by(deletion_order_cmp);
    result
}

fn deletion_cmp(
    left: &SyncPlaylistSongDeletion,
    right: &SyncPlaylistSongDeletion,
) -> std::cmp::Ordering {
    left.deleted_at
        .cmp(&right.deleted_at)
        .then_with(|| left.device_id.cmp(&right.device_id))
}

fn deletion_order_cmp(
    left: &SyncPlaylistSongDeletion,
    right: &SyncPlaylistSongDeletion,
) -> std::cmp::Ordering {
    right
        .deleted_at
        .cmp(&left.deleted_at)
        .then_with(|| right.device_id.cmp(&left.device_id))
        .then_with(|| left.identity().cmp(&right.identity()))
        .then_with(|| {
            left.removed_membership_tokens
                .is_empty()
                .cmp(&right.removed_membership_tokens.is_empty())
        })
}

pub(super) fn prune_playlist_song_deletions(
    deletions: &[SyncPlaylistSongDeletion],
    playlists: &[SyncPlaylist],
) -> Vec<SyncPlaylistSongDeletion> {
    let mut result: Vec<SyncPlaylistSongDeletion> = deletions
        .iter()
        .filter(|deletion| {
            if !deletion.removed_membership_tokens.is_empty() {
                return true;
            }
            // 仅当活跃歌带非空 membership token（identity 已由 causal token 接管、
            // 确实是新版本重新添加）且 added_at 晚于删除时刻才裁 legacy 墓碑。
            // legacy 迁移合成的 addedAt（无 token）不可据此判定重新添加，否则
            // 误裁墓碑会让已删歌在所有端复活（对齐 Android SyncPlaylistDeletionPolicy P1-1）
            playlists.iter().all(|playlist| {
                playlist.id != deletion.playlist_id
                    || playlist.is_deleted
                    || playlist.songs.iter().all(|song| {
                        !deletion.matches_song(&playlist.id, song)
                            || normalize_sync_causal_tokens(&song.sync_membership_tokens).is_empty()
                            || effective_added_at(song) <= deletion.deleted_at
                    })
            })
        })
        .cloned()
        .collect();
    result.sort_by(deletion_order_cmp);
    result
}
