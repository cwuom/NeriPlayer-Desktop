// 本地音乐文件扫描
use crate::error::AppResult;
use crate::state::TrackInfo;
use crate::state::TrackSource;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use walkdir::WalkDir;

const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "wav", "m4a", "aac", "opus", "wma", "webm", "eac3",
];
// 含 gif：下载封面 sidecar 可能按 content-type 落成 .gif（ext_from_image_content_type），
// 否则重扫时封面索引不认 gif 导致封面丢失（SC-10）
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif"];
const COVER_NAMES: &[&str] = &["cover", "folder", "front", "albumart", "album"];
// 大小写无关匹配的封面子目录名
const COVER_DIR_NAMES: &[&str] = &["covers", "cover", "artwork", "scans"];
const MAX_SCAN_DEPTH: usize = 32;
const MAX_SCAN_ENTRIES: usize = 100_000;
const MAX_SCAN_TRACKS: usize = 20_000;

fn cover_name_priority(name: &str) -> usize {
    COVER_NAMES
        .iter()
        .position(|candidate| *candidate == name)
        .unwrap_or(COVER_NAMES.len())
}

fn cover_dir_priority(name: &str) -> usize {
    COVER_DIR_NAMES
        .iter()
        .position(|candidate| *candidate == name)
        .unwrap_or(COVER_DIR_NAMES.len())
}

#[derive(Debug, Clone, Default)]
struct ParsedFileNameMetadata {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    source: Option<String>,
}

/// 扫描结果：成功曲目 + 解析失败的文件（供前端展示失败计数，SC-9）
#[derive(Debug, serde::Serialize)]
pub struct ScanResult {
    pub tracks: Vec<TrackInfo>,
    pub skipped: Vec<ScanSkipped>,
}

#[derive(Debug, serde::Serialize)]
pub struct ScanSkipped {
    pub path: String,
    pub reason: String,
}

/// 扫描目录下的所有音频文件，读取元数据
pub fn scan_directory(dir: &str, name_template: Option<&str>) -> AppResult<ScanResult> {
    scan_directory_with_control(dir, name_template, &AtomicBool::new(false), |_, _, _, _| {})
}

pub fn scan_directory_with_control(
    dir: &str,
    name_template: Option<&str>,
    cancelled: &AtomicBool,
    mut on_progress: impl FnMut(usize, usize, usize, &Path),
) -> AppResult<ScanResult> {
    let mut tracks = Vec::new();
    let mut skipped: Vec<ScanSkipped> = Vec::new();
    // 扫描会话级封面索引缓存：同目录的封面查找只列举一次目录
    let mut cover_cache = CoverLookupCache::default();

    // 根目录不可读时必须返回 Err，而不是静默返回空列表（SC-1）：
    // macOS TCC 拒绝、网络卷断开、EPERM 等会让 WalkDir 首个 entry 为 Err 被吞掉，
    // 上层据此把"本地音乐"歌单全量清空并落盘，用户无从区分"目录空"与"读取失败"
    let root = std::fs::canonicalize(Path::new(dir)).map_err(|e| {
        crate::error::AppError::Other(format!("扫描目录不可访问: {dir}: {e}"))
    })?;
    if !root.is_dir() {
        return Err(crate::error::AppError::Other(format!(
            "扫描目录不可访问: {dir}"
        )));
    }
    std::fs::read_dir(&root).map_err(|e| {
        crate::error::AppError::Other(format!("扫描目录读取失败: {dir}: {e}"))
    })?;

    // 默认不跟随符号链接，避免扫描逃逸到根目录或网络挂载点。
    // 规范化路径同时用于去重，防止别名路径重复导入同一个文件（SC-2/SC-13）
    let mut seen_files = HashSet::new();
    let mut visited_entries = 0usize;
    for entry_result in WalkDir::new(&root)
        .follow_links(false)
        .max_depth(MAX_SCAN_DEPTH)
        .into_iter()
        .filter_entry(|entry| entry.depth() == 0 || entry.file_name() != ".tmp")
    {
        if cancelled.load(Ordering::Relaxed) {
            return Err(crate::error::AppError::Other("Scan cancelled".into()));
        }
        visited_entries = visited_entries.saturating_add(1);
        if visited_entries > MAX_SCAN_ENTRIES {
            skipped.push(ScanSkipped {
                path: root.display().to_string(),
                reason: format!("扫描条目超过上限 {}", MAX_SCAN_ENTRIES),
            });
            break;
        }
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(error) => {
                skipped.push(ScanSkipped {
                    path: error
                        .path()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| dir.to_string()),
                    reason: error.to_string(),
                });
                continue;
            }
        };
        if visited_entries == 1 || visited_entries.is_multiple_of(100) {
            on_progress(visited_entries, tracks.len(), skipped.len(), entry.path());
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let path = match std::fs::canonicalize(entry.path()) {
            Ok(path) if path.starts_with(&root) => path,
            Ok(path) => {
                skipped.push(ScanSkipped {
                    path: entry.path().display().to_string(),
                    reason: format!("跳过扫描根目录外的链接目标: {}", path.display()),
                });
                continue;
            }
            Err(error) => {
                skipped.push(ScanSkipped {
                    path: entry.path().display().to_string(),
                    reason: format!("无法规范化文件路径: {error}"),
                });
                continue;
            }
        };
        if !seen_files.insert(path.clone()) {
            continue;
        }

        let ext = path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        if !AUDIO_EXTENSIONS.contains(&ext.as_str()) { continue; }

        if tracks.len() >= MAX_SCAN_TRACKS {
            skipped.push(ScanSkipped {
                path: path.display().to_string(),
                reason: format!("扫描曲目超过上限 {}", MAX_SCAN_TRACKS),
            });
            break;
        }

        match read_track_info(&path, name_template, &mut cover_cache) {
            Ok(track) => tracks.push(track),
            Err(e) => {
                // 解析失败不再仅后端 warn 静默丢弃, 回传给前端展示失败计数（SC-9）
                log::warn!(target: "scanner", "Skip {}: {}", path.display(), e);
                skipped.push(ScanSkipped {
                    path: path.display().to_string(),
                    reason: e.to_string(),
                });
            }
        }
    }

    on_progress(visited_entries, tracks.len(), skipped.len(), &root);
    Ok(ScanResult { tracks, skipped })
}

