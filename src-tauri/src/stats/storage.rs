// 播放统计的 SQLite 持久化（对齐 Android playback_stat / playback_stat_bucket 及计数分片表）
//
// 内存中的 StatsStore 仍是统计口径的唯一实现；这里只负责把它映射到表。保存时
// 按行内容摘要比对，只写入真正变化的聚合行与分桶行，单次收听只会触达几行
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, Row, Transaction};

use super::{daily_shard_key, StatsStore};
use crate::db;
use crate::error::{AppError, AppResult};
use crate::sync::models::{SyncPlaybackCounterShard, SyncPlaybackStatBucket, SyncTrackStat};

pub(crate) const LEGACY_IMPORT_KEY: &str = "legacy_import.playback_stats";
const CLEARED_AT_KEY: &str = "playback_stats.cleared_at";
const LEGACY_FILE: &str = "playback-stats.json";
const LEGACY_METADATA_FILE: &str = "sync-android-metadata.json";

const STAT_COLUMNS: &str = "identity_key, id, name, artist, album, album_id, cover_url, duration_ms,
    total_listen_ms, play_count, last_played_at, first_played_at, media_uri,
    counter_base_listen_ms, counter_base_play_count, provenance_shards_json";

const SHARD_COLUMNS: &str =
    "device_id, epoch_started_at, total_listen_ms, play_count, first_played_at, last_played_at";

pub(crate) fn load_from(connection: &Connection) -> AppResult<StatsStore> {
    let stats = connection
        .prepare(&format!("SELECT {STAT_COLUMNS} FROM playback_stat ORDER BY identity_key"))?
        .query_and_then([], |row| stat_from_row(row, 0))?
        .collect::<AppResult<Vec<_>>>()?;
    let buckets = connection
        .prepare(&format!(
            "SELECT day_start_at, {STAT_COLUMNS} FROM playback_stat_bucket
             ORDER BY day_start_at, identity_key"
        ))?
        .query_and_then([], |row| -> AppResult<SyncPlaybackStatBucket> {
            let day_start_at: i64 = row.get(0)?;
            Ok(bucket_from_stat(day_start_at, stat_from_row(row, 1)?))
        })?
        .collect::<AppResult<Vec<_>>>()?;

    let mut track_shards: HashMap<String, Vec<SyncPlaybackCounterShard>> = HashMap::new();
    {
        let mut statement = connection.prepare(&format!(
            "SELECT identity_key, {SHARD_COLUMNS} FROM playback_stat_counter_shard
             ORDER BY identity_key, device_id, epoch_started_at"
        ))?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            track_shards.entry(row.get(0)?).or_default().push(shard_from_row(row, 1)?);
        }
    }
    let mut daily_shards: HashMap<String, Vec<SyncPlaybackCounterShard>> = HashMap::new();
    {
        let mut statement = connection.prepare(&format!(
            "SELECT day_start_at, identity_key, {SHARD_COLUMNS} FROM playback_stat_daily_counter_shard
             ORDER BY day_start_at, identity_key, device_id, epoch_started_at"
        ))?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let key = daily_shard_key(row.get(0)?, &row.get::<_, String>(1)?);
            daily_shards.entry(key).or_default().push(shard_from_row(row, 2)?);
        }
    }

    Ok(StatsStore {
        stats,
        buckets,
        track_shards,
        daily_shards,
        cleared_at: db::meta::get_i64(connection, CLEARED_AT_KEY)?.unwrap_or(0),
        ..Default::default()
    })
}

pub(crate) fn save_into(transaction: &Transaction<'_>, store: &StatsStore) -> AppResult<()> {
    save_stats(transaction, store)?;
    save_buckets(transaction, store)?;
    db::meta::set_i64(transaction, CLEARED_AT_KEY, store.cleared_at.max(0))
}

