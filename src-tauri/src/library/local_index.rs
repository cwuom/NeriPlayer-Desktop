// 本地音乐库：用户添加的音乐文件夹 + 持久化曲目索引 + 文件系统监视
//
// - 启动时直接读索引出列表，不再每次整夹重扫读标签
// - 扫描按「大小 + 修改时间」指纹增量复用，只解析新增或改动过的文件
// - 监视走系统原生事件（Windows ReadDirectoryChangesW / macOS FSEvents / Linux inotify），
//   没有轮询；事件静默一段时间后才合并重扫，下载、批量拷贝时不会反复折腾
// - 网络盘等收不到事件的位置靠启动时的一次增量扫描兜底

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use super::scanner::{self, IndexedTrack, ScanSkipped};
use crate::error::{AppError, AppResult};
use crate::state::TrackInfo;

const INDEX_FILE: &str = "local-library.json";
const INDEX_VERSION: u32 = 1;
/// 最后一个文件事件之后静默这么久才重扫：拷贝一整张专辑只触发一次
const WATCH_SETTLE: Duration = Duration::from_millis(1500);
pub const CHANGED_EVENT: &str = "local-library-changed";
pub const PROGRESS_EVENT: &str = "local-library-progress";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalFolder {
    pub path: String,
    pub added_at: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct IndexFile {
    version: u32,
    folders: Vec<LocalFolder>,
    /// 文件夹路径 -> 该文件夹下的曲目
    entries: HashMap<String, Vec<IndexedTrack>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderSummary {
    pub path: String,
    pub added_at: i64,
    pub track_count: usize,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalLibraryTrack {
    #[serde(flatten)]
    track: TrackInfo,
    file_modified_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalLibrarySnapshot {
    pub folders: Vec<FolderSummary>,
    pub tracks: Vec<LocalLibraryTrack>,
    pub scanning: bool,
    pub skipped: Vec<ScanSkipped>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanProgress {
    folder: String,
    visited_entries: usize,
    tracks: usize,
    skipped: usize,
    current_path: String,
}

struct LocalLibrary {
    index: IndexFile,
    skipped: Vec<ScanSkipped>,
    name_template: Option<String>,
}

struct Runtime {
    library: Mutex<LocalLibrary>,
    scanning: AtomicBool,
    /// 扫描进行中又来了新的请求：结束后补一轮，保证最后一次改动一定被看到
    rescan_requested: AtomicBool,
    cancelled: AtomicBool,
    watcher: Mutex<Option<RecommendedWatcher>>,
    pending: Mutex<PendingChanges>,
    app: Mutex<Option<AppHandle>>,
}

#[derive(Default)]
struct PendingChanges {
    folders: HashSet<String>,
    /// 封面图片变了：这些目录下的曲目要丢掉缓存重新解析（指纹没变也要重新找封面）
    dirty_dirs: HashSet<PathBuf>,
    last_event: Option<Instant>,
}

fn runtime() -> &'static Arc<Runtime> {
    static RUNTIME: OnceLock<Arc<Runtime>> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        Arc::new(Runtime {
            library: Mutex::new(LocalLibrary { index: load_index(), skipped: Vec::new(), name_template: None }),
            scanning: AtomicBool::new(false),
            rescan_requested: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            watcher: Mutex::new(None),
            pending: Mutex::new(PendingChanges::default()),
            app: Mutex::new(None),
        })
    })
}

fn index_path() -> PathBuf {
    crate::db::user_data_dir().join(INDEX_FILE)
}

fn load_index() -> IndexFile {
    let path = index_path();
    let Ok(bytes) = std::fs::read(&path) else {
        return IndexFile { version: INDEX_VERSION, ..Default::default() };
    };
    match serde_json::from_slice::<IndexFile>(&bytes) {
        Ok(index) if index.version == INDEX_VERSION => index,
        Ok(_) => IndexFile { version: INDEX_VERSION, ..Default::default() },
        Err(error) => {
            // 索引只是缓存，坏了就重建；文件夹列表丢了用户还能再加，但先留现场
            let quarantined = crate::fsutil::quarantine_corrupt_file(&path);
            log::warn!(target: "local-library", "local library index unreadable ({error}), moved to {quarantined:?}");
            IndexFile { version: INDEX_VERSION, ..Default::default() }
        }
    }
}

fn save_index(index: &IndexFile) {
    match serde_json::to_vec(index) {
        Ok(bytes) => {
            if let Err(error) = crate::fsutil::atomic_write(index_path(), bytes) {
                log::warn!(target: "local-library", "save local library index failed: {error}");
            }
        }
        Err(error) => log::warn!(target: "local-library", "serialize local library index failed: {error}"),
    }
}

fn normalize_folder(path: &str) -> AppResult<String> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| AppError::Other(format!("文件夹不可访问: {path}: {error}")))?;
    if !canonical.is_dir() {
        return Err(AppError::Other(format!("不是文件夹: {path}")));
    }
    Ok(strip_verbatim(&canonical))
}

