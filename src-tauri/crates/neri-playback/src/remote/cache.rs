//! 远程音频缓存：磁盘缓存文件的校验、发布与清理，以及内存分段缓存的保护与淘汰

use super::*;

static VALIDATED_AUDIO_CACHE: OnceLock<Mutex<HashMap<PathBuf, ValidatedCacheStamp>>> =
    OnceLock::new();

impl RemoteAudioCache {
    pub fn new(
        root: PathBuf,
        cache_key: &str,
        max_cache_bytes: u64,
        expected_content_length: Option<u64>,
        expected_duration_ms: u64,
    ) -> AppResult<Self> {
        let digest = hex::encode(Sha256::digest(cache_key.as_bytes()));
        let shard = digest.get(0..2).unwrap_or("00");
        let dir = root.join(shard);
        Ok(Self {
            cache_root: root,
            cache_dir: dir.clone(),
            digest: digest.clone(),
            legacy_data_path: dir.join(format!("{}.audio", digest)),
            staging: Arc::new(Mutex::new(CacheStaging { path: None })),
            ready_path: dir.join(format!("{}.ready", digest)),
            expected_content_length: expected_content_length.filter(|length| *length > 0),
            expected_content_md5: None,
            expected_duration_ms: (expected_duration_ms > 0).then_some(expected_duration_ms),
            max_cache_bytes,
            published_path: Arc::new(Mutex::new(None)),
            bypass_ready: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn with_expected_md5(mut self, value: Option<&str>) -> AppResult<Self> {
        if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
            if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(AppError::Api("Playback source checksum is invalid".into()));
            }
            self.expected_content_md5 = Some(value.to_ascii_lowercase());
        }
        Ok(self)
    }

    fn validate_expected_md5(&self, path: &Path) -> io::Result<()> {
        let Some(expected) = self.expected_content_md5.as_deref() else {
            return Ok(());
        };
        let mut file = File::open(path)?;
        let mut digest = md5::Md5::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        if hex::encode(digest.finalize()) != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cache MD5 does not match playback source",
            ));
        }
        Ok(())
    }

    pub fn ready_path(&self) -> Option<PathBuf> {
        let lookup_started = Instant::now();
        let digest_prefix = self.digest.get(..8).unwrap_or(&self.digest);
        if self.bypass_ready.load(Ordering::Acquire) {
            log::info!(
                target: "remote-cache",
                "lookup bypassed digest={} elapsed_ms={}",
                digest_prefix,
                lookup_started.elapsed().as_millis(),
            );
            return None;
        }

        let marker_text = match std::fs::read_to_string(&self.ready_path) {
            Ok(value) => value,
            Err(error) => {
                log::info!(
                    target: "remote-cache",
                    "lookup miss digest={} reason=marker_read error_kind={:?} elapsed_ms={}",
                    digest_prefix,
                    error.kind(),
                    lookup_started.elapsed().as_millis(),
                );
                return None;
            }
        };
        let marker = match parse_cache_marker(marker_text.trim()) {
            Some(marker) => marker,
            None => {
                log::warn!(
                    target: "remote-cache",
                    "lookup miss digest={} reason=marker_parse elapsed_ms={}",
                    digest_prefix,
                    lookup_started.elapsed().as_millis(),
                );
                return None;
            }
        };
        let (path, marked_length, expected_sha256, file_name) = match &marker {
            CacheMarker::Legacy { .. } => {
                log::warn!(
                    target: "remote-cache",
                    "lookup miss digest={} reason=legacy_marker elapsed_ms={}",
                    digest_prefix,
                    lookup_started.elapsed().as_millis(),
                );
                return None;
            }
            CacheMarker::Validated {
                content_length,
                sha256,
                file_name,
            } => (
                self.resolve_marker_file(file_name)?,
                *content_length,
                sha256.as_str(),
                file_name.as_str(),
            ),
        };

        if let Err(err) = validate_published_cache_file(
            &path,
            marked_length,
            self.expected_content_length,
            self.expected_duration_ms,
            expected_sha256,
            file_name,
        ) {
            log::warn!(
                target: "remote-cache",
                "lookup miss digest={} reason=validation error={} elapsed_ms={}",
                digest_prefix,
                err,
                lookup_started.elapsed().as_millis(),
            );
            return None;
        }

        if self.validate_expected_md5(&path).is_err() {
            log::warn!(target: "remote-cache", "lookup miss digest={} reason=source_checksum", digest_prefix);
            return None;
        }
        if let Ok(mut published) = self.published_path.lock() {
            *published = Some(path.clone());
        }
        // 刷新 .ready 的修改时间实现最近使用淘汰；不碰 .audio，其校验戳按路径和修改时间记忆
        neri_common::fsutil::touch_modified(&self.ready_path);
        log::info!(
            target: "remote-cache",
            "lookup hit digest={} bytes={} elapsed_ms={}",
            digest_prefix,
            marked_length,
            lookup_started.elapsed().as_millis(),
        );
        Some(path)
    }

    pub fn bypass_ready_for_session(&self) {
        self.bypass_ready.store(true, Ordering::Release);
        if let Ok(mut published) = self.published_path.lock() {
            *published = None;
        }
    }

    pub fn fresh_staging(&self) -> AppResult<Self> {
        Ok(Self {
            cache_root: self.cache_root.clone(),
            cache_dir: self.cache_dir.clone(),
            digest: self.digest.clone(),
            legacy_data_path: self.legacy_data_path.clone(),
            staging: Arc::new(Mutex::new(CacheStaging { path: None })),
            ready_path: self.ready_path.clone(),
            expected_content_length: self.expected_content_length,
            expected_content_md5: self.expected_content_md5.clone(),
            expected_duration_ms: self.expected_duration_ms,
            max_cache_bytes: self.max_cache_bytes,
            published_path: Arc::new(Mutex::new(None)),
            bypass_ready: Arc::new(AtomicBool::new(true)),
        })
    }

    pub(super) fn staging_path(&self) -> io::Result<PathBuf> {
        let mut staging = self
            .staging
            .lock()
            .map_err(|_| io::Error::other("cache staging lock poisoned"))?;
        if let Some(path) = &staging.path {
            return Ok(path.clone());
        }

        let path = create_cache_staging(&self.cache_dir, &self.digest)?;
        staging.path = Some(path.clone());
        Ok(path)
    }

    pub fn publish_complete_bytes(&self, bytes: &[u8]) -> AppResult<PathBuf> {
        self.prepare(bytes.len() as u64)
            .map_err(|err| AppError::Other(err.to_string()))?;
        self.write_range(0, bytes)
            .map_err(|err| AppError::Other(err.to_string()))?;
        self.mark_ready()
            .map_err(|err| AppError::Other(err.to_string()))?;
        self.published_path()
            .map_err(|err| AppError::Other(err.to_string()))?
            .ok_or_else(|| AppError::Other("cache publish produced no readable file".into()))
    }

    pub fn prepare_sequential_write(&self, total_len: u64) -> AppResult<PathBuf> {
        self.prepare(total_len)
            .map_err(|err| AppError::Other(err.to_string()))?;
        self.staging_path()
            .map_err(|err| AppError::Other(err.to_string()))
    }

    pub fn publish_sequential_write(&self) -> AppResult<PathBuf> {
        self.mark_ready()
            .map_err(|err| AppError::Other(err.to_string()))?;
        self.published_path()
            .map_err(|err| AppError::Other(err.to_string()))?
            .ok_or_else(|| AppError::Other("cache publish produced no readable file".into()))
    }

    pub(super) fn prepare(&self, total_len: u64) -> io::Result<()> {
        if self
            .expected_content_length
            .is_some_and(|expected| expected != total_len)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "remote content length {} does not match expected {}",
                    total_len,
                    self.expected_content_length.unwrap_or_default()
                ),
            ));
        }
        if let Some(path) = self.ready_path() {
            if std::fs::metadata(&path)?.len() == total_len {
                return Ok(());
            }
        }
        if let Ok(mut published) = self.published_path.lock() {
            *published = None;
        }
        let staging_path = self.staging_path()?;
        if let Some(parent) = staging_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(staging_path)?;
        file.set_len(total_len)?;
        Ok(())
    }

    pub(super) fn write_range(&self, start: u64, data: &[u8]) -> io::Result<()> {
        if data.is_empty() || self.published_path().ok().flatten().is_some() {
            return Ok(());
        }
        let end = start
            .checked_add(data.len() as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "cache range overflow"))?;
        let staging_path = self.staging_path()?;
        let staging_len = std::fs::metadata(&staging_path)?.len();
        if end > staging_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("cache range {}..{} exceeds {}", start, end, staging_len),
            ));
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(staging_path)?;
        file.seek(SeekFrom::Start(start))?;
        file.write_all(data)?;
        Ok(())
    }

    pub(super) fn read_cached(&self, start: u64, out: &mut [u8]) -> io::Result<usize> {
        let published = self.published_path().ok().flatten();
        let path = match &published {
            Some(path) => path.clone(),
            None => self.staging_path()?,
        };
        let mut file = match std::fs::OpenOptions::new().read(true).open(path) {
            Ok(file) => file,
            Err(staging_error) if published.is_none() => {
                let Some(published_path) = self.published_path().ok().flatten() else {
                    return Err(staging_error);
                };
                std::fs::OpenOptions::new()
                    .read(true)
                    .open(published_path)?
            }
            Err(err) => return Err(err),
        };
        file.seek(SeekFrom::Start(start))?;
        file.read(out)
    }

    pub(super) fn mark_ready(&self) -> io::Result<()> {
        if self.ready_path().is_some() {
            return Ok(());
        }
        let staging_path = self.staging_path()?;
        self.validate_expected_md5(&staging_path)?;
        let validated = validate_cache_file(
            &staging_path,
            std::fs::metadata(&staging_path)?.len(),
            self.expected_content_length,
            self.expected_duration_ms,
            None,
        )
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        // Windows 的 FlushFileBuffers 需要可写句柄
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&staging_path)?
            .sync_all()?;

        let staging_stem = staging_path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(&self.digest);
        let final_file_name = format!("{}.{}.audio", staging_stem, validated.sha256);
        let final_path = self.cache_dir.join(&final_file_name);
        if final_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "cache publish target already exists: {}",
                    final_path.display()
                ),
            ));
        }
        std::fs::rename(&staging_path, &final_path)?;
        sync_parent_directory(&final_path)?;
        let final_modified = std::fs::metadata(&final_path)
            .and_then(|metadata| metadata.modified())
            .ok();
        remember_validated_audio_cache(&final_path, final_modified, &validated);
        if let Ok(mut published) = self.published_path.lock() {
            *published = Some(final_path.clone());
        }

        let marker = format_cache_marker(
            validated.content_length,
            &validated.sha256,
            &final_file_name,
        );
        atomic_write_cache_marker(&self.ready_path, &marker)?;
        sync_parent_directory(&self.ready_path)?;
        prune_disk_cache(
            &self.cache_root,
            self.max_cache_bytes
                .saturating_sub(validated.content_length),
            &self.digest,
        );
        Ok(())
    }

    fn published_path(&self) -> io::Result<Option<PathBuf>> {
        self.published_path
            .lock()
            .map(|path| path.clone())
            .map_err(|_| io::Error::other("published cache path lock poisoned"))
    }

    fn resolve_marker_file(&self, file_name: &str) -> Option<PathBuf> {
        let path = Path::new(file_name);
        if path.components().count() != 1
            || path.extension().and_then(|value| value.to_str()) != Some("audio")
            || !file_name.starts_with(&self.digest)
        {
            return None;
        }
        Some(self.cache_dir.join(path))
    }

    /// 只看 .ready 标记与已发布文件长度、不做哈希校验的快速判断：
    /// 用来决定是否值得先发网络解析。真正播放前仍走 ready_path 的完整校验
    pub fn has_published_entry(&self) -> bool {
        let Ok(marker) = std::fs::read_to_string(&self.ready_path) else {
            return false;
        };
        let Some(CacheMarker::Validated {
            content_length,
            file_name,
            ..
        }) = parse_cache_marker(marker.trim())
        else {
            return false;
        };
        self.resolve_marker_file(&file_name)
            .and_then(|path| std::fs::metadata(path).ok())
            .is_some_and(|metadata| metadata.is_file() && metadata.len() == content_length)
    }
}

