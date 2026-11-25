//! 播放统计合并：计数分片、清空时间点与日分桶

use super::*;

/// 聚合统计不得小于同曲目日分桶之和
///
/// Android 的「总」读聚合值, 「日/周/月/年」读日分桶求和; 两者用了不同的合并代数
/// (聚合取 max, 分桶按天取 max 后再求和), 于是会出现「年 > 总」。
/// 这里只做单调抬升: 结果只增不减, 因此与对端的 max 合并天然收敛, 不会产生回声。
pub(crate) fn lift_stats_to_bucket_totals(
    stats: &[SyncTrackStat],
    buckets: &[SyncPlaybackStatBucket],
) -> Vec<SyncTrackStat> {
    if buckets.is_empty() {
        return stats.to_vec();
    }
    let mut totals: HashMap<&str, (i64, i64)> = HashMap::new();
    for bucket in buckets {
        let entry = totals.entry(bucket.identity_key.as_str()).or_insert((0, 0));
        entry.0 = entry.0.saturating_add(bucket.total_listen_ms.max(0));
        entry.1 = entry.1.saturating_add(i64::from(bucket.play_count.max(0)));
    }

    stats
        .iter()
        .map(|stat| {
            let Some((listen_ms, play_count)) = totals.get(stat.identity_key.as_str()) else {
                return stat.clone();
            };
            let mut lifted = stat.clone();
            lifted.total_listen_ms = lifted.total_listen_ms.max(*listen_ms);
            lifted.play_count = lifted
                .play_count
                .max(i32::try_from(*play_count).unwrap_or(i32::MAX));
            lifted
        })
        .collect()
}

#[derive(Default)]
struct MergedCounters {
    total_listen_ms: i64,
    play_count: i32,
    first_played_at: i64,
    last_played_at: i64,
    base_listen_ms: i64,
    base_play_count: i32,
    shards: Vec<SyncPlaybackCounterShard>,
}

pub(super) fn merge_counter_shards(
    local: &[SyncPlaybackCounterShard],
    remote: &[SyncPlaybackCounterShard],
) -> Vec<SyncPlaybackCounterShard> {
    let mut grouped: BTreeMap<(String, i64), SyncPlaybackCounterShard> = BTreeMap::new();
    for raw in local.iter().chain(remote.iter()) {
        if raw.device_id.trim().is_empty() {
            continue;
        }
        let mut shard = raw.clone();
        shard.recorded_local_date =
            valid_recorded_playback_date(shard.recorded_local_date.as_deref());
        shard.epoch_started_at = shard.epoch_started_at.max(0);
        shard.total_listen_ms = shard.total_listen_ms.max(0);
        shard.play_count = shard.play_count.max(0);
        shard.last_played_at = shard.last_played_at.max(0);
        shard.first_played_at =
            if shard.first_played_at <= 0 || shard.first_played_at > shard.last_played_at {
                shard.last_played_at
            } else {
                shard.first_played_at
            };
        let key = (shard.device_id.clone(), shard.epoch_started_at);
        grouped
            .entry(key)
            .and_modify(|existing| {
                existing.recorded_local_date = merge_recorded_playback_dates(
                    existing.recorded_local_date.as_deref(),
                    shard.recorded_local_date.as_deref(),
                );
                existing.total_listen_ms = existing.total_listen_ms.max(shard.total_listen_ms);
                existing.play_count = existing.play_count.max(shard.play_count);
                existing.first_played_at =
                    min_positive(existing.first_played_at, shard.first_played_at);
                existing.last_played_at = existing.last_played_at.max(shard.last_played_at);
            })
            .or_insert(shard);
    }
    grouped.into_values().collect()
}

fn retained_counter_shards(
    shards: &[SyncPlaybackCounterShard],
    first: i64,
    last: i64,
    cleared_at: i64,
) -> Option<Vec<SyncPlaybackCounterShard>> {
    let mut normalized = merge_counter_shards(shards, &[]);
    if cleared_at <= 0 {
        return Some(normalized);
    }
    if last < cleared_at {
        return None;
    }
    if normalized.is_empty() {
        return (cleared_at..=last).contains(&first).then_some(normalized);
    }
    // 旧 epoch 无法拆出清除后的增量，最后一次播放更新不能让旧累计值复活
    normalized.retain(|shard| {
        shard.epoch_started_at >= cleared_at && shard.first_played_at >= cleared_at
    });
    (!normalized.is_empty()).then_some(normalized)
}

fn counters_after_clear(shards: &[SyncPlaybackCounterShard]) -> MergedCounters {
    MergedCounters {
        total_listen_ms: shards.iter().fold(0_i64, |total, shard| {
            total.saturating_add(shard.total_listen_ms)
        }),
        play_count: shards
            .iter()
            .fold(0_i32, |total, shard| total.saturating_add(shard.play_count)),
        first_played_at: shards
            .iter()
            .map(|shard| shard.first_played_at)
            .min()
            .unwrap_or(0),
        last_played_at: shards
            .iter()
            .map(|shard| shard.last_played_at)
            .max()
            .unwrap_or(0),
        ..Default::default()
    }
}

