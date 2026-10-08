use crate::error::AppResult;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Copy, Default)]
struct FileStats {
    size_bytes: u64,
    file_count: u64,
}

impl std::ops::AddAssign for FileStats {
    fn add_assign(&mut self, rhs: Self) {
        self.size_bytes = self.size_bytes.saturating_add(rhs.size_bytes);
        self.file_count = self.file_count.saturating_add(rhs.file_count);
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsageItem {
    pub id: String,
    pub size_bytes: u64,
    pub file_count: u64,
    pub path: Option<String>,
    pub cache_kind: Option<String>,
    /// 存在用户数据库里的条目按记录数展示（对齐 Android databaseRecordCount）
    pub record_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsageSection {
    pub id: String,
    pub items: Vec<StorageUsageItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsageSummary {
    pub sections: Vec<StorageUsageSection>,
    pub total_size_bytes: u64,
    pub total_file_count: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StorageCacheClearOptions {
    pub audio_cache: bool,
    pub image_cache: bool,
    pub download_staging: bool,
    pub shared_media: bool,
    pub platform_list: bool,
    pub lyrics_cache: bool,
}

impl Default for StorageCacheClearOptions {
    fn default() -> Self {
        Self {
            audio_cache: true,
            image_cache: true,
            download_staging: false,
            shared_media: false,
            platform_list: false,
            lyrics_cache: false,
        }
    }
}

/// WebView2 配置目录（localStorage、IndexedDB、Cookie）与调试版凭据库都放在缓存目录里，
/// 它们是应用数据而不是可清理缓存
fn webview_data_paths(app_cache_dir: Option<&Path>) -> Vec<PathBuf> {
    let Some(root) = app_cache_dir else {
        return Vec::new();
    };
    let mut paths = vec![root.join("EBWebView")];
    if let Ok(entries) = fs::read_dir(root) {
        paths.extend(entries.flatten().map(|entry| entry.path()).filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("debug-"))
        }));
    }
    paths
}

/// 下载目录里未完成的 `.part` / `.reserve` 文件属于下载暂存，不是已下载的音乐
fn is_download_staging_file(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "part" | "reserve"))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageClearResult {
    pub cleared_bytes: u64,
    pub deleted_files: u64,
    pub failed_count: u64,
}

#[tauri::command]
pub async fn get_storage_usage(
    app: AppHandle,
    download_dir: Option<String>,
) -> AppResult<StorageUsageSummary> {
    let app_data_dir = app.path().app_data_dir().ok();
    let app_cache_dir = app.path().app_cache_dir().ok();
    let default_download_dir = app_data_dir.as_ref().map(|dir| dir.join("downloads"));
    let download_roots = download_roots(default_download_dir.as_deref(), download_dir.as_deref());

    let playback_cache = app_cache_dir.as_ref().map(|dir| dir.join("playback-audio"));
    let image_cache = child_paths(
        app_data_dir.as_deref(),
        &["covers", "thumbnails"],
    );
    let image_cache = append_child_path(
        image_cache,
        app_cache_dir.as_deref(),
        "image_cache",
    );
    let download_staging = child_paths(app_data_dir.as_deref(), &["temp"]);
    let download_staging = append_child_path(
        download_staging,
        app_cache_dir.as_deref(),
        "download_staging",
    );
    let shared_media = child_paths(app_data_dir.as_deref(), &["shared_media"]);
    let shared_media = append_child_path(
        shared_media,
        app_cache_dir.as_deref(),
        "shared_media_exports",
    );
    let platform_list = child_paths(
        app_data_dir.as_deref(),
        &[
            "netease_playlist_cache",
            "bili_favorite_cache",
            "youtube_music_playlist_cache",
        ],
    );
    let webview_data = webview_data_paths(app_cache_dir.as_deref());
    let mut classified_cache_paths = image_cache.clone();
    classified_cache_paths.extend(download_staging.iter().cloned());
    classified_cache_paths.extend(shared_media.iter().cloned());
    classified_cache_paths.extend(platform_list.iter().cloned());
    classified_cache_paths.extend(webview_data.iter().cloned());
    if let Some(path) = playback_cache.as_ref() {
        classified_cache_paths.push(path.clone());
    }

    // 数据库内的缓存与下载索引按估算字节归属到各自条目，剩余部分才计入用户数据库
    let user_database = crate::db::database_files(
        &crate::db::user_data_dir().join(crate::db::DATABASE_FILE),
    );
    let database_stats = user_database.iter().fold(FileStats::default(), |mut total, path| {
        total += stats_for_path(path, &[]);
        total
    });
    let mut unattributed = database_stats.size_bytes;
    let mut attribute = |(bytes, records): (u64, u64)| {
        let bytes = bytes.min(unattributed);
        unattributed -= bytes;
        (bytes, records)
    };
    let (detail_bytes, detail_records) =
        attribute(database_usage(|connection| crate::db::cache::usage(connection, crate::db::cache::PLATFORM_DETAIL_BUCKET)));
    let (lyrics_bytes, lyrics_records) =
        attribute(database_usage(|connection| crate::db::cache::usage(connection, crate::db::cache::LYRICS_BUCKET)));
    let (index_bytes, index_records) = attribute(database_usage(super::download_cmd::catalog::usage));

    let downloads = scan_download_roots(&download_roots);
    let platform_list_files = aggregate_item("platform_list_cache", &platform_list, Some("platform_list"));
    let staging_files = aggregate_item("download_staging", &download_staging, Some("download_staging"));
    let other_cache = other_cache_item(app_cache_dir.as_deref(), &classified_cache_paths);
    let mut sections = vec![StorageUsageSection {
        id: "cleanable_cache".into(),
        items: vec![
            usage_item("audio_cache", playback_cache.as_deref(), Some("audio")),
            aggregate_item("image_cache", &image_cache, Some("image")),
            StorageUsageItem {
                size_bytes: staging_files.size_bytes.saturating_add(downloads.staging.size_bytes),
                file_count: staging_files.file_count.saturating_add(downloads.staging.file_count),
                ..staging_files
            },
            aggregate_item("shared_media", &shared_media, Some("shared_media")),
            StorageUsageItem {
                size_bytes: platform_list_files.size_bytes.saturating_add(detail_bytes),
                record_count: Some(detail_records),
                ..platform_list_files
            },
            StorageUsageItem {
                cache_kind: Some("lyrics".into()),
                ..database_item("lyrics_cache", lyrics_bytes, lyrics_records)
            },
            other_cache,
        ],
    }];

    sections.push(StorageUsageSection {
        id: "downloads".into(),
        items: vec![
            stats_item("downloaded_music", downloads.music, None, None),
            stats_item("downloaded_lyrics", downloads.lyrics, None, None),
            database_item("download_index", index_bytes, index_records),
        ],
    });

    let diagnostics = child_paths(
        app_data_dir.as_deref(),
        &["logs", "crashes", "error_logs", "crash-reports"],
    );
    sections.push(StorageUsageSection {
        id: "diagnostics".into(),
        items: vec![
            aggregate_item("logs", &diagnostics[..2.min(diagnostics.len())], None),
            aggregate_item(
                "crash_logs",
                &diagnostics[2.min(diagnostics.len())..],
                None,
            ),
        ],
    });

    let local_covers = app_data_dir.as_ref().map(|dir| dir.join("local-covers"));
    let custom_background = app_data_dir.as_ref().map(|dir| dir.join("background"));
    let known_data = known_data_paths(
        &image_cache,
        &download_staging,
        &shared_media,
        &platform_list,
        &download_roots,
        local_covers.as_deref(),
        custom_background.as_deref(),
        &user_database,
    );
    sections.push(StorageUsageSection {
        id: "app_data".into(),
        items: vec![
            usage_item("local_covers", local_covers.as_deref(), None),
            usage_item("custom_background", custom_background.as_deref(), None),
            aggregate_item("webview_data", &webview_data, None),
            // 已归属到缓存与下载索引的页面不再重复计入用户数据库，合计才不会双算
            StorageUsageItem {
                size_bytes: unattributed,
                ..stats_item(
                    "playlist_data",
                    database_stats,
                    Some(user_database[0].to_string_lossy().into_owned()),
                    None,
                )
            },
            other_app_data_item(app_data_dir.as_deref(), &known_data),
        ],
    });

    let total_size_bytes = sections
        .iter()
        .flat_map(|section| section.items.iter())
        .map(|item| item.size_bytes)
        .sum();
    let total_file_count = sections
        .iter()
        .flat_map(|section| section.items.iter())
        .map(|item| item.file_count)
        .sum();

    Ok(StorageUsageSummary {
        sections,
        total_size_bytes,
        total_file_count,
    })
}

#[tauri::command]
pub async fn clear_storage_cache(
    app: AppHandle,
    options: StorageCacheClearOptions,
    download_dir: Option<String>,
) -> AppResult<StorageClearResult> {
    let app_data_dir = app.path().app_data_dir().ok();
    let app_cache_dir = app.path().app_cache_dir().ok();
    let playback_cache = app_cache_dir.as_ref().map(|dir| dir.join("playback-audio"));
    let image_cache = append_child_path(
        child_paths(app_data_dir.as_deref(), &["covers", "thumbnails"]),
        app_cache_dir.as_deref(),
        "image_cache",
    );
    let download_staging = append_child_path(
        child_paths(app_data_dir.as_deref(), &["temp"]),
        app_cache_dir.as_deref(),
        "download_staging",
    );
    let shared_media = append_child_path(
        child_paths(app_data_dir.as_deref(), &["shared_media"]),
        app_cache_dir.as_deref(),
        "shared_media_exports",
    );
    let platform_list = child_paths(
        app_data_dir.as_deref(),
        &[
            "netease_playlist_cache",
            "bili_favorite_cache",
            "youtube_music_playlist_cache",
        ],
    );

    let mut targets = Vec::new();
    if options.audio_cache {
        targets.extend(playback_cache);
    }
    if options.image_cache {
        targets.extend(image_cache);
    }
    if options.download_staging {
        targets.extend(download_staging);
    }
    if options.shared_media {
        targets.extend(shared_media);
    }
    if options.platform_list {
        targets.extend(platform_list);
    }

    let mut result = StorageClearResult {
        cleared_bytes: 0,
        deleted_files: 0,
        failed_count: 0,
    };
    let mut seen = HashSet::new();
    for target in targets {
        let key = canonical_key(&target);
        if !seen.insert(key) {
            continue;
        }
        let before = stats_for_path(&target, &[]);
        if before.file_count == 0 && before.size_bytes == 0 {
            continue;
        }
        // 删不掉的条目（被占用、无权限）跳过继续，按实际释放的字节统计
        let failed = clear_directory_contents(&target);
        let after = stats_for_path(&target, &[]);
        result.cleared_bytes = result.cleared_bytes.saturating_add(before.size_bytes.saturating_sub(after.size_bytes));
        result.deleted_files = result.deleted_files.saturating_add(before.file_count.saturating_sub(after.file_count));
        result.failed_count = result.failed_count.saturating_add(failed);
    }

    if options.download_staging {
        let default_download_dir = app_data_dir.as_ref().map(|dir| dir.join("downloads"));
        for root in download_roots(default_download_dir.as_deref(), download_dir.as_deref()) {
            remove_download_staging_files(&root, &mut result);
        }
    }

    let mut buckets = Vec::new();
    if options.platform_list {
        buckets.push(crate::db::cache::PLATFORM_DETAIL_BUCKET);
    }
    if options.lyrics_cache {
        buckets.push(crate::db::cache::LYRICS_BUCKET);
    }
    for bucket in buckets {
        let cleared = crate::db::user_db()
            .and_then(|database| database.write(|transaction| crate::db::cache::clear(transaction, bucket)));
        match cleared {
            Ok((bytes, records)) => {
                result.cleared_bytes = result.cleared_bytes.saturating_add(bytes);
                result.deleted_files = result.deleted_files.saturating_add(records);
            }
            Err(error) => {
                log::warn!(target: "storage", "清除数据库缓存 {bucket} 失败: {error}");
                result.failed_count = result.failed_count.saturating_add(1);
            }
        }
    }

    Ok(result)
}

fn remove_download_staging_files(path: &Path, result: &mut StorageClearResult) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let child = entry.path();
        if child.is_dir() {
            remove_download_staging_files(&child, result);
        } else if is_download_staging_file(&child) {
            let size = fs::metadata(&child).map(|meta| meta.len()).unwrap_or(0);
            if fs::remove_file(&child).is_ok() {
                result.cleared_bytes = result.cleared_bytes.saturating_add(size);
                result.deleted_files = result.deleted_files.saturating_add(1);
            } else {
                result.failed_count = result.failed_count.saturating_add(1);
            }
        }
    }
}