fn save_stats(transaction: &Transaction<'_>, store: &StatsStore) -> AppResult<()> {
    let stored: HashMap<String, i64> = transaction
        .prepare("SELECT identity_key, row_hash FROM playback_stat")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut upsert = transaction.prepare_cached(&format!(
        "INSERT INTO playback_stat ({STAT_COLUMNS}, row_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
         ON CONFLICT(identity_key) DO UPDATE SET id = excluded.id, name = excluded.name,
             artist = excluded.artist, album = excluded.album, album_id = excluded.album_id,
             cover_url = excluded.cover_url, duration_ms = excluded.duration_ms,
             total_listen_ms = excluded.total_listen_ms, play_count = excluded.play_count,
             last_played_at = excluded.last_played_at, first_played_at = excluded.first_played_at,
             media_uri = excluded.media_uri, counter_base_listen_ms = excluded.counter_base_listen_ms,
             counter_base_play_count = excluded.counter_base_play_count,
             provenance_shards_json = excluded.provenance_shards_json, row_hash = excluded.row_hash"
    ))?;
    let mut kept = HashSet::new();
    for stat in &store.stats {
        // record() 总是更新第一条，同键重复项只可能来自旧数据异常
        if stat.identity_key.trim().is_empty() || !kept.insert(stat.identity_key.as_str()) {
            continue;
        }
        let own = store.track_shards.get(&stat.identity_key).map(Vec::as_slice).unwrap_or(&[]);
        let hash = stat_hash(stat, own);
        if stored.get(&stat.identity_key) == Some(&hash) {
            continue;
        }
        upsert.execute(params![
            stat.identity_key,
            stat.id,
            stat.name,
            stat.artist,
            stat.album,
            stat.album_id,
            stat.cover_url,
            stat.duration_ms,
            stat.total_listen_ms,
            stat.play_count,
            stat.last_played_at,
            stat.first_played_at,
            stat.media_uri,
            stat.counter_base_listen_ms,
            stat.counter_base_play_count,
            serde_json::to_string(&stat.counter_shards)?,
            hash,
        ])?;
        transaction.execute(
            "DELETE FROM playback_stat_counter_shard WHERE identity_key = ?1",
            [&stat.identity_key],
        )?;
        let mut insert = transaction.prepare_cached(&format!(
            "INSERT OR REPLACE INTO playback_stat_counter_shard (identity_key, {SHARD_COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
        ))?;
        for shard in own {
            insert.execute(params![
                stat.identity_key,
                shard.device_id,
                shard.epoch_started_at,
                shard.total_listen_ms,
                shard.play_count,
                shard.first_played_at,
                shard.last_played_at,
            ])?;
        }
    }
    for key in stored.keys().filter(|key| !kept.contains(key.as_str())) {
        transaction.execute("DELETE FROM playback_stat WHERE identity_key = ?1", [key])?;
    }
    Ok(())
}