fn normalize_stat_after_clear(stat: &SyncTrackStat, cleared_at: i64) -> Option<SyncTrackStat> {
    let shards = retained_counter_shards(
        &stat.counter_shards,
        stat.first_played_at,
        stat.last_played_at,
        cleared_at,
    )?;
    let mut normalized = stat.clone();
    normalized.total_listen_ms = normalized.total_listen_ms.max(0);
    normalized.play_count = normalized.play_count.max(0);
    normalized.counter_shards = shards;
    if cleared_at > 0 && !normalized.counter_shards.is_empty() {
        let totals = counters_after_clear(&normalized.counter_shards);
        normalized.total_listen_ms = totals.total_listen_ms;
        normalized.play_count = totals.play_count;
        normalized.first_played_at = totals.first_played_at;
        normalized.last_played_at = totals.last_played_at;
        normalized.counter_base_listen_ms = 0;
        normalized.counter_base_play_count = 0;
    }
    Some(normalized)
}

fn normalize_bucket_after_clear(
    bucket: &SyncPlaybackStatBucket,
    cleared_at: i64,
) -> Option<SyncPlaybackStatBucket> {
    let shards = retained_counter_shards(
        &bucket.counter_shards,
        bucket.first_played_at,
        bucket.last_played_at,
        cleared_at,
    )?;
    let mut normalized = bucket.clone();
    normalized.total_listen_ms = normalized.total_listen_ms.max(0);
    normalized.play_count = normalized.play_count.max(0);
    normalized.counter_shards = shards;
    normalized.recorded_local_date =
        valid_recorded_playback_date(bucket.recorded_local_date.as_deref());
    // 小时槽位带 epoch，清空前的 epoch 整体作废，不随计数分片一起折算
    normalized.hourly_totals = merge_sync_playback_hours(&bucket.hourly_totals, &[], cleared_at);
    if cleared_at > 0 && !normalized.counter_shards.is_empty() {
        let totals = counters_after_clear(&normalized.counter_shards);
        normalized.total_listen_ms = totals.total_listen_ms;
        normalized.play_count = totals.play_count;
        normalized.first_played_at = totals.first_played_at;
        normalized.last_played_at = totals.last_played_at;
        normalized.counter_base_listen_ms = 0;
        normalized.counter_base_play_count = 0;
    }
    Some(normalized)
}

/// 参与计数合并的一侧快照
///
/// 聚合统计与日分桶字段完全同构，用同一个快照类型描述，避免把 14 个
/// 平铺参数在调用点排错顺序 —— 这里一旦错位是静默的数据损坏。
struct CounterSide<'a> {
    total_listen_ms: i64,
    play_count: i32,
    first_played_at: i64,
    last_played_at: i64,
    base_listen_ms: i64,
    base_play_count: i32,
    shards: &'a [SyncPlaybackCounterShard],
}

impl CounterSide<'_> {
    /// 分片机制上线前的历史存量：没有分片且 base 为 0 时，总量本身就是 base
    fn effective_base_listen_ms(&self) -> i64 {
        if self.shards.is_empty() && self.base_listen_ms == 0 {
            self.total_listen_ms.max(0)
        } else {
            self.base_listen_ms.max(0)
        }
    }

    fn effective_base_play_count(&self) -> i32 {
        if self.shards.is_empty() && self.base_play_count == 0 {
            self.play_count.max(0)
        } else {
            self.base_play_count.max(0)
        }
    }
}

fn merge_counter_values(local: CounterSide<'_>, remote: CounterSide<'_>) -> MergedCounters {
    let shards = merge_counter_shards(local.shards, remote.shards);
    // 分片为空时 base 显式归零（对齐 Android SyncPlaybackStatsMergePolicy.mergeCounters）：
    // 此时把 total 折算进 base 只会让本端与 Android 的 counter_base 永远不同，
    // has_data_changed 每轮都判「有变化」，双端互相"纠正"形成永不收敛的回声上传；
    // 总量本身仍由下方的 max 兜底，归零不丢任何数据
    if shards.is_empty() {
        return MergedCounters {
            total_listen_ms: local.total_listen_ms.max(remote.total_listen_ms).max(0),
            play_count: local.play_count.max(remote.play_count).max(0),
            first_played_at: min_positive(local.first_played_at, remote.first_played_at),
            last_played_at: local.last_played_at.max(remote.last_played_at),
            base_listen_ms: 0,
            base_play_count: 0,
            shards,
        };
    }
    let base_total = local
        .effective_base_listen_ms()
        .max(remote.effective_base_listen_ms());
    let base_count = local
        .effective_base_play_count()
        .max(remote.effective_base_play_count());
    let sharded_total = shards.iter().fold(base_total, |total, shard| {
        total.saturating_add(shard.total_listen_ms)
    });
    let sharded_count = shards.iter().fold(base_count, |total, shard| {
        total.saturating_add(shard.play_count)
    });
    MergedCounters {
        total_listen_ms: sharded_total
            .max(local.total_listen_ms)
            .max(remote.total_listen_ms)
            .max(0),
        play_count: sharded_count
            .max(local.play_count)
            .max(remote.play_count)
            .max(0),
        first_played_at: min_positive(
            min_positive(local.first_played_at, remote.first_played_at),
            shards
                .iter()
                .map(|shard| shard.first_played_at)
                .fold(0, min_positive),
        ),
        last_played_at: local.last_played_at.max(remote.last_played_at).max(
            shards
                .iter()
                .map(|shard| shard.last_played_at)
                .max()
                .unwrap_or(0),
        ),
        base_listen_ms: base_total,
        base_play_count: base_count,
        shards,
    }
}

