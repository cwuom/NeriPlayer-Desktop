use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::header::{ACCEPT, CONTENT_LENGTH, REFERER, USER_AGENT};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tauri::{AppHandle, Manager, State};
use url::Url;

use crate::error::{AppError, AppResult};
use crate::settings::store;
use crate::state::AppState;

const MAX_COVER_BYTES: u64 = 8 * 1024 * 1024;
const IMAGE_CACHE_DIRECTORY: &str = "image_cache";
const COVER_CACHE_MARKER_VERSION: &str = "v1";
const BILIBILI_REFERER: &str = "https://www.bilibili.com/";
const QQ_REFERER: &str = "https://y.qq.com/";
const NETEASE_REFERER: &str = "https://music.163.com/";
const YOUTUBE_REFERER: &str = "https://music.youtube.com/";
const COVER_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

struct CachedCover {
    bytes: Vec<u8>,
    mime_type: &'static str,
}

#[derive(Clone)]
struct CoverDiskCache {
    root: PathBuf,
    directory: PathBuf,
    digest: String,
    ready_path: PathBuf,
    max_cache_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CoverCacheMarker {
    content_length: u64,
    sha256: String,
    mime_type: String,
    file_name: String,
}

#[tauri::command]
pub async fn fetch_bilibili_cover(
    url: String,
    force_refresh: Option<bool>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let cover_url = normalize_cover_url(&url)?;
    let cover_host = cover_url.host_str().unwrap_or("unknown").to_string();
    let cache_setup_started = Instant::now();
    let cache_setup_queued_at = Instant::now();
    let app_for_setup = app.clone();
    let cache_key_url = cover_url.clone();
    let setup_host = cover_host.clone();
    let config_guard = state.config_persistence_gate.lock().await;
    let (settings, cache) = tokio::task::spawn_blocking(move || {
        log::info!(
            target: "cover-cache",
            "setup worker started host={}, queued_ms={}",
            setup_host,
            cache_setup_queued_at.elapsed().as_millis(),
        );
        let settings = store::load_settings(&app_for_setup)?.settings;
        let cache_limit_mb = settings.max_cache_size as u64;
        let cache_limit_bytes = cache_limit_mb.saturating_mul(1024 * 1024);
        let cache_root = app_for_setup
            .path()
            .app_cache_dir()
            .map_err(|err| AppError::Other(err.to_string()))?
            .join(IMAGE_CACHE_DIRECTORY);
        let cache = CoverDiskCache::new(cache_root, cache_key_url.as_str(), cache_limit_bytes)?;
        Ok::<_, AppError>((settings, cache))
    })
    .await
    .map_err(|err| AppError::Other(err.to_string()))??;
    drop(config_guard);
    log::info!(
        target: "cover-fetch",
        "cache ready host={}, setup_ms={}, force_refresh={}",
        cover_host,
        cache_setup_started.elapsed().as_millis(),
        force_refresh == Some(true),
    );

    if force_refresh != Some(true) {
        let lookup_started = Instant::now();
        let cache_for_lookup = cache.clone();
        let cached = tokio::task::spawn_blocking(move || {
            cache_for_lookup.read().map(|cached| {
                (
                    cover_data_url(&cached.bytes, cached.mime_type),
                    cached.bytes.len(),
                )
            })
        })
        .await
        .map_err(|err| AppError::Other(err.to_string()))?;
        log::info!(
            target: "cover-cache",
            "lookup host={}, hit={}, elapsed_ms={}",
            cover_host,
            cached.is_some(),
            lookup_started.elapsed().as_millis(),
        );
        if let Some((data_url, bytes)) = cached {
            log::info!(
                target: "cover-cache",
                "hit host={}, bytes={}, data_url_chars={}",
                cover_host,
                bytes,
                data_url.len(),
            );
            return Ok(data_url);
        }
    }

    let request_started = Instant::now();
    log::info!(
        target: "cover-fetch",
        "request start host={}, force_refresh={}",
        cover_host,
        force_refresh == Some(true),
    );
    let mut client_builder = reqwest::Client::builder()
        .cookie_provider(state.cookie_jar.clone())
        .user_agent(COVER_USER_AGENT)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || validate_cover_url(attempt.url()).is_err() {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }));
    if settings.bypass_proxy {
        client_builder = client_builder.no_proxy();
    }
    let client = client_builder
        .build()
        .map_err(|err| AppError::Other(err.to_string()))?;
    let response = client
        .get(cover_url.clone())
        .header(REFERER, cover_referer(&cover_url))
        .header(USER_AGENT, COVER_USER_AGENT)
        .header(
            ACCEPT,
            "image/webp,image/png,image/jpeg,image/*;q=0.8,*/*;q=0.1",
        )
        .send()
        .await?
        .error_for_status()?;

    log::info!(
        target: "cover-fetch",
        "response host={}, status={}, content_length={:?}, elapsed_ms={}",
        cover_host,
        response.status(),
        response.content_length(),
        request_started.elapsed().as_millis(),
    );

    validate_cover_url(response.url())?;

    if response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_COVER_BYTES)
    {
        return Err(AppError::Api("Cover image is too large".into()));
    }

    let bytes = response.bytes().await?;
    if bytes.len() as u64 > MAX_COVER_BYTES {
        return Err(AppError::Api("Cover image is too large".into()));
    }
    let mime_type = detect_image_mime(&bytes)
        .ok_or_else(|| AppError::Api("Cover response is not a supported image".into()))?;
    log::info!(
        target: "cover-fetch",
        "body ready host={}, bytes={}, mime={}, elapsed_ms={}",
        cover_host,
        bytes.len(),
        mime_type,
        request_started.elapsed().as_millis(),
    );

    let publish_started = Instant::now();
    let cache_for_publish = cache.clone();
    let body_bytes = bytes.len();
    let publish_bytes = bytes;
    let (data_url, publish_worker_ms) = tokio::task::spawn_blocking(move || {
        let worker_started = Instant::now();
        cache_for_publish.publish(&publish_bytes, mime_type)?;
        let data_url = cover_data_url(&publish_bytes, mime_type);
        Ok::<_, AppError>((data_url, worker_started.elapsed().as_millis()))
    })
    .await
    .map_err(|err| AppError::Other(err.to_string()))??;
    log::info!(
        target: "cover-cache",
        "published host={}, bytes={}, elapsed_ms={}, total_ms={}",
        cover_host,
        body_bytes,
        publish_started.elapsed().as_millis(),
        request_started.elapsed().as_millis(),
    );
    log::info!(
        target: "cover-cache",
        "publish worker host={}, worker_ms={}, data_url_chars={}",
        cover_host,
        publish_worker_ms,
        data_url.len(),
    );

    Ok(data_url)
}

