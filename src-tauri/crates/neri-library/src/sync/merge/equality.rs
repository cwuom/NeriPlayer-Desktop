//! 合并结果与远端快照的语义比较，判断是否需要上传

use super::*;

/// 按稳定键排序后逐条比较
///
/// 直接 zip 比较会把「仅顺序不同」误判为有变更, 触发无意义上传;
/// 对端合并后顺序又可能变回去, 形成永不收敛的回声。
fn pairs_by_key<'a, T, K, F>(left: &'a [T], right: &'a [T], key: F) -> Option<Vec<(&'a T, &'a T)>>
where
    K: Ord,
    F: Fn(&T) -> K,
{
    if left.len() != right.len() {
        return None;
    }
    let mut left: Vec<&T> = left.iter().collect();
    let mut right: Vec<&T> = right.iter().collect();
    left.sort_by_key(|item| key(item));
    right.sort_by_key(|item| key(item));
    Some(left.into_iter().zip(right).collect())
}

pub fn has_data_changed(remote: &SyncData, merged: &SyncData) -> bool {
    let remote = remote.normalized_for_sync();
    let merged = merged.normalized_for_sync();
    match pairs_by_key(&remote.playlists, &merged.playlists, |playlist| {
        playlist.id.clone()
    }) {
        Some(pairs) => {
            if pairs
                .iter()
                .any(|(left, right)| !same_playlist(left, right))
            {
                return true;
            }
        }
        None => return true,
    }
    match pairs_by_key(
        &remote.favorite_playlists,
        &merged.favorite_playlists,
        SyncFavoritePlaylist::group_key,
    ) {
        Some(pairs) => {
            if pairs
                .iter()
                .any(|(left, right)| !same_favorite(left, right))
            {
                return true;
            }
        }
        None => return true,
    }
    match pairs_by_key(&remote.recent_plays, &merged.recent_plays, |play| {
        (play.song.identity().stable_key(), play.played_at)
    }) {
        Some(pairs) => {
            if pairs.iter().any(|(left, right)| {
                left.song_id != right.song_id
                    || left.played_at != right.played_at
                    || left.device_id != right.device_id
                    || left.resume_position_ms != right.resume_position_ms
                    || !same_song(&left.song, &right.song)
            }) {
                return true;
            }
        }
        None => return true,
    }
    match pairs_by_key(
        &remote.recent_play_deletions,
        &merged.recent_play_deletions,
        |deletion| deletion.identity().stable_key(),
    ) {
        Some(pairs) => {
            if pairs.iter().any(|(left, right)| {
                left.identity() != right.identity()
                    || left.deleted_at != right.deleted_at
                    || left.device_id != right.device_id
            }) {
                return true;
            }
        }
        None => return true,
    }
    match pairs_by_key(
        &remote.playlist_song_deletions,
        &merged.playlist_song_deletions,
        |deletion| {
            (
                deletion.identity(),
                deletion.removed_membership_tokens.is_empty(),
            )
        },
    ) {
        Some(pairs) => {
            if pairs.iter().any(|(left, right)| {
                left.identity() != right.identity()
                    || left.deleted_at != right.deleted_at
                    || left.device_id != right.device_id
                    || normalize_sync_causal_tokens(&left.removed_membership_tokens)
                        != normalize_sync_causal_tokens(&right.removed_membership_tokens)
            }) {
                return true;
            }
        }
        None => return true,
    }
    if remote.playback_stats_cleared_at != merged.playback_stats_cleared_at
        || !same_stats(&remote.playback_stats, &merged.playback_stats)
        || !same_buckets(&remote.playback_stat_buckets, &merged.playback_stat_buckets)
    {
        return true;
    }
    if remote.extensions != merged.extensions {
        return true;
    }
    false
}

fn same_playlist(left: &SyncPlaylist, right: &SyncPlaylist) -> bool {
    // 对齐 Android SyncDataChangeDetector: 仅比较内容, 不比较 created_at/modified_at
    // 时间戳单调增长, 若纳入比较会把"仅时间戳变化"误判为改动, 导致同步回流/回声上传
    left.id == right.id
        && left.name == right.name
        && left.is_deleted == right.is_deleted
        && left.song_order_version == right.song_order_version
        && left.songs.len() == right.songs.len()
        && left
            .songs
            .iter()
            .zip(&right.songs)
            .all(|(a, b)| same_song(a, b))
}

fn same_favorite(left: &SyncFavoritePlaylist, right: &SyncFavoritePlaylist) -> bool {
    // 对齐 Android SyncDataChangeDetector.favorite:
    // 比较 isDeleted/modifiedAt/sortOrder/trackCount/song identity+metadata
    // 不比较 name/cover/source 展示字段与 addedTime, 避免仅展示刷新触发回声上传
    left.id == right.id
        && left.source == right.source
        && left.modified_at == right.modified_at
        && left.is_deleted == right.is_deleted
        && left.sort_order == right.sort_order
        && left.track_count == right.track_count
        && left.songs.len() == right.songs.len()
        && left
            .songs
            .iter()
            .zip(&right.songs)
            .all(|(a, b)| same_song(a, b))
}