pub(crate) fn merge_playback_stats(
    local: &[SyncTrackStat],
    remote: &[SyncTrackStat],
    cleared_at: i64,
) -> Vec<SyncTrackStat> {
    let mut grouped: BTreeMap<String, SyncTrackStat> = BTreeMap::new();
    for stat in local.iter().chain(remote.iter()) {
        let Some(stat) = normalize_stat_after_clear(stat, cleared_at) else {
            continue;
        };
        if let Some(existing) = grouped.remove(&stat.identity_key) {
            let newer = if stat.last_played_at >= existing.last_played_at {
                &stat
            } else {
                &existing
            };
            let counters = merge_counter_values(
                CounterSide {
                    total_listen_ms: existing.total_listen_ms,
                    play_count: existing.play_count,
                    first_played_at: existing.first_played_at,
                    last_played_at: existing.last_played_at,
                    base_listen_ms: existing.counter_base_listen_ms,
                    base_play_count: existing.counter_base_play_count,
                    shards: &existing.counter_shards,
                },
                CounterSide {
                    total_listen_ms: stat.total_listen_ms,
                    play_count: stat.play_count,
                    first_played_at: stat.first_played_at,
                    last_played_at: stat.last_played_at,
                    base_listen_ms: stat.counter_base_listen_ms,
                    base_play_count: stat.counter_base_play_count,
                    shards: &stat.counter_shards,
                },
            );
            let mut merged = newer.clone();
            merged.total_listen_ms = counters.total_listen_ms;
            merged.play_count = counters.play_count;
            merged.first_played_at = counters.first_played_at;
            merged.last_played_at = counters.last_played_at;
            merged.counter_base_listen_ms = counters.base_listen_ms;
            merged.counter_base_play_count = counters.base_play_count;
            merged.counter_shards = counters.shards;
            grouped.insert(stat.identity_key.clone(), merged);
        } else {
            grouped.insert(stat.identity_key.clone(), stat);
        }
    }
    grouped.into_values().collect()
}

pub(crate) fn merge_stat_buckets(
    local: &[SyncPlaybackStatBucket],
    remote: &[SyncPlaybackStatBucket],
    cleared_at: i64,
) -> Vec<SyncPlaybackStatBucket> {
    let mut grouped: BTreeMap<(i64, String), SyncPlaybackStatBucket> = BTreeMap::new();
    for bucket in local.iter().chain(remote.iter()) {
        let Some(bucket) = normalize_bucket_after_clear(bucket, cleared_at) else {
            continue;
        };
        let key = (bucket.day_start_at, bucket.identity_key.clone());
        if let Some(existing) = grouped.remove(&key) {
            let newer = if bucket.last_played_at >= existing.last_played_at {
                &bucket
            } else {
                &existing
            };
            let counters = merge_counter_values(
                CounterSide {
                    total_listen_ms: existing.total_listen_ms,
                    play_count: existing.play_count,
                    first_played_at: existing.first_played_at,
                    last_played_at: existing.last_played_at,
                    base_listen_ms: existing.counter_base_listen_ms,
                    base_play_count: existing.counter_base_play_count,
                    shards: &existing.counter_shards,
                },
                CounterSide {
                    total_listen_ms: bucket.total_listen_ms,
                    play_count: bucket.play_count,
                    first_played_at: bucket.first_played_at,
                    last_played_at: bucket.last_played_at,
                    base_listen_ms: bucket.counter_base_listen_ms,
                    base_play_count: bucket.counter_base_play_count,
                    shards: &bucket.counter_shards,
                },
            );
            let mut merged = newer.clone();
            merged.total_listen_ms = counters.total_listen_ms;
            merged.play_count = counters.play_count;
            merged.first_played_at = counters.first_played_at;
            merged.last_played_at = counters.last_played_at;
            merged.counter_base_listen_ms = counters.base_listen_ms;
            merged.counter_base_play_count = counters.base_play_count;
            merged.counter_shards = counters.shards;
            merged.recorded_local_date = merge_recorded_playback_dates(
                existing.recorded_local_date.as_deref(),
                bucket.recorded_local_date.as_deref(),
            );
            merged.hourly_totals =
                merge_sync_playback_hours(&existing.hourly_totals, &bucket.hourly_totals, 0);
            grouped.insert(key, merged);
        } else {
            grouped.insert(key, bucket);
        }
    }
    grouped.into_values().collect()
}