fn usage_item(id: &str, path: Option<&Path>, cache_kind: Option<&str>) -> StorageUsageItem {
    let stats = path.map(|value| stats_for_path(value, &[])).unwrap_or_default();
    stats_item(
        id,
        stats,
        path.map(|value| value.to_string_lossy().to_string()),
        cache_kind,
    )
}

fn aggregate_item(id: &str, paths: &[PathBuf], cache_kind: Option<&str>) -> StorageUsageItem {
    let stats = paths.iter().fold(FileStats::default(), |mut total, path| {
        total += stats_for_path(path, &[]);
        total
    });
    let path = (!paths.is_empty()).then(|| {
        paths
            .iter()
            .map(|value| value.to_string_lossy())
            .collect::<Vec<_>>()
            .join("\n")
    });
    stats_item(id, stats, path, cache_kind)
}

fn stats_item(
    id: &str,
    stats: FileStats,
    path: Option<String>,
    cache_kind: Option<&str>,
) -> StorageUsageItem {
    StorageUsageItem {
        id: id.into(),
        size_bytes: stats.size_bytes,
        file_count: stats.file_count,
        path,
        cache_kind: cache_kind.map(str::to_string),
        record_count: None,
    }
}

fn database_item(id: &str, size_bytes: u64, record_count: u64) -> StorageUsageItem {
    StorageUsageItem {
        record_count: Some(record_count),
        ..stats_item(id, FileStats { size_bytes, file_count: 0 }, None, None)
    }
}