fn same_song(left: &SyncSong, right: &SyncSong) -> bool {
    left.identity_keys() == right.identity_keys()
        && left.name == right.name
        && left.artist == right.artist
        && left.album == right.album
        && left.album_id == right.album_id
        && left.duration_ms == right.duration_ms
        && left.cover_url == right.cover_url
        && left.media_uri == right.media_uri
        && left.added_at == right.added_at
        && left.matched_lyric == right.matched_lyric
        && left.matched_translated_lyric == right.matched_translated_lyric
        && left.matched_lyric_source == right.matched_lyric_source
        && left.matched_song_id == right.matched_song_id
        && left.user_lyric_offset_ms == right.user_lyric_offset_ms
        && left.custom_cover_url == right.custom_cover_url
        && left.custom_name == right.custom_name
        && left.custom_artist == right.custom_artist
        && left.original_name == right.original_name
        && left.original_artist == right.original_artist
        && left.original_cover_url == right.original_cover_url
        && left.original_lyric == right.original_lyric
        && left.original_translated_lyric == right.original_translated_lyric
        && left.matched_romanized_lyric == right.matched_romanized_lyric
        && left.original_romanized_lyric == right.original_romanized_lyric
        && left.lyric_sync_revision == right.lyric_sync_revision
        && left.lyric_sync_edited == right.lyric_sync_edited
        && left.channel_id == right.channel_id
        && left.audio_id == right.audio_id
        && left.sub_audio_id == right.sub_audio_id
        && left.playlist_context_id == right.playlist_context_id
        && left.sync_metadata_version == right.sync_metadata_version
        && normalize_sync_causal_tokens(&left.sync_membership_tokens)
            == normalize_sync_causal_tokens(&right.sync_membership_tokens)
}

fn same_stats(left: &[SyncTrackStat], right: &[SyncTrackStat]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut left = left.iter().collect::<Vec<_>>();
    let mut right = right.iter().collect::<Vec<_>>();
    left.sort_by(|a, b| a.identity_key.cmp(&b.identity_key));
    right.sort_by(|a, b| a.identity_key.cmp(&b.identity_key));
    left.iter().zip(right).all(|(a, b)| {
        a.identity_key == b.identity_key
            && a.name == b.name
            && a.artist == b.artist
            && a.album == b.album
            && a.total_listen_ms == b.total_listen_ms
            && a.play_count == b.play_count
            && a.last_played_at == b.last_played_at
            && a.first_played_at == b.first_played_at
            && a.cover_url == b.cover_url
            && a.duration_ms == b.duration_ms
            && a.media_uri == b.media_uri
            && a.id == b.id
            && a.album_id == b.album_id
            && a.counter_base_listen_ms == b.counter_base_listen_ms
            && a.counter_base_play_count == b.counter_base_play_count
            && merge_counter_shards(&a.counter_shards, &[])
                == merge_counter_shards(&b.counter_shards, &[])
    })
}

fn same_buckets(left: &[SyncPlaybackStatBucket], right: &[SyncPlaybackStatBucket]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut left = left.iter().collect::<Vec<_>>();
    let mut right = right.iter().collect::<Vec<_>>();
    left.sort_by(|a, b| {
        a.day_start_at
            .cmp(&b.day_start_at)
            .then_with(|| a.identity_key.cmp(&b.identity_key))
    });
    right.sort_by(|a, b| {
        a.day_start_at
            .cmp(&b.day_start_at)
            .then_with(|| a.identity_key.cmp(&b.identity_key))
    });
    left.iter().zip(right).all(|(a, b)| {
        a.day_start_at == b.day_start_at
            && a.identity_key == b.identity_key
            && a.name == b.name
            && a.artist == b.artist
            && a.album == b.album
            && a.total_listen_ms == b.total_listen_ms
            && a.play_count == b.play_count
            && a.last_played_at == b.last_played_at
            && a.first_played_at == b.first_played_at
            && a.cover_url == b.cover_url
            && a.duration_ms == b.duration_ms
            && a.media_uri == b.media_uri
            && a.id == b.id
            && a.album_id == b.album_id
            && a.counter_base_listen_ms == b.counter_base_listen_ms
            && a.counter_base_play_count == b.counter_base_play_count
            && merge_counter_shards(&a.counter_shards, &[])
                == merge_counter_shards(&b.counter_shards, &[])
            && a.recorded_local_date == b.recorded_local_date
            && merge_sync_playback_hours(&a.hourly_totals, &[], 0)
                == merge_sync_playback_hours(&b.hourly_totals, &[], 0)
    })
}
