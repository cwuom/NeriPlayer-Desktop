//! 远程音源打开、预热与播放缓存查询/清理

use super::*;

/// 预开下一首的直链（总长 + 首包），切过去时省掉冷连接上的首包等待；失败不影响播放
#[tauri::command]
pub async fn prewarm_remote_audio(
    url: String,
    duration_hint_ms: Option<u64>,
    state: State<'_, AppState>,
) -> AppResult<bool> {
    let parsed = url::Url::parse(&url).map_err(|_| AppError::Audio("Invalid stream URL".into()))?;
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return Err(AppError::Audio("Invalid stream URL".into()));
    }
    let started = std::time::Instant::now();
    let warmed = neri_playback::remote::prewarm(
        &state.http(),
        &url,
        playback_referer(&url),
        duration_hint_ms.unwrap_or(0),
    )
    .await
    .inspect_err(|error| {
        log::info!(target: "remote-audio", "prewarm skipped host={}: {error}", parsed.host_str().unwrap_or("unknown"));
    })?;
    if warmed {
        log::info!(
            target: "remote-audio",
            "prewarm ready host={}, elapsed_ms={}",
            parsed.host_str().unwrap_or("unknown"),
            started.elapsed().as_millis(),
        );
    }
    Ok(warmed)
}

pub(super) async fn open_remote_audio_source(
    url: &str,
    duration_hint_ms: u64,
    request_generation: u64,
    state: &State<'_, AppState>,
    cache: Option<RemoteAudioCache>,
) -> AppResult<RemoteAudioSource> {
    RemoteAudioSource::open(
        state.http(),
        url.to_string(),
        playback_referer(url).to_string(),
        cache,
        duration_hint_ms,
        Arc::clone(&state.playback_generation),
        request_generation,
    )
    .await
}

pub(super) fn playback_cache(
    app: &AppHandle,
    cache_key: Option<&str>,
    cache_limit_bytes: Option<u64>,
    expected_content_length: Option<u64>,
    expected_duration_ms: u64,
) -> Option<RemoteAudioCache> {
    let cache_key = cache_key.map(str::trim).filter(|key| !key.is_empty())?;
    let min_cache_bytes = MIN_MEDIA_CACHE_SIZE_MB as u64 * 1024 * 1024;
    let max_allowed_cache_bytes = MAX_MEDIA_CACHE_SIZE_MB as u64 * 1024 * 1024;
    let max_cache_bytes = cache_limit_bytes
        .unwrap_or(1024 * 1024 * 1024)
        .clamp(min_cache_bytes, max_allowed_cache_bytes);
    let root = playback_cache_root(app)?;
    match RemoteAudioCache::new(
        root,
        cache_key,
        max_cache_bytes,
        expected_content_length,
        expected_duration_ms,
    ) {
        Ok(cache) => Some(cache),
        Err(err) => {
            log::warn!(target: "playback_cache", "unavailable: {}", err);
            None
        }
    }
}

fn playback_cache_root(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_cache_dir()
        .ok()
        .map(|dir| dir.join("playback-audio"))
}

/// 任一缓存键已有完整副本时返回 true（对齐 Android：完整缓存直接离线播放，不发网络解析）
#[tauri::command]
pub async fn has_cached_audio(cache_keys: Vec<String>, app: AppHandle) -> AppResult<bool> {
    let Some(root) = playback_cache_root(&app) else {
        return Ok(false);
    };
    tauri::async_runtime::spawn_blocking(move || {
        cache_keys.iter().any(|key| {
            RemoteAudioCache::new(root.clone(), key, 0, None, 0)
                .is_ok_and(|cache| cache.has_published_entry())
        })
    })
    .await
    .map_err(|error| AppError::Other(error.to_string()))
}

/// 按当前上限裁剪音频与封面缓存：启动时与调小缓存上限后执行（对齐 Android 启动即裁剪），
/// 否则要等下一首下载完成才会裁剪
pub fn prune_media_caches(app: &AppHandle, max_cache_size_mb: i32) {
    let max_bytes = (max_cache_size_mb.max(0) as u64).saturating_mul(1024 * 1024);
    if let Some(root) = playback_cache_root(app) {
        neri_playback::remote::prune_remote_audio_cache(&root, max_bytes);
    }
    if let Ok(cache_dir) = app.path().app_cache_dir() {
        crate::commands::image_cmd::prune_cover_cache_dir(&cache_dir);
    }
}