/// 用户数据库内某类数据的估算占用与记录数；数据库不可用时按空计
fn database_usage(
    measure: impl FnOnce(&rusqlite::Connection) -> AppResult<(u64, u64)>,
) -> (u64, u64) {
    crate::db::user_db()
        .and_then(|database| database.read(measure))
        .unwrap_or_else(|error| {
            log::warn!(target: "storage", "数据库占用统计失败: {error}");
            (0, 0)
        })
}

fn other_cache_item(app_cache_dir: Option<&Path>, excluded: &[PathBuf]) -> StorageUsageItem {
    let stats = app_cache_dir
        .map(|path| stats_for_path(path, excluded))
        .unwrap_or_default();
    stats_item(
        "other_cache",
        stats,
        app_cache_dir.map(|path| path.to_string_lossy().to_string()),
        None,
    )
}

fn other_app_data_item(app_data_dir: Option<&Path>, excluded: &[PathBuf]) -> StorageUsageItem {
    let stats = app_data_dir
        .map(|path| stats_for_path(path, excluded))
        .unwrap_or_default();
    stats_item(
        "app_data",
        stats,
        app_data_dir.map(|path| path.to_string_lossy().to_string()),
        None,
    )
}

fn child_paths(root: Option<&Path>, names: &[&str]) -> Vec<PathBuf> {
    root.into_iter()
        .flat_map(|path| names.iter().map(move |name| path.join(name)))
        .collect()
}