fn normalize_cover_url(raw_url: &str) -> AppResult<Url> {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return Err(AppError::Api("Cover URL is empty".into()));
    }

    let normalized = if trimmed.starts_with("//") {
        format!("https:{trimmed}")
    } else {
        trimmed.to_string()
    };
    let mut url = Url::parse(&normalized)
        .map_err(|_| AppError::Api("Cover URL is invalid".into()))?;
    if url.scheme() == "http" {
        url.set_scheme("https")
            .map_err(|_| AppError::Api("Failed to upgrade cover URL".into()))?;
    }
    url.set_fragment(None);
    validate_cover_url(&url)?;

    Ok(url)
}

fn validate_cover_url(url: &Url) -> AppResult<()> {
    if url.scheme() != "https" {
        return Err(AppError::Api("Cover URL must use HTTPS".into()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::Api("Cover URL credentials are not allowed".into()));
    }
    if url.port().is_some_and(|port| port != 443) {
        return Err(AppError::Api("Cover URL port is not allowed".into()));
    }

    let host = url
        .host_str()
        .ok_or_else(|| AppError::Api("Cover URL has no host".into()))?;
    if !is_allowed_cover_image_host(host) {
        return Err(AppError::Api("Cover image host is not allowed".into()));
    }
    Ok(())
}

impl CoverDiskCache {
    fn new(root: PathBuf, cache_key: &str, max_cache_bytes: u64) -> AppResult<Self> {
        let digest = hex::encode(Sha256::digest(cache_key.as_bytes()));
        let shard = digest.get(0..2).unwrap_or("00");
        let directory = root.join(shard);
        std::fs::create_dir_all(&directory)
            .map_err(|err| AppError::Other(err.to_string()))?;
        Ok(Self {
            root,
            ready_path: directory.join(format!("{}.ready", digest)),
            directory,
            digest,
            max_cache_bytes,
        })
    }

    fn read(&self) -> Option<CachedCover> {
        let marker = std::fs::read_to_string(&self.ready_path).ok()?;
        let marker = parse_cover_cache_marker(marker.trim())?;
        let path = self.resolve_marker_file(&marker.file_name)?;
        match validate_cached_cover(&path, &marker) {
            Ok(cached) => Some(cached),
            Err(err) => {
                log::warn!(
                    target: "cover-cache",
                    "ignoring invalid cache {}: {}",
                    path.display(),
                    err
                );
                None
            }
        }
    }

    fn publish(&self, bytes: &[u8], mime_type: &'static str) -> AppResult<()> {
        let detected_mime = detect_image_mime(bytes)
            .ok_or_else(|| AppError::Api("Cover cache input is not an image".into()))?;
        if detected_mime != mime_type || bytes.is_empty() || bytes.len() as u64 > MAX_COVER_BYTES {
            return Err(AppError::Api("Cover cache input failed validation".into()));
        }
        validate_image_decode(bytes)
            .map_err(|err| AppError::Api(format!("Cover image decode failed: {}", err)))?;

        prune_cover_cache(
            &self.root,
            self.max_cache_bytes.saturating_sub(bytes.len() as u64),
            &self.digest,
        );

        let sha256 = sha256_bytes(bytes);
        let mut temporary = tempfile::Builder::new()
            .prefix(&format!("{}.", self.digest))
            .suffix(".part")
            .tempfile_in(&self.directory)
            .map_err(|err| AppError::Other(err.to_string()))?;
        temporary
            .write_all(bytes)
            .map_err(|err| AppError::Other(err.to_string()))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|err| AppError::Other(err.to_string()))?;

        let temporary_stem = temporary
            .path()
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(&self.digest);
        let file_name = format!("{}.{}.image", temporary_stem, sha256);
        let final_path = self.directory.join(&file_name);
        temporary
            .persist_noclobber(&final_path)
            .map_err(|err| AppError::Other(err.error.to_string()))?;
        sync_parent_directory(&final_path).map_err(|err| AppError::Other(err.to_string()))?;

        let marker = format_cover_cache_marker(bytes.len() as u64, &sha256, mime_type, &file_name);
        atomic_write_cover_marker(&self.ready_path, &marker)
            .map_err(|err| AppError::Other(err.to_string()))?;
        sync_parent_directory(&self.ready_path)
            .map_err(|err| AppError::Other(err.to_string()))?;
        Ok(())
    }

    fn resolve_marker_file(&self, file_name: &str) -> Option<PathBuf> {
        let path = Path::new(file_name);
        if path.components().count() != 1
            || path.extension().and_then(|value| value.to_str()) != Some("image")
            || !file_name.starts_with(&self.digest)
        {
            return None;
        }
        Some(self.directory.join(path))
    }
}