/// Windows canonicalize 会带 `\\?\` 前缀，展示和与播放路径比较时都去掉
fn strip_verbatim(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    text.strip_prefix(r"\\?\").map(str::to_string).unwrap_or_else(|| text.to_string())
}

fn snapshot_of(runtime: &Runtime) -> LocalLibrarySnapshot {
    let library = runtime.library.lock();
    let mut seen = HashSet::new();
    let mut tracks = Vec::new();
    let mut folders = Vec::new();
    for folder in &library.index.folders {
        let entries = library.index.entries.get(&folder.path);
        folders.push(FolderSummary {
            path: folder.path.clone(),
            added_at: folder.added_at,
            track_count: entries.map_or(0, Vec::len),
            available: Path::new(&folder.path).is_dir(),
        });
        // 嵌套添加的文件夹（父子都加了）同一文件只出现一次
        for entry in entries.into_iter().flatten() {
            if seen.insert(entry.track.url.clone()) {
                tracks.push(LocalLibraryTrack {
                    track: entry.track.clone(),
                    file_modified_ms: entry.stamp.modified_ms,
                });
            }
        }
    }
    LocalLibrarySnapshot {
        folders,
        tracks,
        scanning: runtime.scanning.load(Ordering::Acquire),
        skipped: library.skipped.clone(),
    }
}

fn emit_changed(runtime: &Runtime) {
    if let Some(app) = runtime.app.lock().clone() {
        let _ = app.emit(CHANGED_EVENT, ());
    }
}

/// 在后台线程上增量扫描指定文件夹（None = 全部），扫描期间新的请求合并为结束后补一轮
fn spawn_scan(runtime: Arc<Runtime>, only: Option<HashSet<String>>, dirty_dirs: HashSet<PathBuf>) {
    if runtime.scanning.swap(true, Ordering::AcqRel) {
        runtime.rescan_requested.store(true, Ordering::Release);
        if !dirty_dirs.is_empty() || only.is_some() {
            let mut pending = runtime.pending.lock();
            pending.dirty_dirs.extend(dirty_dirs);
            pending.folders.extend(only.unwrap_or_default());
        }
        return;
    }
    runtime.cancelled.store(false, Ordering::Release);
    emit_changed(&runtime);
    std::thread::Builder::new()
        .name("local-library-scan".into())
        .spawn(move || {
            run_scan(&runtime, only, dirty_dirs);
            loop {
                runtime.scanning.store(false, Ordering::Release);
                emit_changed(&runtime);
                if !runtime.rescan_requested.swap(false, Ordering::AcqRel) {
                    break;
                }
                if runtime.scanning.swap(true, Ordering::AcqRel) {
                    break;
                }
                let (folders, dirty) = {
                    let mut pending = runtime.pending.lock();
                    (std::mem::take(&mut pending.folders), std::mem::take(&mut pending.dirty_dirs))
                };
                run_scan(&runtime, (!folders.is_empty()).then_some(folders), dirty);
            }
        })
        .ok();
}

fn run_scan(runtime: &Runtime, only: Option<HashSet<String>>, dirty_dirs: HashSet<PathBuf>) {
    let (targets, template) = {
        let library = runtime.library.lock();
        let targets: Vec<String> = library
            .index
            .folders
            .iter()
            .map(|folder| folder.path.clone())
            .filter(|path| only.as_ref().is_none_or(|set| set.contains(path)))
            .collect();
        (targets, library.name_template.clone())
    };
    let app = runtime.app.lock().clone();
    let mut skipped_all = Vec::new();
    for folder in targets {
        if runtime.cancelled.load(Ordering::Acquire) {
            break;
        }
        let previous: HashMap<String, IndexedTrack> = {
            let library = runtime.library.lock();
            library
                .index
                .entries
                .get(&folder)
                .into_iter()
                .flatten()
                .filter(|entry| {
                    let parent = Path::new(&entry.track.url).parent();
                    !parent.is_some_and(|parent| dirty_dirs.iter().any(|dirty| paths_equal(dirty, parent)))
                })
                .map(|entry| (canonical_key(&entry.track.url), entry.clone()))
                .collect()
        };
        let folder_for_progress = folder.clone();
        let result = scanner::scan_directory_indexed(
            &folder,
            template.as_deref(),
            &runtime.cancelled,
            &previous,
            |visited, tracks, skipped, path| {
                if let Some(app) = &app {
                    let _ = app.emit(PROGRESS_EVENT, ScanProgress {
                        folder: folder_for_progress.clone(),
                        visited_entries: visited,
                        tracks,
                        skipped,
                        current_path: path.display().to_string(),
                    });
                }
            },
        );
        match result {
            Ok(scan) => {
                let mut library = runtime.library.lock();
                // 扫描期间文件夹被移除了就丢弃结果
                if library.index.folders.iter().any(|item| item.path == folder) {
                    let entries = scan
                        .entries
                        .into_iter()
                        .map(|mut entry| {
                            entry.track.url = strip_verbatim(Path::new(&entry.track.url));
                            entry.track.id = format!("local:{}", entry.track.url);
                            entry
                        })
                        .collect();
                    library.index.entries.insert(folder.clone(), entries);
                    save_index(&library.index);
                }
                if scan.parsed > 0 {
                    log::info!(target: "local-library", "rescanned {folder}: {} file(s) parsed", scan.parsed);
                }
                skipped_all.extend(scan.skipped);
            }
            Err(error) => {
                // 文件夹暂时不可用（移动硬盘没插、网络盘断开）：保留上次索引，不清空
                log::warn!(target: "local-library", "scan {folder} failed: {error}");
                skipped_all.push(ScanSkipped { path: folder.clone(), reason: error.to_string() });
            }
        }
    }
    runtime.library.lock().skipped = skipped_all;
}

fn canonical_key(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|canonical| canonical.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string())
}

