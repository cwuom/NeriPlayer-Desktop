use crate::settings::store::{MAX_MEDIA_CACHE_SIZE_MB, MIN_MEDIA_CACHE_SIZE_MB};
use crate::state::AppState;
use futures_util::StreamExt;
use neri_common::error::{AppError, AppResult};
use neri_platform::api::youtube::client::YtStreamType;
use neri_playback::growing::GrowingAudioBuffer;
use neri_playback::player::{
    receive_fade_result, wait_for_play_result, wait_for_seek_result, PlayRequest, PlayerEngine,
};
use neri_playback::remote::{RemoteAudioCache, RemoteAudioSource};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, State};
use tokio::io::AsyncWriteExt;

// 按职责拆分的命令与辅助逻辑，通过 `use super::*` 共享本模块的导入、常量与请求代际工具
mod cache;
mod control;
mod crossfade;
mod play;
mod queue;
mod stream;
pub use cache::*;
pub use control::*;
pub use crossfade::*;
pub use play::*;
pub use queue::*;
pub(crate) use stream::playback_referer;
use stream::*;

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

// Tauri 命令的参数与前端 invoke 字段一一对应；把布尔项打包成结构体会改变 IPC
// 形状，并破坏与 Android 端播放请求字段的对齐。保持扁平签名。
// Tauri command parameters map 1:1 to frontend invoke fields; grouping them would
// change the IPC shape and break alignment with Android's playback request fields.
#[allow(clippy::too_many_arguments)]
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
        state
            .player
            .lock()
            .silence_stale_sessions(request_generation);
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

#[cfg(test)]
mod tests;
