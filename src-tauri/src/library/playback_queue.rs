// 播放队列快照：SQLite 持久化（对齐 Android playback_queue_state / playback_queue_song）
//
// 队列曲目与播放状态分开存：进度每 15 秒保存一次，只更新状态行；队列内容
// 只在摘要变化时重写，长队列不会因为进度保存而反复整表写入
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::error::AppResult;

pub(crate) const LEGACY_IMPORT_KEY: &str = "legacy_import.playback_state";

/// 前端 persistedPlayerState() 的快照；`queue` 为 None 表示队列与上次保存相同
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackStateInput {
    #[serde(default)]
    pub queue: Option<Vec<Value>>,
    #[serde(default = "no_index")]
    pub queue_index: i64,
    #[serde(default)]
    pub has_playback_session: bool,
    #[serde(default)]
    pub current_track_id: Option<String>,
    #[serde(default)]
    pub current_track_playlist_key: Option<String>,
    #[serde(default)]
    pub volume: Option<f64>,
    #[serde(default)]
    pub position_ms: Option<f64>,
    #[serde(default)]
    pub repeat_mode: Option<String>,
    #[serde(default)]
    pub shuffle_enabled: Option<bool>,
}

fn no_index() -> i64 {
    -1
}

/// 还原为与旧版 localStorage 完全相同的对象形状，前端恢复逻辑无需区分来源
pub fn load_from(connection: &Connection) -> AppResult<Option<Value>> {
    let state = connection
        .query_row(
            "SELECT current_index, current_track_id, current_track_playlist_key,
                 has_playback_session, position_ms, repeat_mode, shuffle_enabled, volume
             FROM playback_queue_state WHERE id = 0",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, Option<f64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<bool>>(6)?,
                    row.get::<_, Option<f64>>(7)?,
                ))
            },
        )
        .optional()?;
    let Some((index, track_id, playlist_key, has_session, position, repeat, shuffle, volume)) = state else {
        return Ok(None);
    };
    let queue = connection
        .prepare("SELECT track_payload_json FROM playback_queue_song ORDER BY position")?
        .query_and_then([], |row| -> AppResult<Value> {
            Ok(serde_json::from_str(&row.get::<_, String>(0)?)?)
        })?
        .collect::<AppResult<Vec<_>>>()?;

    let mut object = Map::new();
    object.insert("queue".into(), Value::Array(queue));
    object.insert("queueIndex".into(), index.into());
    object.insert("hasPlaybackSession".into(), has_session.into());
    let optional = [
        ("currentTrackId", track_id.map(Value::from)),
        ("currentTrackPlaylistKey", playlist_key.map(Value::from)),
        ("volume", volume.map(Value::from)),
        ("positionMs", position.map(Value::from)),
        ("repeatMode", repeat.map(Value::from)),
        ("shuffleEnabled", shuffle.map(Value::from)),
    ];
    for (key, value) in optional {
        if let Some(value) = value {
            object.insert(key.into(), value);
        }
    }
    Ok(Some(Value::Object(object)))
}

pub fn save_into(transaction: &Transaction<'_>, state: &PlaybackStateInput) -> AppResult<()> {
    let stored_hash: Option<i64> = transaction
        .query_row("SELECT queue_hash FROM playback_queue_state WHERE id = 0", [], |row| row.get(0))
        .optional()?;
    let mut queue_hash = stored_hash.unwrap_or(0);
    if let Some(queue) = &state.queue {
        let payloads = queue.iter().map(serde_json::to_string).collect::<Result<Vec<_>, _>>()?;
        let hash = super::playlist::payload_hash(&payloads);
        if stored_hash != Some(hash) {
            transaction.execute("DELETE FROM playback_queue_song", [])?;
            let mut insert = transaction.prepare_cached(
                "INSERT INTO playback_queue_song (position, track_id, track_payload_json) VALUES (?1, ?2, ?3)",
            )?;
            for (position, (track, payload)) in queue.iter().zip(&payloads).enumerate() {
                insert.execute(params![
                    position as i64,
                    track["id"].as_str().unwrap_or_default(),
                    payload,
                ])?;
            }
            queue_hash = hash;
        }
    }
    transaction.execute(
        "INSERT INTO playback_queue_state (id, current_index, current_track_id,
             current_track_playlist_key, has_playback_session, position_ms, repeat_mode,
             shuffle_enabled, volume, queue_hash, updated_at)
         VALUES (0, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(id) DO UPDATE SET current_index = excluded.current_index,
             current_track_id = excluded.current_track_id,
             current_track_playlist_key = excluded.current_track_playlist_key,
             has_playback_session = excluded.has_playback_session,
             position_ms = excluded.position_ms, repeat_mode = excluded.repeat_mode,
             shuffle_enabled = excluded.shuffle_enabled, volume = excluded.volume,
             queue_hash = excluded.queue_hash, updated_at = excluded.updated_at",
        params![
            state.queue_index,
            state.current_track_id,
            state.current_track_playlist_key,
            state.has_playback_session,
            state.position_ms.filter(|position| position.is_finite()).map(|position| position.max(0.0)),
            state.repeat_mode,
            state.shuffle_enabled,
            state.volume.filter(|volume| volume.is_finite()),
            queue_hash,
            chrono::Utc::now().timestamp_millis(),
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::UserDatabase;
    use serde_json::json;

    fn input(value: Value) -> PlaybackStateInput {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn state_round_trips_to_the_legacy_object_shape() {
        let database = UserDatabase::open_in_memory().unwrap();
        assert_eq!(database.read(load_from).unwrap(), None);
        let state = json!({
            "queue": [{"id": "netease:1", "title": "A"}, {"id": "netease:2", "title": "B"}],
            "queueIndex": 1,
            "hasPlaybackSession": true,
            "currentTrackId": "netease:2",
            "volume": 0.5,
            "positionMs": 1234.5,
            "repeatMode": "all",
            "shuffleEnabled": false
        });
        database.write(|transaction| save_into(transaction, &input(state.clone()))).unwrap();
        assert_eq!(database.read(load_from).unwrap(), Some(state));
    }

    #[test]
    fn progress_saves_keep_the_stored_queue() {
        let database = UserDatabase::open_in_memory().unwrap();
        database
            .write(|transaction| {
                save_into(transaction, &input(json!({"queue": [{"id": "a"}], "queueIndex": 0, "hasPlaybackSession": true})))
            })
            .unwrap();
        database
            .write(|transaction| {
                save_into(transaction, &input(json!({"queue": null, "queueIndex": 0, "hasPlaybackSession": true, "positionMs": 9000})))
            })
            .unwrap();
        let restored = database.read(load_from).unwrap().unwrap();
        assert_eq!(restored["queue"], json!([{"id": "a"}]));
        assert_eq!(restored["positionMs"], json!(9000.0));
        assert!(restored.get("currentTrackId").is_none());
    }
}
