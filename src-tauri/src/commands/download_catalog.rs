// 已完成下载的曲目目录（对齐 Android managed_library_item）
//
// 目录与音频文件分离保存：音频与 sidecar 留在下载目录，清单落在用户数据库，
// 由 manifest_lock 串行化整表读改写
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{params, Connection, Transaction};

use super::DownloadedTrack;
use crate::db::{self, UserDatabase};
use crate::error::AppResult;

pub(crate) const LEGACY_IMPORT_KEY: &str = "legacy_import.download_manifest";
const LEGACY_BACKUP_NAME: &str = "download-manifest.json";

static IMPORT_CHECKED: AtomicBool = AtomicBool::new(false);

pub(crate) fn load_from(connection: &Connection) -> AppResult<Vec<DownloadedTrack>> {
    let mut statement = connection.prepare(
        "SELECT track_id, title, artist, album, duration_ms, cover_url, source, file_path,
             file_size, downloaded_at
         FROM download_catalog ORDER BY catalog_position",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(DownloadedTrack {
            id: row.get(0)?,
            title: row.get(1)?,
            artist: row.get(2)?,
            album: row.get(3)?,
            duration_ms: row.get::<_, i64>(4)?.max(0) as u64,
            cover_url: row.get(5)?,
            source: row.get(6)?,
            file_path: row.get(7)?,
            file_size: row.get::<_, i64>(8)?.max(0) as u64,
            downloaded_at: row.get::<_, i64>(9)?.max(0) as u64,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub(crate) fn save_into(transaction: &Transaction<'_>, tracks: &[DownloadedTrack]) -> AppResult<()> {
    transaction.execute("DELETE FROM download_catalog", [])?;
    let mut insert = transaction.prepare_cached(
        "INSERT INTO download_catalog (catalog_position, track_id, title, artist, album,
             duration_ms, cover_url, source, file_path, file_size, downloaded_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )?;
    for (position, track) in tracks.iter().enumerate() {
        insert.execute(params![
            position as i64,
            track.id,
            track.title,
            track.artist,
            track.album,
            clamp(track.duration_ms),
            track.cover_url,
            track.source,
            track.file_path,
            clamp(track.file_size),
            clamp(track.downloaded_at),
        ])?;
    }
    Ok(())
}

/// 目录在数据库中的估算占用与记录数（对齐 Android 下载索引的页面归属统计）
pub(crate) fn usage(connection: &Connection) -> AppResult<(u64, u64)> {
    let (bytes, count): (i64, i64) = connection.query_row(
        "SELECT COALESCE(SUM(length(track_id) + length(title) + length(artist) + length(album)
                 + COALESCE(length(cover_url), 0) + length(source) + length(file_path) + 40), 0),
             COUNT(*)
         FROM download_catalog",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok((bytes.max(0) as u64, count.max(0) as u64))
}

/// 旧版下载目录里的 manifest.json 一次性导入；成功后移入用户数据备份目录
pub(crate) fn ensure_imported(database: &UserDatabase, legacy_manifest: &Path) -> AppResult<()> {
    if IMPORT_CHECKED.load(Ordering::Acquire) {
        return Ok(());
    }
    let imported = import_manifest(database, legacy_manifest)?;
    if imported {
        db::legacy::move_to_backup_as(&db::user_data_dir(), legacy_manifest, LEGACY_BACKUP_NAME);
    }
    IMPORT_CHECKED.store(true, Ordering::Release);
    Ok(())
}

fn import_manifest(database: &UserDatabase, legacy_manifest: &Path) -> AppResult<bool> {
    database.write(|transaction| {
        if db::meta::is_flag_set(transaction, LEGACY_IMPORT_KEY)? {
            return Ok(false);
        }
        let tracks = db::legacy::read_json::<Vec<DownloadedTrack>>(legacy_manifest)?;
        if let Some(tracks) = &tracks {
            save_into(transaction, tracks)?;
            log::info!(target: "download", "旧版下载清单已导入数据库: count={}", tracks.len());
        }
        db::meta::set_i64(transaction, LEGACY_IMPORT_KEY, chrono::Utc::now().timestamp_millis())?;
        Ok(tracks.is_some())
    })
}

fn clamp(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, path: &str) -> DownloadedTrack {
        DownloadedTrack {
            id: id.into(),
            title: format!("Title {id}"),
            artist: "Artist".into(),
            album: "Album".into(),
            duration_ms: 200_000,
            cover_url: None,
            source: "netease".into(),
            file_path: path.into(),
            file_size: 1_234,
            downloaded_at: 1_700_000_000_000,
        }
    }

    #[test]
    fn catalog_round_trips_in_order() {
        let database = UserDatabase::open_in_memory().unwrap();
        let tracks = vec![track("netease:2", "D:/b.flac"), track("netease:1", "D:/a.flac")];
        database.write(|transaction| save_into(transaction, &tracks)).unwrap();

        let restored = database.read(load_from).unwrap();
        assert_eq!(
            serde_json::to_value(&restored).unwrap(),
            serde_json::to_value(&tracks).unwrap()
        );
        let (bytes, count) = database.read(usage).unwrap();
        assert_eq!(count, 2);
        assert!(bytes > 0);
    }

    #[test]
    fn legacy_manifest_is_imported_once() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("manifest.json");
        std::fs::write(&manifest, serde_json::to_vec(&vec![track("netease:1", "D:/a.flac")]).unwrap()).unwrap();
        let database = UserDatabase::open_in_memory().unwrap();

        assert!(import_manifest(&database, &manifest).unwrap());
        std::fs::write(&manifest, b"[]").unwrap();
        assert!(!import_manifest(&database, &manifest).unwrap());
        assert_eq!(database.read(load_from).unwrap().len(), 1);
    }
}
