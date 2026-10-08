// 播放统计：对齐 Android PlaybackStatsRepository + PlaybackStatsCounterStore
//
// 数据结构直接复用 sync::models 的三个同步类型，避免再维护一套平行模型：
// - SyncTrackStat            聚合统计（「总」口径）
// - SyncPlaybackStatBucket   按天分桶（「日/周/月/年」口径）
// - SyncPlaybackCounterShard 每（设备 × epoch）增量，跨端合并的唯一真值源
//
// 统计的"计一次播放"规则与 Android 完全一致，见 PlaybackStatsTracker：
// 单次连续收听 >= 30s，或整轨播完（track-ended）即 +1。

use std::collections::HashMap;

use chrono::{Datelike, Local, TimeZone};
use serde::{Deserialize, Serialize};

use crate::sync::models::{
    SyncData, SyncPlaybackCounterShard, SyncPlaybackStatBucket, SyncTrackStat,
};

mod storage;
pub(crate) use storage::{import_legacy_json, LEGACY_IMPORT_KEY};

/// 一次收听增量上报，由前端 PlaybackStatsTracker 产出
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackSession {
    pub identity_key: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub album_id: String,
    #[serde(default)]
    pub cover_url: Option<String>,
    #[serde(default)]
    pub media_uri: Option<String>,
    #[serde(default)]
    pub duration_ms: i64,
    #[serde(default)]
    pub listened_ms: i64,
    #[serde(default)]
    pub play_count_increment: i32,
}

/// 统计查询区间，对齐 Android PlaybackStatsPeriod
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatsPeriod {
    Day,
    Week,
    Month,
    Year,
    All,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackStatsSummary {
    pub period: StatsPeriod,
    pub total_play_count: i64,
    pub total_listen_ms: i64,
    pub track_count: usize,
    pub items: Vec<SyncTrackStat>,
}

// 按天数据的保留窗口：语义对齐 sync/merge.rs 的 STAT_BUCKET_RETENTION_DAYS
// （400 天，见 docs/SYNC-MODEL-CONTRACT.md §4）。本地重复定义而不 import
// sync 模块，避免统计模块与同步模块产生编译期耦合；两处值必须保持一致
const STAT_RETENTION_DAYS: i64 = 400;
const MILLIS_PER_DAY: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsStore {
    #[serde(default)]
    pub stats: Vec<SyncTrackStat>,
    #[serde(default)]
    pub buckets: Vec<SyncPlaybackStatBucket>,
    /// identityKey -> shards
    #[serde(default)]
    pub track_shards: HashMap<String, Vec<SyncPlaybackCounterShard>>,
    /// "dayStartAt|identityKey" -> shards
    #[serde(default)]
    pub daily_shards: HashMap<String, Vec<SyncPlaybackCounterShard>>,
    #[serde(default)]
    pub cleared_at: i64,
    /// 上次执行保留窗口修剪的「天」零点；仅内存态，不落盘
    #[serde(skip)]
    last_pruned_day: i64,
    /// 启动时未能从数据库读出统计；此后拒绝保存，避免用空快照覆盖库中的真实数据
    #[serde(skip)]
    storage_unavailable: bool,
}

pub fn daily_shard_key(day_start_at: i64, identity_key: &str) -> String {
    format!("{}|{}", day_start_at, identity_key)
}

/// 本地时区的当天零点，与 Android Calendar.moveToDayStart 语义一致
pub fn day_start_at(millis: i64) -> i64 {
    let Some(moment) = Local.timestamp_millis_opt(millis).single() else {
        return millis;
    };
    moment
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|naive| Local.from_local_datetime(&naive).single())
        .map(|start| start.timestamp_millis())
        .unwrap_or(millis)
}

/// 区间起点（闭），All 返回 None
pub fn period_start_at(period: StatsPeriod, now_ms: i64) -> Option<i64> {
    if period == StatsPeriod::All {
        return None;
    }
    let moment = Local.timestamp_millis_opt(now_ms).single()?;
    let date = moment.date_naive();
    let start_date = match period {
        StatsPeriod::Day => date,
        // 与 Android moveToWeekStart 一致：周一为一周起点
        StatsPeriod::Week => date - chrono::Duration::days(i64::from(
            date.weekday().num_days_from_monday(),
        )),
        StatsPeriod::Month => date.with_day(1)?,
        StatsPeriod::Year => date.with_month(1)?.with_day(1)?,
        StatsPeriod::All => return None,
    };
    start_date
        .and_hms_opt(0, 0, 0)
        .and_then(|naive| Local.from_local_datetime(&naive).single())
        .map(|start| start.timestamp_millis())
}

fn min_positive(left: i64, right: i64) -> i64 {
    match (left, right) {
        (l, r) if l <= 0 => r.max(0),
        (l, r) if r <= 0 => l,
        (l, r) => l.min(r),
    }
}

/// 同（设备, epoch）取最大值合并，计数在单设备内单调递增
fn upsert_shard(
    shards: &mut Vec<SyncPlaybackCounterShard>,
    device_id: &str,
    epoch_started_at: i64,
    listened_ms: i64,
    play_count_increment: i32,
    played_at: i64,
) {
    if let Some(shard) = shards
        .iter_mut()
        .find(|shard| shard.device_id == device_id && shard.epoch_started_at == epoch_started_at)
    {
        shard.total_listen_ms = shard.total_listen_ms.saturating_add(listened_ms.max(0));
        shard.play_count = shard.play_count.saturating_add(play_count_increment.max(0));
        shard.first_played_at = min_positive(shard.first_played_at, played_at);
        shard.last_played_at = shard.last_played_at.max(played_at);
        return;
    }
    shards.push(SyncPlaybackCounterShard {
        device_id: device_id.to_string(),
        epoch_started_at,
        total_listen_ms: listened_ms.max(0),
        play_count: play_count_increment.max(0),
        first_played_at: played_at,
        last_played_at: played_at,
    });
}

fn snapshot_shards(
    provenance: &[SyncPlaybackCounterShard],
    own: &[SyncPlaybackCounterShard],
) -> Vec<SyncPlaybackCounterShard> {
    let mut merged = std::collections::BTreeMap::<(String, i64), SyncPlaybackCounterShard>::new();
    for shard in provenance.iter().chain(own) {
        merged
            .entry((shard.device_id.clone(), shard.epoch_started_at))
            .and_modify(|current| {
                current.total_listen_ms = current.total_listen_ms.max(shard.total_listen_ms);
                current.play_count = current.play_count.max(shard.play_count);
                current.first_played_at = min_positive(current.first_played_at, shard.first_played_at);
                current.last_played_at = current.last_played_at.max(shard.last_played_at);
            })
            .or_insert_with(|| shard.clone());
    }
    merged.into_values().collect()
}

impl StatsStore {
    /// 按天保留窗口修剪：丢弃超出窗口的 `buckets` 与 `daily_shards`
    ///
    /// 「总」口径的 `stats`/`track_shards` 不动——聚合值必须保持全量，
    /// 否则「年 > 总」类矛盾会重现。仅修剪按「天 × 曲目」无限增长的部分
    pub fn prune_retention(&mut self, now_ms: i64) {
        let cutoff = day_start_at(now_ms).saturating_sub(STAT_RETENTION_DAYS * MILLIS_PER_DAY);
        let before = self.buckets.len() + self.daily_shards.len();
        self.buckets.retain(|bucket| bucket.day_start_at >= cutoff);
        self.daily_shards.retain(|key, _| {
            // key 形如 "dayStartAt|identityKey"；无法解析的键保守保留
            key.split_once('|')
                .and_then(|(day, _)| day.parse::<i64>().ok())
                .is_none_or(|day| day >= cutoff)
        });
        let after = self.buckets.len() + self.daily_shards.len();
        if after < before {
            log::info!(
                target: "stats",
                "retention pruned {} day-scoped entries older than {} days",
                before - after,
                STAT_RETENTION_DAYS,
            );
        }
    }