fn create_cache_staging(directory: &Path, digest: &str) -> io::Result<PathBuf> {
    std::fs::create_dir_all(directory)?;
    let staging_file = tempfile::Builder::new()
        .prefix(&format!("{}.", digest))
        .suffix(".part")
        .tempfile_in(directory)?;
    let (staging_file, staging_path) = staging_file.keep().map_err(|err| err.error)?;
    drop(staging_file);
    Ok(staging_path)
}

pub(super) fn format_cache_marker(content_length: u64, sha256: &str, file_name: &str) -> String {
    format!(
        "{}:{}:{}:{}",
        CACHE_MARKER_VERSION, content_length, sha256, file_name
    )
}

pub(super) fn parse_cache_marker(marker: &str) -> Option<CacheMarker> {
    if let Some(content_length) = marker
        .strip_prefix("v1:")
        .and_then(|value| value.parse::<u64>().ok())
    {
        return (content_length > 0).then_some(CacheMarker::Legacy { content_length });
    }

    let mut parts = marker.splitn(4, ':');
    if parts.next()? != CACHE_MARKER_VERSION {
        return None;
    }
    let content_length = parts.next()?.parse::<u64>().ok()?;
    let sha256 = parts.next()?.trim().to_ascii_lowercase();
    let file_name = parts.next()?.trim().to_string();
    if content_length == 0
        || sha256.len() != 64
        || !sha256.bytes().all(|value| value.is_ascii_hexdigit())
        || file_name.is_empty()
    {
        return None;
    }
    Some(CacheMarker::Validated {
        content_length,
        sha256,
        file_name,
    })
}