fn validate_cached_cover(path: &Path, marker: &CoverCacheMarker) -> Result<CachedCover, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|err| format!("cache metadata unavailable: {}", err))?;
    if !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_COVER_BYTES
        || metadata.len() != marker.content_length
    {
        return Err("cover cache length mismatch".into());
    }

    let mut file = File::open(path).map_err(|err| format!("cache open failed: {}", err))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)
        .map_err(|err| format!("cache read failed: {}", err))?;
    let mime_type = detect_image_mime(&bytes)
        .ok_or_else(|| "cover cache is not a supported image".to_string())?;
    validate_image_decode(&bytes)
        .map_err(|err| format!("cover cache decode failed: {}", err))?;
    if mime_type != marker.mime_type || sha256_bytes(&bytes) != marker.sha256 {
        return Err("cover cache content hash or MIME mismatch".into());
    }
    Ok(CachedCover { bytes, mime_type })
}

fn format_cover_cache_marker(
    content_length: u64,
    sha256: &str,
    mime_type: &str,
    file_name: &str,
) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        COVER_CACHE_MARKER_VERSION, content_length, sha256, mime_type, file_name
    )
}

fn parse_cover_cache_marker(marker: &str) -> Option<CoverCacheMarker> {
    let mut parts = marker.splitn(5, ':');
    if parts.next()? != COVER_CACHE_MARKER_VERSION {
        return None;
    }
    let content_length = parts.next()?.parse::<u64>().ok()?;
    let sha256 = parts.next()?.trim().to_ascii_lowercase();
    let mime_type = parts.next()?.trim().to_string();
    let file_name = parts.next()?.trim().to_string();
    if content_length == 0
        || content_length > MAX_COVER_BYTES
        || sha256.len() != 64
        || !sha256.bytes().all(|value| value.is_ascii_hexdigit())
        || !matches!(
            mime_type.as_str(),
            "image/jpeg" | "image/png" | "image/gif" | "image/webp" | "image/avif"
        )
        || file_name.is_empty()
    {
        return None;
    }
    Some(CoverCacheMarker {
        content_length,
        sha256,
        mime_type,
        file_name,
    })
}