fn read_track_info(
    path: &Path,
    name_template: Option<&str>,
    cover_cache: &mut CoverLookupCache,
) -> AppResult<TrackInfo> {
    use lofty::prelude::*;
    use lofty::probe::Probe;

    let managed = crate::commands::download_cmd::metadata::read_metadata(path);
    if managed.as_ref().is_some_and(|metadata| metadata.download_finalized == Some(false)) {
        return Err(crate::error::AppError::Metadata("下载尚未完成".into()));
    }

    // 按文件内容认格式：旧版下载把 FLAC 存成了 .mp3，按扩展名当 MP3 解析会跳过或算出几小时的时长
    let tagged = Probe::open(path)
        .map_err(|e| crate::error::AppError::Metadata(e.to_string()))?
        .guess_file_type()?
        .read()
        .map_err(|e| crate::error::AppError::Metadata(e.to_string()))?;
    let properties = tagged.properties();
    let duration_ms = properties.duration().as_millis() as u64;

    // 尝试读取标签
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let file_stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown")
        .to_string();
    let parsed = parse_file_name_metadata(path, name_template);

    let raw_title = tag.and_then(|t| t.title().map(|s| s.to_string()))
        .unwrap_or_else(|| {
            file_stem.clone()
        });
    let raw_artist = tag.and_then(|t| t.artist().map(|s| s.to_string()))
        .unwrap_or_else(|| "Unknown Artist".to_string());
    let raw_album = tag.and_then(|t| t.album().map(|s| s.to_string()))
        .unwrap_or_else(|| "Unknown Album".to_string());

    let title = if should_use_parsed_title(&raw_title, &file_stem) {
        parsed
            .as_ref()
            .and_then(|p| p.title.clone())
            .unwrap_or(raw_title)
    } else {
        raw_title
    };
    let artist = if is_unknown_artist(&raw_artist) || parsed_source_matches_artist(&raw_artist, parsed.as_ref()) {
        parsed
            .as_ref()
            .and_then(|p| p.artist.clone())
            .unwrap_or(raw_artist)
    } else {
        raw_artist
    };
    let album = if is_unknown_album(&raw_album) {
        parsed
            .as_ref()
            .and_then(|p| p.album.clone())
            .unwrap_or(raw_album)
    } else {
        raw_album
    };
    let title = managed.as_ref().and_then(|metadata| metadata.custom_name.as_ref().or(metadata.name.as_ref())).filter(|value| !value.trim().is_empty()).cloned().unwrap_or(title);
    let artist = managed.as_ref().and_then(|metadata| metadata.custom_artist.as_ref().or(metadata.artist.as_ref())).filter(|value| !value.trim().is_empty()).cloned().unwrap_or(artist);
    let album = managed.as_ref().and_then(|metadata| metadata.album.clone()).filter(|value| !value.trim().is_empty()).unwrap_or(album);
    let cover_url = find_nearby_cover(path, cover_cache)
        .or_else(|| tag.and_then(|tag| {
            let picture = tag.pictures().iter().find(|picture| picture.pic_type() == lofty::picture::PictureType::CoverFront)
                .or_else(|| tag.pictures().first())?;
            let cache_dir = cover_cache.embedded_dir.clone().or_else(default_embedded_cover_dir)?;
            cache_embedded_cover(picture.data(), &cache_dir)
        }))
        .map(|p| p.to_string_lossy().to_string());

    Ok(TrackInfo {
        id: format!("local:{}", path.display()),
        title,
        artist,
        album,
        duration_ms,
        source: TrackSource::Local,
        url: path.to_string_lossy().to_string(),
        cover_url,
        added_at: managed.as_ref().and_then(|metadata| metadata.download_time_ms).unwrap_or(0).min(i64::MAX as u64) as i64,
        sync_payload: None,
        playlist_key: managed.and_then(|metadata| metadata.stable_key),
    })
}

