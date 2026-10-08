// 前端详情与歌词缓存的数据库命令
use serde_json::Value;

use crate::db::{self, cache};
use crate::error::{AppError, AppResult};

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| AppError::Other(error.to_string()))?
}

#[tauri::command]
pub async fn cache_get(bucket: String, key: String, max_age_ms: i64) -> AppResult<Option<Value>> {
    cache::validate_bucket(&bucket)?;
    blocking(move || {
        db::user_db()?.read(|connection| cache::get(connection, &bucket, &key, max_age_ms, now_ms()))
    })
    .await
}

#[tauri::command]
pub async fn cache_put(
    bucket: String,
    key: String,
    value: Value,
    max_age_ms: i64,
    max_entries: i64,
    max_bytes: i64,
) -> AppResult<()> {
    cache::validate_bucket(&bucket)?;
    let policy = cache::CachePolicy { max_age_ms, max_entries, max_bytes };
    blocking(move || {
        db::user_db()?.write(|transaction| cache::put(transaction, &bucket, &key, &value, policy, now_ms()))
    })
    .await
}

#[tauri::command]
pub async fn cache_remove(bucket: String, key: String) -> AppResult<()> {
    cache::validate_bucket(&bucket)?;
    blocking(move || db::user_db()?.write(|transaction| cache::remove(transaction, &bucket, &key))).await
}
