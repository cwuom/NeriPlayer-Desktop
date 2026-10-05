use crate::error::AppResult;
use crate::lyrics::manager::LyricsManager;
use crate::lyrics::parser::{self, LyricLine};
use crate::state::AppState;
use std::time::Instant;
use tauri::State;

#[tauri::command]
pub async fn parse_lrc_content(content: String) -> AppResult<Vec<LyricLine>> {
    // 自动识别 LRC / YRC(逐字), 支持逐字歌词编辑往返
    Ok(parser::parse_auto(&content))
}

#[tauri::command]
pub async fn fetch_word_timed_lyrics(
    title: String,
    artist: String,
    duration_ms: u64,
    state: State<'_, AppState>,
) -> AppResult<Vec<LyricLine>> {
    let manager = LyricsManager::with_transport(
        state.transport("lyrics"),
        crate::auth::cookies::read_netease_csrf(&state.cookie_jar),
    );
    manager.fetch_word_timed_lyrics(&title, &artist, duration_ms).await
}

#[tauri::command]
pub async fn load_lyrics_file(path: String) -> AppResult<Vec<LyricLine>> {
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| crate::error::AppError::Other(format!("Read lyrics: {}", e)))?;
    // 本地歌词文件可能是 LRC 或 YRC, 统一自动解析
    Ok(parser::parse_auto(&content))
}

/// 多源歌词获取
// Tauri 命令签名由 IPC 契约决定：参数必须平铺，改成结构体会同时改掉前端调用点
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn fetch_lyrics(
    title: String,
    artist: String,
    duration_secs: u64,
    audio_path: Option<String>,
    netease_id: Option<u64>,
    qq_song_mid: Option<String>,
    youtube_video_id: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<LyricLine>> {
    let started = Instant::now();
    log::info!(
        target: "lyrics-command",
        "begin netease_id={:?}, qq_mid={}, yt={}, duration_secs={}, local_audio={}",
        netease_id,
        qq_song_mid.is_some(),
        youtube_video_id.is_some(),
        duration_secs,
        audio_path.is_some(),
    );
    let manager = LyricsManager::with_transport(
        state.transport("lyrics"),
        crate::auth::cookies::read_netease_csrf(&state.cookie_jar),
    );
    let result = manager
        .fetch_lyrics(
            &title,
            &artist,
            duration_secs,
            audio_path.as_deref(),
            netease_id,
            qq_song_mid.as_deref(),
            youtube_video_id.as_deref(),
        )
        .await;
    log::info!(
        target: "lyrics-command",
        "end ok={}, lines={}, elapsed_ms={}",
        result.is_ok(),
        result.as_ref().map_or(0, Vec::len),
        started.elapsed().as_millis(),
    );
    result
}