fn validate_cache_file(
    path: &Path,
    marked_length: u64,
    expected_length: Option<u64>,
    expected_duration_ms: Option<u64>,
    expected_sha256: Option<&str>,
) -> Result<ValidatedCacheFile, String> {
    let metadata =
        std::fs::metadata(path).map_err(|err| format!("cache metadata unavailable: {}", err))?;
    let content_length = metadata.len();
    if !metadata.is_file() || content_length == 0 || content_length != marked_length {
        return Err(format!(
            "cache length mismatch: marked={}, actual={}",
            marked_length, content_length
        ));
    }
    if expected_length.is_some_and(|expected| expected != content_length) {
        return Err(format!(
            "cache length does not match source: expected={}, actual={}",
            expected_length.unwrap_or_default(),
            content_length
        ));
    }
    let modified = metadata.modified().ok();
    if let Some(validated) =
        reuse_validated_audio_cache(path, content_length, modified, expected_sha256)
    {
        return Ok(validated);
    }

    validate_audio_file(path, expected_duration_ms)?;
    let sha256 = sha256_file(path).map_err(|err| format!("cache hash failed: {}", err))?;
    if expected_sha256.is_some_and(|expected| !sha256.eq_ignore_ascii_case(expected)) {
        return Err("cache SHA-256 mismatch".into());
    }
    let validated = ValidatedCacheFile {
        content_length,
        sha256,
    };
    remember_validated_audio_cache(path, modified, &validated);
    Ok(validated)
}