    /// 每天首次记录时顺带修剪一次，避免每笔写入都全表扫描
    fn maybe_prune_daily(&mut self, now_ms: i64) {
        let today = day_start_at(now_ms);
        if self.last_pruned_day != today {
            self.prune_retention(now_ms);
            self.last_pruned_day = today;
        }
    }

    pub fn record(&mut self, session: &PlaybackSession, device_id: &str, played_at: i64) {
        if session.identity_key.trim().is_empty() {
            return;
        }
        self.maybe_prune_daily(played_at);
        let listened = session.listened_ms.max(0);
        let increment = session.play_count_increment.max(0);
        if listened <= 0 && increment <= 0 {
            return;
        }
        let epoch = self.cleared_at.max(0);
        let day = day_start_at(played_at);
        let daily_key = daily_shard_key(day, &session.identity_key);
        // 旧投影恢复后本机来源可能尚未进入增量表，先续接原计数再记录
        if let Some(stat) = self.stats.iter().find(|stat| stat.identity_key == session.identity_key) {
            let provenance = retain_own_shards(&stat.counter_shards, device_id);
            if !provenance.is_empty() {
                let own = self.track_shards.entry(session.identity_key.clone()).or_default();
                *own = snapshot_shards(&provenance, own);
            }
        }
        if let Some(bucket) = self.buckets.iter().find(|bucket| {
            bucket.day_start_at == day && bucket.identity_key == session.identity_key
        }) {
            let provenance = retain_own_shards(&bucket.counter_shards, device_id);
            if !provenance.is_empty() {
                let own = self.daily_shards.entry(daily_key.clone()).or_default();
                *own = snapshot_shards(&provenance, own);
            }
        }
        let has_track_shards = self
            .track_shards
            .get(&session.identity_key)
            .is_some_and(|shards| !shards.is_empty());
        let has_daily_shards = self
            .daily_shards
            .get(&daily_key)
            .is_some_and(|shards| !shards.is_empty());

        // 聚合
        if let Some(stat) = self
            .stats
            .iter_mut()
            .find(|stat| stat.identity_key == session.identity_key)
        {
            // 无来源的旧累计值先固化为 base，新建本机分片后仍能保留这段历史
            if stat.counter_shards.is_empty() && !has_track_shards {
                if stat.counter_base_listen_ms == 0 {
                    stat.counter_base_listen_ms = stat.total_listen_ms.max(0);
                }
                if stat.counter_base_play_count == 0 {
                    stat.counter_base_play_count = stat.play_count.max(0);
                }
            }
            stat.total_listen_ms = stat.total_listen_ms.saturating_add(listened);
            stat.play_count = stat.play_count.saturating_add(increment);
            stat.last_played_at = stat.last_played_at.max(played_at);
            stat.first_played_at = min_positive(stat.first_played_at, played_at);
            apply_session_metadata(stat_meta_mut(stat), session);
        } else {
            let mut stat = SyncTrackStat {
                identity_key: session.identity_key.clone(),
                total_listen_ms: listened,
                play_count: increment,
                last_played_at: played_at,
                first_played_at: played_at,
                ..Default::default()
            };
            apply_session_metadata(stat_meta_mut(&mut stat), session);
            self.stats.push(stat);
        }

        // 日分桶
        if let Some(bucket) = self
            .buckets
            .iter_mut()
            .find(|bucket| bucket.day_start_at == day && bucket.identity_key == session.identity_key)
        {
            if bucket.counter_shards.is_empty() && !has_daily_shards {
                if bucket.counter_base_listen_ms == 0 {
                    bucket.counter_base_listen_ms = bucket.total_listen_ms.max(0);
                }
                if bucket.counter_base_play_count == 0 {
                    bucket.counter_base_play_count = bucket.play_count.max(0);
                }
            }
            bucket.total_listen_ms = bucket.total_listen_ms.saturating_add(listened);
            bucket.play_count = bucket.play_count.saturating_add(increment);
            bucket.last_played_at = bucket.last_played_at.max(played_at);
            bucket.first_played_at = min_positive(bucket.first_played_at, played_at);
            apply_session_metadata(bucket_meta_mut(bucket), session);
        } else {
            let mut bucket = SyncPlaybackStatBucket {
                day_start_at: day,
                identity_key: session.identity_key.clone(),
                total_listen_ms: listened,
                play_count: increment,
                last_played_at: played_at,
                first_played_at: played_at,
                ..Default::default()
            };
            apply_session_metadata(bucket_meta_mut(&mut bucket), session);
            self.buckets.push(bucket);
        }

        // 分片：跨端合并只信这里
        upsert_shard(
            self.track_shards.entry(session.identity_key.clone()).or_default(),
            device_id,
            epoch,
            listened,
            increment,
            played_at,
        );
        upsert_shard(
            self.daily_shards
                .entry(daily_key)
                .or_default(),
            device_id,
            epoch,
            listened,
            increment,
            played_at,
        );
    }

    /// 上传用快照：保留已同步来源，并以本机最新分片更新对应计数
    pub fn sync_snapshot(&self) -> (Vec<SyncTrackStat>, Vec<SyncPlaybackStatBucket>, i64) {
        let stats = self
            .stats
            .iter()
            .map(|stat| {
                let mut out = stat.clone();
                out.counter_shards = snapshot_shards(
                    &stat.counter_shards,
                    self.track_shards.get(&stat.identity_key).map(Vec::as_slice).unwrap_or(&[]),
                );
                out
            })
            .collect();
        let buckets = self
            .buckets
            .iter()
            .map(|bucket| {
                let mut out = bucket.clone();
                out.counter_shards = snapshot_shards(
                    &bucket.counter_shards,
                    self.daily_shards
                        .get(&daily_shard_key(bucket.day_start_at, &bucket.identity_key))
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                );
                out
            })
            .collect();
        (stats, buckets, self.cleared_at.max(0))
    }

    /// 合并结果回写本地，并按本机 deviceId 重建分片
    pub fn apply_merged(
        &mut self,
        stats: &[SyncTrackStat],
        buckets: &[SyncPlaybackStatBucket],
        cleared_at: i64,
        device_id: &str,
    ) {
        // 回写时重新合并当前分片和清除时间，保留同步期间产生的本机变更
        let (current_stats, current_buckets, current_cleared_at) = self.sync_snapshot();
        let current = SyncData {
            device_id: device_id.to_string(),
            playback_stats: current_stats,
            playback_stat_buckets: current_buckets,
            playback_stats_cleared_at: current_cleared_at,
            ..Default::default()
        };
        let incoming = SyncData {
            playback_stats: stats.to_vec(),
            playback_stat_buckets: buckets.to_vec(),
            playback_stats_cleared_at: cleared_at,
            ..Default::default()
        };
        let merged = crate::sync::merge::three_way_merge(
            &current,
            &incoming,
            0,
            &HashMap::new(),
        );
        self.cleared_at = merged.playback_stats_cleared_at;
        self.stats = merged.playback_stats;
        self.buckets = merged.playback_stat_buckets;
        self.track_shards = self.stats
            .iter()
            .map(|stat| (stat.identity_key.clone(), retain_own_shards(&stat.counter_shards, device_id)))
            .filter(|(_, shards)| !shards.is_empty())
            .collect();
        self.daily_shards = self.buckets
            .iter()
            .map(|bucket| {
                (
                    daily_shard_key(bucket.day_start_at, &bucket.identity_key),
                    retain_own_shards(&bucket.counter_shards, device_id),
                )
            })
            .filter(|(_, shards)| !shards.is_empty())
            .collect();
    }

