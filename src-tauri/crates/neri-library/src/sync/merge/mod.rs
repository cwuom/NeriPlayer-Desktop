// 三方合并算法：对齐 Android GitHubSyncManager.performThreeWayMerge
use std::collections::{BTreeMap, HashMap, HashSet};

use super::models::*;

// 按数据域拆分的子模块，通过 `use super::*` 共享模型与公共工具
mod equality;
mod extensions;
mod favorites;
mod lyrics;
mod playlists;
mod recent;
mod songs;
mod stats;
pub use equality::has_data_changed;
use extensions::*;
pub use extensions::{record_local_playlist_play, record_playlist_open};
use favorites::*;
pub use lyrics::{converge_lyrics, record_lyric_override};
use playlists::*;
use recent::*;
use songs::*;
use stats::*;
// 同步引擎在 sync 内部直接复用的局部合并
pub(super) use recent::{merge_recent_play_deletions, merge_recent_plays};
pub(super) use stats::{lift_stats_to_bucket_totals, merge_playback_stats, merge_stat_buckets};

const MAX_SYNC_LOG: usize = 100;
#[cfg(test)]
const MILLIS_PER_DAY: i64 = 24 * 60 * 60 * 1000;

pub fn three_way_merge(
    local: &SyncData,
    remote: &SyncData,
    last_sync_time: i64,
    base_snapshot: &HashMap<String, HashSet<String>>,
) -> SyncData {
    let mut local = local.normalized_for_sync();
    let mut remote = remote.normalized_for_sync();
    // 先收集各候选的歌词版本，展示元数据的选择不能丢掉独立歌词修改
    for data in [&mut local, &mut remote] {
        if let Err(error) =
            super::archive::capture_legacy_lyrics(data).and_then(|_| converge_lyrics(data, false))
        {
            log::warn!(target:"sync","invalid lyric metadata will be rejected before publication: {error}");
        }
    }
    let playlist_deletions = merge_playlist_song_deletions(
        &local.playlist_song_deletions,
        &remote.playlist_song_deletions,
    );
    let playlists = merge_playlists(
        &local.playlists,
        &remote.playlists,
        last_sync_time,
        base_snapshot,
        &playlist_deletions,
    );
    let playlist_deletions = prune_playlist_song_deletions(&playlist_deletions, &playlists);
    let favorites = merge_favorite_playlists(&local.favorite_playlists, &remote.favorite_playlists);
    let recent_deletions =
        merge_recent_play_deletions(&local.recent_play_deletions, &remote.recent_play_deletions);
    let recent = merge_recent_plays(&local.recent_plays, &remote.recent_plays, &recent_deletions);
    let recent_deletions = prune_recent_play_deletions(&recent_deletions, &recent);
    let playback_stats_cleared_at = local
        .playback_stats_cleared_at
        .max(remote.playback_stats_cleared_at)
        .max(0);

    let stat_buckets = merge_stat_buckets(
        &local.playback_stat_buckets,
        &remote.playback_stat_buckets,
        playback_stats_cleared_at,
    );
    let playback_stats = merge_playback_stats(
        &local.playback_stats,
        &remote.playback_stats,
        playback_stats_cleared_at,
    );
    // 全量分桶抬升聚合值，历史数据由归档预算保护，不能靠裁剪绕过预算
    let playback_stats = lift_stats_to_bucket_totals(&playback_stats, &stat_buckets);

    let mut result = SyncData {
        version: "2.0".into(),
        device_id: local.device_id.clone(),
        device_name: local.device_name.clone(),
        last_modified: chrono::Utc::now().timestamp_millis(),
        playlists,
        favorite_playlists: favorites,
        recent_plays: recent,
        sync_log: merge_sync_log(&local.sync_log, &remote.sync_log),
        recent_play_deletions: recent_deletions,
        playback_stats,
        playback_stats_cleared_at,
        playback_stat_buckets: stat_buckets,
        playlist_song_deletions: playlist_deletions,
        extensions: merge_extensions(&local.extensions, &remote.extensions),
    };
    // 歌词版本独立于歌单重排和歌曲展示元数据
    let _ = converge_lyrics(&mut result, false);
    result
}

fn merge_sync_log(local: &[SyncLogEntry], remote: &[SyncLogEntry]) -> Vec<SyncLogEntry> {
    let mut unique = BTreeMap::new();
    for entry in local.iter().chain(remote.iter()) {
        let key = format!(
            "{}|{}|{}|{}|{}|{}",
            entry.timestamp,
            entry.device_id,
            entry.action,
            entry.playlist_id.as_deref().unwrap_or_default(),
            entry.song_id.as_deref().unwrap_or_default(),
            entry.details.as_deref().unwrap_or_default()
        );
        unique.insert(key, entry.clone());
    }
    let mut result: Vec<SyncLogEntry> = unique.into_values().collect();
    result.sort_by(|left, right| {
        right
            .timestamp
            .cmp(&left.timestamp)
            .then_with(|| right.device_id.cmp(&left.device_id))
            .then_with(|| left.action.cmp(&right.action))
    });
    result.truncate(MAX_SYNC_LOG);
    result
}

fn min_positive(left: i64, right: i64) -> i64 {
    match (left > 0, right > 0) {
        (false, false) => 0,
        (false, true) => right,
        (true, false) => left,
        (true, true) => left.min(right),
    }
}

#[cfg(test)]
mod tests;
