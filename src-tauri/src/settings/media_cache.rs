//! 音频与封面缓存的根目录（#55）：默认在系统缓存目录，可改到其他磁盘。
//!
//! 只有 playback-audio 与 image_cache 两个大缓存跟着走；设置文件、WebView 数据、
//! 日志与下载暂存仍留在默认位置。搬移和清理只碰这两个子目录里认得出的缓存文件，
//! 用户选中的目录里其它文件一律不动。

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, RwLock};

use neri_common::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

pub const PLAYBACK_AUDIO_DIR: &str = "playback-audio";
pub const IMAGE_CACHE_DIR: &str = "image_cache";
const MEDIA_CACHE_SUBDIRS: [&str; 2] = [PLAYBACK_AUDIO_DIR, IMAGE_CACHE_DIR];
/// 音频缓存（.audio）、封面缓存（.image）与它们的发布标记（.ready）
const MOVABLE_EXTENSIONS: [&str; 3] = ["audio", "image", "ready"];
/// 写入中的暂存文件：可能仍被播放线程占用，搬移时跳过，只在丢弃旧缓存时尝试删除
const STAGING_EXTENSIONS: [&str; 2] = ["part", "tmp"];
const WRITE_PROBE_PREFIX: &str = ".neri_write_test";

static CUSTOM_ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);
static FALLBACK_WARNED: AtomicBool = AtomicBool::new(false);
static RELOCATING: AtomicBool = AtomicBool::new(false);
static WRITE_PROBE_SEQ: AtomicU64 = AtomicU64::new(0);
// 普通保存与改缓存位置都是「读设置 → 改 → 写回」，串行化以免后写的一方带回旧目录
static SETTINGS_WRITE: Mutex<()> = Mutex::new(());

pub fn settings_write_lock() -> MutexGuard<'static, ()> {
    SETTINGS_WRITE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// 按已保存的设置切换运行时缓存根目录；空串回到默认目录
pub fn set_custom_root(value: &str) {
    let value = value.trim();
    let next = (!value.is_empty()).then(|| PathBuf::from(value));
    *CUSTOM_ROOT
        .write()
        .unwrap_or_else(|error| error.into_inner()) = next;
    FALLBACK_WARNED.store(false, Ordering::Relaxed);
}