pub(super) fn validate_published_cache_file(
    path: &Path,
    marked_length: u64,
    expected_length: Option<u64>,
    _expected_duration_ms: Option<u64>,
    expected_sha256: &str,
    file_name: &str,
) -> Result<ValidatedCacheFile, String> {
    let metadata =
        std::fs::metadata(path).map_err(|err| format!("cache metadata unavailable: {}", err))?;
    let content_length = metadata.len();
    if !metadata.is_file() || content_length == 0 || content_length != marked_length {
        return Err(format!(
            "cache length mismatch: marked={}, actual={}",
            marked_length, content_length
        ));
    }
    if expected_length.is_some_and(|expected| expected != content_length) {
        return Err(format!(
            "cache length does not match source: expected={}, actual={}",
            expected_length.unwrap_or_default(),
            content_length
        ));
    }
    let expected_suffix = format!(".{}.audio", expected_sha256);
    if !file_name.ends_with(&expected_suffix) {
        return Err("cache file name does not match published SHA-256".into());
    }

    if reuse_validated_audio_cache(
        path,
        content_length,
        metadata.modified().ok(),
        Some(expected_sha256),
    )
    .is_none()
    {
        // 冷启动或文件已变化时重新检查内容，文件名不能证明文件仍然完整
        // 校验摘要即可，无需再次解析整个音频容器
        let actual = sha256_file(path).map_err(|err| format!("cache hash failed: {err}"))?;
        if !actual.eq_ignore_ascii_case(expected_sha256) {
            return Err("cache SHA-256 mismatch".into());
        }
    }
    let validated = ValidatedCacheFile {
        content_length,
        sha256: expected_sha256.to_string(),
    };
    remember_validated_audio_cache(path, metadata.modified().ok(), &validated);
    Ok(validated)
}