/// 单次扫描会话内按目录缓存的封面索引。
///
/// 旧实现每个音频文件最多做 3 轮 `read_dir` 全扫，平铺大目录下整体
/// 近似 O(N²)；缓存后每个目录只列举一次，整体 O(N)
#[derive(Default)]
struct CoverLookupCache {
    dirs: HashMap<PathBuf, DirCoverIndex>,
    embedded_dir: Option<PathBuf>,
}

fn default_embedded_cover_dir() -> Option<PathBuf> {
    static DIRECTORY: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DIRECTORY.get_or_init(|| {
        let config: serde_json::Value = serde_json::from_str(include_str!("../../tauri.conf.json")).ok()?;
        let identifier = config.get("identifier")?.as_str()?;
        Some(dirs_next::data_dir()?.join(identifier).join("local-covers"))
    }).clone()
}

fn cache_embedded_cover(bytes: &[u8], directory: &Path) -> Option<PathBuf> {
    use sha2::{Digest, Sha256};
    if bytes.is_empty() || bytes.len() > 8 * 1024 * 1024 { return None; }
    let suffix = match image::guess_format(bytes).ok()? {
        image::ImageFormat::Jpeg => "jpg",
        image::ImageFormat::Png => "png",
        image::ImageFormat::Gif => "gif",
        image::ImageFormat::WebP => "webp",
        _ => return None,
    };
    let path = directory.join(format!("{}.{suffix}", hex::encode(Sha256::digest(bytes))));
    static LOCK: std::sync::OnceLock<parking_lot::Mutex<()>> = std::sync::OnceLock::new();
    let _guard = LOCK.get_or_init(|| parking_lot::Mutex::new(())).lock();
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() == bytes.len() as u64) {
        return Some(path);
    }
    crate::fsutil::atomic_write(&path, bytes).ok()?;
    Some(path)
}

struct DirCoverIndex {
    /// (小写去扩展名文件名, 完整路径)，按候选名优先级和路径稳定排序
    images: Vec<(String, PathBuf)>,
    /// 首个大小写无关命中的封面子目录（Covers/cover/artwork/scans）
    cover_subdir: Option<PathBuf>,
}

impl CoverLookupCache {
    fn index_of(&mut self, dir: &Path) -> &DirCoverIndex {
        self.dirs
            .entry(dir.to_path_buf())
            .or_insert_with(|| build_dir_cover_index(dir))
    }

    /// 在目录中查找文件名（小写、去扩展名）满足断言且扩展名为图片的文件
    fn find_image<F>(&mut self, dir: &Path, matcher: F) -> Option<PathBuf>
    where
        F: Fn(&str) -> bool,
    {
        self.index_of(dir)
            .images
            .iter()
            .find(|(stem, _)| matcher(stem))
            .map(|(_, path)| path.clone())
    }