fn atomic_write_cover_marker(path: &Path, marker: &str) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("cover marker has no parent directory"))?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".ready-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    temporary.write_all(marker.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|err| err.error)
}

fn prune_cover_cache(root: &Path, max_cache_bytes: u64, keep_digest: &str) {
    if !root.exists() {
        return;
    }

    let mut groups: HashMap<String, Vec<(PathBuf, u64, std::time::SystemTime)>> = HashMap::new();
    collect_cover_cache_files(root, &mut groups, keep_digest);
    let mut total = groups
        .values()
        .flat_map(|files| files.iter())
        .map(|(_, size, _)| *size)
        .sum::<u64>();
    if total <= max_cache_bytes {
        return;
    }

    let mut ordered = groups.into_iter().collect::<Vec<_>>();
    ordered.sort_by_key(|(_, files)| {
        files
            .iter()
            .map(|(_, _, modified)| *modified)
            .min()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    for (_, files) in ordered {
        if total <= max_cache_bytes {
            break;
        }
        for (path, size, _) in files {
            if std::fs::remove_file(path).is_ok() {
                total = total.saturating_sub(size);
            }
        }
    }
}

fn collect_cover_cache_files(
    root: &Path,
    groups: &mut HashMap<String, Vec<(PathBuf, u64, std::time::SystemTime)>>,
    keep_digest: &str,
) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_cover_cache_files(&path, groups, keep_digest);
            continue;
        }
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        if !matches!(extension, "image" | "part" | "ready") {
            continue;
        }
        let Some(digest) = cache_file_digest(&path) else {
            continue;
        };
        if digest == keep_digest {
            continue;
        }
        let metadata = entry.metadata().ok();
        groups.entry(digest.to_string()).or_default().push((
            path,
            metadata.as_ref().map(|value| value.len()).unwrap_or(0),
            metadata
                .and_then(|value| value.modified().ok())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
        ));
    }
}

fn cache_file_digest(path: &Path) -> Option<&str> {
    let file_name = path.file_name()?.to_str()?;
    let digest = file_name.get(0..64)?;
    digest
        .bytes()
        .all(|value| value.is_ascii_hexdigit())
        .then_some(digest)
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn cover_data_url(bytes: &[u8], mime_type: &str) -> String {
    format!("data:{};base64,{}", mime_type, STANDARD.encode(bytes))
}

fn cover_referer(url: &Url) -> &'static str {
    match url.host_str().unwrap_or_default().to_ascii_lowercase().as_str() {
        host if is_host_or_subdomain(host, "hdslb.com")
            || is_host_or_subdomain(host, "biliimg.com") => BILIBILI_REFERER,
        host if is_host_or_subdomain(host, "y.qq.com")
            || is_host_or_subdomain(host, "qqmusic.qq.com")
            || is_host_or_subdomain(host, "y.gtimg.cn") => QQ_REFERER,
        host if is_host_or_subdomain(host, "ytimg.com")
            || is_host_or_subdomain(host, "ggpht.com")
            || is_host_or_subdomain(host, "googleusercontent.com") => YOUTUBE_REFERER,
        _ => NETEASE_REFERER,
    }
}

fn detect_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

fn validate_image_decode(bytes: &[u8]) -> Result<(), image::ImageError> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8_192);
    limits.max_image_height = Some(8_192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode()?;
    if image.width() == 0 || image.height() == 0 {
        return Err(image::ImageError::Limits(image::error::LimitError::from_kind(
            image::error::LimitErrorKind::DimensionError,
        )));
    }
    Ok(())
}

fn is_allowed_cover_image_host(host: &str) -> bool {
    [
        "hdslb.com",
        "biliimg.com",
        "y.qq.com",
        "qqmusic.qq.com",
        "y.gtimg.cn",
        "music.126.net",
        "ytimg.com",
        "ggpht.com",
        "googleusercontent.com",
    ]
        .iter()
        .any(|domain| is_host_or_subdomain(host, domain))
}

