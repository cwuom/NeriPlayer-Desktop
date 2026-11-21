// 前端用户状态（播放历史、播放队列、逐曲歌词偏移）的数据库命令
//
// 启动时前端一次性拉取快照再创建 store，恢复保持同步语义；旧版 localStorage
// 数据由 import_legacy_user_data 按数据域各导入一次
use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

use crate::state::AppState;
use neri_common::error::{AppError, AppResult};
use neri_library::db::{self, meta, UserDatabase};
use neri_library::library::{bili_video_skip, lyric_offsets, play_history, playback_queue};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMigrationState {
    pub playback_state: bool,
    pub history: bool,
    pub lyric_offsets: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserDataSnapshot {
    pub playback_state: Option<Value>,
    pub history: play_history::PlayHistory,
    pub lyric_offsets: BTreeMap<String, i64>,
    pub bili_video_skip_rules: Vec<bili_video_skip::BiliVideoSkipRule>,
    pub migrated: LegacyMigrationState,
}

fn snapshot_from(database: &UserDatabase) -> AppResult<UserDataSnapshot> {
    database.read(|connection| {
        Ok(UserDataSnapshot {
            playback_state: playback_queue::load_from(connection)?,
            history: play_history::load_from(connection)?,
            lyric_offsets: lyric_offsets::load_from(connection)?,
            bili_video_skip_rules: bili_video_skip::load_from(connection)?,
            migrated: LegacyMigrationState {
                playback_state: meta::is_flag_set(connection, playback_queue::LEGACY_IMPORT_KEY)?,
                history: meta::is_flag_set(connection, play_history::LEGACY_IMPORT_KEY)?,
                lyric_offsets: meta::is_flag_set(connection, lyric_offsets::LEGACY_IMPORT_KEY)?,
            },
        })
    })
}

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce(&'static UserDatabase) -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tokio::task::spawn_blocking(move || operation(db::user_db()?))
        .await
        .map_err(|error| AppError::Other(error.to_string()))?
}

#[tauri::command]
pub async fn load_user_data_snapshot() -> AppResult<UserDataSnapshot> {
    blocking(snapshot_from).await
}

/// 导入旧版 localStorage 数据；没有旧数据的数据域同样写入迁移标记，
/// 之后残留或回滚产生的旧数据都不会再覆盖数据库
#[tauri::command]
pub async fn import_legacy_user_data(
    playback_state: Option<playback_queue::PlaybackStateInput>,
    history: Option<play_history::PlayHistory>,
    lyric_offsets: Option<BTreeMap<String, i64>>,
) -> AppResult<UserDataSnapshot> {
    blocking(move |database| {
        import_legacy(database, playback_state, history, lyric_offsets)?;
        snapshot_from(database)
    })
    .await
}

fn import_legacy(
    database: &UserDatabase,
    playback_state: Option<playback_queue::PlaybackStateInput>,
    history: Option<play_history::PlayHistory>,
    offsets: Option<BTreeMap<String, i64>>,
) -> AppResult<()> {
    database.write(|transaction| {
        let now = chrono::Utc::now().timestamp_millis();
        if !meta::is_flag_set(transaction, playback_queue::LEGACY_IMPORT_KEY)? {
            if let Some(state) = &playback_state {
                playback_queue::save_into(transaction, state)?;
            }
            meta::set_i64(transaction, playback_queue::LEGACY_IMPORT_KEY, now)?;
        }
        if !meta::is_flag_set(transaction, play_history::LEGACY_IMPORT_KEY)? {
            if let Some(history) = &history {
                play_history::replace(transaction, history)?;
            }
            meta::set_i64(transaction, play_history::LEGACY_IMPORT_KEY, now)?;
        }
        if !meta::is_flag_set(transaction, lyric_offsets::LEGACY_IMPORT_KEY)? {
            if let Some(offsets) = &offsets {
                lyric_offsets::replace(transaction, offsets)?;
            }
            meta::set_i64(transaction, lyric_offsets::LEGACY_IMPORT_KEY, now)?;
        }
        Ok(())
    })
}

#[tauri::command]
pub async fn save_playback_state(state: playback_queue::PlaybackStateInput) -> AppResult<()> {
    blocking(move |database| {
        database.write(|transaction| playback_queue::save_into(transaction, &state))
    })
    .await
}

#[tauri::command]
pub async fn record_play_history(
    track: Value,
    played_at: i64,
    resume_position_ms: Option<i64>,
) -> AppResult<()> {
    blocking(move |database| {
        database.write(|transaction| {
            play_history::record(transaction, &track, played_at, resume_position_ms).map(|_| ())
        })
    })
    .await
}

#[tauri::command]
pub async fn update_play_history_metadata(
    entries: Vec<play_history::HistoryEntry>,
) -> AppResult<()> {
    blocking(move |database| {
        database.write(|transaction| play_history::update_metadata(transaction, &entries))
    })
    .await
}

#[tauri::command]
pub async fn remove_play_history(identity_key: String, deleted_at: i64) -> AppResult<()> {
    blocking(move |database| {
        database.write(|transaction| {
            play_history::remove(transaction, &identity_key, deleted_at).map(|_| ())
        })
    })
    .await
}

#[tauri::command]
pub async fn clear_play_history(deleted_at: i64) -> AppResult<()> {
    blocking(move |database| {
        database.write(|transaction| play_history::clear(transaction, deleted_at).map(|_| ()))
    })
    .await
}

#[tauri::command]
pub async fn replace_play_history(history: play_history::PlayHistory) -> AppResult<()> {
    blocking(move |database| {
        database.write(|transaction| play_history::replace(transaction, &history))
    })
    .await
}

#[tauri::command]
pub async fn set_lyric_offset(track_key: String, offset_ms: i64) -> AppResult<()> {
    blocking(move |database| {
        database.write(|transaction| lyric_offsets::set(transaction, &track_key, offset_ms))
    })
    .await
}

#[tauri::command]
pub async fn replace_lyric_offsets(offsets: BTreeMap<String, i64>) -> AppResult<()> {
    blocking(move |database| {
        database.write(|transaction| lyric_offsets::replace(transaction, &offsets))
    })
    .await
}

#[tauri::command]
pub async fn get_bili_video_skip_rules() -> AppResult<Vec<bili_video_skip::BiliVideoSkipRule>> {
    blocking(|database| database.read(bili_video_skip::load_from)).await
}

#[tauri::command]
pub async fn get_bili_video_skip_targets(
    bvid: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<bili_video_skip::BiliVideoSkipTargetOption>> {
    let bvid = bvid.trim();
    if bvid.is_empty() {
        return Ok(Vec::new());
    }
    let info = state.bilibili().get_video_info(bvid).await?;
    Ok(bili_video_skip::target_options(&info))
}

#[tauri::command]
pub async fn set_bili_video_skip_rule(
    app: AppHandle,
    bvid: String,
    cid: i64,
    intervals: Vec<bili_video_skip::BiliVideoSkipInterval>,
    duration_ms: Option<i64>,
) -> AppResult<Option<bili_video_skip::BiliVideoSkipRule>> {
    let (rule, changed) = blocking(move |database| {
        // 与同步共用写入版本，网络请求期间保存的区间不能被旧快照覆盖
        let _guard = neri_library::library::playlist::lock_io();
        let result = database.write(|transaction| {
            bili_video_skip::set(
                transaction,
                &bvid,
                cid,
                &intervals,
                duration_ms.unwrap_or(0),
                chrono::Utc::now().timestamp_millis(),
            )
        })?;
        if result.1 {
            neri_library::library::playlist::mark_io_changed();
        }
        Ok(result)
    })
    .await?;
    if changed {
        let _ = app.emit("playlists-changed", ());
    }
    Ok(rule)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn snapshot_includes_bili_skip_rules_without_requiring_legacy_migration() {
        let database = UserDatabase::open_in_memory().unwrap();
        let rules = json!([{
            "bvid": "BV1test",
            "cid": 9,
            "intervals": [{"startMs": 1000, "endMs": 2500}],
            "modifiedAt": 10,
            "isDeleted": false
        }]);
        database
            .write(|transaction| {
                neri_library::sync::storage::update_archive_extensions(transaction, |extensions| {
                    extensions.insert("biliVideoSkipRules".into(), rules.clone());
                    Ok(())
                })
            })
            .unwrap();
        let snapshot = serde_json::to_value(snapshot_from(&database).unwrap()).unwrap();
        assert_eq!(snapshot["biliVideoSkipRules"], rules);
    }

    #[test]
    fn legacy_import_runs_once_per_domain_even_without_data() {
        let database = UserDatabase::open_in_memory().unwrap();
        let history: play_history::PlayHistory = serde_json::from_value(json!({
            "entries": [{"track": {"id": "netease:1", "title": "A"}, "playedAt": 5}],
            "deletions": []
        }))
        .unwrap();
        import_legacy(&database, None, Some(history), None).unwrap();
        let snapshot = snapshot_from(&database).unwrap();
        assert!(
            snapshot.migrated.playback_state
                && snapshot.migrated.history
                && snapshot.migrated.lyric_offsets
        );
        assert_eq!(snapshot.history.entries.len(), 1);

        let stale: play_history::PlayHistory =
            serde_json::from_value(json!({"entries": [], "deletions": []})).unwrap();
        import_legacy(
            &database,
            None,
            Some(stale),
            Some(BTreeMap::from([("qq:1".into(), 5)])),
        )
        .unwrap();
        let snapshot = snapshot_from(&database).unwrap();
        assert_eq!(
            snapshot.history.entries.len(),
            1,
            "stale legacy data must never overwrite the database"
        );
        assert!(snapshot.lyric_offsets.is_empty());
    }
}