    /// 大小写无关查找封面子目录（Covers/cover/artwork...）
    fn cover_subdir(&mut self, dir: &Path) -> Option<PathBuf> {
        self.index_of(dir).cover_subdir.clone()
    }
}

fn build_dir_cover_index(dir: &Path) -> DirCoverIndex {
    let mut images = Vec::new();
    let mut cover_subdir = None;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return DirCoverIndex {
            images,
            cover_subdir,
        };
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            if cover_subdir.is_none() {
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_lowercase())
                    .unwrap_or_default();
                if COVER_DIR_NAMES.contains(&name.as_str()) {
                    let replace = cover_subdir.as_ref().is_none_or(|current| {
                        let current_name = current
                            .file_name()
                            .and_then(|value| value.to_str())
                            .map(|value| value.to_lowercase())
                            .unwrap_or_default();
                        (cover_dir_priority(&name), path.clone())
                            < (cover_dir_priority(&current_name), current.clone())
                    });
                    if replace {
                        cover_subdir = Some(path);
                    }
                }
            }
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        if !IMAGE_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_default();
        images.push((stem, path));
    }
    // 常见封面名按 cover/folder/front 等既定优先级选择，再以路径打破同名平局；
    // 不依赖文件系统 read_dir 顺序，保证跨平台扫描结果一致（SC-11）
    images.sort_by(|a, b| {
        cover_name_priority(&a.0)
            .cmp(&cover_name_priority(&b.0))
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| a.1.cmp(&b.1))
    });
    DirCoverIndex {
        images,
        cover_subdir,
    }
}

fn find_nearby_cover(audio_path: &Path, cache: &mut CoverLookupCache) -> Option<PathBuf> {
    let dir = audio_path.parent()?;
    let audio_name = audio_path.file_name()?.to_str()?.to_lowercase();
    let stem = audio_path.file_stem()?.to_str()?.to_lowercase();

    let managed = crate::commands::download_cmd::metadata::read_metadata(audio_path);
    if let Some(cover) = managed.as_ref().and_then(|metadata| metadata.cover_path.as_deref())
        .and_then(|reference| crate::commands::download_cmd::metadata::resolve_asset(audio_path, reference, "Covers"))
    {
        return Some(cover);
    }
    if let (Some(sub), Some(stable_key)) = (cache.cover_subdir(dir), managed.as_ref().and_then(|metadata| metadata.stable_key.as_deref())) {
        let stable_name = format!("{stem}-{}", crate::commands::download_cmd::metadata::cover_suffix(stable_key));
        if let Some(cover) = cache.find_image(&sub, |name| name == stable_name) {
            return Some(cover);
        }
    }

    // 下载器新写入的 sidecar 使用完整音频名，例如 Song.m4a.jpg
    if let Some(cover) = cache.find_image(dir, |name| name == audio_name) {
        return Some(cover);
    }

    // 同目录下与音频同名的图片（大小写无关）
    if let Some(cover) = cache.find_image(dir, |name| name == stem) {
        return Some(cover);
    }

    // 大小写无关的封面子目录下与音频同名的图片
    if let Some(sub) = cache.cover_subdir(dir) {
        if let Some(cover) = cache.find_image(&sub, |name| name == audio_name) {
            return Some(cover);
        }
        if let Some(cover) = cache.find_image(&sub, |name| name == stem) {
            return Some(cover);
        }
        // 子目录内任意常见封面名
        if let Some(cover) = cache.find_image(&sub, |name| COVER_NAMES.contains(&name)) {
            return Some(cover);
        }
    }

    // 同目录下常见封面文件名（cover/folder/front/albumart...，大小写无关）
    if let Some(cover) = cache.find_image(dir, |name| COVER_NAMES.contains(&name)) {
        return Some(cover);
    }

    None
}

fn parse_file_name_metadata(
    path: &Path,
    active_template: Option<&str>,
) -> Option<ParsedFileNameMetadata> {
    let base_name = path.file_stem()?.to_str()?.trim();
    if base_name.is_empty() {
        return None;
    }

    candidate_templates(active_template)
        .into_iter()
        .filter_map(|template| parse_base_name_with_template(base_name, &template))
        .find(|parsed| {
            parsed.title.as_deref().is_some_and(|s| !s.is_empty())
                || parsed.artist.as_deref().is_some_and(|s| !s.is_empty())
                || parsed.album.as_deref().is_some_and(|s| !s.is_empty())
        })
}