/// Windows、macOS 默认文件系统不分大小写，Linux 分：/Music/A.flac 与 /Music/a.flac 是两首歌
const CASE_INSENSITIVE_PATHS: bool = cfg!(any(windows, target_os = "macos"));

fn paths_equal(left: &Path, right: &Path) -> bool {
    let (left, right) = (strip_verbatim(left), strip_verbatim(right));
    if CASE_INSENSITIVE_PATHS {
        left.to_lowercase() == right.to_lowercase()
    } else {
        left == right
    }
}

fn folder_of(runtime: &Runtime, path: &Path) -> Option<String> {
    let text = strip_verbatim(path);
    let library = runtime.library.lock();
    library
        .index
        .folders
        .iter()
        .filter(|folder| starts_with_folder(&text, &folder.path))
        .max_by_key(|folder| folder.path.len())
        .map(|folder| folder.path.clone())
}

fn starts_with_folder(path: &str, folder: &str) -> bool {
    let (path, folder) = if CASE_INSENSITIVE_PATHS {
        (path.to_lowercase(), folder.to_lowercase())
    } else {
        (path.to_string(), folder.to_string())
    };
    // 去掉根目录和目录末尾的分隔符，Unix 文件名里的反斜杠仍需保留
    let folder = folder.trim_end_matches(std::path::is_separator);
    path == folder
        || path.strip_prefix(folder).is_some_and(|rest| rest.starts_with(std::path::is_separator))
}

