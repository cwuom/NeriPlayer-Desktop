use crate::audio::growing::GrowingAudioBuffer;
use crate::api::youtube::client::YtStreamType;
use crate::audio::player::{
    receive_fade_result, wait_for_play_result, wait_for_seek_result, PlayRequest, PlayerEngine,
};
use crate::audio::remote::{RemoteAudioCache, RemoteAudioSource};
use crate::error::{AppError, AppResult};
use crate::settings::store::{MAX_MEDIA_CACHE_SIZE_MB, MIN_MEDIA_CACHE_SIZE_MB};
use crate::state::AppState;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use parking_lot::Mutex;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager, State};
use tokio::io::AsyncWriteExt;

const STREAM_START_BUFFER_BYTES: usize = 64 * 1024;
const STREAM_START_TIMEOUT: Duration = Duration::from_secs(12);
// 流式下载 per-chunk 空闲超时：断网/黑洞网络下 stream.next() 会永久悬挂
// （全局 client 无读超时），超过此时长无新数据即判定网络停滞并 fail，
// 使解码侧从阻塞等待中解脱、走 fallback
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const CACHE_LOOKUP_TIMEOUT: Duration = Duration::from_millis(80);
const MAX_CACHE_LOOKUP_CANDIDATES: usize = 16;
const PLAYBACK_SUPERSEDED_ERROR: &str = "Playback request superseded";

fn playback_url_host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|value| value.host_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}

fn summarize_playback_error(error: &dyn std::fmt::Display) -> String {
    let mut message = error.to_string();
    for scheme in ["https://", "http://"] {
        while let Some(start) = message.find(scheme) {
            let tail = &message[start..];
            let end = tail
                .find(|value: char| value.is_whitespace() || matches!(value, ')' | ']' | '}'))
                .map(|offset| start + offset)
                .unwrap_or(message.len());
            message.replace_range(start..end, "[url]");
        }
    }
    message
}

fn playback_path_kind(path: &str) -> &'static str {
    match path {
        "__remote__" => "remote",
        "__growing__" => "growing",
        "__bytes__" => "bytes",
        _ => "file",
    }
}

async fn run_player_blocking<T>(
    player: Arc<Mutex<PlayerEngine>>,
    action: impl FnOnce(&mut PlayerEngine) -> AppResult<T> + Send + 'static,
) -> AppResult<T>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let mut player = player.lock();
        action(&mut player)
    })
    .await
    .map_err(|error| AppError::Other(error.to_string()))?
}

async fn lookup_ready_playback_cache(
    cache: Option<&RemoteAudioCache>,
    request_generation: u64,
) -> AppResult<Option<PathBuf>> {
    let Some(cache) = cache.cloned() else {
        return Ok(None);
    };
    let queued_at = std::time::Instant::now();
    log::info!(
        target: "playback-cache",
        "ready lookup queued generation={}",
        request_generation,
    );
    let path = tokio::task::spawn_blocking(move || {
        log::info!(
            target: "playback-cache",
            "ready lookup worker started generation={}, queued_ms={}",
            request_generation,
            queued_at.elapsed().as_millis(),
        );
        cache.ready_path()
    })
    .await
    .map_err(|error| AppError::Other(error.to_string()))?;
    log::info!(
        target: "playback-cache",
        "ready lookup finished generation={}, hit={}, total_ms={}",
        request_generation,
        path.is_some(),
        queued_at.elapsed().as_millis(),
    );
    Ok(path)
}