fn candidate_templates(active_template: Option<&str>) -> Vec<String> {
    let mut templates = Vec::new();
    if let Some(tpl) = active_template.map(str::trim).filter(|s| !s.is_empty()) {
        templates.push(tpl.to_string());
    }
    // 三段式（DEFAULT）必须排在两段式（LEGACY）之前，对齐 Android candidateManagedDownloadFileNameTemplates
    // 的 active → DEFAULT → LEGACY 顺序：否则 "netease - Halsey - Without Me" 会先被
    // "{artist} - {title}" 的非贪婪正则命中，误解析成 artist=netease（SC-6）
    for tpl in [
        "%title% - %artist% - %album% - %source%",
        "%title% - %artist% [%hash%]",
        "{source} - {artist} - {title}",
        "%source% - %artist% - %title%",
        "{artist} - {title}",
        "%artist% - %title%",
    ] {
        if !templates.iter().any(|existing| existing == tpl) {
            templates.push(tpl.to_string());
        }
    }
    templates
}

fn parse_base_name_with_template(
    base_name: &str,
    template: &str,
) -> Option<ParsedFileNameMetadata> {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Field {
        Title,
        Artist,
        Album,
        Source,
        Id,
        AudioId,
        SubAudioId,
        Hash,
    }

    let placeholders = [
        ("{title}", Field::Title),
        ("{artist}", Field::Artist),
        ("{album}", Field::Album),
        ("{source}", Field::Source),
        ("{id}", Field::Id),
        ("{audioId}", Field::AudioId),
        ("{subAudioId}", Field::SubAudioId),
        ("{hash}", Field::Hash),
        ("%title%", Field::Title),
        ("%artist%", Field::Artist),
        ("%album%", Field::Album),
        ("%source%", Field::Source),
        ("%id%", Field::Id),
        ("%audioId%", Field::AudioId),
        ("%subAudioId%", Field::SubAudioId),
        ("%hash%", Field::Hash),
    ];

    let mut fields = Vec::new();
    let mut pattern = String::from("^");
    let mut cursor = 0;
    let mut matched_any = false;

    while cursor < template.len() {
        let next = placeholders
            .iter()
            .filter_map(|(token, field)| {
                template[cursor..]
                    .find(token)
                    .map(|idx| (cursor + idx, *token, *field))
            })
            .min_by_key(|(idx, _, _)| *idx);

        let Some((idx, token, field)) = next else {
            pattern.push_str(&regex::escape(&template[cursor..]));
            break;
        };

        pattern.push_str(&regex::escape(&template[cursor..idx]));
        if fields.contains(&field) {
            return None;
        }
        fields.push(field);
        pattern.push_str("(.*?)");
        cursor = idx + token.len();
        matched_any = true;
    }

    if !matched_any {
        return None;
    }

    pattern.push('$');
    let regex = Regex::new(&pattern).ok()?;
    let captures = regex.captures(base_name)?;

    let mut parsed = ParsedFileNameMetadata::default();
    for (index, field) in fields.iter().enumerate() {
        let value = captures
            .get(index + 1)
            .map(|m| m.as_str().trim().to_string())
            .filter(|s| !s.is_empty());
        match field {
            Field::Title => parsed.title = value,
            Field::Artist => parsed.artist = value,
            Field::Album => parsed.album = value,
            Field::Source => parsed.source = value,
            Field::Id | Field::AudioId | Field::SubAudioId | Field::Hash => {}
        }
    }
    Some(parsed)
}

fn should_use_parsed_title(current_title: &str, file_stem: &str) -> bool {
    let normalized = normalize_metadata_value(current_title);
    normalized.is_empty()
        || normalized == "unknown"
        || normalized == normalize_metadata_value(file_stem)
}

fn is_unknown_artist(value: &str) -> bool {
    matches!(
        normalize_metadata_value(value).as_str(),
        "" | "unknown" | "unknown artist" | "<unknown>"
    )
}

fn is_unknown_album(value: &str) -> bool {
    matches!(
        normalize_metadata_value(value).as_str(),
        "" | "unknown" | "unknown album" | "<unknown>"
    )
}

fn parsed_source_matches_artist(
    artist: &str,
    parsed: Option<&ParsedFileNameMetadata>,
) -> bool {
    parsed
        .and_then(|p| p.source.as_deref())
        .map(|source| normalize_metadata_value(source) == normalize_metadata_value(artist))
        .unwrap_or(false)
}