fn is_host_or_subdomain(host: &str, domain: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == domain || host.ends_with(&format!(".{domain}"))
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("cover cache file has no parent directory"))?;
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        detect_image_mime, normalize_cover_url, parse_cover_cache_marker, CoverDiskCache,
    };
    use base64::{engine::general_purpose::STANDARD, Engine};

    fn valid_png() -> Vec<u8> {
        STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
            .expect("valid PNG fixture")
    }

    #[test]
    fn normalizes_protocol_relative_url() {
        let url = normalize_cover_url("//i0.hdslb.com/bfs/archive/test.jpg")
            .expect("URL should be accepted");
        assert_eq!(url.as_str(), "https://i0.hdslb.com/bfs/archive/test.jpg");
    }

    #[test]
    fn upgrades_allowed_http_url() {
        let url = normalize_cover_url("http://archive.biliimg.com/test.webp")
            .expect("URL should be accepted");
        assert_eq!(url.as_str(), "https://archive.biliimg.com/test.webp");
    }

    #[test]
    fn accepts_synced_qq_and_netease_cover_hosts() {
        assert!(normalize_cover_url("https://y.qq.com/music/photo_new/cover.jpg").is_ok());
        assert!(normalize_cover_url("https://p4.music.126.net/cover.jpg").is_ok());
        assert!(normalize_cover_url("https://y.gtimg.cn/music/photo_new/cover.jpg").is_ok());
        assert!(normalize_cover_url("https://i.ytimg.com/vi/example/maxresdefault.jpg").is_ok());
    }

    #[test]
    fn rejects_untrusted_cover_host() {
        assert!(normalize_cover_url("https://example.com/test.jpg").is_err());
        assert!(normalize_cover_url("https://evily.qq.com.example.com/test.jpg").is_err());
    }

    #[test]
    fn rejects_non_https_port() {
        assert!(normalize_cover_url("https://i0.hdslb.com:8443/test.jpg").is_err());
    }

    #[test]
    fn removes_fragment_from_cache_equivalent_url() {
        let url = normalize_cover_url(
            "https://i0.hdslb.com/bfs/archive/test.jpg#ignored-fragment",
        )
        .expect("URL should be accepted");
        assert_eq!(url.as_str(), "https://i0.hdslb.com/bfs/archive/test.jpg");
    }

    #[test]
    fn rejects_url_credentials() {
        assert!(normalize_cover_url("https://user@i0.hdslb.com/test.jpg").is_err());
    }

    #[test]
    fn detects_supported_image_bytes() {
        assert_eq!(
            detect_image_mime(&[0xff, 0xd8, 0xff, 0x00]),
            Some("image/jpeg")
        );
        assert_eq!(
            detect_image_mime(b"\x89PNG\r\n\x1a\nrest"),
            Some("image/png")
        );
        assert_eq!(detect_image_mime(b"GIF89arest"), Some("image/gif"));
        assert_eq!(detect_image_mime(b"RIFF0000WEBPrest"), Some("image/webp"));
    }

    #[test]
    fn rejects_non_image_bytes_even_with_image_headers() {
        assert_eq!(detect_image_mime(b"<html>not an image</html>"), None);
    }

    #[test]
    fn publishes_only_fully_decodable_images() {
        let root = tempfile::tempdir().expect("image cache root");
        let cache = CoverDiskCache::new(
            root.path().to_path_buf(),
            "https://i0.hdslb.com/cover.png",
            1024 * 1024,
        )
        .expect("cache");
        let png = valid_png();

        cache.publish(&png, "image/png").expect("publish image");
        let cached = cache.read().expect("read image cache");

        assert_eq!(cached.bytes, png);
        assert_eq!(cached.mime_type, "image/png");
        assert!(parse_cover_cache_marker(
            &std::fs::read_to_string(&cache.ready_path).expect("marker")
        )
        .is_some());
    }

    #[test]
    fn malformed_image_header_never_creates_ready_marker() {
        let root = tempfile::tempdir().expect("image cache root");
        let cache = CoverDiskCache::new(
            root.path().to_path_buf(),
            "https://i0.hdslb.com/bad.png",
            1024 * 1024,
        )
        .expect("cache");

        assert!(cache.publish(b"\x89PNG\r\n\x1a\nnot-a-png", "image/png").is_err());
        assert!(!cache.ready_path.exists());
    }
}