fn append_child_path(mut paths: Vec<PathBuf>, root: Option<&Path>, name: &str) -> Vec<PathBuf> {
    if let Some(path) = root {
        paths.push(path.join(name));
    }
    paths
}

fn stats_for_path(path: &Path, excluded: &[PathBuf]) -> FileStats {
    if !path.exists() || is_excluded(path, excluded) {
        return FileStats::default();
    }
    if path.is_file() {
        return FileStats {
            size_bytes: fs::metadata(path).map(|meta| meta.len()).unwrap_or(0),
            file_count: 1,
        };
    }

    let mut stats = FileStats::default();
    let Ok(entries) = fs::read_dir(path) else {
        return stats;
    };
    for entry in entries.flatten() {
        let child = entry.path();
        if is_excluded(&child, excluded) {
            continue;
        }
        stats += stats_for_path(&child, excluded);
    }
    stats
}

fn is_excluded(path: &Path, excluded: &[PathBuf]) -> bool {
    let path_key = canonical_key(path);
    excluded.iter().any(|root| {
        let root_key = canonical_key(root);
        path_key == root_key || path_key.starts_with(&format!("{}{}", root_key, std::path::MAIN_SEPARATOR))
    })
}

fn canonical_key(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

/// 清空目录内容，遇到删不掉的条目继续处理其余条目；返回失败条目数
fn clear_directory_contents(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return 1;
    };
    let mut failed = 0;
    for entry in entries {
        let Ok(entry) = entry else {
            failed += 1;
            continue;
        };
        let child = entry.path();
        let removed = if child.is_dir() {
            fs::remove_dir_all(&child)
        } else {
            fs::remove_file(&child)
        };
        if removed.is_err() {
            failed += 1;
        }
    }
    failed
}

fn download_roots(default_root: Option<&Path>, custom_root: Option<&str>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = default_root {
        roots.push(root.to_path_buf());
    }
    if let Some(custom) = custom_root.map(str::trim).filter(|value| !value.is_empty()) {
        let path = PathBuf::from(custom);
        if !roots.iter().any(|root| canonical_key(root) == canonical_key(&path)) {
            roots.push(path);
        }
    }
    roots
}

#[derive(Default)]
struct DownloadStats {
    music: FileStats,
    lyrics: FileStats,
    staging: FileStats,
}

fn scan_download_roots(roots: &[PathBuf]) -> DownloadStats {
    let mut total = DownloadStats::default();
    for root in roots {
        scan_download_path(root, &mut total);
    }
    total
}

