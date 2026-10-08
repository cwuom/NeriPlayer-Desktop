//! 键值元数据：`migration_metadata` 存迁移标记与计数器，`sync_metadata`
//! 存同步协议需要整体读写的 JSON 文档（归档扩展、云端历史快照等）

use rusqlite::{Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::{AppError, AppResult};

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn get(connection: &Connection, key: &str) -> AppResult<Option<String>> {
    Ok(connection
        .query_row(
            "SELECT value FROM migration_metadata WHERE key = ?1",
            [key],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
}

pub fn set(connection: &Connection, key: &str, value: &str) -> AppResult<()> {
    connection.execute(
        "INSERT INTO migration_metadata (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        rusqlite::params![key, value, now_ms()],
    )?;
    Ok(())
}

pub fn get_i64(connection: &Connection, key: &str) -> AppResult<Option<i64>> {
    get(connection, key)?
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|_| AppError::Other(format!("Metadata {key} is not an integer: {value}")))
        })
        .transpose()
}

pub fn set_i64(connection: &Connection, key: &str, value: i64) -> AppResult<()> {
    set(connection, key, &value.to_string())
}

pub fn is_flag_set(connection: &Connection, key: &str) -> AppResult<bool> {
    Ok(get(connection, key)?.is_some())
}

pub fn get_document<T: DeserializeOwned>(connection: &Connection, key: &str) -> AppResult<Option<T>> {
    let raw: Option<String> = connection
        .query_row(
            "SELECT value_json FROM sync_metadata WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()?;
    raw.map(|value| {
        serde_json::from_str(&value)
            .map_err(|error| AppError::Other(format!("Stored document {key} is invalid: {error}")))
    })
    .transpose()
}

pub fn set_document<T: Serialize + ?Sized>(connection: &Connection, key: &str, value: &T) -> AppResult<()> {
    connection.execute(
        "INSERT INTO sync_metadata (key, value_json, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
        rusqlite::params![key, serde_json::to_string(value)?, now_ms()],
    )?;
    Ok(())
}

pub fn remove_document(connection: &Connection, key: &str) -> AppResult<()> {
    connection.execute("DELETE FROM sync_metadata WHERE key = ?1", [key])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::UserDatabase;

    #[test]
    fn metadata_round_trips_text_integers_and_documents() {
        let database = UserDatabase::open_in_memory().unwrap();
        database
            .write(|transaction| {
                set(transaction, "text", "value")?;
                set_i64(transaction, "number", -42)?;
                set_document(transaction, "doc", &serde_json::json!({"a": [1, 2]}))
            })
            .unwrap();
        database
            .read(|connection| {
                assert_eq!(get(connection, "text")?.as_deref(), Some("value"));
                assert_eq!(get_i64(connection, "number")?, Some(-42));
                assert_eq!(
                    get_document::<serde_json::Value>(connection, "doc")?,
                    Some(serde_json::json!({"a": [1, 2]}))
                );
                assert!(is_flag_set(connection, "text")?);
                assert!(!is_flag_set(connection, "missing")?);
                Ok(())
            })
            .unwrap();
    }
}