fn on_fs_event(result: notify::Result<notify::Event>) {
    let Ok(event) = result else { return };
    if matches!(event.kind, notify::EventKind::Access(_)) {
        return;
    }
    let runtime = runtime();
    let mut touched = false;
    for path in &event.paths {
        let relevant = scanner::is_audio_path(path)
            || scanner::is_image_path(path)
            // 目录增删改名没有扩展名
            || path.extension().is_none();
        if !relevant || path.components().any(|part| part.as_os_str() == ".tmp") {
            continue;
        }
        let Some(folder) = folder_of(runtime, path) else { continue };
        let mut pending = runtime.pending.lock();
        pending.folders.insert(folder);
        if scanner::is_image_path(path) {
            if let Some(parent) = path.parent() {
                pending.dirty_dirs.insert(parent.to_path_buf());
                // 封面子目录（Covers/）变了影响的是上一级目录的曲目
                if let Some(grand) = parent.parent() {
                    pending.dirty_dirs.insert(grand.to_path_buf());
                }
            }
        }
        pending.last_event = Some(Instant::now());
        touched = true;
    }
    if touched {
        ensure_settle_thread();
    }
}

/// 事件静默 WATCH_SETTLE 后把累计的变化交给一次扫描
fn ensure_settle_thread() {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::Builder::new()
        .name("local-library-settle".into())
        .spawn(|| {
            let runtime = runtime();
            loop {
                std::thread::sleep(WATCH_SETTLE / 3);
                let ready = {
                    let mut pending = runtime.pending.lock();
                    match pending.last_event {
                        Some(at) if at.elapsed() >= WATCH_SETTLE => {
                            pending.last_event = None;
                            Some((std::mem::take(&mut pending.folders), std::mem::take(&mut pending.dirty_dirs)))
                        }
                        Some(_) => None,
                        None => {
                            RUNNING.store(false, Ordering::Release);
                            return;
                        }
                    }
                };
                if let Some((folders, dirty)) = ready {
                    if !folders.is_empty() {
                        spawn_scan(Arc::clone(runtime), Some(folders), dirty);
                    }
                }
            }
        })
        .ok();
}

fn rebuild_watcher(runtime: &Runtime) {
    let folders: Vec<String> = runtime.library.lock().index.folders.iter().map(|folder| folder.path.clone()).collect();
    let mut slot = runtime.watcher.lock();
    *slot = None;
    if folders.is_empty() {
        return;
    }
    let mut watcher = match notify::recommended_watcher(on_fs_event) {
        Ok(watcher) => watcher,
        Err(error) => {
            log::warn!(target: "local-library", "file watcher unavailable: {error}");
            return;
        }
    };
    for folder in &folders {
        if let Err(error) = watcher.watch(Path::new(folder), RecursiveMode::Recursive) {
            log::warn!(target: "local-library", "watch {folder} failed: {error}");
        }
    }
    *slot = Some(watcher);
}

/// 应用启动时调用：接上事件出口、开始监视，并在后台做一次增量校对
pub fn start(app: AppHandle) {
    let runtime = runtime();
    *runtime.app.lock() = Some(app);
    rebuild_watcher(runtime);
    if !runtime.library.lock().index.folders.is_empty() {
        spawn_scan(Arc::clone(runtime), None, HashSet::new());
    }
}

pub fn snapshot() -> LocalLibrarySnapshot {
    snapshot_of(runtime())
}

pub fn set_name_template(template: Option<String>) {
    runtime().library.lock().name_template = template.filter(|value| !value.trim().is_empty());
}

