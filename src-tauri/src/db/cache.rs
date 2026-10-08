//! 平台详情与歌词缓存（对齐 Android platform_playlist_cache 的 Room 缓存）
//!
//! 取代 localStorage 缓存桶：不再受 5 MB 配额限制，按「过期时间 + 条数 + 字节」
//! 裁剪，写入失败不会让整桶永久失效
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::Value;

use crate::error::{AppError, AppResult};

/// 歌单、专辑与创作者详情
pub const PLATFORM_DETAIL_BUCKET: &str = "platform_detail";
/// 解析后的歌词行
pub const LYRICS_BUCKET: &str = "lyrics";
/// 音频缓存按首选音质建键；这里记下该键实际播放的音质，缓存命中时据此展示
pub const PLAYBACK_QUALITY_BUCKET: &str = "playback_quality";

const ALLOWED_BUCKETS: &[&str] = &[PLATFORM_DETAIL_BUCKET, LYRICS_BUCKET, PLAYBACK_QUALITY_BUCKET];
/// 单桶字节上限的硬顶，防止前端传入异常值让缓存无限增长
const MAX_BUCKET_BYTES: i64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct CachePolicy {
    pub max_age_ms: i64,
    pub max_entries: i64,
    pub max_bytes: i64,
}

pub fn validate_bucket(bucket: &str) -> AppResult<()> {
    if ALLOWED_BUCKETS.contains(&bucket) {
        Ok(())
    } else {
        Err(AppError::Other(format!("Unknown cache bucket: {bucket}")))
    }
}

