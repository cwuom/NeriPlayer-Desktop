// 播放历史：SQLite 持久化（对齐 Android play_history）
//
// 「最近播放」按曲目身份去重，最新的排在最前，和 Android 一样不设条数上限；
// 删除记录单独保留，同步时据此把删除传播到其它设备。曲目载荷以前端 TrackInfo 原样保存
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AppResult;

pub(crate) const LEGACY_IMPORT_KEY: &str = "legacy_import.play_history";
pub(crate) const IDENTITY_KEY_MIGRATION: &str = "migration.play_history_identity";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub track: Value,
    pub played_at: i64,
    /// 长音频续播位置；None 表示本机还不知道（升级前的条目）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_position_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryDeletion {
    pub track: Value,
    pub deleted_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayHistory {
    #[serde(default)]
    pub entries: Vec<HistoryEntry>,
    #[serde(default)]
    pub deletions: Vec<HistoryDeletion>,
}

pub fn load_from(connection: &Connection) -> AppResult<PlayHistory> {
    let entries = connection
        .prepare(
            "SELECT track_payload_json, played_at, resume_position_ms FROM play_history
             ORDER BY played_at DESC, rowid DESC",
        )?
        .query_and_then([], |row| -> AppResult<HistoryEntry> {
            Ok(HistoryEntry {
                track: serde_json::from_str(&row.get::<_, String>(0)?)?,
                played_at: row.get(1)?,
                resume_position_ms: row.get(2)?,
            })
        })?
        .collect::<AppResult<Vec<_>>>()?;
    let deletions = connection
        .prepare(
            "SELECT track_payload_json, deleted_at FROM play_history_deletion
             ORDER BY deleted_at DESC, rowid DESC",
        )?
        .query_and_then([], |row| -> AppResult<HistoryDeletion> {
            Ok(HistoryDeletion {
                track: serde_json::from_str(&row.get::<_, String>(0)?)?,
                deleted_at: row.get(1)?,
            })
        })?
        .collect::<AppResult<Vec<_>>>()?;
    Ok(PlayHistory { entries, deletions })
}

/// 历史条目的身份：曲目 id；同一个 B 站视频的不同分 P 再按 cid 区分
///
/// 前端 `historyEntryKey` 必须给出相同的结果
pub fn identity_key(track: &Value) -> Option<String> {
    let id = track_id(track)?;
    match bilibili_page(track, &id) {
        Some(cid) => Some(format!("{id}#{cid}")),
        None => Some(id),
    }
}

/// 记录一次播放：同一曲目只保留最新一条，并撤销它的删除记录
///
/// 没给续播位置时保留原条目的位置（对齐 Android record 只合并曲目信息）
pub fn record(
    transaction: &Transaction<'_>,
    track: &Value,
    played_at: i64,
    resume_position_ms: Option<i64>,
) -> AppResult<bool> {
    let Some(key) = identity_key(track) else { return Ok(false) };
    if played_at <= 0 {
        return Ok(false);
    }
    let resume_position_ms = match resume_position_ms {
        Some(position) => Some(position.max(0)),
        None => transaction
            .query_row(
                "SELECT resume_position_ms FROM play_history WHERE identity_key = ?1",
                [&key],
                |row| row.get::<_, Option<i64>>(0),
            )
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                error => Err(error),
            })?,
    };
    transaction.execute("DELETE FROM play_history_deletion WHERE identity_key = ?1", [&key])?;
    insert_entry(transaction, &key, track, played_at, resume_position_ms)?;
    Ok(true)
}

/// 删除单条历史，并留下删除记录供同步传播
pub fn remove(transaction: &Transaction<'_>, key: &str, deleted_at: i64) -> AppResult<bool> {
    let payload: Option<String> = transaction
        .query_row(
            "SELECT track_payload_json FROM play_history WHERE identity_key = ?1",
            [key],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            error => Err(error),
        })?;
    let Some(payload) = payload else { return Ok(false) };
    transaction.execute("DELETE FROM play_history WHERE identity_key = ?1", [key])?;
    transaction.execute(
        "INSERT OR REPLACE INTO play_history_deletion (identity_key, deleted_at, track_payload_json)
         VALUES (?1, ?2, ?3)",
        params![key, deleted_at.max(0), payload],
    )?;
    Ok(true)
}