    pub fn clear(&mut self, now_ms: i64) {
        self.stats.clear();
        self.buckets.clear();
        self.track_shards.clear();
        self.daily_shards.clear();
        self.cleared_at = now_ms.max(0);
    }

    pub fn remove_tracks(&mut self, keys: &[String]) {
        if keys.is_empty() {
            return;
        }
        let removed: std::collections::HashSet<&str> =
            keys.iter().map(String::as_str).collect();
        self.stats.retain(|stat| !removed.contains(stat.identity_key.as_str()));
        self.buckets
            .retain(|bucket| !removed.contains(bucket.identity_key.as_str()));
        self.track_shards.retain(|key, _| !removed.contains(key.as_str()));
        self.daily_shards.retain(|key, _| {
            key.split_once('|')
                .is_none_or(|(_, identity)| !removed.contains(identity))
        });
    }

    /// 区间聚合
    ///
    /// All 直接读聚合值，不回退到分桶求和 —— 分桶有保留窗口，求和会小于真实总量，
    /// 也正是 Android「年 > 总」的来源。其余区间按天求和。
    pub fn summarize(&self, period: StatsPeriod, now_ms: i64) -> PlaybackStatsSummary {
        let mut items: Vec<SyncTrackStat> = if period == StatsPeriod::All {
            self.stats.clone()
        } else {
            let Some(start) = period_start_at(period, now_ms) else {
                return self.summarize(StatsPeriod::All, now_ms);
            };
            let mut grouped: HashMap<String, SyncTrackStat> = HashMap::new();
            for bucket in self
                .buckets
                .iter()
                .filter(|bucket| bucket.day_start_at >= start)
            {
                match grouped.get_mut(&bucket.identity_key) {
                    Some(stat) => {
                        stat.total_listen_ms =
                            stat.total_listen_ms.saturating_add(bucket.total_listen_ms.max(0));
                        stat.play_count = stat.play_count.saturating_add(bucket.play_count.max(0));
                        if bucket.last_played_at >= stat.last_played_at {
                            copy_bucket_display_fields(stat, bucket);
                        }
                        stat.last_played_at = stat.last_played_at.max(bucket.last_played_at);
                        stat.first_played_at =
                            min_positive(stat.first_played_at, bucket.first_played_at);
                    }
                    None => {
                        grouped.insert(bucket.identity_key.clone(), bucket_to_stat(bucket));
                    }
                }
            }
            grouped.into_values().collect()
        };

        items.retain(|stat| stat.play_count > 0 || stat.total_listen_ms > 0);
        items.sort_by(|left, right| {
            right
                .play_count
                .cmp(&left.play_count)
                .then_with(|| right.total_listen_ms.cmp(&left.total_listen_ms))
                .then_with(|| left.identity_key.cmp(&right.identity_key))
        });

        PlaybackStatsSummary {
            period,
            total_play_count: items.iter().map(|stat| i64::from(stat.play_count)).sum(),
            total_listen_ms: items.iter().map(|stat| stat.total_listen_ms).sum(),
            track_count: items.len(),
            items,
        }
    }
}

fn retain_own_shards(
    shards: &[SyncPlaybackCounterShard],
    device_id: &str,
) -> Vec<SyncPlaybackCounterShard> {
    shards
        .iter()
        .filter(|shard| shard.device_id == device_id)
        .cloned()
        .collect()
}

/// 展示字段的可变引用集合，聚合与分桶共用同一套写入逻辑
struct StatMeta<'a> {
    id: &'a mut String,
    name: &'a mut String,
    artist: &'a mut String,
    album: &'a mut String,
    album_id: &'a mut String,
    cover_url: &'a mut Option<String>,
    media_uri: &'a mut Option<String>,
    duration_ms: &'a mut i64,
}

fn stat_meta_mut(stat: &mut SyncTrackStat) -> StatMeta<'_> {
    StatMeta {
        id: &mut stat.id,
        name: &mut stat.name,
        artist: &mut stat.artist,
        album: &mut stat.album,
        album_id: &mut stat.album_id,
        cover_url: &mut stat.cover_url,
        media_uri: &mut stat.media_uri,
        duration_ms: &mut stat.duration_ms,
    }
}

fn bucket_meta_mut(bucket: &mut SyncPlaybackStatBucket) -> StatMeta<'_> {
    StatMeta {
        id: &mut bucket.id,
        name: &mut bucket.name,
        artist: &mut bucket.artist,
        album: &mut bucket.album,
        album_id: &mut bucket.album_id,
        cover_url: &mut bucket.cover_url,
        media_uri: &mut bucket.media_uri,
        duration_ms: &mut bucket.duration_ms,
    }
}

fn apply_session_metadata(meta: StatMeta<'_>, session: &PlaybackSession) {
    if !session.id.is_empty() {
        *meta.id = session.id.clone();
    }
    if !session.name.is_empty() {
        *meta.name = session.name.clone();
    }
    if !session.artist.is_empty() {
        *meta.artist = session.artist.clone();
    }
    if !session.album.is_empty() {
        *meta.album = session.album.clone();
    }
    if !session.album_id.is_empty() {
        *meta.album_id = session.album_id.clone();
    }
    if session.cover_url.as_deref().is_some_and(|url| !url.is_empty()) {
        *meta.cover_url = session.cover_url.clone();
    }
    if session.media_uri.as_deref().is_some_and(|uri| !uri.is_empty()) {
        *meta.media_uri = session.media_uri.clone();
    }
    if session.duration_ms > 0 {
        *meta.duration_ms = session.duration_ms;
    }
}

fn bucket_to_stat(bucket: &SyncPlaybackStatBucket) -> SyncTrackStat {
    SyncTrackStat {
        identity_key: bucket.identity_key.clone(),
        name: bucket.name.clone(),
        artist: bucket.artist.clone(),
        album: bucket.album.clone(),
        total_listen_ms: bucket.total_listen_ms.max(0),
        play_count: bucket.play_count.max(0),
        last_played_at: bucket.last_played_at,
        first_played_at: bucket.first_played_at,
        cover_url: bucket.cover_url.clone(),
        duration_ms: bucket.duration_ms,
        media_uri: bucket.media_uri.clone(),
        id: bucket.id.clone(),
        album_id: bucket.album_id.clone(),
        counter_base_listen_ms: 0,
        counter_base_play_count: 0,
        counter_shards: Vec::new(),
    }
}

fn copy_bucket_display_fields(stat: &mut SyncTrackStat, bucket: &SyncPlaybackStatBucket) {
    stat.name = bucket.name.clone();
    stat.artist = bucket.artist.clone();
    stat.album = bucket.album.clone();
    stat.id = bucket.id.clone();
    stat.album_id = bucket.album_id.clone();
    if bucket.cover_url.is_some() {
        stat.cover_url = bucket.cover_url.clone();
    }
    if bucket.media_uri.is_some() {
        stat.media_uri = bucket.media_uri.clone();
    }
    if bucket.duration_ms > 0 {
        stat.duration_ms = bucket.duration_ms;
    }
}