/// 读取缓存；过期条目视为未命中
pub fn get(connection: &Connection, bucket: &str, key: &str, max_age_ms: i64, now: i64) -> AppResult<Option<Value>> {
    let row: Option<(String, i64)> = connection
        .query_row(
            "SELECT payload_json, saved_at_ms FROM cache_entry WHERE bucket = ?1 AND cache_key = ?2",
            params![bucket, key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((payload, saved_at)) = row else { return Ok(None) };
    if now.saturating_sub(saved_at) > max_age_ms.max(0) {
        return Ok(None);
    }
    // 缓存内容损坏按未命中处理，下一次写入会覆盖
    Ok(serde_json::from_str(&payload).ok())
}

pub fn put(
    transaction: &Transaction<'_>,
    bucket: &str,
    key: &str,
    value: &Value,
    policy: CachePolicy,
    now: i64,
) -> AppResult<()> {
    let payload = serde_json::to_string(value)?;
    let max_bytes = policy.max_bytes.clamp(1, MAX_BUCKET_BYTES);
    if payload.len() as i64 > max_bytes {
        transaction.execute(
            "DELETE FROM cache_entry WHERE bucket = ?1 AND cache_key = ?2",
            params![bucket, key],
        )?;
        return Ok(());
    }
    transaction.execute(
        "INSERT INTO cache_entry (bucket, cache_key, payload_json, payload_bytes, saved_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(bucket, cache_key) DO UPDATE SET payload_json = excluded.payload_json,
             payload_bytes = excluded.payload_bytes, saved_at_ms = excluded.saved_at_ms",
        params![bucket, key, payload, payload.len() as i64, now],
    )?;
    trim(transaction, bucket, policy.max_age_ms, policy.max_entries.max(1), max_bytes, now)
}

pub fn remove(transaction: &Transaction<'_>, bucket: &str, key: &str) -> AppResult<()> {
    transaction.execute(
        "DELETE FROM cache_entry WHERE bucket = ?1 AND cache_key = ?2",
        params![bucket, key],
    )?;
    Ok(())
}

/// 清空指定缓存桶，返回 (释放的字节, 删除的条目)
pub fn clear(transaction: &Transaction<'_>, bucket: &str) -> AppResult<(u64, u64)> {
    let usage = usage(transaction, bucket)?;
    transaction.execute("DELETE FROM cache_entry WHERE bucket = ?1", [bucket])?;
    Ok(usage)
}

/// 缓存桶的载荷字节与条目数，供存储管理页归属数据库空间
pub fn usage(connection: &Connection, bucket: &str) -> AppResult<(u64, u64)> {
    let (bytes, count): (i64, i64) = connection.query_row(
        "SELECT COALESCE(SUM(payload_bytes + length(cache_key)), 0), COUNT(*)
         FROM cache_entry WHERE bucket = ?1",
        [bucket],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok((bytes.max(0) as u64, count.max(0) as u64))
}

fn trim(
    transaction: &Transaction<'_>,
    bucket: &str,
    max_age_ms: i64,
    max_entries: i64,
    max_bytes: i64,
    now: i64,
) -> AppResult<()> {
    transaction.execute(
        "DELETE FROM cache_entry WHERE bucket = ?1 AND saved_at_ms < ?2",
        params![bucket, now.saturating_sub(max_age_ms.max(0))],
    )?;
    transaction.execute(
        "DELETE FROM cache_entry WHERE bucket = ?1 AND cache_key NOT IN (
             SELECT cache_key FROM cache_entry WHERE bucket = ?1
             ORDER BY saved_at_ms DESC, cache_key LIMIT ?2)",
        params![bucket, max_entries],
    )?;
    // 字节预算按新到旧累计，超出部分从最旧开始淘汰
    let mut statement = transaction.prepare(
        "SELECT cache_key, payload_bytes FROM cache_entry WHERE bucket = ?1
         ORDER BY saved_at_ms DESC, cache_key",
    )?;
    let rows = statement
        .query_map([bucket], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut used = 0_i64;
    let mut evicted = Vec::new();
    for (key, bytes) in rows {
        used = used.saturating_add(bytes);
        if used > max_bytes {
            evicted.push(key);
        }
    }
    for key in evicted {
        remove(transaction, bucket, &key)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::UserDatabase;
    use serde_json::json;

    const POLICY: CachePolicy = CachePolicy { max_age_ms: 1_000, max_entries: 2, max_bytes: 10_000 };

    #[test]
    fn entries_expire_and_are_trimmed_by_count() {
        let database = UserDatabase::open_in_memory().unwrap();
        database
            .write(|transaction| {
                put(transaction, LYRICS_BUCKET, "a", &json!([1]), POLICY, 100)?;
                put(transaction, LYRICS_BUCKET, "b", &json!([2]), POLICY, 200)?;
                put(transaction, LYRICS_BUCKET, "c", &json!([3]), POLICY, 300)
            })
            .unwrap();
        database
            .read(|connection| {
                assert_eq!(get(connection, LYRICS_BUCKET, "a", 1_000, 300)?, None, "count limit evicts the oldest");
                assert_eq!(get(connection, LYRICS_BUCKET, "c", 1_000, 300)?, Some(json!([3])));
                assert_eq!(get(connection, LYRICS_BUCKET, "b", 50, 300)?, None, "expired entries miss");
                assert_eq!(usage(connection, LYRICS_BUCKET)?.1, 2);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn byte_budget_evicts_oldest_and_rejects_oversized_values() {
        let database = UserDatabase::open_in_memory().unwrap();
        let policy = CachePolicy { max_age_ms: 10_000, max_entries: 10, max_bytes: 30 };
        let big = json!("x".repeat(15));
        database
            .write(|transaction| {
                put(transaction, PLATFORM_DETAIL_BUCKET, "old", &big, policy, 1)?;
                put(transaction, PLATFORM_DETAIL_BUCKET, "new", &big, policy, 2)?;
                put(transaction, PLATFORM_DETAIL_BUCKET, "huge", &json!("y".repeat(64)), policy, 3)
            })
            .unwrap();
        database
            .read(|connection| {
                assert_eq!(get(connection, PLATFORM_DETAIL_BUCKET, "old", 10_000, 3)?, None);
                assert!(get(connection, PLATFORM_DETAIL_BUCKET, "new", 10_000, 3)?.is_some());
                assert_eq!(get(connection, PLATFORM_DETAIL_BUCKET, "huge", 10_000, 3)?, None);
                Ok(())
            })
            .unwrap();
        let (_, removed) = database.write(|transaction| clear(transaction, PLATFORM_DETAIL_BUCKET)).unwrap();
        assert_eq!(removed, 1);
        assert!(validate_bucket("arbitrary").is_err());
    }
}
