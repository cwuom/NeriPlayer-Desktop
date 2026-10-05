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
use std::path::PathBuf;

use chrono::{Datelike, Local, TimeZone};
use serde::{Deserialize, Serialize};

use crate::sync::models::{
    SyncData, SyncPlaybackCounterShard, SyncPlaybackStatBucket, SyncTrackStat,
};

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

        // 聚合
        if let Some(stat) = self
            .stats
            .iter_mut()
            .find(|stat| stat.identity_key == session.identity_key)
        {
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
                .entry(daily_shard_key(day, &session.identity_key))
                .or_default(),
            device_id,
            epoch,
            listened,
            increment,
            played_at,
        );
    }

    /// 上传用快照：把本地分片挂到统计条目上
    pub fn sync_snapshot(&self) -> (Vec<SyncTrackStat>, Vec<SyncPlaybackStatBucket>, i64) {
        let stats = self
            .stats
            .iter()
            .map(|stat| {
                let mut out = stat.clone();
                out.counter_shards = self
                    .track_shards
                    .get(&stat.identity_key)
                    .cloned()
                    .unwrap_or_default();
                out
            })
            .collect();
        let buckets = self
            .buckets
            .iter()
            .map(|bucket| {
                let mut out = bucket.clone();
                out.counter_shards = self
                    .daily_shards
                    .get(&daily_shard_key(bucket.day_start_at, &bucket.identity_key))
                    .cloned()
                    .unwrap_or_default();
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
        for stat in &mut self.stats {
            stat.counter_shards.clear();
        }
        for bucket in &mut self.buckets {
            bucket.counter_shards.clear();
        }
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

pub fn stats_path() -> PathBuf {
    let mut path = dirs_next::data_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("NeriPlayer");
    path.push("playback-stats.json");
    path
}

pub fn load() -> StatsStore {
    let path = stats_path();
    let Ok(content) = std::fs::read_to_string(&path) else {
        return StatsStore::default();
    };
    let mut store: StatsStore = match serde_json::from_str(&content) {
        Ok(store) => store,
        Err(error) => {
            // 统计可容忍从空重建（分片会随同步找回），但必须保留现场：
            // 静默清空 + 后续覆盖写会让损坏原因永远无法排查
            let quarantined = crate::fsutil::quarantine_corrupt_file(&path);
            log::warn!(
                target: "stats",
                "playback-stats.json 解析失败, 现场已隔离到 {:?}, 从空统计重建: {}",
                quarantined,
                error,
            );
            return StatsStore::default();
        }
    };
    // 启动即修剪保留窗口，避免历史膨胀文件一直原样滚动
    store.prune_retention(chrono::Utc::now().timestamp_millis());
    store
}

pub fn save(store: &StatsStore) {
    let path = stats_path();
    if let Ok(content) = serde_json::to_string(store) {
        // 原子写：崩溃/断电不能留下半截 JSON（半截会被 load 判损坏隔离）
        if let Err(error) = crate::fsutil::atomic_write(&path, content) {
            log::warn!(target: "stats", "playback-stats.json 写入失败: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
