// 逐曲歌词偏移（用户 delta）：SQLite 持久化
//
// 只保存手动调整过的曲目，delta 归零即删除记录；有效偏移 = 来源默认偏移 + delta
use std::collections::{BTreeMap, HashMap};

use rusqlite::{params, Connection, Transaction};

use crate::error::AppResult;

pub(crate) const LEGACY_IMPORT_KEY: &str = "legacy_import.lyric_offsets";

pub fn load_from(connection: &Connection) -> AppResult<BTreeMap<String, i64>> {
    Ok(connection
        .prepare("SELECT track_key, offset_ms FROM lyric_offset")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?)
}

pub fn set(transaction: &Transaction<'_>, track_key: &str, offset_ms: i64) -> AppResult<()> {
    if track_key.is_empty() {
        return Ok(());
    }
    if offset_ms == 0 {
        transaction.execute("DELETE FROM lyric_offset WHERE track_key = ?1", [track_key])?;
        return Ok(());
    }
    transaction.execute(
        "INSERT INTO lyric_offset (track_key, offset_ms, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(track_key) DO UPDATE SET offset_ms = excluded.offset_ms, updated_at = excluded.updated_at",
        params![track_key, offset_ms, chrono::Utc::now().timestamp_millis()],
    )?;
    Ok(())
}

/// 默认偏移变化后整体 rebase 时使用
pub fn replace(transaction: &Transaction<'_>, offsets: &BTreeMap<String, i64>) -> AppResult<()> {
    transaction.execute("DELETE FROM lyric_offset", [])?;
    for (key, offset) in offsets {
        set(transaction, key, *offset)?;
    }
    Ok(())
}

/// 同步写回时用合并后歌单里的逐曲偏移校正本地记录，有改动时返回新的完整映射
///
/// `synced` 是每首曲目在各歌单副本里出现过的非 0 偏移。合并时 0 表示没有信息、不会盖掉
/// 非 0 值，所以只采纳非 0 值；还有副本保留本地值说明本机的修改在那里胜出，保持不动
pub fn reconcile_with_synced(
    transaction: &Transaction<'_>,
    synced: &HashMap<String, Vec<i64>>,
) -> AppResult<Option<BTreeMap<String, i64>>> {
    let mut changed = false;
    for (key, offset) in load_from(transaction)? {
        let Some(values) = synced.get(&key) else { continue };
        if values.contains(&offset) {
            continue;
        }
        if let Some(&adopted) = values.first() {
            set(transaction, &key, adopted)?;
            changed = true;
        }
    }
    if changed {
        Ok(Some(load_from(transaction)?))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::UserDatabase;

    #[test]
    fn zero_offsets_are_not_stored() {
        let database = UserDatabase::open_in_memory().unwrap();
        database
            .write(|transaction| {
                set(transaction, "netease:1", 300)?;
                set(transaction, "netease:2", -50)?;
                set(transaction, "netease:2", 0)?;
                Ok(())
            })
            .unwrap();
        assert_eq!(
            database.read(load_from).unwrap(),
            BTreeMap::from([("netease:1".to_string(), 300)])
        );
        database
            .write(|transaction| replace(transaction, &BTreeMap::from([("qq:1".into(), 10), ("qq:2".into(), 0)])))
            .unwrap();
        assert_eq!(database.read(load_from).unwrap(), BTreeMap::from([("qq:1".to_string(), 10)]));
    }

    #[test]
    fn synced_offsets_replace_stale_local_values_only_when_another_device_won() {
        let database = UserDatabase::open_in_memory().unwrap();
        database
            .write(|transaction| {
                set(transaction, "netease:changed", 300)?;
                set(transaction, "netease:kept", 300)?;
                set(transaction, "netease:not-synced", 300)?;
                Ok(())
            })
            .unwrap();
        let synced = HashMap::from([
            ("netease:changed".to_string(), vec![-200]),
            ("netease:kept".to_string(), vec![-200, 300]),
            ("netease:only-remote".to_string(), vec![500]),
        ]);
        let reconciled = database.write(|transaction| reconcile_with_synced(transaction, &synced)).unwrap();
        let expected = BTreeMap::from([
            ("netease:changed".to_string(), -200),
            ("netease:kept".to_string(), 300),
            ("netease:not-synced".to_string(), 300),
        ]);
        assert_eq!(reconciled, Some(expected.clone()));
        assert_eq!(database.read(load_from).unwrap(), expected);
        assert_eq!(database.write(|transaction| reconcile_with_synced(transaction, &synced)).unwrap(), None);
    }
}
