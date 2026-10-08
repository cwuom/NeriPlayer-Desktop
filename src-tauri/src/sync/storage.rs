// 同步本地状态：归档扩展与统计来源、云端最近播放快照、三方合并基线、WebDAV
// 租约能力，统一落在用户数据库（取代 sync-android-metadata.json 等侧车文件）
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};

use super::models::{
    SyncData, SyncPlaybackStatBucket, SyncRecentPlay, SyncRecentPlayDeletion, SyncTrackStat,
};
use crate::db;
use crate::error::AppResult;

pub(crate) const LEGACY_IMPORT_KEY: &str = "legacy_import.sync_metadata";
const ARCHIVE_METADATA_KEY: &str = "sync.archive_metadata";
const RECENT_PLAY_HISTORY_KEY: &str = "sync.recent_play_history";
const WEBDAV_LEASE_TARGETS_KEY: &str = "sync.webdav_finite_lease_targets";
const LEGACY_METADATA_FILE: &str = "sync-android-metadata.json";
const LEGACY_HISTORY_FILE: &str = "recent-play-history.json";
const LEGACY_LEASES_FILE: &str = "sync-webdav-finite-leases.json";
const LEGACY_BASE_PREFIX: &str = "sync-base-snapshot-";

/// Android 归档中桌面端不直接建模的扩展段与统计来源，回传时原样保留
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArchiveMetadata {
    #[serde(default)]
    pub extensions: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub playback_stats: Vec<SyncTrackStat>,
    #[serde(default)]
    pub playback_stat_buckets: Vec<SyncPlaybackStatBucket>,
    #[serde(default)]
    pub playback_stats_cleared_at: i64,
}

/// 上一轮同步合并后的最近播放（含其它设备的续播进度）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecentPlayHistory {
    #[serde(default)]
    pub recent_plays: Vec<SyncRecentPlay>,
    #[serde(default)]
    pub recent_play_deletions: Vec<SyncRecentPlayDeletion>,
}

pub(crate) fn load_archive_metadata(connection: &Connection) -> AppResult<ArchiveMetadata> {
    Ok(db::meta::get_document(connection, ARCHIVE_METADATA_KEY)?.unwrap_or_default())
}

pub(crate) fn save_archive_metadata(connection: &Connection, data: &SyncData) -> AppResult<()> {
    db::meta::set_document(
        connection,
        ARCHIVE_METADATA_KEY,
        &ArchiveMetadata {
            extensions: data.extensions.clone(),
            playback_stats: data.playback_stats.clone(),
            playback_stat_buckets: data.playback_stat_buckets.clone(),
            playback_stats_cleared_at: data.playback_stats_cleared_at,
        },
    )
}

/// 只改扩展段，统计与清空时间原样保留
pub(crate) fn update_archive_extensions(
    connection: &Connection,
    update: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>) -> AppResult<()>,
) -> AppResult<()> {
    let mut metadata = load_archive_metadata(connection)?;
    update(&mut metadata.extensions)?;
    db::meta::set_document(connection, ARCHIVE_METADATA_KEY, &metadata)
}

pub(crate) fn load_recent_play_history(connection: &Connection) -> AppResult<RecentPlayHistory> {
    Ok(db::meta::get_document(connection, RECENT_PLAY_HISTORY_KEY)?.unwrap_or_default())
}

pub(crate) fn save_recent_play_history(connection: &Connection, data: &SyncData) -> AppResult<()> {
    db::meta::set_document(
        connection,
        RECENT_PLAY_HISTORY_KEY,
        &RecentPlayHistory {
            recent_plays: data.recent_plays.clone(),
            recent_play_deletions: data.recent_play_deletions.clone(),
        },
    )
}

/// 上次同步后每个歌单的歌曲 stable_key 集合，供三方合并做删除检测
pub(crate) fn load_base_snapshot(
    connection: &Connection,
    scope: &str,
) -> AppResult<HashMap<String, HashSet<String>>> {
    let mut snapshot: HashMap<String, HashSet<String>> = HashMap::new();
    let mut statement = connection
        .prepare("SELECT playlist_id, song_key FROM sync_base_snapshot WHERE scope = ?1")?;
    let mut rows = statement.query([scope])?;
    while let Some(row) = rows.next()? {
        snapshot.entry(row.get(0)?).or_default().insert(row.get(1)?);
    }
    Ok(snapshot)
}

pub(crate) fn save_base_snapshot(connection: &Connection, data: &SyncData, scope: &str) -> AppResult<()> {
    let snapshot: HashMap<String, Vec<String>> = data
        .playlists
        .iter()
        .filter(|playlist| !playlist.is_deleted)
        .map(|playlist| {
            (
                playlist.id.clone(),
                playlist.songs.iter().map(|song| song.identity().stable_key()).collect(),
            )
        })
        .collect();
    replace_base_snapshot(connection, scope, &snapshot)
}

fn replace_base_snapshot(
    connection: &Connection,
    scope: &str,
    snapshot: &HashMap<String, Vec<String>>,
) -> AppResult<()> {
    connection.execute("DELETE FROM sync_base_snapshot WHERE scope = ?1", [scope])?;
    let mut insert = connection.prepare_cached(
        "INSERT OR IGNORE INTO sync_base_snapshot (scope, playlist_id, song_key) VALUES (?1, ?2, ?3)",
    )?;
    for (playlist_id, keys) in snapshot {
        for key in keys {
            insert.execute(params![scope, playlist_id, key])?;
        }
    }
    Ok(())
}

