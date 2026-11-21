//! 开始播放的命令：本地文件、缓存音频、URL 直链/流式/HLS 与回退

use super::*;

#[tauri::command]
pub async fn play_file(
    path: String,
    start_position_ms: Option<u64>,
    duration_hint_ms: Option<u64>,
    request_generation: u64,
    state: State<'_, AppState>,
) -> AppResult<u64> {
    claim_playback_request(&state, request_generation)?;
    run_player_play(Arc::clone(&state.player), move |player| {
        player.request_play_file_at_with_hint(
            &path,
            duration_hint_ms.unwrap_or(0),
            start_position_ms.unwrap_or(0),
            request_generation,
        )
    })
    .await
}

/// 使用稳定缓存键直接播放已验证音频，避免命中缓存时仍等待平台 URL 解析
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedAudioPlaybackRequest {
    pub(super) cache_key: String,
    duration_hint_ms: u64,
    pub(super) start_position_ms: Option<u64>,
    use_crossfade: bool,
    fade_out_ms: u32,
    fade_in_ms: u32,
    cache_limit_bytes: Option<u64>,
    pub(super) request_generation: u64,
}

#[tauri::command]
pub async fn play_cached_audio(
    request: CachedAudioPlaybackRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Option<u64>> {
    let CachedAudioPlaybackRequest {
        cache_key,
        duration_hint_ms,
        start_position_ms,
        use_crossfade,
        fade_out_ms,
        fade_in_ms,
        cache_limit_bytes,
        request_generation,
    } = request;
    claim_playback_request(&state, request_generation)?;
    let Some(cache) = playback_cache(
        &app,
        Some(&cache_key),
        cache_limit_bytes,
        None,
        duration_hint_ms,
    ) else {
        return Ok(None);
    };

    let validation_started = std::time::Instant::now();
    let path = tokio::task::spawn_blocking(move || cache.ready_path())
        .await
        .map_err(|err| AppError::Other(err.to_string()))?;
    let Some(path) = path else {
        return Ok(None);
    };
    ensure_playback_request(&state, request_generation)?;
    log::info!(
        target: "play_cached_audio",
        "cache hit in {}ms: {}",
        validation_started.elapsed().as_millis(),
        path.display()
    );

    let path = path.to_string_lossy().into_owned();
    let duration_ms = run_player_play(Arc::clone(&state.player), move |player| {
        if use_crossfade {
            player.request_crossfade_file_with_hint(
                &path,
                duration_hint_ms,
                fade_out_ms,
                fade_in_ms,
                request_generation,
            )
        } else {
            player.request_play_file_at_with_hint(
                &path,
                duration_hint_ms,
                start_position_ms.unwrap_or(0),
                request_generation,
            )
        }
    })
    .await?;
    Ok(Some(if duration_ms > 0 {
        duration_ms
    } else {
        duration_hint_ms
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedAudioPlaybackCandidate {
    cache_key: String,
    source: String,
    quality_key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedAudioCandidatesPlaybackRequest {
    candidates: Vec<CachedAudioPlaybackCandidate>,
    duration_hint_ms: u64,
    start_position_ms: Option<u64>,
    use_crossfade: bool,
    fade_out_ms: u32,
    fade_in_ms: u32,
    cache_limit_bytes: Option<u64>,
    request_generation: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedAudioPlaybackResult {
    duration_ms: u64,
    source: String,
    quality_key: String,
}

/// 一次后台任务按优先级检查所有稳定缓存键，避免每个音质候选各跨一次 IPC
#[tauri::command]
pub async fn play_cached_audio_candidates(
    request: CachedAudioCandidatesPlaybackRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Option<CachedAudioPlaybackResult>> {
    let CachedAudioCandidatesPlaybackRequest {
        candidates,
        duration_hint_ms,
        start_position_ms,
        use_crossfade,
        fade_out_ms,
        fade_in_ms,
        cache_limit_bytes,
        request_generation,
    } = request;
    claim_playback_request(&state, request_generation)?;
    log::info!(
        target: "play_cached_audio_candidates",
        "begin generation={}, requested={}",
        request_generation,
        candidates.len().min(MAX_CACHE_LOOKUP_CANDIDATES),
    );

    let caches = candidates
        .into_iter()
        .take(MAX_CACHE_LOOKUP_CANDIDATES)
        .filter_map(|candidate| {
            playback_cache(
                &app,
                Some(&candidate.cache_key),
                cache_limit_bytes,
                None,
                duration_hint_ms,
            )
            .map(|cache| (candidate, cache))
        })
        .collect::<Vec<_>>();
    if caches.is_empty() {
        return Ok(None);
    }

    let validation_started = std::time::Instant::now();
    let candidate_count = caches.len();
    let validation_queued_at = std::time::Instant::now();
    log::info!(
        target: "play_cached_audio_candidates",
        "validation queued candidates={}, timeout_ms={}",
        candidate_count,
        CACHE_LOOKUP_TIMEOUT.as_millis(),
    );
    let validation = tokio::task::spawn_blocking(move || {
        log::info!(
            target: "play_cached_audio_candidates",
            "validation worker started queued_ms={}",
            validation_queued_at.elapsed().as_millis(),
        );
        for (index, (candidate, cache)) in caches.into_iter().enumerate() {
            let candidate_started = std::time::Instant::now();
            let quality = candidate.quality_key.clone();
            let source = candidate.source.clone();
            log::info!(
                target: "play_cached_audio_candidates",
                "candidate start index={}, quality={}, source={}",
                index,
                quality,
                source,
            );
            let path = cache.ready_path();
            log::info!(
                target: "play_cached_audio_candidates",
                "candidate end index={}, quality={}, hit={}, elapsed_ms={}",
                index,
                quality,
                path.is_some(),
                candidate_started.elapsed().as_millis(),
            );
            if let Some(path) = path {
                return Some((candidate, path));
            }
        }
        None
    });
    let found = match tokio::time::timeout(CACHE_LOOKUP_TIMEOUT, validation).await {
        Ok(result) => result.map_err(|err| AppError::Other(err.to_string()))?,
        Err(_) => {
            log::warn!(
                target: "play_cached_audio_candidates",
                "timed out configured_ms={}, actual_ms={}",
                CACHE_LOOKUP_TIMEOUT.as_millis(),
                validation_started.elapsed().as_millis(),
            );
            return Ok(None);
        }
    };
    let Some((candidate, path)) = found else {
        log::info!(
            target: "play_cached_audio_candidates",
            "miss in {}ms",
            validation_started.elapsed().as_millis()
        );
        return Ok(None);
    };
    ensure_playback_request(&state, request_generation)?;
    log::info!(
        target: "play_cached_audio_candidates",
        "hit {}/{} in {}ms: {}",
        candidate.quality_key,
        candidate_count,
        validation_started.elapsed().as_millis(),
        path.display()
    );
    log::info!(
        target: "play_cached_audio_candidates",
        "starting cached player source={}, start_position_ms={}, crossfade={}",
        candidate.source,
        start_position_ms.unwrap_or(0),
        use_crossfade,
    );

    let path = path.to_string_lossy().into_owned();
    let duration_ms = run_player_play(Arc::clone(&state.player), move |player| {
        if use_crossfade {
            player.request_crossfade_file_with_hint(
                &path,
                duration_hint_ms,
                fade_out_ms,
                fade_in_ms,
                request_generation,
            )
        } else {
            player.request_play_file_at_with_hint(
                &path,
                duration_hint_ms,
                start_position_ms.unwrap_or(0),
                request_generation,
            )
        }
    })
    .await?;
    Ok(Some(CachedAudioPlaybackResult {
        duration_ms: if duration_ms > 0 {
            duration_ms
        } else {
            duration_hint_ms
        },
        source: candidate.source,
        quality_key: candidate.quality_key,
    }))
}

/// 从 URL 下载音频并播放（网易云 / B站 / YouTube 流式播放）
#[tauri::command]
pub async fn play_url(
    url: String,
    duration_hint_ms: u64,
    start_position_ms: Option<u64>,
    request_generation: u64,
    state: State<'_, AppState>,
) -> AppResult<u64> {
    claim_playback_request(&state, request_generation)?;
    log::info!(
        target: "play_url",
        "start: url_len={}, hint={}ms",
        url.len(),
        duration_hint_ms
    );

    let referer = playback_referer(&url);

    let start = std::time::Instant::now();
    let resp = state
        .http()
        .get(&url)
        .header("Referer", referer)
        .header("User-Agent", playback_user_agent(&url))
        .send()
        .await
        .map_err(|e| {
            log::error!(
                target: "play_url",
                "HTTP send error: {}",
                summarize_playback_error(&e),
            );
            AppError::Network(e)
        })?;

    let status = resp.status();
    log::info!(target: "play_url", "HTTP status: {}", status);

    if !status.is_success() {
        return Err(AppError::Api(format!(
            "HTTP {}: stream fetch failed",
            status
        )));
    }

    let bytes = resp.bytes().await.map_err(|e| {
        log::error!(
            target: "play_url",
            "body read error: {}",
            summarize_playback_error(&e),
        );
        AppError::Network(e)
    })?;

    log::info!(
        target: "play_url",
        "downloaded {} bytes in {}ms",
        bytes.len(),
        start.elapsed().as_millis()
    );

    if bytes.is_empty() {
        return Err(AppError::Audio("Empty audio data received".into()));
    }

    ensure_playback_request(&state, request_generation)?;
    let data = bytes.to_vec();
    run_player_play(Arc::clone(&state.player), move |player| {
        player.request_play_bytes_at(
            data,
            duration_hint_ms,
            start_position_ms.unwrap_or(0),
            request_generation,
        )
    })
    .await
}

/// 快速播放 URL：把响应落到临时文件后按本地文件播放。
/// 相比 play_url 的整首下载到 Vec 再 Cursor 解码，File + BufReader 通常能减少内存拷贝和解码初始化等待。
// Tauri 命令签名由 IPC 契约决定：参数必须平铺，改成结构体会同时改掉前端调用点
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn play_url_fast(
    url: String,
    stream_type: Option<YtStreamType>,
    duration_hint_ms: u64,
    start_position_ms: Option<u64>,
    cache_key: Option<String>,
    cache_limit_bytes: Option<u64>,
    expected_content_length: Option<u64>,
    expected_content_md5: Option<String>,
    request_generation: u64,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<u64> {
    claim_playback_request(&state, request_generation)?;
    if stream_type == Some(YtStreamType::Hls) {
        let cache = playback_cache(
            &app,
            cache_key.as_deref(),
            cache_limit_bytes,
            None,
            duration_hint_ms,
        );
        return play_hls_url(
            url,
            duration_hint_ms,
            start_position_ms.unwrap_or(0),
            None,
            &state,
            request_generation,
            cache,
        )
        .await;
    }
    log::info!(
        target: "play_url_fast",
        "start: url_len={}, hint={}ms",
        url.len(),
        duration_hint_ms
    );
    let cache = playback_cache(
        &app,
        cache_key.as_deref(),
        cache_limit_bytes,
        expected_content_length,
        duration_hint_ms,
    );
    let cache = cache
        .map(|cache| cache.with_expected_md5(expected_content_md5.as_deref()))
        .transpose()?;
    let path = match cache {
        Some(cache) => download_url_to_playback_cache(&url, &state, cache, request_generation)
            .await?
            .to_string_lossy()
            .to_string(),
        None => download_url_to_temp_audio(&url, &state, request_generation).await?,
    };
    ensure_playback_request(&state, request_generation)?;
    log::info!(target: "play_url_fast", "temp ready: {}", path);
    let dur = run_player_play(Arc::clone(&state.player), move |player| {
        player.request_play_file_at_with_hint(
            &path,
            duration_hint_ms,
            start_position_ms.unwrap_or(0),
            request_generation,
        )
    })
    .await?;
    Ok(if dur > 0 { dur } else { duration_hint_ms })
}

/// 远程 URL 播放主路径：优先 HTTP Range 按需读取，Range 不可用时再退回增长缓冲
// Tauri 命令签名由 IPC 契约决定：参数必须平铺，改成结构体会同时改掉前端调用点
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn play_url_streaming(
    url: String,
    stream_type: Option<YtStreamType>,
    duration_hint_ms: u64,
    start_position_ms: Option<u64>,
    cache_key: Option<String>,
    cache_limit_bytes: Option<u64>,
    expected_content_length: Option<u64>,
    expected_content_md5: Option<String>,
    request_generation: u64,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<u64> {
    let command_started = std::time::Instant::now();
    let host = playback_url_host(&url);
    claim_playback_request(&state, request_generation)?;
    if stream_type == Some(YtStreamType::Hls) {
        let cache = playback_cache(
            &app,
            cache_key.as_deref(),
            cache_limit_bytes,
            None,
            duration_hint_ms,
        );
        return play_hls_url(
            url,
            duration_hint_ms,
            start_position_ms.unwrap_or(0),
            None,
            &state,
            request_generation,
            cache,
        )
        .await;
    }
    log::info!(
        target: "play_url_streaming",
        "start generation={}, host={}, url_len={}, hint_ms={}, start_ms={}, cache={}",
        request_generation,
        host,
        url.len(),
        duration_hint_ms,
        start_position_ms.unwrap_or(0),
        cache_key.is_some(),
    );
    let start_position_ms = start_position_ms.unwrap_or(0);
    let cache = playback_cache(
        &app,
        cache_key.as_deref(),
        cache_limit_bytes,
        expected_content_length,
        duration_hint_ms,
    );

    let cache = cache
        .map(|cache| cache.with_expected_md5(expected_content_md5.as_deref()))
        .transpose()?;
    if let Some(path) = lookup_ready_playback_cache(cache.as_ref(), request_generation).await? {
        log::info!(
            target: "play_url_streaming",
            "playback cache hit generation={}, elapsed_ms={}",
            request_generation,
            command_started.elapsed().as_millis(),
        );
        ensure_playback_request(&state, request_generation)?;
        let cached_path = path.to_string_lossy().to_string();
        let result = run_player_play(Arc::clone(&state.player), move |player| {
            player.request_play_file_at_with_hint(
                &cached_path,
                duration_hint_ms,
                start_position_ms,
                request_generation,
            )
        })
        .await;
        match result {
            Ok(dur) => return Ok(if dur > 0 { dur } else { duration_hint_ms }),
            Err(err) => {
                ensure_playback_request(&state, request_generation)?;
                log::warn!(
                    target: "play_url_streaming",
                    "cache playback failed, bypassing for this request: {}",
                    summarize_playback_error(&err),
                );
                if let Some(cache) = &cache {
                    cache.bypass_ready_for_session();
                }
            }
        }
    }

    let fallback_cache = cache
        .as_ref()
        .and_then(|cache| match cache.fresh_staging() {
            Ok(cache) => Some(cache),
            Err(err) => {
                log::warn!(target: "play_url_streaming", "fallback cache unavailable: {}", err);
                None
            }
        });

    let remote_open_started = std::time::Instant::now();
    log::info!(
        target: "play_url_streaming",
        "remote open begin generation={}, host={}, elapsed_ms={}",
        request_generation,
        host,
        command_started.elapsed().as_millis(),
    );
    match open_remote_audio_source(&url, duration_hint_ms, request_generation, &state, cache).await
    {
        Ok(reader) => {
            log::info!(
                target: "play_url_streaming",
                "remote open ready generation={}, host={}, open_ms={}, elapsed_ms={}",
                request_generation,
                host,
                remote_open_started.elapsed().as_millis(),
                command_started.elapsed().as_millis(),
            );
            ensure_playback_request(&state, request_generation)?;
            let result = run_player_play(Arc::clone(&state.player), move |player| {
                player.request_play_remote_at(
                    reader,
                    duration_hint_ms,
                    start_position_ms,
                    request_generation,
                )
            })
            .await;
            match result {
                Ok(dur) => return Ok(if dur > 0 { dur } else { duration_hint_ms }),
                Err(err) => {
                    ensure_playback_request(&state, request_generation)?;
                    log::warn!(
                        target: "play_url_streaming",
                        "remote playback failed: {}",
                        summarize_playback_error(&err),
                    );
                }
            }
        }
        Err(err) => {
            ensure_playback_request(&state, request_generation)?;
            log::warn!(
                target: "play_url_streaming",
                "remote source unavailable generation={}, host={}, open_ms={}, error={}",
                request_generation,
                host,
                remote_open_started.elapsed().as_millis(),
                summarize_playback_error(&err),
            );
        }
    }

    log::info!(
        target: "play_url_streaming",
        "fallback begin generation={}, host={}, elapsed_ms={}",
        request_generation,
        host,
        command_started.elapsed().as_millis(),
    );
    play_url_streaming_fallback(
        &url,
        duration_hint_ms,
        start_position_ms,
        request_generation,
        fallback_cache,
        &state,
    )
    .await
}

pub(super) async fn play_hls_url(
    url: String,
    duration_hint_ms: u64,
    start_position_ms: u64,
    fade: Option<(u32, u32)>,
    state: &State<'_, AppState>,
    request_generation: u64,
    cache: Option<RemoteAudioCache>,
) -> AppResult<u64> {
    if let Some(path) = lookup_ready_playback_cache(cache.as_ref(), request_generation).await? {
        ensure_playback_request(state, request_generation)?;
        let path = path.to_string_lossy().to_string();
        let result = run_player_play(Arc::clone(&state.player), move |player| {
            if let Some((fade_out_ms, fade_in_ms)) = fade {
                player.request_crossfade_file_with_hint(
                    &path,
                    duration_hint_ms,
                    fade_out_ms,
                    fade_in_ms,
                    request_generation,
                )
            } else {
                player.request_play_file_at_with_hint(
                    &path,
                    duration_hint_ms,
                    start_position_ms,
                    request_generation,
                )
            }
        })
        .await;
        match result {
            Ok(duration) => {
                return Ok(if duration > 0 {
                    duration
                } else {
                    duration_hint_ms
                })
            }
            Err(_) => {
                ensure_playback_request(state, request_generation)?;
                if let Some(cache) = &cache {
                    cache.bypass_ready_for_session();
                }
            }
        }
    }
    let stream = neri_playback::hls::start_hls_stream(
        state.hls_transport(),
        url,
        Arc::clone(&state.playback_generation),
        request_generation,
        cache,
    )
    .await?;
    let startup = stream.buffer.clone();
    let ready =
        tokio::task::spawn_blocking(move || startup.wait_for_buffer(1, STREAM_START_TIMEOUT))
            .await
            .map_err(|error| AppError::Other(error.to_string()))
            .and_then(|result| result.map_err(AppError::Audio));
    if let Err(error) = ready {
        stream.buffer.abort();
        return Err(error);
    }
    if let Err(error) = ensure_playback_request(state, request_generation) {
        stream.buffer.abort();
        return Err(error);
    }
    let reader = stream.buffer.reader();
    let result = run_player_play(Arc::clone(&state.player), move |player| {
        if let Some((fade_out_ms, fade_in_ms)) = fade {
            player.request_crossfade_stream(
                reader,
                duration_hint_ms,
                fade_out_ms,
                fade_in_ms,
                request_generation,
            )
        } else {
            player.request_play_stream_at(
                reader,
                duration_hint_ms,
                start_position_ms,
                request_generation,
            )
        }
    })
    .await;
    match result {
        Ok(duration) => {
            stream.commit();
            Ok(if duration > 0 {
                duration
            } else {
                duration_hint_ms
            })
        }
        Err(error) => {
            stream.buffer.abort();
            Err(error)
        }
    }
}

async fn play_url_streaming_fallback(
    url: &str,
    duration_hint_ms: u64,
    start_position_ms: u64,
    request_generation: u64,
    cache: Option<RemoteAudioCache>,
    state: &State<'_, AppState>,
) -> AppResult<u64> {
    ensure_playback_request(state, request_generation)?;
    if start_position_ms > 0 {
        // 中途起播也复用本次已创建的隔离 staging，完整下载成功后发布为可复用缓存。
        // 没有缓存配置时才落到临时文件，避免每次 Range 降级都遗留整曲副本
        let path: PathBuf = match cache {
            Some(cache) => {
                download_url_to_playback_cache(url, state, cache, request_generation).await?
            }
            None => download_url_to_temp_audio(url, state, request_generation)
                .await?
                .into(),
        };
        ensure_playback_request(state, request_generation)?;
        let path = path.to_string_lossy().into_owned();
        let dur = run_player_play(Arc::clone(&state.player), move |player| {
            player.request_play_file_at_with_hint(
                &path,
                duration_hint_ms,
                start_position_ms,
                request_generation,
            )
        })
        .await?;
        return Ok(if dur > 0 { dur } else { duration_hint_ms });
    }

    let buffer =
        start_streaming_download(url, state, "play_url_streaming", request_generation, cache)
            .await?;
    let buffer_wait_started = std::time::Instant::now();
    log::info!(
        target: "play_url_streaming",
        "startup buffer wait generation={}, target_bytes={}",
        request_generation,
        STREAM_START_BUFFER_BYTES,
    );
    let buffered = match wait_for_stream_start(buffer.clone()).await {
        Ok(buffered) => buffered,
        Err(err) => {
            buffer.abort();
            return Err(err);
        }
    };
    log::info!(
        target: "play_url_streaming",
        "startup buffer ready generation={}, bytes={}, wait_ms={}",
        request_generation,
        buffered,
        buffer_wait_started.elapsed().as_millis(),
    );

    let reader = buffer.reader();
    if let Err(err) = ensure_playback_request(state, request_generation) {
        buffer.abort();
        return Err(err);
    }
    let dur = match run_player_play(Arc::clone(&state.player), move |player| {
        player.request_play_stream_at(
            reader,
            duration_hint_ms,
            start_position_ms,
            request_generation,
        )
    })
    .await
    {
        Ok(dur) => dur,
        Err(err) => {
            buffer.abort();
            return Err(err);
        }
    };
    Ok(if dur > 0 { dur } else { duration_hint_ms })
}