/// 清空历史：现有条目全部转成同一时间的删除记录
pub fn clear(transaction: &Transaction<'_>, deleted_at: i64) -> AppResult<bool> {
    let changed = transaction.execute(
        "INSERT OR REPLACE INTO play_history_deletion (identity_key, deleted_at, track_payload_json)
         SELECT identity_key, ?1, track_payload_json FROM play_history",
        [deleted_at.max(0)],
    )?;
    transaction.execute("DELETE FROM play_history", [])?;
    Ok(changed > 0)
}

/// 用同步结果整体替换历史
pub fn replace(transaction: &Transaction<'_>, history: &PlayHistory) -> AppResult<()> {
    transaction.execute("DELETE FROM play_history", [])?;
    transaction.execute("DELETE FROM play_history_deletion", [])?;
    // 列表按新到旧给出，逆序插入：同一身份以最新的一条为准，相同时间戳时仍保持原相对顺序
    for entry in history.entries.iter().rev() {
        if let Some(key) = identity_key(&entry.track) {
            if entry.played_at > 0 {
                insert_entry(transaction, &key, &entry.track, entry.played_at, entry.resume_position_ms)?;
            }
        }
    }
    for deletion in history.deletions.iter().rev() {
        if let Some(key) = identity_key(&deletion.track) {
            if deletion.deleted_at > 0 {
                transaction.execute(
                    "INSERT OR REPLACE INTO play_history_deletion (identity_key, deleted_at, track_payload_json)
                     VALUES (?1, ?2, ?3)",
                    params![key, deletion.deleted_at, serde_json::to_string(&deletion.track)?],
                )?;
            }
        }
    }
    Ok(())
}

/// 一次性迁移：旧库以曲目 id 为键，B 站分 P 记录改用带 cid 的身份
pub(crate) fn adopt_identity_keys_once(transaction: &Transaction<'_>, _directory: &Path) -> AppResult<Vec<PathBuf>> {
    for table in ["play_history", "play_history_deletion"] {
        let rows = transaction
            .prepare(&format!("SELECT identity_key, track_payload_json FROM {table}"))?
            .query_and_then([], |row| -> AppResult<(String, Value)> {
                Ok((row.get(0)?, serde_json::from_str(&row.get::<_, String>(1)?)?))
            })?
            .collect::<AppResult<Vec<_>>>()?;
        for (stored, track) in rows {
            match identity_key(&track) {
                Some(key) if key != stored => {
                    transaction.execute(
                        &format!("UPDATE OR REPLACE {table} SET identity_key = ?1 WHERE identity_key = ?2"),
                        [&key, &stored],
                    )?;
                }
                _ => {}
            }
        }
    }
    Ok(Vec::new())
}

fn insert_entry(
    transaction: &Transaction<'_>,
    key: &str,
    track: &Value,
    played_at: i64,
    resume_position_ms: Option<i64>,
) -> AppResult<()> {
    transaction.execute("DELETE FROM play_history WHERE identity_key = ?1", [key])?;
    transaction.execute(
        "INSERT INTO play_history (identity_key, played_at, resume_position_ms, name, artist, album,
             duration_ms, cover_url, source, track_payload_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            key,
            played_at,
            resume_position_ms.map(|position| position.max(0)),
            text(track, &["title"]).unwrap_or_default(),
            text(track, &["artist"]).unwrap_or_default(),
            text(track, &["album"]).unwrap_or_default(),
            number(track, &["durationMs", "duration_ms"]),
            text(track, &["coverUrl", "cover_url"]).filter(|url| !url.is_empty()),
            text(track, &["source"]),
            serde_json::to_string(track)?,
        ],
    )?;
    Ok(())
}

fn track_id(track: &Value) -> Option<String> {
    match &track["id"] {
        Value::String(id) if !id.is_empty() => Some(id.clone()),
        Value::Number(id) => Some(id.to_string()),
        _ => None,
    }
}

/// 与播放取流选分 P 的规则一致：先看同步载荷的 subAudioId，再看 `Bilibili|<cid>` 专辑标记
fn bilibili_page(track: &Value, id: &str) -> Option<String> {
    if !id.starts_with("bilibili:") {
        return None;
    }
    let from_payload = ["syncPayload", "sync_payload"]
        .into_iter()
        .map(|key| &track[key])
        .filter(|payload| payload.is_object())
        .flat_map(|payload| ["subAudioId", "sub_audio_id"].into_iter().map(move |key| &payload[key]))
        .find_map(|value| match value {
            Value::String(cid) if !cid.trim().is_empty() => Some(cid.trim().to_string()),
            Value::Number(cid) => Some(cid.as_i64().map_or_else(|| cid.to_string(), |cid| cid.to_string())),
            _ => None,
        });
    from_payload.or_else(|| {
        let album = text(track, &["album"])?;
        let marker = album.get(..9).filter(|prefix| prefix.eq_ignore_ascii_case("bilibili|"))?;
        let cid: String = album[marker.len()..].chars().take_while(char::is_ascii_digit).collect();
        (!cid.is_empty()).then_some(cid)
    })
}

