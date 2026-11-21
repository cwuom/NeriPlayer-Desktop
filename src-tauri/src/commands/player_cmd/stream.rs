//! 取流请求头与下载：整段下载、临时文件与边下边播

use super::*;

/// B 站音频流：bilivideo CDN 与 `*.mountaintoys.cn` 边缘节点（对齐 Android BiliStreamUrls）
fn is_bili_stream_url(url: &str) -> bool {
    if url.contains("bilibili.com") {
        return true;
    }
    match reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
    {
        Some(host) => host.contains("bilivideo.") || host.ends_with(".mountaintoys.cn"),
        None => url.contains("bilivideo."),
    }
}

pub(crate) fn playback_referer(url: &str) -> &'static str {
    if is_bili_stream_url(url) {
        "https://www.bilibili.com"
    } else if url.contains("youtube.com") || url.contains("googlevideo.com") {
        "https://music.youtube.com"
    } else if url.contains("qqmusic.qq.com") || url.contains("y.qq.com") {
        "https://y.qq.com"
    } else {
        "https://music.163.com"
    }
}

// 通用桌面 Chrome UA (非 YouTube 直链使用)
const DEFAULT_PLAYBACK_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// 按 URL 选择拉流 User-Agent.
/// YouTube googlevideo CDN 会校验拉流 UA 与直链 `c=` 客户端一致, 不一致直接 403;
/// 因此 googlevideo 直链必须按铸造它的客户端 (IOS/ANDROID/...) 选择匹配 UA (对齐 Android).
/// 其它平台仍用桌面 Chrome UA.
pub(super) fn playback_user_agent(url: &str) -> &'static str {
    if url.contains("googlevideo.com") || url.contains("youtube.com") {
        neri_platform::api::youtube::playback::stream_user_agent_for_url(url)
    } else {
        DEFAULT_PLAYBACK_USER_AGENT
    }
}

pub(super) async fn download_url_bytes(
    url: &str,
    state: &State<'_, AppState>,
    tag: &str,
    request_generation: u64,
) -> AppResult<Vec<u8>> {
    let start = std::time::Instant::now();
    ensure_playback_request(state, request_generation)?;
    let download = async {
        let resp = state
            .http()
            .get(url)
            .header("Referer", playback_referer(url))
            .header("User-Agent", playback_user_agent(url))
            .send()
            .await
            .map_err(AppError::Network)?;

        if !resp.status().is_success() {
            return Err(AppError::Api(format!(
                "HTTP {}: stream fetch failed",
                resp.status()
            )));
        }

        let bytes = resp.bytes().await.map_err(AppError::Network)?;
        if bytes.is_empty() {
            return Err(AppError::Audio("Empty audio data received".into()));
        }
        Ok(bytes)
    };
    tokio::pin!(download);
    let bytes = tokio::select! {
        result = &mut download => result?,
        () = wait_for_playback_superseded(
            &state.playback_generation,
            request_generation,
        ) => {
            return Err(AppError::Audio(PLAYBACK_SUPERSEDED_ERROR.into()));
        }
    };
    log::info!(
        target: tag,
        "downloaded {} bytes in {}ms",
        bytes.len(),
        start.elapsed().as_millis()
    );
    Ok(bytes.to_vec())
}

