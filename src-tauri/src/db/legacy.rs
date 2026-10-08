//! 旧版 JSON 一次性导入
//!
//! 每个数据域由独立的迁移标记守护：读取旧文件、写入新表与写入标记在同一个
//! 事务里完成，提交成功后才把源文件移入 `legacy-json-backup/`。标记一旦存在
//! 就永不再读旧文件，旧数据不会覆盖迁移之后的新编辑

use std::path::{Path, PathBuf};

use rusqlite::Transaction;

use super::{meta, UserDatabase};
use crate::error::{AppError, AppResult};

pub const BACKUP_DIR: &str = "legacy-json-backup";

/// 单个数据域的导入器：返回成功消费、需要移入备份目录的旧文件
pub type LegacyImporter = fn(&Transaction<'_>, &Path) -> AppResult<Vec<PathBuf>>;

/// (迁移标记, 导入器)；顺序即导入顺序，依赖其它数据域的导入器必须排在后面
const IMPORTERS: &[(&str, LegacyImporter)] = &[
    (
        crate::library::playlist::LEGACY_IMPORT_KEY,
        crate::library::playlist::import_legacy_json,
    ),
    // 不读文件：只把旧版按名字认的系统歌单改成固定 id，必须在导入旧歌单之后
    (
        crate::library::playlist::SYSTEM_IDS_KEY,
        crate::library::playlist::adopt_system_playlist_ids_once,
    ),
    (
        crate::library::favorites::LEGACY_IMPORT_KEY,
        crate::library::favorites::import_legacy_json,
    ),
    // 旧统计投影要用同步侧车补回分片来源，必须先于同步元数据导入读取侧车
    (crate::stats::LEGACY_IMPORT_KEY, crate::stats::import_legacy_json),
    (
        crate::sync::storage::LEGACY_IMPORT_KEY,
        crate::sync::storage::import_legacy_json,
    ),
    // 不读文件：把 v1 库里按曲目 id 存的播放历史改用身份键
    (
        crate::library::play_history::IDENTITY_KEY_MIGRATION,
        crate::library::play_history::adopt_identity_keys_once,
    ),
];

pub fn import_all(database: &UserDatabase, directory: &Path) -> AppResult<()> {
    for (key, importer) in IMPORTERS {
        run_once(database, directory, key, *importer)?;
    }
    Ok(())
}

/// 执行一次性导入；返回本次是否真的执行了导入
pub fn run_once(
    database: &UserDatabase,
    directory: &Path,
    key: &str,
    importer: LegacyImporter,
) -> AppResult<bool> {
    let consumed = database.write(|transaction| {
        if meta::is_flag_set(transaction, key)? {
            return Ok(None);
        }
        let consumed = importer(transaction, directory)?;
        meta::set_i64(transaction, key, chrono::Utc::now().timestamp_millis())?;
        Ok(Some(consumed))
    })?;
    let Some(files) = consumed else {
        return Ok(false);
    };
    for file in files {
        move_to_backup(directory, &file);
    }
    log::info!(target: "database", "旧版数据已导入: {key}");
    Ok(true)
}

/// 读取可选的旧版 JSON
///
/// 文件不存在表示没有可导入的数据；解析失败时隔离现场并按「无数据」继续，
/// 与旧实现「损坏文件隔离后视为空库」的语义一致；其它 IO 错误中止导入，
/// 留待下次启动重试，不能当成空数据写入标记
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> AppResult<Option<T>> {
    let content = match std::fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(AppError::Other(format!(
                "Failed to read legacy data {}: {error}",
                path.display()
            )))
        }
    };
    match serde_json::from_slice(&content) {
        Ok(value) => Ok(Some(value)),
        Err(error) => {
            let quarantined = crate::fsutil::quarantine_corrupt_file(path);
            log::error!(
                target: "database",
                "旧版数据 {} 无法解析, 已隔离到 {quarantined:?}, 跳过导入: {error}",
                path.display()
            );
            Ok(None)
        }
    }
}

fn move_to_backup(directory: &Path, file: &Path) {
    let Some(name) = file.file_name().and_then(|name| name.to_str()) else { return };
    move_to_backup_as(directory, file, name);
}

/// 把已导入的旧文件移入 `<directory>/legacy-json-backup/<backup_name>`
pub(crate) fn move_to_backup_as(directory: &Path, file: &Path, backup_name: &str) {
    if !file.exists() {
        return;
    }
    let backup_dir = directory.join(BACKUP_DIR);
    if let Err(error) = std::fs::create_dir_all(&backup_dir) {
        log::warn!(target: "database", "无法创建旧数据备份目录 {backup_dir:?}: {error}");
        return;
    }
    let mut target = backup_dir.join(backup_name);
    if target.exists() {
        target = backup_dir.join(format!("{backup_name}.{}", chrono::Utc::now().timestamp_millis()));
    }
    if let Err(error) = std::fs::rename(file, &target) {
        // 标记已经提交，留在原处的旧文件不会再被读取
        log::warn!(target: "database", "旧数据 {file:?} 移入备份目录失败: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn import_marker(transaction: &Transaction<'_>, directory: &Path) -> AppResult<Vec<PathBuf>> {
        let source = directory.join("legacy.json");
        let Some(value) = read_json::<serde_json::Value>(&source)? else {
            return Ok(Vec::new());
        };
        meta::set(transaction, "imported.value", &value.to_string())?;
        Ok(vec![source])
    }

    fn failing_import(transaction: &Transaction<'_>, _directory: &Path) -> AppResult<Vec<PathBuf>> {
        meta::set(transaction, "imported.partial", "1")?;
        Err(AppError::Other("fixture failure".into()))
    }

    #[test]
    fn import_runs_once_and_moves_source_to_backup() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("legacy.json"), br#"{"a":1}"#).unwrap();
        let database = UserDatabase::open_in_memory().unwrap();

        assert!(run_once(&database, directory.path(), "legacy.test", import_marker).unwrap());
        assert!(!directory.path().join("legacy.json").exists());
        assert!(directory.path().join(BACKUP_DIR).join("legacy.json").exists());

        std::fs::write(directory.path().join("legacy.json"), br#"{"a":2}"#).unwrap();
        assert!(!run_once(&database, directory.path(), "legacy.test", import_marker).unwrap());
        let value = database
            .read(|connection| meta::get(connection, "imported.value"))
            .unwrap();
        assert_eq!(value.as_deref(), Some(r#"{"a":1}"#));
        assert!(directory.path().join("legacy.json").exists(), "a stale file must not be re-imported");
    }

    #[test]
    fn failed_import_rolls_back_and_retries_later() {
        let directory = tempfile::tempdir().unwrap();
        let database = UserDatabase::open_in_memory().unwrap();

        assert!(run_once(&database, directory.path(), "legacy.fail", failing_import).is_err());
        database
            .read(|connection| {
                assert!(!meta::is_flag_set(connection, "legacy.fail")?);
                assert!(!meta::is_flag_set(connection, "imported.partial")?);
                Ok(())
            })
            .unwrap();
        assert!(run_once(&database, directory.path(), "legacy.fail", import_marker).unwrap());
    }

    #[test]
    fn corrupt_legacy_file_is_quarantined_and_skipped() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.json");
        std::fs::write(&path, b"{broken").unwrap();

        assert_eq!(read_json::<serde_json::Value>(&path).unwrap(), None);
        assert!(!path.exists());
        assert!(std::fs::read_dir(directory.path())
            .unwrap()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("legacy.json.corrupt-")));
    }
}