fn text(track: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| track[*key].as_str().map(str::to_string))
}

fn number(track: &Value, keys: &[&str]) -> i64 {
    keys.iter()
        .find_map(|key| track[*key].as_f64())
        .map(|value| value.max(0.0).round() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::UserDatabase;
    use serde_json::json;

    fn track(id: &str) -> Value {
        json!({"id": id, "title": format!("Song {id}"), "artist": "A", "album": "B", "durationMs": 1000, "coverUrl": "", "source": "netease"})
    }

    fn page(cid: &str) -> Value {
        json!({"id": "bilibili:BV1xx", "title": format!("P {cid}"), "artist": "U", "album": format!("Bilibili|{cid}"), "durationMs": 1000, "source": "bilibili"})
    }

    #[test]
    fn record_deduplicates_and_orders_newest_first() {
        let database = UserDatabase::open_in_memory().unwrap();
        database
            .write(|transaction| {
                record(transaction, &track("netease:1"), 10, None)?;
                record(transaction, &track("netease:2"), 20, None)?;
                record(transaction, &track("netease:1"), 30, None)?;
                Ok(())
            })
            .unwrap();
        let history = database.read(load_from).unwrap();
        let ids: Vec<&str> = history.entries.iter().map(|entry| entry.track["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["netease:1", "netease:2"]);
        assert_eq!(history.entries[0].played_at, 30);
        assert_eq!(history.entries[0].track, track("netease:1"));
    }

    #[test]
    fn bilibili_pages_of_one_video_are_separate_entries() {
        let database = UserDatabase::open_in_memory().unwrap();
        let mut payload_page = page("0");
        payload_page["album"] = json!("");
        payload_page["syncPayload"] = json!({"subAudioId": 3});
        database
            .write(|transaction| {
                record(transaction, &page("1"), 10, None)?;
                record(transaction, &page("2"), 20, None)?;
                record(transaction, &payload_page, 30, None)?;
                record(transaction, &page("1"), 40, None)?;
                assert!(remove(transaction, "bilibili:BV1xx#2", 50)?);
                Ok(())
            })
            .unwrap();
        let history = database.read(load_from).unwrap();
        let titles: Vec<&str> = history.entries.iter().map(|entry| entry.track["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["P 1", "P 0"]);
        assert_eq!(history.deletions.len(), 1);
        assert_eq!(history.deletions[0].track["title"], "P 2");
        assert_eq!(identity_key(&payload_page).as_deref(), Some("bilibili:BV1xx#3"));
        assert_eq!(identity_key(&json!({"id": "bilibili:BV1xx", "album": ""})).as_deref(), Some("bilibili:BV1xx"));
        assert_eq!(identity_key(&json!({"id": "netease:1", "album": "Bilibili|9"})).as_deref(), Some("netease:1"));
    }

    #[test]
    fn remove_and_clear_leave_deletions_and_record_revives() {
        let database = UserDatabase::open_in_memory().unwrap();
        database
            .write(|transaction| {
                record(transaction, &track("a"), 10, None)?;
                record(transaction, &track("b"), 20, None)?;
                assert!(remove(transaction, "a", 30)?);
                assert!(!remove(transaction, "missing", 31)?);
                assert!(clear(transaction, 40)?);
                record(transaction, &track("b"), 50, None)?;
                Ok(())
            })
            .unwrap();
        let history = database.read(load_from).unwrap();
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].track["id"], "b");
        assert_eq!(history.deletions.len(), 1);
        assert_eq!(history.deletions[0].track["id"], "a");
        assert_eq!(history.deletions[0].deleted_at, 30);
    }

    #[test]
    fn history_and_deletions_are_not_capped() {
        let database = UserDatabase::open_in_memory().unwrap();
        let entries: Vec<HistoryEntry> = (0..1502)
            .map(|index| HistoryEntry { track: track(&format!("t{index}")), played_at: 5000 - index, resume_position_ms: None })
            .collect();
        let history = PlayHistory {
            entries,
            deletions: vec![HistoryDeletion { track: track("gone"), deleted_at: 5 }],
        };
        database.write(|transaction| replace(transaction, &history)).unwrap();
        let restored = database.read(load_from).unwrap();
        assert_eq!(restored.entries.len(), 1502);
        assert_eq!(restored.entries[0].track["id"], "t0");
        assert_eq!(restored.deletions.len(), 1);

        database.write(|transaction| clear(transaction, 6000).map(|_| ())).unwrap();
        let cleared = database.read(load_from).unwrap();
        assert!(cleared.entries.is_empty());
        assert_eq!(cleared.deletions.len(), 1503, "every cleared entry keeps its tombstone");
    }

    #[test]
    fn replace_keeps_the_newest_entry_per_identity() {
        let database = UserDatabase::open_in_memory().unwrap();
        let mut older = track("netease:1");
        older["title"] = json!("old");
        let history = PlayHistory {
            entries: vec![
                HistoryEntry { track: track("netease:1"), played_at: 30, resume_position_ms: None },
                HistoryEntry { track: page("1"), played_at: 20, resume_position_ms: Some(7) },
                HistoryEntry { track: older, played_at: 10, resume_position_ms: None },
                HistoryEntry { track: page("2"), played_at: 5, resume_position_ms: None },
            ],
            deletions: Vec::new(),
        };
        database.write(|transaction| replace(transaction, &history)).unwrap();
        let restored = database.read(load_from).unwrap();
        let titles: Vec<&str> = restored.entries.iter().map(|entry| entry.track["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["Song netease:1", "P 1", "P 2"]);
        assert_eq!(restored.entries[1].resume_position_ms, Some(7));
        assert_eq!(restored.entries[0].resume_position_ms, None);
    }

    #[test]
    fn record_keeps_the_resume_position_unless_given_one() {
        let database = UserDatabase::open_in_memory().unwrap();
        database
            .write(|transaction| {
                record(transaction, &track("netease:1"), 10, None)?;
                record(transaction, &track("netease:2"), 15, Some(1_000))?;
                record(transaction, &track("netease:2"), 20, None)?;
                Ok(())
            })
            .unwrap();
        let history = database.read(load_from).unwrap();
        assert_eq!(history.entries[0].resume_position_ms, Some(1_000), "replaying keeps the remembered position");
        assert_eq!(history.entries[1].resume_position_ms, None, "an entry without a known position stays unknown");

        database.write(|transaction| record(transaction, &track("netease:2"), 30, Some(0)).map(|_| ())).unwrap();
        assert_eq!(database.read(load_from).unwrap().entries[0].resume_position_ms, Some(0));
    }

    #[test]
    fn identity_migration_rekeys_bilibili_pages_once() {
        let database = UserDatabase::open_in_memory().unwrap();
        database
            .write(|transaction| {
                for (key, payload, played_at) in [("bilibili:BV1xx", page("7"), 10), ("netease:1", track("netease:1"), 20)] {
                    transaction.execute(
                        "INSERT INTO play_history (identity_key, played_at, name, artist, album, duration_ms, track_payload_json)
                         VALUES (?1, ?2, '', '', '', 0, ?3)",
                        params![key, played_at, payload.to_string()],
                    )?;
                }
                transaction.execute(
                    "INSERT INTO play_history_deletion (identity_key, deleted_at, track_payload_json) VALUES (?1, ?2, ?3)",
                    params!["bilibili:BV1xx", 30, page("8").to_string()],
                )?;
                Ok(())
            })
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        assert!(crate::db::legacy::run_once(&database, directory.path(), IDENTITY_KEY_MIGRATION, adopt_identity_keys_once).unwrap());
        database
            .write(|transaction| {
                record(transaction, &page("7"), 40, None)?;
                assert!(remove(transaction, "netease:1", 50)?);
                Ok(())
            })
            .unwrap();
        let history = database.read(load_from).unwrap();
        assert_eq!(history.entries.len(), 1, "replaying a migrated page updates its entry instead of adding one");
        assert_eq!(history.entries[0].played_at, 40);
        let keys: Vec<String> = database
            .read(|connection| {
                Ok(connection
                    .prepare("SELECT identity_key FROM play_history_deletion ORDER BY deleted_at")?
                    .query_map([], |row| row.get(0))?
                    .collect::<Result<Vec<String>, _>>()?)
            })
            .unwrap();
        assert_eq!(keys, ["bilibili:BV1xx#8", "netease:1"]);
        assert!(!crate::db::legacy::run_once(&database, directory.path(), IDENTITY_KEY_MIGRATION, adopt_identity_keys_once).unwrap());
    }
}
