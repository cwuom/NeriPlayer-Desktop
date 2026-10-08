//! 用户数据库：对齐 Android NeriUserDataDatabase（neri_user_data.db）
//!
//! 歌单、收藏、播放历史、播放队列、播放统计、下载目录与同步元数据统一落在
//! SQLite，取代此前分散的 JSON 文件与 WebView localStorage。所有写入都在单一
//! 连接上的 IMMEDIATE 事务内完成，崩溃或断电只会回滚到上一次完整提交，
//! 不会再出现「半截 JSON 被误判为空库再覆盖」的数据丢失

pub mod cache;
pub mod legacy;
pub mod meta;
mod schema;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use parking_lot::Mutex;
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};

use crate::error::{AppError, AppResult};

pub const DATABASE_FILE: &str = "neri_user_data.db";

static USER_DATABASE: OnceLock<UserDatabase> = OnceLock::new();
static OPEN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 用户数据根目录（与历史 JSON 文件同目录，便于迁移与备份）
#[cfg(not(test))]
pub fn user_data_dir() -> PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("NeriPlayer")
}

/// 测试进程绝不能触碰真实用户目录
#[cfg(test)]
pub fn user_data_dir() -> PathBuf {
    std::env::temp_dir().join(format!("neri-user-data-test-{}", std::process::id()))
}

pub struct UserDatabase {
    connection: Mutex<Connection>,
    path: Option<PathBuf>,
}

impl UserDatabase {
    /// 打开（或创建）数据库文件并迁移到最新 schema
    ///
    /// 文件损坏时把原文件连同 WAL 一起隔离到 `.corrupt-<ts>` 保留现场，再建新库。
    /// 新库没有同步基线，下一轮同步只会从云端补回数据而不会传播删除
    pub fn open(path: &Path) -> AppResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match Self::open_checked(path) {
            Err(error) if is_corruption(&error) => {
                let quarantined = quarantine_database(path);
                log::error!(
                    target: "database",
                    "用户数据库损坏, 原文件已隔离到 {quarantined:?}, 重新建库: {error}"
                );
                Self::open_checked(path)
            }
            result => result,
        }
    }

    fn open_checked(path: &Path) -> AppResult<Self> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        configure(&connection, true)?;
        let check: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if !check.eq_ignore_ascii_case("ok") {
            return Err(AppError::Database(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
                Some(format!("quick_check failed: {check}")),
            )));
        }
        schema::migrate(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
            path: Some(path.to_path_buf()),
        })
    }

    /// 内存库：单元测试与无磁盘场景使用，schema 与文件库完全一致
    pub fn open_in_memory() -> AppResult<Self> {
        let connection = Connection::open_in_memory()?;
        configure(&connection, false)?;
        schema::migrate(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
            path: None,
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// 只读访问；多条语句需要一致快照时请改用 `write`
    pub fn read<T>(&self, operation: impl FnOnce(&Connection) -> AppResult<T>) -> AppResult<T> {
        let connection = self.connection.lock();
        operation(&connection)
    }

    /// 在 IMMEDIATE 事务内执行写入，闭包返回错误时整体回滚
    pub fn write<T>(
        &self,
        operation: impl FnOnce(&Transaction<'_>) -> AppResult<T>,
    ) -> AppResult<T> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let value = operation(&transaction)?;
        transaction.commit()?;
        Ok(value)
    }
}

/// 进程级用户数据库，首次访问时打开、迁移并导入旧版 JSON
///
/// 打开或导入失败不缓存句柄，下一次访问会重试，避免一次瞬时 IO 错误
/// 让整个进程永久失去用户数据
pub fn user_db() -> AppResult<&'static UserDatabase> {
    if let Some(database) = USER_DATABASE.get() {
        return Ok(database);
    }
    let _guard = OPEN_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(database) = USER_DATABASE.get() {
        return Ok(database);
    }
    let directory = user_data_dir();
    let database = UserDatabase::open(&directory.join(DATABASE_FILE))?;
    legacy::import_all(&database, &directory)?;
    let _ = USER_DATABASE.set(database);
    Ok(USER_DATABASE.get().expect("user database must be initialized"))
}