pub(crate) fn load_lease_targets(connection: &Connection) -> AppResult<HashSet<String>> {
    Ok(db::meta::get_document(connection, WEBDAV_LEASE_TARGETS_KEY)?.unwrap_or_default())
}

pub(crate) fn save_lease_targets(connection: &Connection, targets: &HashSet<String>) -> AppResult<()> {
    let mut sorted: Vec<&String> = targets.iter().collect();
    sorted.sort();
    db::meta::set_document(connection, WEBDAV_LEASE_TARGETS_KEY, &sorted)
}

/// 旧版同步侧车文件一次性导入
pub(crate) fn import_legacy_json(transaction: &Transaction<'_>, directory: &Path) -> AppResult<Vec<PathBuf>> {
    let mut consumed = Vec::new();

    let metadata_path = directory.join(LEGACY_METADATA_FILE);
    if let Some(metadata) = db::legacy::read_json::<ArchiveMetadata>(&metadata_path)? {
        db::meta::set_document(transaction, ARCHIVE_METADATA_KEY, &metadata)?;
        consumed.push(metadata_path);
    }

    let history_path = directory.join(LEGACY_HISTORY_FILE);
    if let Some(history) = db::legacy::read_json::<RecentPlayHistory>(&history_path)? {
        db::meta::set_document(transaction, RECENT_PLAY_HISTORY_KEY, &history)?;
        consumed.push(history_path);
    }

    let leases_path = directory.join(LEGACY_LEASES_FILE);
    if let Some(targets) = db::legacy::read_json::<HashSet<String>>(&leases_path)? {
        save_lease_targets(transaction, &targets)?;
        consumed.push(leases_path);
    }

    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(consumed),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let path = entry?.path();
        let Some(scope) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix(LEGACY_BASE_PREFIX))
            .and_then(|name| name.strip_suffix(".json"))
            .map(str::to_string)
        else {
            continue;
        };
        if let Some(snapshot) = db::legacy::read_json::<HashMap<String, Vec<String>>>(&path)? {
            replace_base_snapshot(transaction, &scope, &snapshot)?;
            consumed.push(path);
        }
    }
    log::info!(target: "sync", "旧版同步侧车已导入数据库: files={}", consumed.len());
    Ok(consumed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::UserDatabase;
    use crate::sync::models::{SyncPlaylist, SyncSong};

    #[test]
    fn documents_and_base_snapshots_round_trip_per_scope() {
        let database = UserDatabase::open_in_memory().unwrap();
        let mut data = SyncData::default();
        data.extensions.insert("playlistUsageStats".into(), serde_json::json!([{"id": 1}]));
        data.playback_stats_cleared_at = 42;
        let playlist = |id: &str, is_deleted: bool, songs: Vec<SyncSong>| SyncPlaylist {
            id: id.into(),
            name: String::new(),
            songs,
            created_at: 0,
            modified_at: 0,
            is_deleted,
            song_order_version: 0,
        };
        data.playlists = vec![
            playlist(
                "1",
                false,
                vec![SyncSong { id: "9".into(), album: "netease".into(), ..Default::default() }],
            ),
            playlist("2", true, Vec::new()),
        ];
        database
            .write(|transaction| {
                save_archive_metadata(transaction, &data)?;
                save_recent_play_history(transaction, &data)?;
                save_base_snapshot(transaction, &data, "github-a")?;
                save_base_snapshot(transaction, &SyncData::default(), "webdav-b")
            })
            .unwrap();

        database
            .read(|connection| {
                let metadata = load_archive_metadata(connection)?;
                assert_eq!(metadata.extensions["playlistUsageStats"], serde_json::json!([{"id": 1}]));
                assert_eq!(metadata.playback_stats_cleared_at, 42);
                let base = load_base_snapshot(connection, "github-a")?;
                assert_eq!(base.len(), 1);
                assert!(base["1"].contains(&data.playlists[0].songs[0].identity().stable_key()));
                assert!(load_base_snapshot(connection, "webdav-b")?.is_empty());
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn legacy_sidecars_are_imported_and_moved() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join(LEGACY_METADATA_FILE),
            br#"{"extensions":{"lyricOverrides":[]},"playbackStatsClearedAt":7}"#,
        )
        .unwrap();
        std::fs::write(
            directory.path().join(LEGACY_HISTORY_FILE),
            br#"{"recentPlays":[{"songId":1,"song":{"id":1},"playedAt":5,"deviceId":"android","resumePositionMs":99}]}"#,
        )
        .unwrap();
        std::fs::write(directory.path().join(LEGACY_LEASES_FILE), br#"["https://dav/a"]"#).unwrap();
        std::fs::write(
            directory.path().join("sync-base-snapshot-github-owner-repo.json"),
            br#"{"1":["9|netease|"]}"#,
        )
        .unwrap();
        let database = UserDatabase::open_in_memory().unwrap();

        db::legacy::run_once(&database, directory.path(), LEGACY_IMPORT_KEY, import_legacy_json).unwrap();
        database
            .read(|connection| {
                assert_eq!(load_archive_metadata(connection)?.playback_stats_cleared_at, 7);
                let history = load_recent_play_history(connection)?;
                assert_eq!(history.recent_plays[0].resume_position_ms, 99);
                assert!(load_lease_targets(connection)?.contains("https://dav/a"));
                assert!(load_base_snapshot(connection, "github-owner-repo")?["1"].contains("9|netease|"));
                Ok(())
            })
            .unwrap();
        let remaining: Vec<String> = std::fs::read_dir(directory.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".json"))
            .collect();
        assert!(remaining.is_empty(), "imported sidecars must be moved to the backup folder: {remaining:?}");
    }
}