fn restore_saved_stats_provenance(
    store: &mut StatsStore,
    provenance: &crate::sync::manager::SyncStatsPayload,
) {
    if store.cleared_at != provenance.cleared_at {
        return;
    }

    let mut track_counts = HashMap::new();
    for stat in &store.stats {
        *track_counts.entry(stat.identity_key.clone()).or_insert(0_usize) += 1;
    }
    let mut track_sources: HashMap<&str, Option<&SyncTrackStat>> = HashMap::new();
    for stat in &provenance.stats {
        track_sources.entry(&stat.identity_key)
            .and_modify(|source| *source = None)
            .or_insert(Some(stat));
    }
    for stat in &mut store.stats {
        if stat.identity_key.is_empty() || !stat.counter_shards.is_empty()
            || track_counts.get(&stat.identity_key) != Some(&1) {
            continue;
        }
        let Some(Some(source)) = track_sources.get(stat.identity_key.as_str()) else { continue };
        if !source.counter_shards.is_empty()
            && (stat.total_listen_ms, stat.play_count, stat.counter_base_listen_ms,
                stat.counter_base_play_count, stat.first_played_at, stat.last_played_at)
                == (source.total_listen_ms, source.play_count, source.counter_base_listen_ms,
                    source.counter_base_play_count, source.first_played_at, source.last_played_at) {
            stat.counter_shards = source.counter_shards.clone();
        }
    }

    let mut bucket_counts = HashMap::new();
    for bucket in &store.buckets {
        *bucket_counts.entry((bucket.day_start_at, bucket.identity_key.clone())).or_insert(0_usize) += 1;
    }
    let mut bucket_sources: HashMap<(i64, &str), Option<&SyncPlaybackStatBucket>> = HashMap::new();
    for bucket in &provenance.buckets {
        bucket_sources.entry((bucket.day_start_at, &bucket.identity_key))
            .and_modify(|source| *source = None)
            .or_insert(Some(bucket));
    }
    for bucket in &mut store.buckets {
        if bucket.identity_key.is_empty() || !bucket.counter_shards.is_empty()
            || bucket_counts.get(&(bucket.day_start_at, bucket.identity_key.clone())) != Some(&1) {
            continue;
        }
        let Some(Some(source)) = bucket_sources.get(&(bucket.day_start_at, bucket.identity_key.as_str())) else { continue };
        if !source.counter_shards.is_empty()
            && (bucket.total_listen_ms, bucket.play_count, bucket.counter_base_listen_ms,
                bucket.counter_base_play_count, bucket.first_played_at, bucket.last_played_at)
                == (source.total_listen_ms, source.play_count, source.counter_base_listen_ms,
                    source.counter_base_play_count, source.first_played_at, source.last_played_at) {
            bucket.counter_shards = source.counter_shards.clone();
        }
    }
}

/// 从用户数据库读取统计
///
/// 读取失败时返回空统计并标记为不可保存：统计可以暂时显示为空，
/// 但绝不能让空快照在后续保存时把库里的真实数据按差异删掉
pub fn load() -> StatsStore {
    let loaded = crate::db::user_db().and_then(|database| database.read(storage::load_from));
    match loaded {
        Ok(mut store) => {
            // 启动即修剪保留窗口，超期的日分桶不再随每次保存原样保留
            store.prune_retention(chrono::Utc::now().timestamp_millis());
            store
        }
        Err(error) => {
            log::error!(target: "stats", "播放统计读取失败, 本次运行不再写入统计: {error}");
            StatsStore { storage_unavailable: true, ..Default::default() }
        }
    }
}

/// 读取旧版 playback-stats.json，并用同步侧车补回旧投影缺失的分片来源
///
/// 统计可容忍从空重建（分片会随同步找回），解析失败时隔离现场按无数据处理
pub(crate) fn read_legacy_files(
    path: &std::path::Path,
    metadata_path: &std::path::Path,
) -> crate::error::AppResult<Option<StatsStore>> {
    let Some(mut store) = crate::db::legacy::read_json::<StatsStore>(path)? else {
        return Ok(None);
    };
    match crate::sync::manager::load_saved_stats_provenance(metadata_path) {
        Ok(provenance) => restore_saved_stats_provenance(&mut store, &provenance),
        Err(error) => log::warn!(target: "stats", "统计来源侧车读取失败，保留本地累计值: {error}"),
    }
    store.prune_retention(chrono::Utc::now().timestamp_millis());
    Ok(Some(store))
}

pub fn save(store: &StatsStore) {
    if let Err(error) = save_checked(store) {
        log::warn!(target: "stats", "播放统计写入失败: {error}");
    }
}