/// 数据库文件与 WAL/SHM 的合计占用，供存储管理页展示
pub fn database_files(path: &Path) -> Vec<PathBuf> {
    ["", "-wal", "-shm"]
        .iter()
        .map(|suffix| {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            PathBuf::from(name)
        })
        .collect()
}

fn configure(connection: &Connection, file_backed: bool) -> AppResult<()> {
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    if file_backed {
        let mode: String =
            connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            log::warn!(target: "database", "WAL 不可用, 回退到 {mode} 日志模式");
        }
        // 用户数据宁可慢一点也要每次提交都 fsync，与旧 atomic_write 的持久化语义一致
        connection.execute_batch("PRAGMA synchronous = FULL;")?;
    }
    Ok(())
}

fn is_corruption(error: &AppError) -> bool {
    matches!(
        error,
        AppError::Database(rusqlite::Error::SqliteFailure(failure, _))
            if matches!(
                failure.code,
                rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
            )
    )
}

fn quarantine_database(path: &Path) -> Option<PathBuf> {
    let primary = crate::fsutil::quarantine_corrupt_file(path);
    for sidecar in database_files(path).into_iter().skip(1) {
        if sidecar.exists() {
            let _ = crate::fsutil::quarantine_corrupt_file(&sidecar);
        }
    }
    primary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_database_reaches_latest_schema() {
        let database = UserDatabase::open_in_memory().unwrap();
        let version = database
            .read(|connection| {
                Ok(connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))?)
            })
            .unwrap();
        assert_eq!(version, schema::SCHEMA_VERSION);
    }

    #[test]
    fn failed_write_rolls_back_every_statement() {
        let database = UserDatabase::open_in_memory().unwrap();
        let result = database.write(|transaction| {
            meta::set(transaction, "rollback.probe", "1")?;
            Err::<(), _>(AppError::Other("fixture failure".into()))
        });
        assert!(result.is_err());
        let value = database
            .read(|connection| meta::get(connection, "rollback.probe"))
            .unwrap();
        assert_eq!(value, None);
    }

    #[test]
    fn file_database_uses_wal_and_reopens_with_data() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(DATABASE_FILE);
        {
            let database = UserDatabase::open(&path).unwrap();
            database
                .write(|transaction| meta::set(transaction, "reopen.probe", "kept"))
                .unwrap();
            let mode = database
                .read(|connection| {
                    Ok(connection.query_row("PRAGMA journal_mode", [], |row| {
                        row.get::<_, String>(0)
                    })?)
                })
                .unwrap();
            assert!(mode.eq_ignore_ascii_case("wal"));
        }
        let database = UserDatabase::open(&path).unwrap();
        let value = database
            .read(|connection| meta::get(connection, "reopen.probe"))
            .unwrap();
        assert_eq!(value.as_deref(), Some("kept"));
    }

    #[test]
    fn corrupt_database_is_quarantined_and_recreated() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(DATABASE_FILE);
        std::fs::write(&path, b"this is definitely not an sqlite database file").unwrap();

        let database = UserDatabase::open(&path).unwrap();
        database
            .write(|transaction| meta::set(transaction, "fresh", "1"))
            .unwrap();
        let quarantined = std::fs::read_dir(directory.path())
            .unwrap()
            .flatten()
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{DATABASE_FILE}.corrupt-"))
            });
        assert!(quarantined, "the corrupt original must be preserved");
    }

    #[test]
    fn newer_schema_is_refused_instead_of_downgraded() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(DATABASE_FILE);
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .pragma_update(None, "user_version", schema::SCHEMA_VERSION + 1)
                .unwrap();
        }
        let error = UserDatabase::open(&path).err().expect("newer schema must be refused");
        assert!(error.to_string().contains("newer"));
        assert!(path.exists(), "a newer database must never be quarantined or replaced");
    }
}