pub(super) fn reuse_validated_audio_cache(
    path: &Path,
    content_length: u64,
    modified: Option<SystemTime>,
    expected_sha256: Option<&str>,
) -> Option<ValidatedCacheFile> {
    let modified = modified?;
    let expected_sha256 = expected_sha256?;
    let cache = VALIDATED_AUDIO_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let cache = cache.lock().ok()?;
    let stamp = cache.get(path)?;
    if stamp.content_length != content_length
        || stamp.modified != modified
        || !stamp.sha256.eq_ignore_ascii_case(expected_sha256)
    {
        return None;
    }
    Some(ValidatedCacheFile {
        content_length,
        sha256: stamp.sha256.clone(),
    })
}

pub(super) fn remember_validated_audio_cache(
    path: &Path,
    modified: Option<SystemTime>,
    validated: &ValidatedCacheFile,
) {
    let Some(modified) = modified else {
        return;
    };
    let cache = VALIDATED_AUDIO_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut cache) = cache.lock() else {
        return;
    };
    if cache.len() >= MAX_VALIDATED_CACHE_STAMPS && !cache.contains_key(path) {
        cache.clear();
    }
    cache.insert(
        path.to_path_buf(),
        ValidatedCacheStamp {
            content_length: validated.content_length,
            modified,
            sha256: validated.sha256.clone(),
        },
    );
}

fn validate_audio_file(path: &Path, expected_duration_ms: Option<u64>) -> Result<(), String> {
    let decoder = SymphoniaAudioDecoder::new_file(path)?;
    let actual_duration_ms = decoder
        .total_duration()
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    if cache_duration_is_suspicious(expected_duration_ms, actual_duration_ms) {
        return Err(format!(
            "cache duration is suspiciously short: expected={}ms, actual={}ms",
            expected_duration_ms.unwrap_or_default(),
            actual_duration_ms
        ));
    }
    Ok(())
}