fn normalize_metadata_value(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{find_nearby_cover, scan_directory, CoverLookupCache};
    use std::path::PathBuf;

    /// 回归：旧版下载把 FLAC 存成 .mp3，按扩展名解析时 30/35 首被跳过，其余算出 12 小时这类时长
    #[test]
    fn a_flac_saved_with_an_mp3_name_is_read_as_flac() {
        let root = tempfile::tempdir().unwrap();
        let audio = root.path().join("Song.mp3");
        std::fs::write(&audio, include_bytes!("../audio/fixtures/ffmpeg/flac-s16-stereo-0.5s.flac")).unwrap();
        let result = scan_directory(root.path().to_str().unwrap(), None).unwrap();
        assert!(result.skipped.is_empty(), "{:?}", result.skipped.iter().map(|s| &s.reason).collect::<Vec<_>>());
        assert_eq!(result.tracks.len(), 1);
        assert!((450..=550).contains(&result.tracks[0].duration_ms), "{}", result.tracks[0].duration_ms);
    }

    #[test]
    fn managed_android_metadata_restores_display_cover_and_identity_without_audio_tags() {
        let root = tempfile::tempdir().unwrap();
        let audio = root.path().join("Song.aac");
        std::fs::write(&audio, include_bytes!("../audio/fixtures/hls-silence.aac")).unwrap();
        let covers = root.path().join("Covers");
        std::fs::create_dir_all(&covers).unwrap();
        let key = "42|Netease专辑|";
        let cover = covers.join(format!("Song-{}.jpg", crate::commands::download_cmd::metadata::cover_suffix(key)));
        std::fs::write(&cover, b"cover").unwrap();
        let mut metadata = crate::commands::download_cmd::metadata::DownloadMetadata {
            name: Some("原始歌曲".into()), custom_name: Some("修正歌曲".into()),
            artist: Some("歌手".into()), album: Some("专辑".into()),
            stable_key: Some(key.into()), download_finalized: Some(true),
            download_time_ms: Some(123), cover_path: Some("content://android/unavailable".into()),
            ..Default::default()
        };
        metadata.write(&audio).unwrap();
        let result = scan_directory(root.path().to_str().unwrap(), None).unwrap();
        assert_eq!(result.tracks.len(), 1);
        let track = &result.tracks[0];
        assert_eq!(track.title, "修正歌曲");
        assert_eq!(track.artist, "歌手");
        assert_eq!(track.playlist_key.as_deref(), Some(key));
        assert_eq!(track.added_at, 123);
        let cover = cover.canonicalize().unwrap();
        assert_eq!(track.cover_url.as_deref(), Some(cover.to_str().unwrap()));
    }

    #[test]
    fn controlled_scanner_ignores_temporary_audio_reports_progress_and_honors_cancel() {
        let root = tempfile::tempdir().unwrap();
        let temporary = root.path().join(".tmp");
        std::fs::create_dir_all(&temporary).unwrap();
        std::fs::write(temporary.join("hidden.aac"), include_bytes!("../audio/fixtures/hls-silence.aac")).unwrap();
        let mut updates = Vec::new();
        let result = super::scan_directory_with_control(root.path().to_str().unwrap(), None, &std::sync::atomic::AtomicBool::new(false), |visited, tracks, skipped, _| {
            updates.push((visited, tracks, skipped));
        }).unwrap();
        assert!(result.tracks.is_empty());
        assert!(result.skipped.is_empty());
        assert!(updates.len() >= 2);
        let error = super::scan_directory_with_control(root.path().to_str().unwrap(), None, &std::sync::atomic::AtomicBool::new(true), |_, _, _, _| {}).unwrap_err();
        assert!(error.to_string().contains("cancelled"));
    }

    #[test]
    fn scanner_caches_embedded_cover_when_audio_has_no_sidecars() {
        let root = tempfile::tempdir().unwrap();
        let audio = root.path().join("Song.aac");
        std::fs::write(&audio, include_bytes!("../audio/fixtures/hls-silence.aac")).unwrap();
        let cover = root.path().join("Covers").join("Song.jpg");
        std::fs::create_dir_all(cover.parent().unwrap()).unwrap();
        image::RgbImage::from_pixel(2, 2, image::Rgb([40, 80, 120])).save(&cover).unwrap();
        let metadata = crate::commands::download_cmd::metadata::DownloadMetadata {
            name: Some("Song".into()), artist: Some("Artist".into()),
            cover_path: Some(cover.to_string_lossy().to_string()),
            ..Default::default()
        };
        crate::commands::download_cmd::metadata::prepare_audio_tags(&audio, &metadata, false).unwrap().persist(&audio).unwrap();
        std::fs::remove_file(&cover).unwrap();
        let cache_directory = root.path().join("local-covers");
        let mut cache = super::CoverLookupCache { embedded_dir: Some(cache_directory.clone()), ..Default::default() };
        let track = super::read_track_info(&audio, None, &mut cache).unwrap();
        let cover = PathBuf::from(track.cover_url.unwrap());
        assert!(cover.starts_with(cache_directory));
        assert!(image::open(cover).is_ok());
    }

    fn temp_scan_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "neri-scanner-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn cover_lookup_prefers_same_stem_then_common_names() {
        let dir = temp_scan_dir("flat");
        std::fs::write(dir.join("song.jpg"), b"img").expect("write image");
        std::fs::write(dir.join("cover.png"), b"img").expect("write image");
        let audio = dir.join("song.mp3");
        std::fs::write(&audio, b"audio").expect("write audio");
        let other = dir.join("other.flac");
        std::fs::write(&other, b"audio").expect("write audio");

        let mut cache = CoverLookupCache::default();
        assert_eq!(
            find_nearby_cover(&audio, &mut cache),
            Some(dir.join("song.jpg"))
        );
        // 同名不命中时回退到常见封面名
        assert_eq!(
            find_nearby_cover(&other, &mut cache),
            Some(dir.join("cover.png"))
        );
        // 目录只被列举并缓存一次
        assert_eq!(cache.dirs.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cover_lookup_finds_image_in_cover_subdir() {
        let dir = temp_scan_dir("subdir");
        let sub = dir.join("Covers");
        std::fs::create_dir_all(&sub).expect("create cover subdir");
        std::fs::write(sub.join("track.webp"), b"img").expect("write image");
        let audio = dir.join("track.mp3");
        std::fs::write(&audio, b"audio").expect("write audio");

        let mut cache = CoverLookupCache::default();
        assert_eq!(
            find_nearby_cover(&audio, &mut cache),
            Some(sub.join("track.webp"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cover_lookup_prefers_audio_scoped_download_sidecar() {
        let dir = temp_scan_dir("scoped-sidecar");
        let audio = dir.join("Song.m4a");
        let scoped = dir.join("Song.m4a.jpg");
        let legacy = dir.join("Song.jpg");
        std::fs::write(&audio, b"audio").expect("write audio");
        std::fs::write(&scoped, b"scoped").expect("write image");
        std::fs::write(&legacy, b"legacy").expect("write image");

        let mut cache = CoverLookupCache::default();
        assert_eq!(find_nearby_cover(&audio, &mut cache), Some(scoped));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cover_lookup_uses_declared_common_name_priority() {
        let dir = temp_scan_dir("cover-priority");
        let audio = dir.join("track.mp3");
        std::fs::write(&audio, b"audio").expect("write audio");
        std::fs::write(dir.join("folder.jpg"), b"folder").expect("write folder cover");
        std::fs::write(dir.join("cover.png"), b"cover").expect("write cover");

        let mut cache = CoverLookupCache::default();
        assert_eq!(find_nearby_cover(&audio, &mut cache), Some(dir.join("cover.png")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_directory_rejects_missing_root_instead_of_returning_empty_success() {
        let path = std::env::temp_dir().join(format!(
            "neri-scanner-missing-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));

        assert!(scan_directory(path.to_str().unwrap(), None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn scan_directory_does_not_follow_audio_symlinks_outside_root() {
        use std::os::unix::fs::symlink;

        let root = temp_scan_dir("symlink-root");
        let outside = temp_scan_dir("symlink-outside");
        let target = outside.join("outside.wav");
        std::fs::write(&target, b"not an audio file").expect("write target");
        symlink(&target, root.join("linked.wav")).expect("create symlink");

        let result = scan_directory(root.to_str().unwrap(), None).expect("scan root");
        assert!(result.tracks.is_empty());
        assert!(result.skipped.is_empty());

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }
}
