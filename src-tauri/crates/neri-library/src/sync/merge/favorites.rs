//! 收藏歌单合并

use super::*;

pub(super) fn merge_favorite_playlists(
    local: &[SyncFavoritePlaylist],
    remote: &[SyncFavoritePlaylist],
) -> Vec<SyncFavoritePlaylist> {
    let mut groups: BTreeMap<String, Vec<SyncFavoritePlaylist>> = BTreeMap::new();
    for favorite in local.iter().chain(remote.iter()) {
        groups
            .entry(favorite.group_key())
            .or_default()
            .push(favorite.normalized_for_sync());
    }
    let mut result: Vec<SyncFavoritePlaylist> = groups
        .into_values()
        .map(|snapshots| {
            snapshots
                .into_iter()
                .reduce(|left, right| merge_single_favorite(&left, &right))
                .unwrap()
        })
        .collect();
    result.sort_by(|left, right| {
        right
            .sort_order
            .cmp(&left.sort_order)
            .then_with(|| right.modified_at.cmp(&left.modified_at))
            .then_with(|| right.added_time.cmp(&left.added_time))
            .then_with(|| left.group_key().cmp(&right.group_key()))
    });
    result
}

fn merge_single_favorite(
    left: &SyncFavoritePlaylist,
    right: &SyncFavoritePlaylist,
) -> SyncFavoritePlaylist {
    let left = left.normalized_for_sync();
    let right = right.normalized_for_sync();
    let newer = if right.modified_at > left.modified_at {
        &right
    } else {
        &left
    };
    let older = if std::ptr::eq(newer, &left) {
        &right
    } else {
        &left
    };
    if left.is_deleted != right.is_deleted {
        if left.modified_at == right.modified_at {
            let mut result = if left.is_deleted {
                left.clone()
            } else {
                right.clone()
            };
            result.songs.clear();
            result.track_count = 0;
            result.added_time = left.added_time.max(right.added_time);
            result.modified_at = left.modified_at.max(right.modified_at);
            result.sort_order = left.sort_order.max(right.sort_order);
            return result;
        }
        if newer.is_deleted {
            let mut result = newer.clone();
            result.songs.clear();
            result.track_count = 0;
            result.sort_order = left.sort_order.max(right.sort_order);
            return result;
        }
        let mut result = newer.clone();
        result.songs = deduplicate_songs(&[left.songs.clone(), right.songs.clone()].concat());
        result.track_count = left
            .track_count
            .max(right.track_count)
            .max(result.songs.len() as i32);
        if result.sort_order == 0 {
            result.sort_order = older.sort_order;
        }
        return result;
    }
    if newer.is_deleted {
        let mut result = newer.clone();
        result.songs.clear();
        result.track_count = 0;
        result.added_time = left.added_time.max(right.added_time);
        result.sort_order = left.sort_order.max(right.sort_order);
        return result;
    }
    let mut result = newer.clone();
    result.cover_url = if !newer.cover_url.is_empty() {
        newer.cover_url.clone()
    } else {
        older.cover_url.clone()
    };
    result.songs = deduplicate_songs(&[left.songs.clone(), right.songs.clone()].concat());
    result.track_count = left
        .track_count
        .max(right.track_count)
        .max(result.songs.len() as i32);
    result.added_time = left.added_time.max(right.added_time);
    result.modified_at = left.modified_at.max(right.modified_at);
    if result.sort_order == 0 {
        result.sort_order = older.sort_order;
    }
    result.is_deleted = false;
    result
}