pub(super) fn cache_duration_is_suspicious(
    expected_duration_ms: Option<u64>,
    actual_duration_ms: u64,
) -> bool {
    let Some(expected_duration_ms) = expected_duration_ms.filter(|duration| *duration > 0) else {
        return false;
    };
    actual_duration_ms > 0
        && actual_duration_ms.saturating_add(CACHE_MIN_DURATION_GAP_MS) < expected_duration_ms
        && u128::from(actual_duration_ms) * 100
            < u128::from(expected_duration_ms) * CACHE_MIN_DURATION_RATIO_PERCENT as u128
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn atomic_write_cache_marker(path: &Path, marker: &str) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("cache marker has no parent directory"))?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".ready-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    temporary.write_all(marker.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map(|_| ()).map_err(|err| err.error)
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("cache file has no parent directory"))?;
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn prune_disk_cache(root: &PathBuf, max_cache_bytes: u64, keep_digest: &str) {
    if !root.exists() {
        return;
    }

    let mut groups: HashMap<String, Vec<(PathBuf, u64, std::time::SystemTime)>> = HashMap::new();
    collect_disk_cache_files(root, &mut groups, keep_digest);
    let mut total = groups
        .values()
        .flat_map(|files| files.iter())
        .map(|(_, size, _)| *size)
        .sum::<u64>();
    if total <= max_cache_bytes {
        return;
    }

    // 按每组最新的修改时间排序：命中会刷新 .ready，常听的曲目排在后面（对齐 Android LRU 淘汰）
    let mut ordered = groups.into_iter().collect::<Vec<_>>();
    ordered.sort_by_key(|(_, files)| {
        files
            .iter()
            .map(|(_, _, modified)| *modified)
            .max()
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

/// 按上限裁剪整个音频缓存（启动时、调小缓存上限后调用；平时只在新文件发布时裁剪）
pub fn prune_remote_audio_cache(root: &Path, max_cache_bytes: u64) {
    prune_disk_cache(&root.to_path_buf(), max_cache_bytes, "");
}

fn collect_disk_cache_files(
    root: &PathBuf,
    groups: &mut HashMap<String, Vec<(PathBuf, u64, std::time::SystemTime)>>,
    keep_digest: &str,
) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_disk_cache_files(&path, groups, keep_digest);
            continue;
        }
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        if !matches!(extension, "audio" | "part" | "ready") {
            continue;
        }
        let metadata = entry.metadata().ok();
        if extension == "part"
            && metadata
                .as_ref()
                .and_then(|value| value.modified().ok())
                .and_then(|modified| modified.elapsed().ok())
                .is_none_or(|age| age < STALE_CACHE_PART_AGE)
        {
            continue;
        }
        let Some(digest) = cache_file_digest(&path) else {
            continue;
        };
        if digest == keep_digest {
            continue;
        }
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
    (digest.bytes().all(|value| value.is_ascii_hexdigit())).then_some(digest)
}

pub(super) fn contiguous_cached_end(inner: &RemoteAudioInner, pos: u64) -> u64 {
    let mut ranges = Vec::new();
    if let Ok(cache) = inner.cache.lock() {
        ranges.extend(cache.iter().map(|segment| CachedRange {
            start: segment.start,
            end: segment.start.saturating_add(segment.data.len() as u64),
        }));
    }
    if let Ok(disk_ranges) = inner.disk_ranges.lock() {
        ranges.extend(disk_ranges.iter().copied());
    }
    ranges.sort_by_key(|range| range.start);

    let mut end = pos.min(inner.total_len);
    for range in ranges {
        if range.end <= end {
            continue;
        }
        if range.start > end {
            break;
        }
        end = range.end.min(inner.total_len);
    }
    end
}

pub(super) fn write_disk_range(inner: &RemoteAudioInner, start: u64, data: &[u8]) {
    let Some(cache) = &inner.disk_cache else {
        return;
    };
    if cache.write_range(start, data).is_ok() {
        record_disk_range(inner, start, data.len() as u64);
    }
}

fn record_disk_range(inner: &RemoteAudioInner, start: u64, len: u64) {
    if len == 0 {
        return;
    }
    let mut next = CachedRange {
        start,
        end: start.saturating_add(len).min(inner.total_len),
    };
    let Ok(mut ranges) = inner.disk_ranges.lock() else {
        return;
    };
    ranges.push(next);
    ranges.sort_by_key(|range| range.start);

    let mut merged: Vec<CachedRange> = Vec::with_capacity(ranges.len());
    for range in ranges.iter().copied() {
        if let Some(last) = merged.last_mut() {
            if range.start <= last.end {
                last.end = last.end.max(range.end);
                continue;
            }
        }
        next = range;
        merged.push(next);
    }
    *ranges = merged;
}

pub(super) fn disk_cached_len(
    inner: &RemoteAudioInner,
    pos: u64,
    wanted_len: usize,
) -> Option<usize> {
    let ranges = inner.disk_ranges.lock().ok()?;
    ranges.iter().find_map(|range| {
        if pos < range.start || pos >= range.end {
            return None;
        }
        Some((range.end - pos).min(wanted_len as u64) as usize)
    })
}

pub(super) fn try_mark_disk_cache_ready(inner: &RemoteAudioInner) {
    let Some(cache) = &inner.disk_cache else {
        return;
    };
    let is_complete = inner
        .disk_ranges
        .lock()
        .map(|ranges| ranges_cover_source(&ranges, inner.total_len))
        .unwrap_or(false);
    if !is_complete
        || inner
            .cache_finalize_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        return;
    }

    let cache = cache.clone();
    if let Err(spawn_error) = std::thread::Builder::new()
        .name("audio-cache-finalize".into())
        .spawn(move || {
            if let Err(err) = cache.mark_ready() {
                log::warn!(target: "remote-audio", "cache finalize rejected: {}", err);
            }
        })
    {
        log::warn!(
            target: "remote-audio",
            "cache finalize worker unavailable: {}",
            spawn_error
        );
    }
}