fn save_buckets(transaction: &Transaction<'_>, store: &StatsStore) -> AppResult<()> {
    let stored: HashMap<(i64, String), i64> = transaction
        .prepare("SELECT day_start_at, identity_key, row_hash FROM playback_stat_bucket")?
        .query_map([], |row| Ok(((row.get(0)?, row.get(1)?), row.get(2)?)))?
        .collect::<Result<_, _>>()?;
    let mut upsert = transaction.prepare_cached(&format!(
        "INSERT INTO playback_stat_bucket (day_start_at, {STAT_COLUMNS}, row_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
         ON CONFLICT(day_start_at, identity_key) DO UPDATE SET id = excluded.id,
             name = excluded.name, artist = excluded.artist, album = excluded.album,
             album_id = excluded.album_id, cover_url = excluded.cover_url,
             duration_ms = excluded.duration_ms, total_listen_ms = excluded.total_listen_ms,
             play_count = excluded.play_count, last_played_at = excluded.last_played_at,
             first_played_at = excluded.first_played_at, media_uri = excluded.media_uri,
             counter_base_listen_ms = excluded.counter_base_listen_ms,
             counter_base_play_count = excluded.counter_base_play_count,
             provenance_shards_json = excluded.provenance_shards_json, row_hash = excluded.row_hash"
    ))?;
    let mut kept = HashSet::new();
    for bucket in &store.buckets {
        if bucket.identity_key.trim().is_empty()
            || !kept.insert((bucket.day_start_at, bucket.identity_key.clone()))
        {
            continue;
        }
        let own = store
            .daily_shards
            .get(&daily_shard_key(bucket.day_start_at, &bucket.identity_key))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let hash = bucket_hash(bucket, own);
        if stored.get(&(bucket.day_start_at, bucket.identity_key.clone())) == Some(&hash) {
            continue;
        }
        upsert.execute(params![
            bucket.day_start_at,
            bucket.identity_key,
            bucket.id,
            bucket.name,
            bucket.artist,
            bucket.album,
            bucket.album_id,
            bucket.cover_url,
            bucket.duration_ms,
            bucket.total_listen_ms,
            bucket.play_count,
            bucket.last_played_at,
            bucket.first_played_at,
            bucket.media_uri,
            bucket.counter_base_listen_ms,
            bucket.counter_base_play_count,
            serde_json::to_string(&bucket.counter_shards)?,
            hash,
        ])?;
        transaction.execute(
            "DELETE FROM playback_stat_daily_counter_shard WHERE day_start_at = ?1 AND identity_key = ?2",
            params![bucket.day_start_at, bucket.identity_key],
        )?;
        let mut insert = transaction.prepare_cached(&format!(
            "INSERT OR REPLACE INTO playback_stat_daily_counter_shard (day_start_at, identity_key, {SHARD_COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
        ))?;
        for shard in own {
            insert.execute(params![
                bucket.day_start_at,
                bucket.identity_key,
                shard.device_id,
                shard.epoch_started_at,
                shard.total_listen_ms,
                shard.play_count,
                shard.first_played_at,
                shard.last_played_at,
            ])?;
        }
    }
    for (day_start_at, identity_key) in stored.keys().filter(|key| !kept.contains(*key)) {
        transaction.execute(
            "DELETE FROM playback_stat_bucket WHERE day_start_at = ?1 AND identity_key = ?2",
            params![day_start_at, identity_key],
        )?;
    }
    Ok(())
}

/// 旧版 playback-stats.json 一次性导入
///
/// 旧投影可能缺少分片来源，导入时用同步侧车补回，之后数据库里的分片就是完整的
pub(crate) fn import_legacy_json(transaction: &Transaction<'_>, directory: &Path) -> AppResult<Vec<PathBuf>> {
    let path = directory.join(LEGACY_FILE);
    let Some(store) = super::read_legacy_files(&path, &directory.join(LEGACY_METADATA_FILE))? else {
        return Ok(Vec::new());
    };
    save_into(transaction, &store)?;
    log::info!(
        target: "stats",
        "旧版播放统计已导入数据库: tracks={}, buckets={}",
        store.stats.len(),
        store.buckets.len(),
    );
    Ok(vec![path])
}

fn stat_from_row(row: &Row<'_>, offset: usize) -> AppResult<SyncTrackStat> {
    let provenance: String = row.get(offset + 15)?;
    Ok(SyncTrackStat {
        identity_key: row.get(offset)?,
        id: row.get(offset + 1)?,
        name: row.get(offset + 2)?,
        artist: row.get(offset + 3)?,
        album: row.get(offset + 4)?,
        album_id: row.get(offset + 5)?,
        cover_url: row.get(offset + 6)?,
        duration_ms: row.get(offset + 7)?,
        total_listen_ms: row.get(offset + 8)?,
        play_count: row.get(offset + 9)?,
        last_played_at: row.get(offset + 10)?,
        first_played_at: row.get(offset + 11)?,
        media_uri: row.get(offset + 12)?,
        counter_base_listen_ms: row.get(offset + 13)?,
        counter_base_play_count: row.get(offset + 14)?,
        counter_shards: serde_json::from_str(&provenance).map_err(|error| {
            AppError::Other(format!("Stored playback stat shards are invalid: {error}"))
        })?,
    })
}