/// 两阶段播放：短锁发命令 -> 锁外等待 -> 短锁更新状态。不阻塞其他命令。
async fn run_player_play(
    player: Arc<Mutex<PlayerEngine>>,
    action: impl FnOnce(&mut PlayerEngine) -> AppResult<PlayRequest> + Send + 'static,
) -> AppResult<u64> {
    let dispatch_started = std::time::Instant::now();
    let dispatch_queued_at = std::time::Instant::now();
    let player2 = Arc::clone(&player);
    let request = tokio::task::spawn_blocking(move || {
        log::info!(
            target: "player-command",
            "dispatch worker started queued_ms={}",
            dispatch_queued_at.elapsed().as_millis(),
        );
        let mut p = player2.lock();
        action(&mut p)
    })
    .await
    .map_err(|error| AppError::Other(error.to_string()))??;

    let source_kind = playback_path_kind(&request.current_path);
    log::info!(
        target: "player-command",
        "queued source={}, generation={}, dispatch_ms={}",
        source_kind,
        request.expected_generation,
        dispatch_started.elapsed().as_millis(),
    );

    let wait_started = std::time::Instant::now();
    let wait_queued_at = std::time::Instant::now();
    let (started, generation, path) = tokio::task::spawn_blocking(move || {
        log::info!(
            target: "player-command",
            "wait worker started generation={}, queued_ms={}",
            request.expected_generation,
            wait_queued_at.elapsed().as_millis(),
        );
        let result = wait_for_play_result(&request)?;
        Ok::<_, AppError>((
            result,
            request.expected_generation,
            request.current_path.clone(),
        ))
    })
    .await
    .map_err(|error| AppError::Other(error.to_string()))??;

    log::info!(
        target: "player-command",
        "audio ready source={}, generation={}, duration_ms={}, wait_ms={}",
        playback_path_kind(&path),
        generation,
        started.duration_ms,
        wait_started.elapsed().as_millis(),
    );

    let commit_started = std::time::Instant::now();
    let duration = tokio::task::spawn_blocking(move || {
        let mut p = player.lock();
        p.complete_start(started, generation, path)
    })
    .await
    .map_err(|error| AppError::Other(error.to_string()))??;
    log::info!(
        target: "player-command",
        "state committed generation={}, duration_ms={}, commit_ms={}",
        generation,
        duration,
        commit_started.elapsed().as_millis(),
    );
    Ok(duration)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackUiTraceRequest {
    stage: String,
    track_id: Option<String>,
    source: Option<String>,
    detail: Option<String>,
    request_generation: Option<u64>,
}

fn playback_trace_field(value: Option<&str>) -> String {
    summarize_playback_error(&value.unwrap_or("-"))
        .chars()
        .map(|character| {
            if character == '\n' || character == '\r' {
                ' '
            } else {
                character
            }
        })
        .take(240)
        .collect()
}

#[tauri::command]
pub fn trace_playback_ui(request: PlaybackUiTraceRequest) {
    if cfg!(debug_assertions) {
        log::info!(
            target: "playback-ui",
            "stage={}, generation={}, id={}, source={}, detail={}",
            playback_trace_field(Some(&request.stage)),
            request
                .request_generation
                .map_or_else(|| "-".to_string(), |generation| generation.to_string()),
            playback_trace_field(request.track_id.as_deref()),
            playback_trace_field(request.source.as_deref()),
            playback_trace_field(request.detail.as_deref()),
        );
    }
}

fn claim_generation(generation: &AtomicU64, request_generation: u64) -> bool {
    generation.fetch_max(request_generation, Ordering::AcqRel) <= request_generation
}

fn is_generation_current(generation: &AtomicU64, request_generation: u64) -> bool {
    generation.load(Ordering::Acquire) == request_generation
}

fn claim_playback_request(state: &State<'_, AppState>, request_generation: u64) -> AppResult<()> {
    if claim_generation(&state.playback_generation, request_generation) {
        Ok(())
    } else {
        Err(AppError::Audio(PLAYBACK_SUPERSEDED_ERROR.into()))
    }
}

fn ensure_playback_request(state: &State<'_, AppState>, request_generation: u64) -> AppResult<()> {
    if is_generation_current(&state.playback_generation, request_generation) {
        Ok(())
    } else {
        Err(AppError::Audio(PLAYBACK_SUPERSEDED_ERROR.into()))
    }
}

fn advance_playback_request(state: &State<'_, AppState>) -> u64 {
    state.playback_generation.fetch_add(1, Ordering::AcqRel) + 1
}

#[tauri::command]
pub async fn begin_playback_request(
    request_generation: u64,
    track_id: Option<String>,
    source: Option<String>,
    has_cover: Option<bool>,
    has_audio_url: Option<bool>,
    has_sync_payload: Option<bool>,
    silence_previous: Option<bool>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    log::info!(
        target: "playback-request",
        "begin generation={}, id={}, source={}, cover={}, direct_url={}, sync_payload={}, silence_previous={}",
        request_generation,
        track_id.as_deref().unwrap_or("unknown"),
        source.as_deref().unwrap_or("unknown"),
        has_cover.unwrap_or(false),
        has_audio_url.unwrap_or(false),
        has_sync_payload.unwrap_or(false),
        silence_previous.unwrap_or(false)
    );
    claim_playback_request(&state, request_generation)?;
    if silence_previous.unwrap_or(false) {
        state.player.lock().silence_stale_sessions(request_generation);
    }
    Ok(())
}

#[derive(Serialize)]
pub struct PlayerStateDto {
    pub is_playing: bool,
    pub volume: f32,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub current_track: Option<crate::state::TrackInfo>,
    pub repeat_mode: crate::state::RepeatMode,
    pub shuffle: bool,
}

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
    cache_key: String,
    duration_hint_ms: u64,
    start_position_ms: Option<u64>,
    use_crossfade: bool,
    fade_out_ms: u32,
    fade_in_ms: u32,
    cache_limit_bytes: Option<u64>,
    request_generation: u64,
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
    let resp = state.http().get(&url)
        .header("Referer", referer)
        .header("User-Agent", playback_user_agent(&url))
        .send().await
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
        let cache = playback_cache(&app, cache_key.as_deref(), cache_limit_bytes, None, duration_hint_ms);
        return play_hls_url(url, duration_hint_ms, start_position_ms.unwrap_or(0), None, &state, request_generation, cache).await;
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
    let cache = cache.map(|cache| cache.with_expected_md5(expected_content_md5.as_deref())).transpose()?;
    let path = match cache {
        Some(cache) => download_url_to_playback_cache(
            &url,
            &state,
            cache,
            request_generation,
        )
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
        let cache = playback_cache(&app, cache_key.as_deref(), cache_limit_bytes, None, duration_hint_ms);
        return play_hls_url(url, duration_hint_ms, start_position_ms.unwrap_or(0), None, &state, request_generation, cache).await;
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

    let cache = cache.map(|cache| cache.with_expected_md5(expected_content_md5.as_deref())).transpose()?;
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

    let fallback_cache = cache.as_ref().and_then(|cache| match cache.fresh_staging() {
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
    match open_remote_audio_source(
        &url,
        duration_hint_ms,
        request_generation,
        &state,
        cache,
    )
    .await
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

async fn play_hls_url(
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
    let stream = crate::audio::hls::start_hls_stream(
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
            Some(cache) => download_url_to_playback_cache(
                url,
                state,
                cache,
                request_generation,
            )
            .await?,
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

    let buffer = start_streaming_download(
        url,
        state,
        "play_url_streaming",
        request_generation,
        cache,
    )
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

#[tauri::command]
pub async fn pause(state: State<'_, AppState>) -> AppResult<()> {
    state.player.lock().pause();
    Ok(())
}

#[tauri::command]
pub async fn resume(state: State<'_, AppState>) -> AppResult<()> {
    state.player.lock().resume();
    Ok(())
}

#[tauri::command]
pub async fn toggle_play_pause(state: State<'_, AppState>) -> AppResult<bool> {
    let mut player = state.player.lock();
    if player.is_playing {
        player.pause();
    } else {
        player.resume();
    }
    Ok(player.is_playing)
}

#[tauri::command]
pub async fn set_volume(level: f32, state: State<'_, AppState>) -> AppResult<()> {
    state.player.lock().set_volume(level);
    Ok(())
}

#[tauri::command]
pub async fn list_audio_output_devices() -> AppResult<Vec<crate::audio::player::AudioOutputDevice>> {
    tokio::task::spawn_blocking(crate::audio::player::list_audio_output_devices)
        .await
        .map_err(|error| AppError::Other(error.to_string()))?
}

#[tauri::command]
pub async fn set_audio_output_device(name: Option<String>, state: State<'_, AppState>) -> AppResult<()> {
    let request = state.player.lock().request_output_device(name)?;
    tokio::task::spawn_blocking(move || request.wait())
        .await.map_err(|error| AppError::Other(error.to_string()))?
}

#[tauri::command]
pub async fn seek(
    position_ms: u64,
    request_generation: u64,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let receiver = state
        .player
        .lock()
        .request_seek(position_ms, request_generation)?;
    tokio::task::spawn_blocking(move || wait_for_seek_result(receiver))
        .await
        .map_err(|error| AppError::Other(error.to_string()))?
}

#[tauri::command]
pub async fn stop(state: State<'_, AppState>) -> AppResult<()> {
    state.player.lock().stop();
    Ok(())
}

pub(crate) async fn release_player_file(
    player: Arc<Mutex<PlayerEngine>>,
    path: String,
) -> AppResult<bool> {
    tokio::task::spawn_blocking(move || {
        let request = player.lock().request_file_release(path)?;
        let released = request.wait()?;
        Ok(player.lock().complete_file_release(released))
    }).await.map_err(|error| AppError::Other(error.to_string()))?
}

#[tauri::command]
pub async fn release_audio_file(path: String, state: State<'_, AppState>) -> AppResult<bool> {
    release_player_file(Arc::clone(&state.player), path).await
}

#[tauri::command]
pub async fn get_playback_audio_info(
    request_generation: u64,
    state: State<'_, AppState>,
) -> AppResult<Option<crate::audio::remote::SourceAudioInfo>> {
    run_player_blocking(Arc::clone(&state.player), move |player| {
        Ok(player.playback_audio_info(request_generation))
    }).await
}

/// 当前能解码的编码：前端据此决定能否选择 Opus、杜比等音轨
#[tauri::command]
pub async fn get_decoder_capabilities() -> AppResult<crate::audio::decoder::DecoderCapabilities> {
    // 首次调用可能要加载 FFmpeg 动态库，不能占用异步运行时的线程
    tokio::task::spawn_blocking(crate::audio::decoder::decoder_capabilities)
        .await
        .map_err(|error| AppError::Audio(format!("Could not query decoder capabilities: {error}")))
}

/// 多声道（AC-3/E-AC-3）音轨是否保留码流自带的动态范围压缩；对之后打开的音轨生效
#[tauri::command]
pub async fn set_multichannel_drc(enabled: bool) -> AppResult<()> {
    crate::audio::decoder::set_keep_dynamic_range_compression(enabled);
    log::info!(
        target: "audio-decoder",
        "multichannel dynamic range compression {}",
        if enabled { "kept" } else { "off" },
    );
    Ok(())
}

#[tauri::command]
pub async fn set_speed(speed: f32, state: State<'_, AppState>) -> AppResult<()> {
    state.player.lock().set_speed(speed);
    Ok(())
}

#[tauri::command]
pub async fn set_loudness_gain(gain_mb: i32, state: State<'_, AppState>) -> AppResult<()> {
    state.player.lock().set_loudness_gain(gain_mb);
    Ok(())
}

#[tauri::command]
pub async fn set_normalize_volume(enabled: bool, state: State<'_, AppState>) -> AppResult<()> {
    state.player.lock().set_normalize_volume(enabled);
    Ok(())
}

/// 声道平衡，-1（只剩左声道）～1（只剩右声道），按 0.01 取整
#[tauri::command]
pub async fn set_volume_balance(balance: f32, state: State<'_, AppState>) -> AppResult<()> {
    state.player.lock().set_balance(balance_centi(balance)?);
    Ok(())
}

fn balance_centi(balance: f32) -> AppResult<i32> {
    if !balance.is_finite() {
        return Err(AppError::Audio(format!("Invalid channel balance: {balance}")));
    }
    Ok((balance.clamp(-1.0, 1.0) * 100.0).round() as i32)
}

#[tauri::command]
pub async fn set_equalizer(
    enabled: bool,
    band_levels_mb: Vec<i32>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    state.player.lock().set_equalizer(enabled, &band_levels_mb);
    Ok(())
}

#[tauri::command]
pub async fn reset_audio_effects(state: State<'_, AppState>) -> AppResult<()> {
    state.player.lock().reset_effects();
    Ok(())
}

#[tauri::command]
pub async fn pause_with_fade(duration_ms: u32, state: State<'_, AppState>) -> AppResult<()> {
    // 两段式（同 run_player_play）：短锁发命令 -> 锁外等淡化完成 -> 短锁提交。
    // 旧实现持 player 锁等完整淡化（上限 30s+），会阻塞期间所有播放命令
    let player = Arc::clone(&state.player);
    let receiver = run_player_blocking(Arc::clone(&player), move |engine| {
        engine.request_pause_with_fade(duration_ms)
    })
    .await?;
    tokio::task::spawn_blocking(move || receive_fade_result(receiver, duration_ms))
        .await
        .map_err(|error| AppError::Other(error.to_string()))??;
    run_player_blocking(player, |engine| {
        engine.commit_fade_pause();
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn resume_with_fade(duration_ms: u32, state: State<'_, AppState>) -> AppResult<()> {
    // 两段式，理由同 pause_with_fade
    let player = Arc::clone(&state.player);
    let receiver = run_player_blocking(Arc::clone(&player), move |engine| {
        engine.request_resume_with_fade(duration_ms)
    })
    .await?;
    tokio::task::spawn_blocking(move || receive_fade_result(receiver, duration_ms))
        .await
        .map_err(|error| AppError::Other(error.to_string()))??;
    run_player_blocking(player, |engine| {
        engine.commit_fade_resume();
        Ok(())
    })
    .await
}

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
    let data = download_url_bytes(
        &url,
        &state,
        "crossfade_url",
        request_generation,
    )
    .await?;
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
        let cache = playback_cache(&app, cache_key.as_deref(), cache_limit_bytes, None, duration_hint_ms);
        return play_hls_url(url, duration_hint_ms, 0, Some((fade_out_ms, fade_in_ms)), &state, request_generation, cache).await;
    }
    let cache = playback_cache(
        &app,
        cache_key.as_deref(),
        cache_limit_bytes,
        expected_content_length,
        duration_hint_ms,
    );
    let cache = cache.map(|cache| cache.with_expected_md5(expected_content_md5.as_deref())).transpose()?;
    let path = match cache {
        Some(cache) => download_url_to_playback_cache(
            &url,
            &state,
            cache,
            request_generation,
        )
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
        let cache = playback_cache(&app, cache_key.as_deref(), cache_limit_bytes, None, duration_hint_ms);
        return play_hls_url(url, duration_hint_ms, 0, Some((fade_out_ms, fade_in_ms)), &state, request_generation, cache).await;
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

    let cache = cache.map(|cache| cache.with_expected_md5(expected_content_md5.as_deref())).transpose()?;
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

    let fallback_cache = cache.as_ref().and_then(|cache| match cache.fresh_staging() {
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

    match open_remote_audio_source(
        &url,
        duration_hint_ms,
        request_generation,
        &state,
        cache,
    )
    .await
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
fn playback_user_agent(url: &str) -> &'static str {
    if url.contains("googlevideo.com") || url.contains("youtube.com") {
        crate::api::youtube::playback::stream_user_agent_for_url(url)
    } else {
        DEFAULT_PLAYBACK_USER_AGENT
    }
}

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
    let warmed = crate::audio::remote::prewarm(
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

async fn open_remote_audio_source(
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

fn playback_cache(
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
        crate::audio::remote::prune_remote_audio_cache(&root, max_bytes);
    }
    if let Ok(cache_dir) = app.path().app_cache_dir() {
        super::image_cmd::prune_cover_cache_dir(&cache_dir);
    }
}

async fn download_url_bytes(
    url: &str,
    state: &State<'_, AppState>,
    tag: &str,
    request_generation: u64,
) -> AppResult<Vec<u8>> {
    let start = std::time::Instant::now();
    ensure_playback_request(state, request_generation)?;
    let download = async {
        let resp = state.http().get(url)
            .header("Referer", playback_referer(url))
            .header("User-Agent", playback_user_agent(url))
            .send().await
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

async fn download_url_to_temp_audio(
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

async fn download_url_to_playback_cache(
    url: &str,
    state: &State<'_, AppState>,
    cache: RemoteAudioCache,
    request_generation: u64,
) -> AppResult<PathBuf> {
    let data = download_url_bytes(
        url,
        state,
        "playback_cache_fallback",
        request_generation,
    )
    .await?;
    ensure_playback_request(state, request_generation)?;
    tokio::task::spawn_blocking(move || cache.publish_complete_bytes(&data))
        .await
        .map_err(|err| AppError::Other(err.to_string()))?
}

async fn start_streaming_download(
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
    let request = state.http().get(url)
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
            let prepared = tokio::task::spawn_blocking(move || {
                cache.prepare_sequential_write(total_len)
            })
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

fn stream_length_matches(expected: Option<u64>, downloaded: u64) -> bool {
    expected.is_none_or(|expected| expected == downloaded)
}

async fn wait_for_stream_start(buffer: GrowingAudioBuffer) -> AppResult<usize> {
    tokio::task::spawn_blocking(move || {
        buffer.wait_for_buffer(STREAM_START_BUFFER_BYTES, STREAM_START_TIMEOUT)
    })
    .await
    .map_err(|e| AppError::Other(e.to_string()))?
    .map_err(AppError::Audio)
}

#[tauri::command]
pub async fn get_player_state(state: State<'_, AppState>) -> AppResult<PlayerStateDto> {
    let player = state.player.lock();
    let queue = state.queue.lock();
    Ok(PlayerStateDto {
        is_playing: player.is_playing,
        volume: player.volume,
        position_ms: player.position_ms(),
        duration_ms: player.duration_ms,
        current_track: queue.current().cloned(),
        repeat_mode: queue.repeat_mode,
        shuffle: queue.shuffle,
    })
}

/// 前端切歌时推送当前曲目元数据到 AppState 镜像, 供 ticker 更新系统媒体会话（PB-01）
#[tauri::command]
pub async fn update_media_metadata(
    state: State<'_, AppState>,
    id: String,
    title: String,
    artist: String,
    album: String,
    cover_url: Option<String>,
    duration_ms: u64,
) -> AppResult<()> {
    *state.media_metadata.lock() = Some(crate::state::MediaMetadataMirror {
        id,
        title,
        artist,
        album,
        cover_url,
        duration_ms,
    });
    Ok(())
}

// 废弃/危险: next_track/prev_track/set_queue 以 request_play_file_with_hint 播放
// TrackInfo.url——远程曲目该字段是 https 直链, 会走 File 解码器打开失败。前端对这些命令
// 零调用（队列推进全在 player.ts）。改接媒体键前务必先改为按 source 解析直链, 否则所有
// 远程曲目播放失败（PB-07）
#[tauri::command]
pub async fn next_track(state: State<'_, AppState>) -> AppResult<Option<crate::state::TrackInfo>> {
    let request_generation = advance_playback_request(&state);
    let track = {
        let mut queue = state.queue.lock();
        queue.next().cloned()
    };
    if let Some(ref t) = track {
        let url = t.url.clone();
        let duration_ms = t.duration_ms;
        run_player_play(Arc::clone(&state.player), move |player| {
            player.request_play_file_with_hint(&url, duration_ms, request_generation)
        })
        .await?;
    }
    Ok(track)
}

#[tauri::command]
pub async fn prev_track(state: State<'_, AppState>) -> AppResult<Option<crate::state::TrackInfo>> {
    let request_generation = advance_playback_request(&state);
    let track = {
        let mut queue = state.queue.lock();
        queue.prev().cloned()
    };
    if let Some(ref t) = track {
        let url = t.url.clone();
        let duration_ms = t.duration_ms;
        run_player_play(Arc::clone(&state.player), move |player| {
            player.request_play_file_with_hint(&url, duration_ms, request_generation)
        })
        .await?;
    }
    Ok(track)
}

#[tauri::command]
pub async fn set_queue(
    tracks: Vec<crate::state::TrackInfo>,
    start_index: usize,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let request_generation = advance_playback_request(&state);
    let track = {
        let mut queue = state.queue.lock();
        queue.set_tracks(tracks, start_index);
        queue.current().cloned()
    };
    if let Some(track) = track {
        let url = track.url.clone();
        let duration_ms = track.duration_ms;
        run_player_play(Arc::clone(&state.player), move |player| {
            player.request_play_file_with_hint(&url, duration_ms, request_generation)
        })
        .await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn toggle_shuffle(state: State<'_, AppState>) -> AppResult<bool> {
    let mut queue = state.queue.lock();
    queue.toggle_shuffle();
    Ok(queue.shuffle)
}

#[tauri::command]
pub async fn cycle_repeat(state: State<'_, AppState>) -> AppResult<crate::state::RepeatMode> {
    let mut queue = state.queue.lock();
    Ok(queue.cycle_repeat())
}

#[cfg(test)]
mod tests {
    use super::{
        balance_centi, claim_generation, is_generation_current, playback_referer,
        playback_trace_field, stream_length_matches, CachedAudioPlaybackRequest,
        PlaybackUiTraceRequest,
    };
    use std::sync::atomic::AtomicU64;

    #[test]
    fn balance_is_rounded_to_hundredths_and_rejects_non_finite_values() {
        assert_eq!(balance_centi(0.0).unwrap(), 0);
        assert_eq!(balance_centi(0.354).unwrap(), 35);
        assert_eq!(balance_centi(-0.35).unwrap(), -35);
        assert_eq!(balance_centi(2.5).unwrap(), 100);
        assert_eq!(balance_centi(-7.0).unwrap(), -100);
        assert!(balance_centi(f32::NAN).is_err());
        assert!(balance_centi(f32::INFINITY).is_err());
    }

    #[test]
    fn bilibili_edge_hosts_get_the_bilibili_referer() {
        for url in [
            "https://b-demo.edge.mountaintoys.cn/upgcxcode/demo.m4s",
            "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/a.m4s",
            "https://xy1x2x3x4xy.mcdn.bilivideo.cn:4483/upgcxcode/a.m4s",
        ] {
            assert_eq!(playback_referer(url), "https://www.bilibili.com", "{url}");
        }
        // 只看主机名：路径里出现 mountaintoys 的其他站点不算
        assert_eq!(playback_referer("https://m701.music.126.net/x.mountaintoys.cn.flac"), "https://music.163.com");
        assert_eq!(playback_referer("https://rr1.googlevideo.com/videoplayback"), "https://music.youtube.com");
    }

    #[test]
    fn playback_generation_never_moves_backwards() {
        let generation = AtomicU64::new(10);

        assert!(!claim_generation(&generation, 9));
        assert!(is_generation_current(&generation, 10));
        assert!(claim_generation(&generation, 10));
        assert!(claim_generation(&generation, 11));
        assert!(is_generation_current(&generation, 11));
        assert!(!is_generation_current(&generation, 10));
    }

    #[test]
    fn sequential_stream_length_guard_rejects_truncated_content() {
        assert!(stream_length_matches(Some(128), 128));
        assert!(!stream_length_matches(Some(128), 127));
        assert!(stream_length_matches(None, 127));
    }

    #[test]
    fn cached_audio_request_accepts_frontend_camel_case_fields() {
        let request: CachedAudioPlaybackRequest = serde_json::from_value(serde_json::json!({
            "cacheKey": "bili-BV1-high",
            "durationHintMs": 272_000,
            "startPositionMs": 22_720,
            "useCrossfade": false,
            "fadeOutMs": 0,
            "fadeInMs": 0,
            "cacheLimitBytes": 1024_u64 * 1024 * 1024,
            "requestGeneration": 42,
        }))
        .expect("cached playback request");

        assert_eq!(request.cache_key, "bili-BV1-high");
        assert_eq!(request.start_position_ms, Some(22_720));
        assert_eq!(request.request_generation, 42);
    }

    #[test]
    fn playback_ui_trace_accepts_camel_case_and_sanitizes_lines() {
        let request: PlaybackUiTraceRequest = serde_json::from_value(serde_json::json!({
            "stage": "store_play_enter",
            "trackId": "netease:42",
            "source": "netease",
            "detail": "first\nsecond",
            "requestGeneration": 9,
        }))
        .expect("playback UI trace request");

        assert_eq!(request.track_id.as_deref(), Some("netease:42"));
        assert_eq!(request.request_generation, Some(9));
        assert_eq!(playback_trace_field(request.detail.as_deref()), "first second");
        assert_eq!(
            playback_trace_field(Some("failed https://media.example/audio?id=secret")),
            "failed [url]",
        );
    }
}