/// 回收系统临时目录中遗留的 neri-playback-*.audio 孤儿文件（SR-09）
///
/// 回退播放路径用 .keep() 持久化临时整曲文件后交给播放器打开，其生命周期脱离本模块
/// 且无清理钩子，会在临时目录无限堆积。用 mtime 年龄阈值只回收明显过期的：单曲播放
/// 时长远小于阈值，正在使用的文件 mtime 必新于阈值，不会被误删（best-effort，忽略 IO 错误）
fn sweep_orphaned_temp_audio() {
    const ORPHAN_MAX_AGE: Duration = Duration::from_secs(6 * 60 * 60);
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with("neri-playback-") || !name.ends_with(".audio") {
            continue;
        }
        let expired = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= ORPHAN_MAX_AGE);
        if expired {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

pub(super) async fn download_url_to_temp_audio(
    url: &str,
    state: &State<'_, AppState>,
    request_generation: u64,
) -> AppResult<String> {
    let data = download_url_bytes(url, state, "playback_temp", request_generation).await?;
    tokio::task::spawn_blocking(move || -> AppResult<String> {
        // 创建新临时文件前顺带回收上次会话遗留的同前缀孤儿文件
        sweep_orphaned_temp_audio();
        let mut file = tempfile::Builder::new()
            .prefix("neri-playback-")
            .suffix(".audio")
            .tempfile()
            .map_err(|e| AppError::Other(e.to_string()))?;
        file.write_all(&data)
            .map_err(|e| AppError::Other(e.to_string()))?;
        let (_file, path) = file
            .keep()
            .map_err(|e| AppError::Other(e.error.to_string()))?;
        Ok(path.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| AppError::Other(e.to_string()))?
}

pub(super) async fn download_url_to_playback_cache(
    url: &str,
    state: &State<'_, AppState>,
    cache: RemoteAudioCache,
    request_generation: u64,
) -> AppResult<PathBuf> {
    let data =
        download_url_bytes(url, state, "playback_cache_fallback", request_generation).await?;
    ensure_playback_request(state, request_generation)?;
    tokio::task::spawn_blocking(move || cache.publish_complete_bytes(&data))
        .await
        .map_err(|err| AppError::Other(err.to_string()))?
}

pub(super) async fn start_streaming_download(
    url: &str,
    state: &State<'_, AppState>,
    tag: &'static str,
    request_generation: u64,
    cache: Option<RemoteAudioCache>,
) -> AppResult<GrowingAudioBuffer> {
    let start = std::time::Instant::now();
    let host = playback_url_host(url);
    ensure_playback_request(state, request_generation)?;
    log::info!(
        target: tag,
        "HTTP stream begin generation={}, host={}, cache={}",
        request_generation,
        host,
        cache.is_some(),
    );
    let request = state
        .http()
        .get(url)
        .header("Referer", playback_referer(url))
        .header("User-Agent", playback_user_agent(url))
        .send();
    tokio::pin!(request);
    let resp = tokio::select! {
        result = &mut request => result.map_err(|error| {
            log::error!(
                target: tag,
                "HTTP send error: {}",
                summarize_playback_error(&error),
            );
            AppError::Network(error)
        })?,
        () = wait_for_playback_superseded(
            &state.playback_generation,
            request_generation,
        ) => {
            return Err(AppError::Audio(PLAYBACK_SUPERSEDED_ERROR.into()));
        }
    };

    let status = resp.status();
    log::info!(
        target: tag,
        "HTTP headers generation={}, host={}, status={}, content_length={:?}, elapsed_ms={}",
        request_generation,
        host,
        status,
        resp.content_length(),
        start.elapsed().as_millis(),
    );
    if !status.is_success() {
        return Err(AppError::Api(format!(
            "HTTP {}: stream fetch failed",
            status
        )));
    }

    let total_len = resp.content_length();
    let buffer = GrowingAudioBuffer::new();
    buffer.set_total_len(total_len);
    let writer = buffer.clone();
    let mut stream = resp.bytes_stream();
    let playback_generation = Arc::clone(&state.playback_generation);

    let mut cache_file = match (cache.as_ref(), total_len) {
        (Some(cache), Some(total_len)) => {
            let cache = cache.clone();
            let prepared =
                tokio::task::spawn_blocking(move || cache.prepare_sequential_write(total_len))
                    .await;
            match prepared {
                Ok(Ok(path)) => match tokio::fs::OpenOptions::new().write(true).open(path).await {
                    Ok(file) => Some(file),
                    Err(error) => {
                        log::warn!(target: tag, "sequential cache unavailable: {}", error);
                        None
                    }
                },
                Ok(Err(error)) => {
                    log::warn!(target: tag, "sequential cache unavailable: {}", error);
                    None
                }
                Err(error) => {
                    log::warn!(target: tag, "sequential cache task unavailable: {}", error);
                    None
                }
            }
        }
        _ => None,
    };

    tauri::async_runtime::spawn(async move {
        let mut downloaded: usize = 0;
        let mut cache = cache;
        let mut first_chunk_logged = false;
        loop {
            let item = tokio::select! {
                item = tokio::time::timeout(STREAM_IDLE_TIMEOUT, stream.next()) => {
                    match item {
                        Ok(item) => item,
                        Err(_) => {
                            // 空闲超时 = 网络停滞：必须 fail 而非静默挂着，
                            // 否则 Growing read 侧会带着半首歌永久阻塞
                            log::error!(
                                target: tag,
                                "stream stalled after {} bytes (no data for {}s)",
                                downloaded,
                                STREAM_IDLE_TIMEOUT.as_secs(),
                            );
                            writer.fail("stream stalled".into());
                            return;
                        }
                    }
                }
                () = wait_for_playback_superseded(
                    &playback_generation,
                    request_generation,
                ) => {
                    writer.abort();
                    log::info!(target: tag, "download superseded after {} bytes", downloaded);
                    return;
                }
            };
            // 每轮兜底检查代际：select 分支被数据流持续命中时，
            // superseded 等待分支可能长期得不到调度
            if playback_generation.load(Ordering::Acquire) != request_generation {
                writer.abort();
                log::info!(target: tag, "download superseded after {} bytes", downloaded);
                return;
            }
            let Some(item) = item else {
                break;
            };
            if writer.is_aborted() {
                log::info!(target: tag, "download aborted after {} bytes", downloaded);
                return;
            }
            match item {
                Ok(chunk) => {
                    downloaded += chunk.len();
                    if !first_chunk_logged {
                        first_chunk_logged = true;
                        log::info!(
                            target: tag,
                            "first chunk generation={}, bytes={}, elapsed_ms={}",
                            request_generation,
                            chunk.len(),
                            start.elapsed().as_millis(),
                        );
                    }
                    writer.append(&chunk);
                    if let Some(file) = cache_file.as_mut() {
                        if let Err(error) = file.write_all(&chunk).await {
                            log::warn!(target: tag, "sequential cache write failed: {}", error);
                            cache_file = None;
                            cache = None;
                        }
                    }
                }
                Err(e) => {
                    log::error!(
                        target: tag,
                        "body stream error: {}",
                        summarize_playback_error(&e),
                    );
                    writer.fail(e.to_string());
                    return;
                }
            }
        }
        log::info!(
            target: tag,
            "downloaded {} bytes in {}ms",
            downloaded,
            start.elapsed().as_millis()
        );
        if !stream_length_matches(total_len, downloaded as u64) {
            let expected = total_len.unwrap_or_default();
            log::error!(
                target: tag,
                "stream truncated: downloaded {} bytes, expected {}",
                downloaded,
                expected,
            );
            writer.fail(format!(
                "stream truncated: downloaded {} bytes, expected {}",
                downloaded, expected
            ));
            return;
        }
        writer.finish();
        if let (Some(mut file), Some(cache)) = (cache_file, cache) {
            if let Err(error) = file.flush().await {
                log::warn!(target: tag, "sequential cache flush failed: {}", error);
                return;
            }
            drop(file);
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(error) = cache.publish_sequential_write() {
                    log::warn!(target: tag, "sequential cache publish failed: {}", error);
                }
            });
        }
    });

    Ok(buffer)
}

async fn wait_for_playback_superseded(generation: &AtomicU64, expected: u64) {
    while generation.load(Ordering::Acquire) == expected {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub(super) fn stream_length_matches(expected: Option<u64>, downloaded: u64) -> bool {
    expected.is_none_or(|expected| expected == downloaded)
}

pub(super) async fn wait_for_stream_start(buffer: GrowingAudioBuffer) -> AppResult<usize> {
    tokio::task::spawn_blocking(move || {
        buffer.wait_for_buffer(STREAM_START_BUFFER_BYTES, STREAM_START_TIMEOUT)
    })
    .await
    .map_err(|e| AppError::Other(e.to_string()))?
    .map_err(AppError::Audio)
}