fn bucket_from_stat(day_start_at: i64, stat: SyncTrackStat) -> SyncPlaybackStatBucket {
    SyncPlaybackStatBucket {
        day_start_at,
        identity_key: stat.identity_key,
        name: stat.name,
        artist: stat.artist,
        album: stat.album,
        total_listen_ms: stat.total_listen_ms,
        play_count: stat.play_count,
        last_played_at: stat.last_played_at,
        first_played_at: stat.first_played_at,
        cover_url: stat.cover_url,
        duration_ms: stat.duration_ms,
        media_uri: stat.media_uri,
        id: stat.id,
        album_id: stat.album_id,
        counter_base_listen_ms: stat.counter_base_listen_ms,
        counter_base_play_count: stat.counter_base_play_count,
        counter_shards: stat.counter_shards,
    }
}

fn shard_from_row(row: &Row<'_>, offset: usize) -> rusqlite::Result<SyncPlaybackCounterShard> {
    Ok(SyncPlaybackCounterShard {
        device_id: row.get(offset)?,
        epoch_started_at: row.get(offset + 1)?,
        total_listen_ms: row.get(offset + 2)?,
        play_count: row.get(offset + 3)?,
        first_played_at: row.get(offset + 4)?,
        last_played_at: row.get(offset + 5)?,
    })
}

fn hash_shards(hasher: &mut impl Hasher, shards: &[SyncPlaybackCounterShard]) {
    shards.len().hash(hasher);
    for shard in shards {
        shard.device_id.hash(hasher);
        shard.epoch_started_at.hash(hasher);
        shard.total_listen_ms.hash(hasher);
        shard.play_count.hash(hasher);
        shard.first_played_at.hash(hasher);
        shard.last_played_at.hash(hasher);
    }
}

#[allow(clippy::too_many_arguments)]
fn hash_counters(
    hasher: &mut impl Hasher,
    display: [&str; 5],
    cover_url: &Option<String>,
    media_uri: &Option<String>,
    counters: [i64; 7],
    provenance: &[SyncPlaybackCounterShard],
    own: &[SyncPlaybackCounterShard],
) {
    display.hash(hasher);
    cover_url.hash(hasher);
    media_uri.hash(hasher);
    counters.hash(hasher);
    hash_shards(hasher, provenance);
    hash_shards(hasher, own);
}

fn stat_hash(stat: &SyncTrackStat, own: &[SyncPlaybackCounterShard]) -> i64 {
    let mut hasher = std::hash::DefaultHasher::new();
    hash_counters(
        &mut hasher,
        [&stat.id, &stat.name, &stat.artist, &stat.album, &stat.album_id],
        &stat.cover_url,
        &stat.media_uri,
        [
            stat.duration_ms,
            stat.total_listen_ms,
            i64::from(stat.play_count),
            stat.last_played_at,
            stat.first_played_at,
            stat.counter_base_listen_ms,
            i64::from(stat.counter_base_play_count),
        ],
        &stat.counter_shards,
        own,
    );
    hasher.finish() as i64
}

fn bucket_hash(bucket: &SyncPlaybackStatBucket, own: &[SyncPlaybackCounterShard]) -> i64 {
    let mut hasher = std::hash::DefaultHasher::new();
    hash_counters(
        &mut hasher,
        [&bucket.id, &bucket.name, &bucket.artist, &bucket.album, &bucket.album_id],
        &bucket.cover_url,
        &bucket.media_uri,
        [
            bucket.duration_ms,
            bucket.total_listen_ms,
            i64::from(bucket.play_count),
            bucket.last_played_at,
            bucket.first_played_at,
            bucket.counter_base_listen_ms,
            i64::from(bucket.counter_base_play_count),
        ],
        &bucket.counter_shards,
        own,
    );
    hasher.finish() as i64
}