fn custom_root() -> Option<PathBuf> {
    CUSTOM_ROOT
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

pub fn default_root(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_cache_dir().ok()
}

/// 当前生效的缓存根目录
pub fn media_cache_root(app: &AppHandle) -> Option<PathBuf> {
    resolve_root(custom_root().as_deref(), || default_root(app))
}

fn resolve_root(
    custom: Option<&Path>,
    default: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(custom) = custom {
        // 移动硬盘拔掉、卷改成只读或目录失去写权限时退回默认目录，播放和封面不能因为缓存位置失效而失败。
        // 目录已存在时 create_dir_all 照样成功，所以必须真写一次
        if is_writable_dir(custom) {
            return Some(custom.to_path_buf());
        }
        if !FALLBACK_WARNED.swap(true, Ordering::Relaxed) {
            log::warn!(
                target: "storage",
                "自定义缓存目录不可用，暂时使用默认目录: {}",
                custom.display()
            );
        }
    }
    default()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaCacheLocation {
    /// 当前生效的目录（自定义目录不可用时为默认目录）
    pub path: String,
    pub default_path: String,
    /// 已保存的自定义目录，空串为默认
    pub custom_path: String,
    pub is_default: bool,
}

pub fn location(app: &AppHandle) -> MediaCacheLocation {
    let custom = custom_root();
    MediaCacheLocation {
        path: media_cache_root(app).map(display_path).unwrap_or_default(),
        default_path: default_root(app).map(display_path).unwrap_or_default(),
        custom_path: custom.clone().map(display_path).unwrap_or_default(),
        is_default: custom.is_none(),
    }
}

/// 旧缓存的处理方式：搬到新目录，或直接丢弃从头缓存
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExistingCacheAction {
    Move,
    Discard,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelocationReport {
    pub moved_files: u64,
    pub moved_bytes: u64,
    pub removed_files: u64,
    /// 搬不动或删不掉的文件数（多为正在写入的暂存文件）；它们留在旧目录
    pub failed_count: u64,
}

/// 校验并准备新的缓存根目录：必须是绝对路径、可创建且可写；返回规范化路径
pub fn prepare_root(path: &Path) -> AppResult<PathBuf> {
    if !path.is_absolute() {
        return Err(AppError::Other("CACHE_DIR_NOT_ABSOLUTE".into()));
    }
    if path.exists() && !path.is_dir() {
        return Err(AppError::Other("CACHE_DIR_NOT_DIRECTORY".into()));
    }
    if !is_writable_dir(path) {
        return Err(AppError::Other("CACHE_DIR_NOT_WRITABLE".into()));
    }
    Ok(normalized(path))
}

/// 按需创建目录并实际写入一个探测文件。探测文件名每次不同，
/// 并发的封面和音频请求不会互删对方的探测文件而误判不可写
fn is_writable_dir(path: &Path) -> bool {
    if fs::create_dir_all(path).is_err() {
        return false;
    }
    let seq = WRITE_PROBE_SEQ.fetch_add(1, Ordering::Relaxed);
    let probe = path.join(format!("{WRITE_PROBE_PREFIX}-{}-{seq}", std::process::id()));
    let writable = fs::write(&probe, b"ok").is_ok();
    let _ = fs::remove_file(&probe);
    writable
}

/// 规范化最长的已存在前缀再接回其余部分：尚未创建的目录也能和已存在的目录比较
/// （macOS 的 /var 实为 /private/var，只规范化一边会误判不重叠）
fn normalized(path: &Path) -> PathBuf {
    let mut existing = path;
    let mut rest = Vec::new();
    let canonical = loop {
        if let Ok(canonical) = fs::canonicalize(existing) {
            break rest
                .iter()
                .rev()
                .fold(canonical, |path: PathBuf, part| path.join(part));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => break path.to_path_buf(),
        }
    };
    match canonical.to_str().and_then(strip_verbatim_prefix) {
        Some(stripped) => PathBuf::from(stripped),
        None => canonical,
    }
}

/// Windows canonicalize 带 \\?\ 前缀，展示和保存都用常规写法。
/// 网络共享是 \\?\UNC\server\share，只去掉 \\?\ 会变成相对路径 UNC\server\share，必须还原成 \\server\share
fn strip_verbatim_prefix(text: &str) -> Option<String> {
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return Some(format!(r"\\{rest}"));
    }
    text.strip_prefix(r"\\?\").map(str::to_owned)
}

fn display_path(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

pub fn same_root(left: &Path, right: &Path) -> bool {
    normalized(left) == normalized(right)
}

/// 新旧缓存子目录互相嵌套时搬移会把目录搬进自身，直接拒绝
pub fn ensure_no_overlap(old_root: &Path, new_root: &Path) -> AppResult<()> {
    let old_root = normalized(old_root);
    let new_root = normalized(new_root);
    for old_sub in MEDIA_CACHE_SUBDIRS.map(|name| old_root.join(name)) {
        for new_sub in MEDIA_CACHE_SUBDIRS.map(|name| new_root.join(name)) {
            if old_sub.starts_with(&new_sub) || new_sub.starts_with(&old_sub) {
                return Err(AppError::Other("CACHE_DIR_OVERLAPS".into()));
            }
        }
    }
    Ok(())
}

/// 播放器正在读写、换目录时必须留在原处的缓存
#[derive(Debug, Default)]
pub struct CacheInUse {
    /// 本地文件会话正在读的文件
    pub files: Vec<PathBuf>,
    /// 远程流的缓存条目：.audio / .ready / 暂存 .part 都以 `<条目>.` 开头。
    /// 远程源按原路径重开已发布文件，且认为已落盘的区间不必重下，搬走或删掉会让后续读取卡住
    pub entries: Vec<String>,
}

struct InUseMatcher {
    files: HashSet<PathBuf>,
    entry_prefixes: Vec<String>,
}

impl InUseMatcher {
    fn new(in_use: &CacheInUse) -> Self {
        Self {
            files: in_use.files.iter().map(|path| normalized(path)).collect(),
            entry_prefixes: in_use
                .entries
                .iter()
                .filter(|entry| !entry.is_empty())
                .map(|entry| format!("{entry}."))
                .collect(),
        }
    }

    fn contains(&self, path: &Path) -> bool {
        let in_entry = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                self.entry_prefixes
                    .iter()
                    .any(|prefix| name.starts_with(prefix.as_str()))
            });
        in_entry || self.files.contains(&normalized(path))
    }
}

/// 处理旧根目录下的缓存：搬到新根目录或删除。只处理两个缓存子目录里认得出的缓存文件。
///
/// `in_use` 是播放器正在读写的缓存：会话 seek 时会按原路径重开，搬走或删掉它
/// 会让下一次 seek 和回滚都失败。这类文件搬移时只复制（新目录供以后播放），丢弃时保留
pub fn relocate(
    old_root: &Path,
    new_root: &Path,
    action: ExistingCacheAction,
    in_use: &CacheInUse,
) -> RelocationReport {
    let mut report = RelocationReport::default();
    if same_root(old_root, new_root) {
        return report;
    }
    let in_use = InUseMatcher::new(in_use);
    for name in MEDIA_CACHE_SUBDIRS {
        let old_sub = old_root.join(name);
        match action {
            ExistingCacheAction::Move => {
                move_tree(&old_sub, &new_root.join(name), &in_use, &mut report)
            }
            ExistingCacheAction::Discard => discard_tree(&old_sub, &in_use, &mut report),
        }
        remove_empty_dirs(&old_sub);
    }
    report
}

fn extension_in(path: &Path, set: &[&str]) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| set.contains(&extension.to_ascii_lowercase().as_str()))
}

