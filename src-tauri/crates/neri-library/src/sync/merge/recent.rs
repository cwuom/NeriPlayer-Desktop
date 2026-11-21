//! 最近播放与其删除记录的合并

use super::*;

pub(crate) fn merge_recent_plays(
    local: &[SyncRecentPlay],
    remote: &[SyncRecentPlay],
    deletions: &[SyncRecentPlayDeletion],
) -> Vec<SyncRecentPlay> {
    let mut all: Vec<SyncRecentPlay> = local.iter().chain(remote.iter()).cloned().collect();
    all.sort_by(|left, right| {
        right
            .played_at
            .cmp(&left.played_at)
            .then_with(|| right.resume_position_ms.cmp(&left.resume_position_ms))
            .then_with(|| right.device_id.cmp(&left.device_id))
            .then_with(|| recent_song_key(left).cmp(&recent_song_key(right)))
    });
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for mut recent in all {
        recent.song = recent.song.normalized_for_sync();
        let key = recent_song_key(&recent);
        if !seen.insert(key.clone()) || recent_is_deleted(&recent.song, recent.played_at, deletions)
        {
            continue;
        }
        result.push(recent);
    }
    result
}

fn recent_song_key(recent: &SyncRecentPlay) -> String {
    recent.song.identity().stable_key()
}

fn recent_is_deleted(
    song: &SyncSong,
    played_at: i64,
    deletions: &[SyncRecentPlayDeletion],
) -> bool {
    deletions.iter().any(|deletion| {
        let matches = deletion
            .identity_keys()
            .iter()
            .any(|key| song.identity_keys().iter().any(|song_key| song_key == key));
        matches && deletion.deleted_at >= played_at
    })
}

pub(crate) fn merge_recent_play_deletions(
    local: &[SyncRecentPlayDeletion],
    remote: &[SyncRecentPlayDeletion],
) -> Vec<SyncRecentPlayDeletion> {
    let mut groups: BTreeMap<String, Vec<SyncRecentPlayDeletion>> = BTreeMap::new();
    for deletion in local.iter().chain(remote.iter()) {
        groups
            .entry(deletion.identity().stable_key())
            .or_default()
            .push(deletion.clone());
    }
    let mut result: Vec<SyncRecentPlayDeletion> = groups
        .into_values()
        .filter_map(|snapshots| snapshots.into_iter().max_by(recent_deletion_cmp))
        .collect();
    result.sort_by(|left, right| {
        right
            .deleted_at
            .cmp(&left.deleted_at)
            .then_with(|| right.device_id.cmp(&left.device_id))
            .then_with(|| {
                left.identity()
                    .stable_key()
                    .cmp(&right.identity().stable_key())
            })
    });
    result
}

fn recent_deletion_cmp(
    left: &SyncRecentPlayDeletion,
    right: &SyncRecentPlayDeletion,
) -> std::cmp::Ordering {
    left.deleted_at
        .cmp(&right.deleted_at)
        .then_with(|| left.device_id.cmp(&right.device_id))
}

pub(super) fn prune_recent_play_deletions(
    deletions: &[SyncRecentPlayDeletion],
    recent: &[SyncRecentPlay],
) -> Vec<SyncRecentPlayDeletion> {
    let mut result: Vec<SyncRecentPlayDeletion> = deletions
        .iter()
        .filter(|deletion| {
            recent.iter().all(|played| {
                let matches = deletion.identity_keys().iter().any(|key| {
                    played
                        .song
                        .identity_keys()
                        .iter()
                        .any(|song_key| song_key == key)
                });
                !matches || played.played_at <= deletion.deleted_at
            })
        })
        .cloned()
        .collect();
    result.sort_by(|left, right| {
        right
            .deleted_at
            .cmp(&left.deleted_at)
            .then_with(|| right.device_id.cmp(&left.device_id))
    });
    result
}