pub fn add_folder(path: &str) -> AppResult<LocalLibrarySnapshot> {
    let normalized = normalize_folder(path)?;
    let runtime = runtime();
    {
        let mut library = runtime.library.lock();
        if library.index.folders.iter().any(|folder| folder.path == normalized) {
            drop(library);
            return Ok(snapshot_of(runtime));
        }
        library.index.folders.push(LocalFolder {
            path: normalized.clone(),
            added_at: chrono::Utc::now().timestamp_millis(),
        });
        save_index(&library.index);
    }
    rebuild_watcher(runtime);
    spawn_scan(Arc::clone(runtime), Some(HashSet::from([normalized])), HashSet::new());
    Ok(snapshot_of(runtime))
}

pub fn remove_folder(path: &str) -> LocalLibrarySnapshot {
    let runtime = runtime();
    {
        let mut library = runtime.library.lock();
        library.index.folders.retain(|folder| folder.path != path);
        library.index.entries.remove(path);
        save_index(&library.index);
    }
    rebuild_watcher(runtime);
    emit_changed(runtime);
    snapshot_of(runtime)
}

/// `full`：作废全部指纹强制重读标签，给外部批量改了标签却没改修改时间的情况用
pub fn rescan(full: bool) -> LocalLibrarySnapshot {
    let runtime = runtime();
    if full {
        let mut library = runtime.library.lock();
        for entries in library.index.entries.values_mut() {
            for entry in entries.iter_mut() {
                entry.stamp.modified_ms = -1;
            }
        }
    }
    spawn_scan(Arc::clone(runtime), None, HashSet::new());
    snapshot_of(runtime)
}

pub fn cancel() {
    runtime().cancelled.store(true, Ordering::Release);
}