fn move_tree(source: &Path, target: &Path, in_use: &InUseMatcher, report: &mut RelocationReport) {
    let Ok(entries) = fs::read_dir(source) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            move_tree(&path, &target.join(entry.file_name()), in_use, report);
        } else if file_type.is_file() && extension_in(&path, &MOVABLE_EXTENSIONS) {
            let target = target.join(entry.file_name());
            let result = if in_use.contains(&path) {
                copy_file(&path, &target)
            } else {
                move_file(&path, &target)
            };
            match result {
                Ok(bytes) => {
                    report.moved_files += 1;
                    report.moved_bytes = report.moved_bytes.saturating_add(bytes);
                }
                Err(error) => {
                    log::warn!(target: "storage", "搬移缓存文件失败 {}: {error}", path.display());
                    report.failed_count += 1;
                }
            }
        }
    }
}

/// 正在播放的文件：复制一份到新目录，源文件留给当前会话继续 seek
fn copy_file(source: &Path, target: &Path) -> io::Result<u64> {
    if target.exists() {
        return Ok(0);
    }
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "cache target has no parent"))?;
    fs::create_dir_all(parent)?;
    let mut staging = tempfile::Builder::new()
        .prefix(".copying-")
        .suffix(".part")
        .tempfile_in(parent)?;
    let copied = io::copy(&mut fs::File::open(source)?, staging.as_file_mut())?;
    staging.persist(target).map_err(|error| error.error)?;
    Ok(copied)
}

/// 同盘直接改名；跨盘先复制到暂存文件、核对长度后再改名，最后才删源文件
fn move_file(source: &Path, target: &Path) -> io::Result<u64> {
    let length = fs::metadata(source)?.len();
    if target.exists() {
        // 同名即同一缓存键的同一份内容，新目录里已有就只清掉旧的
        fs::remove_file(source)?;
        return Ok(0);
    }
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "cache target has no parent"))?;
    fs::create_dir_all(parent)?;
    if fs::rename(source, target).is_ok() {
        return Ok(length);
    }
    let mut staging = tempfile::Builder::new()
        .prefix(".moving-")
        .suffix(".part")
        .tempfile_in(parent)?;
    let copied = io::copy(&mut fs::File::open(source)?, staging.as_file_mut())?;
    if copied != length {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "cache file changed while moving",
        ));
    }
    staging.as_file().sync_all()?;
    staging.persist(target).map_err(|error| error.error)?;
    fs::remove_file(source)?;
    Ok(length)
}

