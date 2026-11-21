//! 交叉淡入淡出切歌命令

use super::*;

#[tauri::command]
pub async fn crossfade_url(
    url: String,
    duration_hint_ms: u64,
    fade_out_ms: u32,
    fade_in_ms: u32,
    request_generation: u64,
    state: State<'_, AppState>,
) -> AppResult<u64> {
    claim_playback_request(&state, request_generation)?;
    let data = download_url_bytes(&url, &state, "crossfade_url", request_generation).await?;
    ensure_playback_request(&state, request_generation)?;
    run_player_play(Arc::clone(&state.player), move |player| {
        player.request_crossfade_bytes(
            data,
            duration_hint_ms,
            fade_out_ms,
            fade_in_ms,
            request_generation,
        )
    })
    .await
}

// Tauri 命令签名由 IPC 契约决定：参数必须平铺，改成结构体会同时改掉前端调用点
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn crossfade_url_fast(
    url: String,
    stream_type: Option<YtStreamType>,
    duration_hint_ms: u64,
    fade_out_ms: u32,
    fade_in_ms: u32,
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
            0,
            Some((fade_out_ms, fade_in_ms)),
            &state,
            request_generation,
            cache,
        )
        .await;
    }
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
    let dur = run_player_play(Arc::clone(&state.player), move |player| {
        player.request_crossfade_file_with_hint(
            &path,
            duration_hint_ms,
            fade_out_ms,
            fade_in_ms,
            request_generation,
        )
    })
    .await?;
    Ok(if dur > 0 { dur } else { duration_hint_ms })
}

// Tauri 命令签名由 IPC 契约决定：参数必须平铺，改成结构体会同时改掉前端调用点
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn crossfade_url_streaming(
    url: String,
    stream_type: Option<YtStreamType>,
    duration_hint_ms: u64,
    fade_out_ms: u32,
    fade_in_ms: u32,
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
            0,
            Some((fade_out_ms, fade_in_ms)),
            &state,
            request_generation,
            cache,
        )
        .await;
    }
    log::info!(
        target: "crossfade_url_streaming",
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
    if let Some(path) = lookup_ready_playback_cache(cache.as_ref(), request_generation).await? {
        log::info!(
            target: "crossfade_url_streaming",
            "playback cache hit generation={}",
            request_generation,
        );
        ensure_playback_request(&state, request_generation)?;
        let cached_path = path.to_string_lossy().to_string();
        let result = run_player_play(Arc::clone(&state.player), move |player| {
            player.request_crossfade_file_with_hint(
                &cached_path,
                duration_hint_ms,
                fade_out_ms,
                fade_in_ms,
                request_generation,
            )
        })
        .await;
        match result {
            Ok(dur) => return Ok(if dur > 0 { dur } else { duration_hint_ms }),
            Err(err) => {
                ensure_playback_request(&state, request_generation)?;
                log::warn!(
                    target: "crossfade_url_streaming",
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
                log::warn!(
                    target: "crossfade_url_streaming",
                    "fallback cache unavailable: {}",
                    err
                );
                None
            }
        });

    match open_remote_audio_source(&url, duration_hint_ms, request_generation, &state, cache).await
    {
        Ok(reader) => {
            ensure_playback_request(&state, request_generation)?;
            let result = run_player_play(Arc::clone(&state.player), move |player| {
                player.request_crossfade_remote(
                    reader,
                    duration_hint_ms,
                    fade_out_ms,
                    fade_in_ms,
                    request_generation,
                )
            })
            .await;
            match result {
                Ok(dur) => return Ok(if dur > 0 { dur } else { duration_hint_ms }),
                Err(err) => {
                    ensure_playback_request(&state, request_generation)?;
                    log::warn!(
                        target: "crossfade_url_streaming",
                        "remote playback failed: {}",
                        summarize_playback_error(&err),
                    )
                }
            }
        }
        Err(err) => {
            ensure_playback_request(&state, request_generation)?;
            log::warn!(
                target: "crossfade_url_streaming",
                "remote source unavailable: {}",
                summarize_playback_error(&err),
            );
        }
    }

    crossfade_url_streaming_fallback(
        &url,
        duration_hint_ms,
        fade_out_ms,
        fade_in_ms,
        request_generation,
        fallback_cache,
        &state,
    )
    .await
}

async fn crossfade_url_streaming_fallback(
    url: &str,
    duration_hint_ms: u64,
    fade_out_ms: u32,
    fade_in_ms: u32,
    request_generation: u64,
    cache: Option<RemoteAudioCache>,
    state: &State<'_, AppState>,
) -> AppResult<u64> {
    ensure_playback_request(state, request_generation)?;
    let buffer = start_streaming_download(
        url,
        state,
        "crossfade_url_streaming",
        request_generation,
        cache,
    )
    .await?;
    let buffer_wait_started = std::time::Instant::now();
    log::info!(
        target: "crossfade_url_streaming",
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
        target: "crossfade_url_streaming",
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
        player.request_crossfade_stream(
            reader,
            duration_hint_ms,
            fade_out_ms,
            fade_in_ms,
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

#[tauri::command]
pub async fn crossfade_file(
    path: String,
    duration_hint_ms: Option<u64>,
    fade_out_ms: u32,
    fade_in_ms: u32,
    request_generation: u64,
    state: State<'_, AppState>,
) -> AppResult<u64> {
    claim_playback_request(&state, request_generation)?;
    run_player_play(Arc::clone(&state.player), move |player| {
        player.request_crossfade_file_with_hint(
            &path,
            duration_hint_ms.unwrap_or(0),
            fade_out_ms,
            fade_in_ms,
            request_generation,
        )
    })
    .await
}