/// 标签编辑后同步更新索引里的那一条，不必等监视器回调
pub fn update_track_tags(file_path: &str, title: &str, artist: &str, album: &str) {
    let runtime = runtime();
    let mut library = runtime.library.lock();
    let mut changed = false;
    for entries in library.index.entries.values_mut() {
        for entry in entries.iter_mut().filter(|entry| paths_equal(Path::new(&entry.track.url), Path::new(file_path))) {
            entry.track.title = title.to_string();
            entry.track.artist = artist.to_string();
            entry.track.album = album.to_string();
            if let Ok(metadata) = std::fs::metadata(file_path) {
                entry.stamp = scanner::FileStamp::of(&metadata);
            }
            changed = true;
        }
    }
    if changed {
        save_index(&library.index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_exposes_each_files_modification_time_without_changing_track_metadata() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().to_string_lossy().to_string();
        let track = |name: &str| TrackInfo {
            id: format!("local:{name}"),
            title: name.into(),
            artist: "Artist".into(),
            album: "Album".into(),
            duration_ms: 1000,
            source: crate::state::TrackSource::Local,
            url: root.path().join(name).to_string_lossy().to_string(),
            cover_url: None,
            added_at: 7,
            sync_payload: None,
            playlist_key: None,
        };
        let runtime = Runtime {
            library: Mutex::new(LocalLibrary {
                index: IndexFile {
                    version: INDEX_VERSION,
                    folders: vec![LocalFolder { path: folder.clone(), added_at: 1 }],
                    entries: HashMap::from([(folder, vec![
                        IndexedTrack { track: track("a.flac"), stamp: scanner::FileStamp { size: 10, modified_ms: 1234 } },
                        IndexedTrack { track: track("b.flac"), stamp: scanner::FileStamp { size: 20, modified_ms: 5678 } },
                    ])]),
                },
                skipped: Vec::new(),
                name_template: None,
            }),
            scanning: AtomicBool::new(false),
            rescan_requested: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            watcher: Mutex::new(None),
            pending: Mutex::new(PendingChanges::default()),
            app: Mutex::new(None),
        };

        let snapshot = serde_json::to_value(snapshot_of(&runtime)).unwrap();
        assert_eq!(snapshot["tracks"][0]["id"], "local:a.flac");
        assert_eq!(snapshot["tracks"][0]["file_modified_ms"], 1234);
        assert_eq!(snapshot["tracks"][1]["id"], "local:b.flac");
        assert_eq!(snapshot["tracks"][1]["file_modified_ms"], 5678);
        assert_eq!(snapshot["tracks"][0]["duration_ms"], 1000);
        assert_eq!(snapshot["tracks"][0]["added_at"], 7);
        assert!(snapshot["tracks"][0].get("track").is_none(), "track metadata stays flattened for existing clients");
    }

    #[test]
    fn folder_prefix_requires_a_separator_boundary() {
        assert!(starts_with_folder("/music/a/song.mp3", "/music/a"));
        assert!(starts_with_folder("/music/a", "/music/a"));
        assert!(!starts_with_folder("/music/ab/song.mp3", "/music/a"));
    }

    #[test]
    fn folder_prefix_matches_root_and_trailing_separators() {
        assert!(starts_with_folder("/music/song.mp3", "/"));
        assert!(starts_with_folder("/", "/"));
        assert!(starts_with_folder("/music/song.mp3", "/music/"));
        assert!(!starts_with_folder("/music-other/song.mp3", "/music/"));
    }

    #[cfg(windows)]
    #[test]
    fn folder_prefix_matches_windows_drive_and_unc_roots() {
        assert!(starts_with_folder(r"D:\Music\song.mp3", r"D:\"));
        assert!(starts_with_folder(r"d:\music\song.mp3", r"D:\Music\"));
        assert!(starts_with_folder(r"\\nas\music\song.mp3", r"\\nas\music\"));
        assert!(!starts_with_folder(r"D:\Musical\song.mp3", r"D:\Music\"));
    }

    #[cfg(not(windows))]
    #[test]
    fn folder_prefix_preserves_literal_backslashes_in_unix_names() {
        assert!(starts_with_folder(r"/music\/song.mp3", r"/music\"));
        assert!(!starts_with_folder("/music/song.mp3", r"/music\"));
        assert!(!starts_with_folder(r"/music\other/song.mp3", "/music"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_path_comparisons_preserve_case() {
        assert!(paths_equal(Path::new("/Music/A.flac"), Path::new("/Music/A.flac")));
        assert!(!paths_equal(Path::new("/Music/A.flac"), Path::new("/Music/a.flac")));
        assert!(!starts_with_folder("/Music/song.mp3", "/music"));
    }

    #[test]
    fn verbatim_prefix_is_removed_for_display() {
        assert_eq!(strip_verbatim(Path::new(r"\\?\D:\Music")), r"D:\Music");
        assert_eq!(strip_verbatim(Path::new(r"\\?\UNC\nas\music")), r"\\nas\music");
        assert_eq!(strip_verbatim(Path::new("/Users/me/Music")), "/Users/me/Music");
    }

    #[test]
    fn incremental_scan_reuses_unchanged_files_and_drops_removed_ones() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("a.aac");
        let second = root.path().join("b.aac");
        std::fs::write(&first, include_bytes!("../audio/fixtures/hls-silence.aac")).unwrap();
        std::fs::write(&second, include_bytes!("../audio/fixtures/hls-silence.aac")).unwrap();
        let dir = root.path().to_str().unwrap();
        let never = AtomicBool::new(false);

        let initial = scanner::scan_directory_indexed(dir, None, &never, &HashMap::new(), |_, _, _, _| {}).unwrap();
        assert_eq!(initial.entries.len(), 2);
        assert_eq!(initial.parsed, 2);

        let previous: HashMap<String, IndexedTrack> = initial
            .entries
            .iter()
            .map(|entry| (entry.track.url.clone(), entry.clone()))
            .collect();
        let again = scanner::scan_directory_indexed(dir, None, &never, &previous, |_, _, _, _| {}).unwrap();
        assert_eq!(again.entries.len(), 2);
        assert_eq!(again.parsed, 0, "指纹没变的文件不应重新解析");

        std::fs::remove_file(&second).unwrap();
        let after_delete = scanner::scan_directory_indexed(dir, None, &never, &previous, |_, _, _, _| {}).unwrap();
        assert_eq!(after_delete.entries.len(), 1);
        assert_eq!(after_delete.parsed, 0);
    }
}