/// 同步提交需要知道落盘结果，失败时保留进度供下一轮重试
pub(crate) fn save_checked(store: &StatsStore) -> crate::error::AppResult<()> {
    if store.storage_unavailable {
        return Err(crate::error::AppError::Other(
            "Playback statistics storage is unavailable until restart".into(),
        ));
    }
    crate::db::user_db()?.write(|transaction| storage::save_into(transaction, store))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::db::UserDatabase;
    use std::path::PathBuf;

    /// 旧版 playback-stats.json 的写法，仅用于构造导入测试的旧文件
    fn save_checked_at(path: &std::path::Path, store: &StatsStore) -> crate::error::AppResult<()> {
        crate::fsutil::atomic_write(path, serde_json::to_vec(store)?)?;
        Ok(())
    }

    fn load_at(path: &std::path::Path, metadata_path: &std::path::Path) -> StatsStore {
        read_legacy_files(path, metadata_path).unwrap().unwrap_or_default()
    }

    fn save_db(database: &UserDatabase, store: &StatsStore) {
        database.write(|transaction| storage::save_into(transaction, store)).unwrap();
    }

    fn load_db(database: &UserDatabase) -> StatsStore {
        database.read(storage::load_from).unwrap()
    }

    #[test]
    fn database_round_trip_preserves_counters_shards_and_clear_epoch() {
        let database = UserDatabase::open_in_memory().unwrap();
        let mut store = StatsStore::default();
        let now = day_start_at(chrono::Utc::now().timestamp_millis()) + 3_600_000;
        store.clear(now - 10);
        store.record(&session("k", 30_000, 1), "desktop", now);
        store.stats[0].counter_shards.push(SyncPlaybackCounterShard {
            device_id: "android".into(),
            epoch_started_at: now - 10,
            total_listen_ms: 5,
            play_count: 1,
            first_played_at: now,
            last_played_at: now,
        });
        save_db(&database, &store);

        let restored = load_db(&database);
        assert_eq!(restored.cleared_at, now - 10);
        assert_eq!(restored.stats.len(), 1);
        assert_eq!(restored.stats[0].play_count, 1);
        assert_eq!(restored.stats[0].counter_shards, store.stats[0].counter_shards);
        assert_eq!(restored.track_shards["k"], store.track_shards["k"]);
        assert_eq!(restored.buckets[0].total_listen_ms, 30_000);
        assert_eq!(restored.daily_shards, store.daily_shards);
        assert_eq!(
            serde_json::to_value(restored.sync_snapshot().0).unwrap(),
            serde_json::to_value(store.sync_snapshot().0).unwrap(),
        );
    }

    #[test]
    fn database_save_only_rewrites_changed_rows() {
        let database = UserDatabase::open_in_memory().unwrap();
        let mut store = StatsStore::default();
        let now = day_start_at(chrono::Utc::now().timestamp_millis()) + 3_600_000;
        store.record(&session("stable", 30_000, 1), "desktop", now);
        store.record(&session("played", 30_000, 1), "desktop", now);
        save_db(&database, &store);
        database
            .write(|transaction| {
                transaction.execute("UPDATE playback_stat SET name = 'untouched' WHERE identity_key = 'stable'", [])?;
                Ok(())
            })
            .unwrap();

        store.record(&session("played", 30_000, 1), "desktop", now + 1_000);
        save_db(&database, &store);

        let restored = load_db(&database);
        let stable = restored.stats.iter().find(|stat| stat.identity_key == "stable").unwrap();
        let played = restored.stats.iter().find(|stat| stat.identity_key == "played").unwrap();
        assert_eq!(stable.name, "untouched");
        assert_eq!(played.play_count, 2);
        assert_eq!(restored.track_shards["played"][0].play_count, 2);
    }

    #[test]
    fn database_save_deletes_removed_tracks_and_cascades_shards() {
        let database = UserDatabase::open_in_memory().unwrap();
        let mut store = StatsStore::default();
        let now = day_start_at(chrono::Utc::now().timestamp_millis()) + 3_600_000;
        store.record(&session("gone", 30_000, 1), "desktop", now);
        store.record(&session("kept", 30_000, 1), "desktop", now);
        save_db(&database, &store);
        store.remove_tracks(&["gone".into()]);
        save_db(&database, &store);

        let counts: (i64, i64) = database
            .read(|connection| {
                Ok((
                    connection.query_row("SELECT COUNT(*) FROM playback_stat_counter_shard", [], |row| row.get(0))?,
                    connection.query_row("SELECT COUNT(*) FROM playback_stat_daily_counter_shard", [], |row| row.get(0))?,
                ))
            })
            .unwrap();
        assert_eq!(counts, (1, 1));
        let restored = load_db(&database);
        assert_eq!(restored.stats.len(), 1);
        assert_eq!(restored.stats[0].identity_key, "kept");
    }

    #[test]
    fn unavailable_store_refuses_to_overwrite_the_database() {
        let store = StatsStore { storage_unavailable: true, ..Default::default() };
        assert!(save_checked(&store).is_err());
    }

    #[test]
    fn legacy_stats_json_is_imported_with_sidecar_provenance() {
        let now = day_start_at(chrono::Utc::now().timestamp_millis()) + 3_600_000;
        let (projection, payload) = old_stats_projection("android", 1, 30_000, now);
        let directory = tempfile::tempdir().unwrap();
        write_old_stats_projection(directory.path(), &projection, &payload);
        let database = UserDatabase::open_in_memory().unwrap();

        assert!(crate::db::legacy::run_once(&database, directory.path(), LEGACY_IMPORT_KEY, import_legacy_json).unwrap());
        let restored = load_db(&database);
        assert_eq!(restored.stats[0].play_count, 1);
        assert_eq!(restored.stats[0].counter_shards, payload.stats[0].counter_shards);
        assert!(!directory.path().join("playback-stats.json").exists());
        assert!(directory.path().join("sync-android-metadata.json").exists(), "the sync sidecar belongs to the sync importer");
    }

    fn session(identity: &str, listened_ms: i64, increment: i32) -> PlaybackSession {
        PlaybackSession {
            identity_key: identity.into(),
            id: "42".into(),
            name: "Song".into(),
            artist: "Artist".into(),
            album: "netease".into(),
            album_id: "0".into(),
            cover_url: Some("https://cover".into()),
            media_uri: None,
            duration_ms: 200_000,
            listened_ms,
            play_count_increment: increment,
        }
    }

    #[test]
    fn recording_accumulates_aggregate_bucket_and_shard_consistently() {
        let mut store = StatsStore::default();
        let now = day_start_at(chrono::Utc::now().timestamp_millis()) + 3_600_000;

        store.record(&session("k", 30_000, 1), "desktop", now);
        store.record(&session("k", 45_000, 1), "desktop", now);

        assert_eq!(store.stats.len(), 1);
        assert_eq!(store.stats[0].play_count, 2);
        assert_eq!(store.stats[0].total_listen_ms, 75_000);
        assert_eq!(store.buckets.len(), 1);
        assert_eq!(store.buckets[0].play_count, 2);

        // 分片是跨端合并的唯一真值源，必须与聚合值一致
        let shards = &store.track_shards["k"];
        assert_eq!(shards.len(), 1);
        assert_eq!(shards[0].play_count, 2);
        assert_eq!(shards[0].total_listen_ms, 75_000);
    }

    #[test]
    fn bucket_sum_never_exceeds_all_period_total() {
        let mut store = StatsStore::default();
        let now = chrono::Utc::now().timestamp_millis();
        let today = day_start_at(now) + 3_600_000;
        let yesterday = today - 86_400_000;

        store.record(&session("k", 60_000, 1), "desktop", yesterday);
        store.record(&session("k", 60_000, 1), "desktop", today);

        let all = store.summarize(StatsPeriod::All, now);
        let year = store.summarize(StatsPeriod::Year, now);
        let day = store.summarize(StatsPeriod::Day, now);

        assert_eq!(all.total_play_count, 2);
        assert!(all.total_play_count >= year.total_play_count);
        assert_eq!(day.total_play_count, 1);
    }

    #[test]
    fn apply_merged_keeps_only_local_shards() {
        let mut store = StatsStore::default();
        let now = chrono::Utc::now().timestamp_millis();
        store.record(&session("k", 60_000, 1), "desktop", now);

        let (mut stats, buckets, cleared_at) = store.sync_snapshot();
        // 模拟云端合并进来的他机分片
        stats[0].counter_shards.push(SyncPlaybackCounterShard {
            device_id: "android".into(),
            epoch_started_at: 0,
            total_listen_ms: 90_000,
            play_count: 3,
            first_played_at: now,
            last_played_at: now,
        });
        stats[0].play_count = 4;

        store.apply_merged(&stats, &buckets, cleared_at, "desktop");

        assert_eq!(store.stats[0].play_count, 4);
        // 只留本机分片，否则下轮上传会把他机增量重复累加
        let shards = &store.track_shards["k"];
        assert_eq!(shards.len(), 1);
        assert_eq!(shards[0].device_id, "desktop");
    }

    #[test]
    fn android_alignment_stats_apply_preserves_recorded_during_sync() {
        let mut store = StatsStore::default();
        let now = day_start_at(chrono::Utc::now().timestamp_millis()) + 3_600_000;
        store.record(&session("k", 30_000, 1), "desktop", now);
        let (stats, buckets, cleared_at) = store.sync_snapshot();

        store.record(&session("k", 30_000, 1), "desktop", now + 1_000);
        store.apply_merged(&stats, &buckets, cleared_at, "desktop");
        assert_eq!(store.stats[0].play_count, 2);
        assert_eq!(store.stats[0].total_listen_ms, 60_000);
        assert_eq!(store.buckets[0].play_count, 2);
        assert_eq!(store.track_shards["k"][0].play_count, 2);
        assert_eq!(store.daily_shards.values().next().unwrap()[0].play_count, 2);

        store.apply_merged(&stats, &buckets, cleared_at, "desktop");
        assert_eq!(store.stats[0].play_count, 2);
        store.record(&session("k", 30_000, 1), "desktop", now + 2_000);
        assert_eq!(store.stats[0].play_count, 3);
        assert_eq!(store.track_shards["k"][0].play_count, 3);
    }

    #[test]
    fn android_alignment_stats_apply_preserves_clear_and_new_epoch() {
        let mut store = StatsStore::default();
        let now = day_start_at(chrono::Utc::now().timestamp_millis()) + 3_600_000;
        store.record(&session("old", 30_000, 1), "desktop", now);
        let (stats, buckets, cleared_at) = store.sync_snapshot();
        let cleared = now + 1_000;
        store.clear(cleared);

        store.apply_merged(&stats, &buckets, cleared_at, "desktop");
        assert_eq!(store.cleared_at, cleared);
        assert!(store.stats.is_empty());
        assert!(store.buckets.is_empty());
        assert!(store.track_shards.is_empty());
        assert!(store.daily_shards.is_empty());

        store.record(&session("new", 30_000, 1), "desktop", now + 2_000);
        store.apply_merged(&stats, &buckets, cleared_at, "desktop");
        assert_eq!(store.stats.len(), 1);
        assert_eq!(store.stats[0].identity_key, "new");
        assert_eq!(store.stats[0].play_count, 1);
        assert_eq!(store.buckets.len(), 1);
        assert_eq!(store.buckets[0].identity_key, "new");
        assert_eq!(store.track_shards["new"][0].epoch_started_at, cleared);
        assert!(store.daily_shards.values().all(|shards| shards[0].epoch_started_at == cleared));
    }

    #[test]
    fn android_alignment_stats_foreign_provenance_repeated_apply_is_idempotent() {
        let now = day_start_at(chrono::Utc::now().timestamp_millis()) + 3_600_000;
        let mut android = StatsStore::default();
        android.record(&session("k", 30_000, 1), "android", now);
        let (stats, buckets, cleared_at) = android.sync_snapshot();
        let mut store = StatsStore::default();

        for _ in 0..3 {
            store.apply_merged(&stats, &buckets, cleared_at, "desktop");
            assert_eq!(store.stats[0].play_count, 1);
            assert_eq!(store.stats[0].total_listen_ms, 30_000);
            assert_eq!(store.buckets[0].play_count, 1);
            assert_eq!(store.buckets[0].total_listen_ms, 30_000);
            assert!(store.track_shards.is_empty());
            assert!(store.daily_shards.is_empty());
        }
        let (snapshot, daily, _) = store.sync_snapshot();
        assert_eq!(snapshot[0].counter_shards, stats[0].counter_shards);
        assert_eq!(daily[0].counter_shards, buckets[0].counter_shards);
        assert_eq!(snapshot[0].counter_base_play_count, 0);
    }

    #[test]
    fn android_alignment_stats_foreign_provenance_survives_save_and_reload() {
        let now = day_start_at(chrono::Utc::now().timestamp_millis()) + 3_600_000;
        let mut android = StatsStore::default();
        android.record(&session("k", 30_000, 1), "android", now);
        let (stats, buckets, cleared_at) = android.sync_snapshot();
        let mut store = StatsStore::default();
        store.apply_merged(&stats, &buckets, cleared_at, "desktop");

        let database = UserDatabase::open_in_memory().unwrap();
        save_db(&database, &store);
        let mut restored = load_db(&database);
        restored.apply_merged(&stats, &buckets, cleared_at, "desktop");
        assert_eq!(restored.stats[0].play_count, 1);
        assert_eq!(restored.stats[0].total_listen_ms, 30_000);
        assert_eq!(restored.buckets[0].play_count, 1);
        assert_eq!(restored.buckets[0].total_listen_ms, 30_000);
        let (snapshot, daily, _) = restored.sync_snapshot();
        assert_eq!(snapshot[0].counter_shards, stats[0].counter_shards);
        assert_eq!(daily[0].counter_shards, buckets[0].counter_shards);
    }

    #[test]
    fn android_alignment_stats_foreign_provenance_retains_new_own_records() {
        let now = day_start_at(chrono::Utc::now().timestamp_millis()) + 3_600_000;
        let mut store = StatsStore::default();
        store.record(&session("k", 30_000, 1), "desktop", now);
        let (local_stats, local_buckets, _) = store.sync_snapshot();
        let mut android = StatsStore::default();
        android.record(&session("k", 45_000, 2), "android", now);
        let (stats, buckets, _) = android.sync_snapshot();
        let merged = crate::sync::merge::three_way_merge(
            &SyncData { playback_stats: local_stats, playback_stat_buckets: local_buckets, ..Default::default() },
            &SyncData { playback_stats: stats, playback_stat_buckets: buckets, ..Default::default() },
            0,
            &HashMap::new(),
        );
        store.apply_merged(&merged.playback_stats, &merged.playback_stat_buckets, 0, "desktop");
        store.record(&session("k", 30_000, 1), "desktop", now + 1_000);
        store.apply_merged(&merged.playback_stats, &merged.playback_stat_buckets, 0, "desktop");
        let (snapshot, daily, _) = store.sync_snapshot();
        assert_eq!(snapshot[0].play_count, 4);
        assert_eq!(snapshot[0].total_listen_ms, 105_000);
        assert_eq!(daily[0].play_count, 4);
        assert_eq!(daily[0].total_listen_ms, 105_000);
        for shards in [&snapshot[0].counter_shards, &daily[0].counter_shards] {
            assert_eq!(shards.len(), 2);
            assert_eq!(shards.iter().find(|shard| shard.device_id == "desktop").unwrap().play_count, 2);
            assert_eq!(shards.iter().find(|shard| shard.device_id == "android").unwrap().play_count, 2);
        }
        assert_eq!(store.track_shards["k"].len(), 1);
        assert_eq!(store.track_shards["k"][0].play_count, 2);
        store.apply_merged(&merged.playback_stats, &merged.playback_stat_buckets, 0, "desktop");
        assert_eq!(store.stats[0].play_count, 4);
        store.record(&session("k", 30_000, 1), "desktop", now + 2_000);
        store.apply_merged(&merged.playback_stats, &merged.playback_stat_buckets, 0, "desktop");
        assert_eq!(store.stats[0].play_count, 5);
        assert_eq!(store.stats[0].total_listen_ms, 135_000);
    }

    #[test]
    fn android_alignment_stats_foreign_provenance_preserves_legacy_before_first_record() {
        let now = day_start_at(chrono::Utc::now().timestamp_millis()) + 3_600_000;
        let legacy = serde_json::json!({
            "stats": [{ "identityKey": "k", "playCount": 5, "totalListenMs": 100_000,
                "firstPlayedAt": now, "lastPlayedAt": now }],
            "buckets": [{ "identityKey": "k", "dayStartAt": day_start_at(now), "playCount": 5,
                "totalListenMs": 100_000, "firstPlayedAt": now, "lastPlayedAt": now }],
        });
        let mut store: StatsStore = serde_json::from_value(legacy).unwrap();
        let (legacy_stats, legacy_buckets, _) = store.sync_snapshot();
        assert!(legacy_stats[0].counter_shards.is_empty());
        assert!(legacy_buckets[0].counter_shards.is_empty());
        assert_eq!(legacy_stats[0].play_count, 5);
        store.record(&session("k", 30_000, 1), "desktop", now + 1_000);
        let mut android = StatsStore::default();
        android.record(&session("k", 40_000, 1), "android", now + 2_000);
        let (stats, buckets, _) = android.sync_snapshot();

        for _ in 0..3 {
            store.apply_merged(&stats, &buckets, 0, "desktop");
            assert_eq!(store.stats[0].play_count, 7);
            assert_eq!(store.stats[0].total_listen_ms, 170_000);
            assert_eq!(store.buckets[0].play_count, 7);
            assert_eq!(store.buckets[0].total_listen_ms, 170_000);
            assert_eq!(store.stats[0].counter_base_play_count, 5);
            assert_eq!(store.buckets[0].counter_base_play_count, 5);
        }
    }

    #[test]
    fn android_alignment_stats_foreign_provenance_clear_keeps_only_the_new_epoch() {
        let now = day_start_at(chrono::Utc::now().timestamp_millis()) + 3_600_000;
        let mut android = StatsStore::default();
        android.record(&session("k", 30_000, 1), "android", now);
        let (stats, buckets, cleared_at) = android.sync_snapshot();
        let mut store = StatsStore::default();
        store.apply_merged(&stats, &buckets, cleared_at, "desktop");
        store.clear(now + 1_000);
        store.record(&session("k", 40_000, 1), "desktop", now + 2_000);
        store.apply_merged(&stats, &buckets, cleared_at, "desktop");
        let (snapshot, daily, cleared) = store.sync_snapshot();
        assert_eq!(cleared, now + 1_000);
        for shards in [&snapshot[0].counter_shards, &daily[0].counter_shards] {
            assert_eq!(shards.len(), 1);
            assert_eq!(shards[0].device_id, "desktop");
            assert_eq!(shards[0].epoch_started_at, cleared);
        }
        assert_eq!(snapshot[0].play_count, 1);
        assert_eq!(snapshot[0].total_listen_ms, 40_000);
        assert_eq!(daily[0].play_count, 1);
        assert_eq!(daily[0].total_listen_ms, 40_000);
    }

    fn old_stats_projection(
        device_id: &str,
        count: i32,
        listened_ms: i64,
        now: i64,
    ) -> (StatsStore, crate::sync::manager::SyncStatsPayload) {
        let mut source = StatsStore::default();
        source.record(&session("k", listened_ms, count), device_id, now);
        let (stats, buckets, cleared_at) = source.sync_snapshot();
        let payload = crate::sync::manager::SyncStatsPayload { stats, buckets, cleared_at };
        let mut projection = StatsStore {
            stats: payload.stats.clone(),
            buckets: payload.buckets.clone(),
            cleared_at,
            ..Default::default()
        };
        for stat in &mut projection.stats { stat.counter_shards.clear(); }
        for bucket in &mut projection.buckets { bucket.counter_shards.clear(); }
        (projection, payload)
    }

    fn write_old_stats_projection(
        directory: &std::path::Path,
        store: &StatsStore,
        provenance: &crate::sync::manager::SyncStatsPayload,
    ) -> (PathBuf, PathBuf) {
        let stats_path = directory.join("playback-stats.json");
        let metadata_path = directory.join("sync-android-metadata.json");
        save_checked_at(&stats_path, store).unwrap();
        std::fs::write(&metadata_path, serde_json::to_vec(&serde_json::json!({
            "playbackStats": provenance.stats,
            "playbackStatBuckets": provenance.buckets,
            "playbackStatsClearedAt": provenance.cleared_at,
        })).unwrap()).unwrap();
        (stats_path, metadata_path)
    }

    #[test]
    fn android_alignment_stats_sidecar_projection_restores_before_repeated_apply() {
        let now = day_start_at(chrono::Utc::now().timestamp_millis()) + 3_600_000;
        let (projection, payload) = old_stats_projection("android", 1, 30_000, now);
        let directory = tempfile::tempdir().unwrap();
        let (path, metadata) = write_old_stats_projection(directory.path(), &projection, &payload);
        let mut store = load_at(&path, &metadata);
        for _ in 0..2 {
            store.apply_merged(&payload.stats, &payload.buckets, payload.cleared_at, "desktop");
            assert_eq!(store.stats[0].play_count, 1);
            assert_eq!(store.stats[0].total_listen_ms, 30_000);
            assert_eq!(store.buckets[0].play_count, 1);
            assert_eq!(store.buckets[0].total_listen_ms, 30_000);
            assert_eq!(store.stats[0].counter_base_play_count, 0);
        }
        save_checked_at(&path, &store).unwrap();
        let restored = load_at(&path, &metadata);
        assert_eq!(restored.sync_snapshot().0[0].counter_shards, payload.stats[0].counter_shards);
    }

    #[test]
    fn android_alignment_stats_sidecar_projection_keeps_true_legacy_base() {
        let now = day_start_at(chrono::Utc::now().timestamp_millis()) + 3_600_000;
        let (mut projection, mut payload) = old_stats_projection("android", 1, 30_000, now);
        for stat in [&mut projection.stats[0], &mut payload.stats[0]] {
            stat.counter_base_play_count = 5;
            stat.counter_base_listen_ms = 100_000;
            stat.play_count = 6;
            stat.total_listen_ms = 130_000;
        }
        for bucket in [&mut projection.buckets[0], &mut payload.buckets[0]] {
            bucket.counter_base_play_count = 5;
            bucket.counter_base_listen_ms = 100_000;
            bucket.play_count = 6;
            bucket.total_listen_ms = 130_000;
        }
        let directory = tempfile::tempdir().unwrap();
        let (path, metadata) = write_old_stats_projection(directory.path(), &projection, &payload);
        let mut store = load_at(&path, &metadata);
        assert_eq!(store.stats[0].counter_shards, payload.stats[0].counter_shards);
        assert_eq!(store.buckets[0].counter_shards, payload.buckets[0].counter_shards);
        for _ in 0..2 {
            store.apply_merged(&payload.stats, &payload.buckets, 0, "desktop");
            assert_eq!(store.stats[0].play_count, 6);
            assert_eq!(store.buckets[0].play_count, 6);
            assert_eq!(store.stats[0].counter_base_play_count, 5);
            assert_eq!(store.stats[0].counter_base_listen_ms, 100_000);
        }
        let mut genuine_legacy = projection;
        genuine_legacy.stats[0].counter_base_play_count = 6;
        genuine_legacy.buckets[0].counter_base_play_count = 6;
        let (path, metadata) = write_old_stats_projection(directory.path(), &genuine_legacy, &payload);
        let unchanged = load_at(&path, &metadata);
        assert!(unchanged.stats[0].counter_shards.is_empty());
        assert!(unchanged.buckets[0].counter_shards.is_empty());
        assert_eq!(unchanged.stats[0].counter_base_play_count, 6);
    }

    #[test]
    fn android_alignment_stats_sidecar_projection_rejects_changed_counters() {
        let now = day_start_at(chrono::Utc::now().timestamp_millis()) + 3_600_000;
        let (projection, payload) = old_stats_projection("android", 1, 30_000, now);
        for field in 0..6 {
            let mut changed = payload.clone();
            match field {
                0 => { changed.stats[0].play_count += 1; changed.buckets[0].play_count += 1; }
                1 => { changed.stats[0].total_listen_ms += 1; changed.buckets[0].total_listen_ms += 1; }
                2 => { changed.stats[0].counter_base_play_count += 1; changed.buckets[0].counter_base_play_count += 1; }
                3 => { changed.stats[0].counter_base_listen_ms += 1; changed.buckets[0].counter_base_listen_ms += 1; }
                4 => { changed.stats[0].first_played_at += 1; changed.buckets[0].first_played_at += 1; }
                _ => { changed.stats[0].last_played_at += 1; changed.buckets[0].last_played_at += 1; }
            }
            let directory = tempfile::tempdir().unwrap();
            let (path, metadata) = write_old_stats_projection(directory.path(), &projection, &changed);
            let store = load_at(&path, &metadata);
            assert!(store.stats[0].counter_shards.is_empty(), "field {field}");
            assert!(store.buckets[0].counter_shards.is_empty(), "field {field}");
            assert_eq!(store.stats[0].play_count, 1);
            assert_eq!(store.buckets[0].total_listen_ms, 30_000);
        }
    }

    #[test]
    fn android_alignment_stats_sidecar_projection_rejects_different_clear_and_duplicates() {
        let now = day_start_at(chrono::Utc::now().timestamp_millis()) + 3_600_000;
        let (projection, payload) = old_stats_projection("android", 1, 30_000, now);
        for scenario in 0..3 {
            let mut changed = payload.clone();
            let mut local = projection.clone();
            match scenario {
                0 => changed.cleared_at = now,
                1 => {
                    changed.stats.push(changed.stats[0].clone());
                    changed.buckets.push(changed.buckets[0].clone());
                }
                _ => {
                    local.stats.push(local.stats[0].clone());
                    local.buckets.push(local.buckets[0].clone());
                }
            }
            let directory = tempfile::tempdir().unwrap();
            let (path, metadata) = write_old_stats_projection(directory.path(), &local, &changed);
            let store = load_at(&path, &metadata);
            assert!(store.stats.iter().all(|stat| stat.counter_shards.is_empty()));
            assert!(store.buckets.iter().all(|bucket| bucket.counter_shards.is_empty()));
            assert_eq!(store.cleared_at, local.cleared_at);
        }
    }

    #[test]
    fn android_alignment_stats_sidecar_projection_preserves_data_on_metadata_io_failure() {
        let now = day_start_at(chrono::Utc::now().timestamp_millis()) + 3_600_000;
        let (projection, payload) = old_stats_projection("android", 1, 30_000, now);
        let directory = tempfile::tempdir().unwrap();
        let (path, metadata) = write_old_stats_projection(directory.path(), &projection, &payload);
        std::fs::remove_file(&metadata).unwrap();
        std::fs::create_dir(&metadata).unwrap();
        let store = load_at(&path, &metadata);
        assert_eq!(store.stats[0].play_count, 1);
        assert_eq!(store.stats[0].total_listen_ms, 30_000);
        assert!(store.stats[0].counter_shards.is_empty());
        assert!(store.buckets[0].counter_shards.is_empty());
    }

    #[test]
    fn android_alignment_stats_sidecar_projection_seeds_missing_own_maps_before_record() {
        let now = day_start_at(chrono::Utc::now().timestamp_millis()) + 3_600_000;
        let (projection, payload) = old_stats_projection("desktop", 5, 150_000, now);
        let directory = tempfile::tempdir().unwrap();
        let (path, metadata) = write_old_stats_projection(directory.path(), &projection, &payload);
        let mut store = load_at(&path, &metadata);
        assert!(store.track_shards.is_empty());
        assert!(store.daily_shards.is_empty());
        store.record(&session("k", 30_000, 1), "desktop", now + 1_000);
        let (stats, buckets, _) = store.sync_snapshot();
        for shards in [&stats[0].counter_shards, &buckets[0].counter_shards] {
            assert_eq!(shards.len(), 1);
            assert_eq!(shards[0].device_id, "desktop");
            assert_eq!(shards[0].play_count, 6);
            assert_eq!(shards[0].total_listen_ms, 180_000);
        }
        assert_eq!(store.track_shards["k"].len(), 1);
        assert_eq!(store.track_shards["k"][0].play_count, 6);
    }

    #[test]
    fn clear_resets_everything_and_advances_epoch() {
        let mut store = StatsStore::default();
        let now = chrono::Utc::now().timestamp_millis();
        store.record(&session("k", 60_000, 1), "desktop", now);
        store.clear(now);

        assert!(store.stats.is_empty());
        assert!(store.buckets.is_empty());
        assert!(store.track_shards.is_empty());
        assert_eq!(store.cleared_at, now);

        // 清除后的新增量落在新 epoch 上
        store.record(&session("k", 60_000, 1), "desktop", now + 1_000);
        assert_eq!(store.track_shards["k"][0].epoch_started_at, now);
    }

    /// 保留窗口修剪：超 400 天的日分桶/日分片被清除，聚合值与窗口内数据保留
    #[test]
    fn retention_prunes_day_scoped_data_but_keeps_aggregates() {
        let mut store = StatsStore::default();
        let now = chrono::Utc::now().timestamp_millis();
        let ancient = now - (STAT_RETENTION_DAYS + 10) * MILLIS_PER_DAY;
        let recent = now - 3 * MILLIS_PER_DAY;

        store.record(&session("old", 60_000, 1), "desktop", ancient);
        store.record(&session("new", 60_000, 1), "desktop", recent);

        store.prune_retention(now);

        // 日口径：仅窗口内保留
        assert_eq!(store.buckets.len(), 1);
        assert_eq!(store.buckets[0].identity_key, "new");
        assert_eq!(store.daily_shards.len(), 1);
        assert!(store
            .daily_shards
            .keys()
            .all(|key| key.ends_with("|new")));
        // 「总」口径不受窗口影响
        assert_eq!(store.stats.len(), 2);
        assert_eq!(store.track_shards.len(), 2);
    }

    /// 每天首次 record 自动触发修剪
    #[test]
    fn record_triggers_daily_prune() {
        let mut store = StatsStore::default();
        let now = chrono::Utc::now().timestamp_millis();
        let ancient = now - (STAT_RETENTION_DAYS + 10) * MILLIS_PER_DAY;

        // 直接注入过期分桶（绕过 record 的自动修剪）
        store.buckets.push(SyncPlaybackStatBucket {
            day_start_at: day_start_at(ancient),
            identity_key: "old".into(),
            total_listen_ms: 1,
            play_count: 1,
            ..Default::default()
        });

        store.record(&session("new", 60_000, 1), "desktop", now);

        assert!(
            store.buckets.iter().all(|bucket| bucket.identity_key != "old"),
            "过期分桶必须在当天首次 record 时被修剪"
        );
    }

    /// record 后立即按「日」查询必须可见，且保留窗口修剪不得动今天的数据
    #[test]
    fn record_is_immediately_visible_today_and_survives_prune() {
        let mut store = StatsStore::default();
        let now = chrono::Utc::now().timestamp_millis();

        store.record(&session("k", 15_000, 0), "desktop", now);
        store.prune_retention(now);

        let day = store.summarize(StatsPeriod::Day, now);
        assert_eq!(day.track_count, 1);
        assert_eq!(day.total_listen_ms, 15_000);
        assert_eq!(store.buckets.len(), 1, "今天的日分桶不得被修剪");
        assert_eq!(store.daily_shards.len(), 1, "今天的日分片不得被修剪");
    }

    #[test]
    fn day_start_is_local_midnight() {
        let now = chrono::Utc::now().timestamp_millis();
        let start = day_start_at(now);
        assert!(start <= now);
        assert_eq!(day_start_at(start), start);
        assert!(now - start < 86_400_000);
    }
}