fn discard_tree(source: &Path, in_use: &InUseMatcher, report: &mut RelocationReport) {
    let Ok(entries) = fs::read_dir(source) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            discard_tree(&path, in_use, report);
        } else if file_type.is_file()
            && (extension_in(&path, &MOVABLE_EXTENSIONS)
                || extension_in(&path, &STAGING_EXTENSIONS))
            && !in_use.contains(&path)
        {
            if fs::remove_file(&path).is_ok() {
                report.removed_files += 1;
            } else {
                report.failed_count += 1;
            }
        }
    }
}

/// 自底向上删除空目录；remove_dir 只删空目录，留下的非缓存文件会让目录保留
fn remove_empty_dirs(path: &Path) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            remove_empty_dirs(&entry.path());
        }
    }
    let _ = fs::remove_dir(path);
}

/// 同一时间只允许一个改目录操作；搬移可能要几分钟
pub struct RelocationGuard(());

impl RelocationGuard {
    pub fn acquire() -> AppResult<Self> {
        if RELOCATING.swap(true, Ordering::AcqRel) {
            return Err(AppError::Other("CACHE_DIR_BUSY".into()));
        }
        Ok(Self(()))
    }
}

impl Drop for RelocationGuard {
    fn drop(&mut self) {
        RELOCATING.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn verbatim_prefix_keeps_unc_roots_absolute() {
        // 回归：\\?\UNC\ 只去掉 \\?\ 会把网络共享变成相对路径，缓存写到进程工作目录
        assert_eq!(
            strip_verbatim_prefix(r"\\?\UNC\server\share\cache").as_deref(),
            Some(r"\\server\share\cache")
        );
        assert_eq!(
            strip_verbatim_prefix(r"\\?\D:\Music\cache").as_deref(),
            Some(r"D:\Music\cache")
        );
        assert_eq!(strip_verbatim_prefix("/Users/me/cache"), None);
    }

    #[test]
    fn custom_root_falls_back_to_default_when_unusable() {
        let temp = tempfile::tempdir().unwrap();
        let custom = temp.path().join("custom");
        let default = temp.path().join("default");
        assert_eq!(
            resolve_root(Some(&custom), || Some(default.clone())),
            Some(custom.clone())
        );
        assert!(custom.is_dir(), "自定义目录按需创建");
        let blocker = temp.path().join("file");
        write(&blocker, b"x");
        assert_eq!(
            resolve_root(Some(&blocker.join("nested")), || Some(default.clone())),
            Some(default.clone())
        );
        assert_eq!(resolve_root(None, || Some(default.clone())), Some(default));
    }

    #[cfg(unix)]
    #[test]
    fn custom_root_falls_back_when_existing_directory_turns_read_only() {
        // 回归：目录还在但变成只读时 create_dir_all 仍成功，曾继续用它导致封面缓存写入全部失败
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let custom = temp.path().join("custom");
        let default = temp.path().join("default");
        fs::create_dir_all(&custom).unwrap();
        fs::set_permissions(&custom, fs::Permissions::from_mode(0o555)).unwrap();
        let writable_anyway = fs::write(custom.join("root-check"), b"x").is_ok();
        let resolved = resolve_root(Some(&custom), || Some(default.clone()));
        fs::set_permissions(&custom, fs::Permissions::from_mode(0o755)).unwrap();
        if writable_anyway {
            // 以 root 运行时权限位不生效，无法构造只读目录
            return;
        }
        assert_eq!(resolved, Some(default));
    }

    #[test]
    fn prepare_root_requires_an_absolute_writable_directory() {
        let temp = tempfile::tempdir().unwrap();
        assert!(prepare_root(Path::new("relative/cache")).is_err());
        let file = temp.path().join("file");
        write(&file, b"x");
        assert!(prepare_root(&file).is_err());
        let created = prepare_root(&temp.path().join("new").join("cache")).unwrap();
        assert!(created.is_dir());
        assert_eq!(
            fs::read_dir(&created).unwrap().count(),
            0,
            "探测文件不能留下"
        );
    }

    #[test]
    fn nested_cache_directories_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        fs::create_dir_all(old.join(PLAYBACK_AUDIO_DIR)).unwrap();
        assert!(ensure_no_overlap(&old, &old.join(PLAYBACK_AUDIO_DIR)).is_err());
        assert!(ensure_no_overlap(&old.join(IMAGE_CACHE_DIR).join("x"), &old).is_err());
        assert!(ensure_no_overlap(&old, &temp.path().join("other")).is_ok());
        // 新目录选在旧根目录本身的其它子目录里是允许的
        assert!(ensure_no_overlap(&old, &old.join("elsewhere")).is_ok());
    }