fn scan_download_path(path: &Path, stats: &mut DownloadStats) {
    if !path.exists() {
        return;
    }
    if path.is_file() {
        let file_stats = FileStats {
            size_bytes: fs::metadata(path).map(|meta| meta.len()).unwrap_or(0),
            file_count: 1,
        };
        let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("");
        if is_download_staging_file(path) {
            stats.staging += file_stats;
        } else if matches!(extension.to_ascii_lowercase().as_str(), "lrc" | "tlrc") {
            stats.lyrics += file_stats;
        } else if path.file_name().and_then(|value| value.to_str()) != Some("manifest.json") {
            stats.music += file_stats;
        }
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        scan_download_path(&entry.path(), stats);
    }
}

// 播放/下载编排函数的参数都是相互独立的运行时上下文，聚成结构体只是换个地方堆字段
#[allow(clippy::too_many_arguments)]
fn known_data_paths(
    image_cache: &[PathBuf],
    download_staging: &[PathBuf],
    shared_media: &[PathBuf],
    platform_list: &[PathBuf],
    download_roots: &[PathBuf],
    local_covers: Option<&Path>,
    custom_background: Option<&Path>,
    user_database: &[PathBuf],
) -> Vec<PathBuf> {
    image_cache
        .iter()
        .chain(download_staging)
        .chain(shared_media)
        .chain(platform_list)
        .chain(download_roots)
        .chain(user_database)
        .cloned()
        .chain(local_covers.map(Path::to_path_buf))
        .chain(custom_background.map(Path::to_path_buf))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clearing_continues_past_entries_that_cannot_be_removed() {
        let root = tempfile::tempdir().unwrap();
        for name in ["a.audio", "b.audio"] {
            fs::write(root.path().join(name), b"1234").unwrap();
        }
        let locked = root.path().join("locked");
        fs::create_dir_all(&locked).unwrap();
        fs::write(locked.join("held.audio"), b"1234").unwrap();
        #[cfg(windows)]
        let guard = {
            use std::os::windows::fs::OpenOptionsExt;
            fs::OpenOptions::new().read(true).share_mode(0).open(locked.join("held.audio")).unwrap()
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();
        }

        assert_eq!(clear_directory_contents(root.path()), 1);
        assert!(!root.path().join("a.audio").exists());
        assert!(!root.path().join("b.audio").exists());

        #[cfg(windows)]
        drop(guard);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert_eq!(clear_directory_contents(&root.path().join("missing")), 0);
    }

    #[test]
    fn webview_profile_and_debug_stores_are_not_cache() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("EBWebView/Default/Local Storage")).unwrap();
        fs::write(root.path().join("EBWebView/Default/Local Storage/leveldb.ldb"), vec![0; 4096]).unwrap();
        fs::create_dir_all(root.path().join("debug-credentials")).unwrap();
        fs::write(root.path().join("debug-credentials/store.bin"), vec![0; 512]).unwrap();
        fs::create_dir_all(root.path().join("playback-audio/ab")).unwrap();
        fs::write(root.path().join("playback-audio/ab/x.audio"), vec![0; 100]).unwrap();
        fs::write(root.path().join("stray.tmp"), vec![0; 10]).unwrap();

        let webview = webview_data_paths(Some(root.path()));
        let mut excluded = webview.clone();
        excluded.push(root.path().join("playback-audio"));
        let other = other_cache_item(Some(root.path()), &excluded);
        assert_eq!(other.size_bytes, 10, "only the stray file is other cache");
        assert_eq!(aggregate_item("webview_data", &webview, None).size_bytes, 4096 + 512);
    }

    #[test]
    fn unfinished_download_files_count_as_staging() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("song.flac"), vec![0; 300]).unwrap();
        fs::write(root.path().join("song.flac.part"), vec![0; 200]).unwrap();
        fs::write(root.path().join("next.mp3.reserve"), vec![0; 50]).unwrap();
        fs::write(root.path().join("song.lrc"), vec![0; 7]).unwrap();
        let stats = scan_download_roots(&[root.path().to_path_buf()]);
        assert_eq!(stats.music.size_bytes, 300);
        assert_eq!(stats.lyrics.size_bytes, 7);
        assert_eq!(stats.staging.size_bytes, 250);
        assert_eq!(stats.staging.file_count, 2);

        let mut result = StorageClearResult { cleared_bytes: 0, deleted_files: 0, failed_count: 0 };
        remove_download_staging_files(root.path(), &mut result);
        assert_eq!(result.cleared_bytes, 250);
        assert!(root.path().join("song.flac").exists());
        assert!(!root.path().join("song.flac.part").exists());
    }
}