pub(super) fn ranges_cover_source(ranges: &[CachedRange], total_len: u64) -> bool {
    if total_len == 0 {
        return false;
    }

    let mut covered_end = 0u64;
    for range in ranges {
        if range.start > covered_end {
            return false;
        }
        covered_end = covered_end.max(range.end);
        if covered_end >= total_len {
            return true;
        }
    }
    false
}

pub(super) fn remember_read_pos(inner: &RemoteAudioInner, pos: u64) {
    if let Ok(mut last_read_pos) = inner.last_read_pos.lock() {
        *last_read_pos = pos;
    }
}

pub(super) fn cache_center(inner: &RemoteAudioInner, fallback: u64) -> u64 {
    inner
        .last_read_pos
        .lock()
        .map(|pos| *pos)
        .unwrap_or(fallback)
}

pub(super) fn cache_contains_position(cache: &[CachedSegment], pos: u64) -> bool {
    cache.iter().any(|segment| {
        let segment_end = segment.start + segment.data.len() as u64;
        pos >= segment.start && pos < segment_end
    })
}

pub(super) fn position_is_cached_range(inner: &RemoteAudioInner, start: u64, end: u64) -> bool {
    if end < start {
        return false;
    }
    // 只要 start 已缓存就认为窗口可用（按需读会补齐）
    if let Ok(cache) = inner.cache.lock() {
        if cache_contains_position(&cache, start) {
            return true;
        }
    }
    disk_cached_len(inner, start, 1).is_some()
}

pub(super) fn protect_cached_range(inner: &RemoteAudioInner, start: u64, len: u64) {
    if len == 0 {
        return;
    }
    let next = CachedRange {
        start,
        end: start.saturating_add(len).min(inner.total_len),
    };
    let Ok(mut ranges) = inner.protected_ranges.lock() else {
        return;
    };
    ranges.push(next);
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<CachedRange> = Vec::with_capacity(ranges.len());
    for range in ranges.iter().copied() {
        if let Some(last) = merged.last_mut() {
            if range.start <= last.end {
                last.end = last.end.max(range.end);
                continue;
            }
        }
        merged.push(range);
    }
    *ranges = merged;
}

fn range_overlaps(a: CachedRange, b: CachedRange) -> bool {
    a.start < b.end && b.start < a.end
}

fn segment_is_protected(inner: &RemoteAudioInner, segment: &CachedSegment) -> bool {
    let segment_range = CachedRange {
        start: segment.start,
        end: segment.start.saturating_add(segment.data.len() as u64),
    };
    let Ok(ranges) = inner.protected_ranges.lock() else {
        return false;
    };
    ranges
        .iter()
        .any(|range| range_overlaps(*range, segment_range))
}

pub(super) fn trim_cache(
    inner: &RemoteAudioInner,
    cache: &mut Vec<CachedSegment>,
    current_pos: u64,
) {
    let mut total: usize = cache.iter().map(|segment| segment.data.len()).sum();
    if total <= REMOTE_MAX_CACHE_BYTES {
        return;
    }

    // 先丢离读头最远且未受保护（非头/尾 moov）的块
    cache.sort_by_key(|segment| {
        let protected = segment_is_protected(inner, segment);
        (protected, segment_distance(segment, current_pos))
    });
    while total > REMOTE_MAX_CACHE_BYTES {
        let Some(segment) = cache.pop() else {
            break;
        };
        // 只剩保护块时停止，避免把 moov 裁掉导致 seek 重开 demuxer 扫全文件
        if segment_is_protected(inner, &segment) {
            cache.push(segment);
            break;
        }
        total = total.saturating_sub(segment.data.len());
    }
    cache.sort_by_key(|segment| segment.start);
}

fn segment_distance(segment: &CachedSegment, current_pos: u64) -> u64 {
    let segment_end = segment.start + segment.data.len() as u64;
    if current_pos < segment.start {
        segment.start - current_pos
    } else {
        current_pos.saturating_sub(segment_end)
    }
}