    #[test]
    fn moving_keeps_cache_layout_and_leaves_unknown_files_alone() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        let new = temp.path().join("new");
        write(
            &old.join(PLAYBACK_AUDIO_DIR).join("ab").join("ab12.ready"),
            b"marker",
        );
        write(
            &old.join(PLAYBACK_AUDIO_DIR)
                .join("ab")
                .join("ab12.x.sha.audio"),
            b"audio-bytes",
        );
        write(
            &old.join(PLAYBACK_AUDIO_DIR).join("ab").join("ab34.part"),
            b"writing",
        );
        write(
            &old.join(IMAGE_CACHE_DIR)
                .join("cd")
                .join("cd12.x.sha.image"),
            b"img",
        );
        write(&old.join(IMAGE_CACHE_DIR).join("notes.txt"), b"user file");
        write(&old.join("unrelated.txt"), b"keep");
        // 新目录已有同一缓存键的文件时保留新目录那份
        write(
            &new.join(IMAGE_CACHE_DIR)
                .join("cd")
                .join("cd12.x.sha.image"),
            b"img",
        );

        let report = relocate(
            &old,
            &new,
            ExistingCacheAction::Move,
            &CacheInUse::default(),
        );
        assert_eq!(report.moved_files, 3);
        assert_eq!(
            report.moved_bytes,
            (b"marker".len() + b"audio-bytes".len()) as u64
        );
        assert_eq!(report.failed_count, 0);
        assert_eq!(
            fs::read(
                new.join(PLAYBACK_AUDIO_DIR)
                    .join("ab")
                    .join("ab12.x.sha.audio")
            )
            .unwrap(),
            b"audio-bytes"
        );
        assert!(new
            .join(PLAYBACK_AUDIO_DIR)
            .join("ab")
            .join("ab12.ready")
            .is_file());
        assert!(!old
            .join(PLAYBACK_AUDIO_DIR)
            .join("ab")
            .join("ab12.ready")
            .exists());
        assert!(
            !old.join(IMAGE_CACHE_DIR).join("cd").exists(),
            "搬空的分片目录被删除"
        );
        assert!(old
            .join(PLAYBACK_AUDIO_DIR)
            .join("ab")
            .join("ab34.part")
            .is_file());
        assert!(old.join(IMAGE_CACHE_DIR).join("notes.txt").is_file());
        assert!(old.join("unrelated.txt").is_file());
    }

    #[test]
    fn discarding_only_removes_cache_files() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        let new = temp.path().join("new");
        write(
            &old.join(PLAYBACK_AUDIO_DIR).join("ab").join("ab12.ready"),
            b"marker",
        );
        write(
            &old.join(PLAYBACK_AUDIO_DIR).join("ab").join("ab34.part"),
            b"writing",
        );
        write(&old.join(IMAGE_CACHE_DIR).join("notes.txt"), b"user file");
        write(
            &old.join("unrelated.audio"),
            b"outside the cache subdirectories",
        );

        let report = relocate(
            &old,
            &new,
            ExistingCacheAction::Discard,
            &CacheInUse::default(),
        );
        assert_eq!(report.removed_files, 2);
        assert_eq!(report.moved_files, 0);
        assert!(!old.join(PLAYBACK_AUDIO_DIR).exists());
        assert!(old.join(IMAGE_CACHE_DIR).join("notes.txt").is_file());
        assert!(old.join("unrelated.audio").is_file());
        assert!(!new.exists(), "丢弃旧缓存不会创建新目录里的任何东西");
    }

    #[test]
    fn files_in_use_by_the_player_stay_readable_at_their_old_path() {
        // 回归：搬移正在播放的缓存音频后，下一次 seek 按旧路径重开解码器失败
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        let new = temp.path().join("new");
        let playing = old.join(PLAYBACK_AUDIO_DIR).join("ab").join("ab12.audio");
        let idle = old.join(PLAYBACK_AUDIO_DIR).join("cd").join("cd34.audio");
        write(&playing, b"playing-bytes");
        write(&idle, b"idle-bytes");

        let report = relocate(
            &old,
            &new,
            ExistingCacheAction::Move,
            &CacheInUse {
                files: vec![playing.clone()],
                ..CacheInUse::default()
            },
        );
        assert_eq!(report.failed_count, 0);
        assert!(playing.is_file(), "正在播放的文件留在原处");
        assert_eq!(
            fs::read(new.join(PLAYBACK_AUDIO_DIR).join("ab").join("ab12.audio")).unwrap(),
            b"playing-bytes",
            "新目录里有一份副本供以后播放"
        );
        assert!(!idle.exists(), "其它文件照常搬走");
        assert!(new
            .join(PLAYBACK_AUDIO_DIR)
            .join("cd")
            .join("cd34.audio")
            .is_file());

        let discarded = relocate(
            &new,
            &old,
            ExistingCacheAction::Discard,
            &CacheInUse {
                files: vec![new.join(PLAYBACK_AUDIO_DIR).join("cd").join("cd34.audio")],
                ..CacheInUse::default()
            },
        );
        assert!(
            new.join(PLAYBACK_AUDIO_DIR)
                .join("cd")
                .join("cd34.audio")
                .is_file(),
            "丢弃时保留正在播放的文件"
        );
        assert!(!new
            .join(PLAYBACK_AUDIO_DIR)
            .join("ab")
            .join("ab12.audio")
            .exists());
        assert_eq!(discarded.removed_files, 1);
    }

    #[test]
    fn live_remote_stream_cache_entry_is_neither_moved_nor_discarded() {
        // 回归：远程流的 current_path 是 "__remote__"，曾照常删掉它的 .part、搬走它的 .audio，
        // 远程源随后按旧路径读、又把已落盘区间当成不用重下，播放和 seek 卡住
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        let new = temp.path().join("new");
        let shard = old.join(PLAYBACK_AUDIO_DIR).join("ab");
        let staging = shard.join("ab12.x7Qz.part");
        let published = shard.join("ab12.audio");
        let marker = shard.join("ab12.ready");
        let other = shard.join("ab99.audio");
        for path in [&staging, &published, &marker, &other] {
            write(path, b"bytes");
        }
        let live = CacheInUse {
            entries: vec!["ab12".into()],
            ..CacheInUse::default()
        };

        relocate(&old, &new, ExistingCacheAction::Move, &live);
        assert!(published.is_file() && marker.is_file() && staging.is_file());
        assert!(new
            .join(PLAYBACK_AUDIO_DIR)
            .join("ab")
            .join("ab12.audio")
            .is_file());
        assert!(!other.exists(), "其它条目照常搬走");

        let report = relocate(&old, &new, ExistingCacheAction::Discard, &live);
        assert_eq!(report.removed_files, 0);
        assert!(staging.is_file(), "丢弃时保留正在写的暂存文件");
        assert!(published.is_file());
    }

    #[test]
    fn relocating_onto_the_same_root_is_a_no_op() {
        let temp = tempfile::tempdir().unwrap();
        write(&temp.path().join(PLAYBACK_AUDIO_DIR).join("a.audio"), b"x");
        let report = relocate(
            temp.path(),
            temp.path(),
            ExistingCacheAction::Discard,
            &CacheInUse::default(),
        );
        assert_eq!(report, RelocationReport::default());
        assert!(temp
            .path()
            .join(PLAYBACK_AUDIO_DIR)
            .join("a.audio")
            .is_file());
    }

    #[test]
    fn only_one_relocation_runs_at_a_time() {
        let guard = RelocationGuard::acquire().unwrap();
        assert!(RelocationGuard::acquire().is_err());
        drop(guard);
        assert!(RelocationGuard::acquire().is_ok());
    }
}
