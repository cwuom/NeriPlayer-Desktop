use crate::error::AppResult;
use crate::lyrics::manager::LyricsManager;
use crate::lyrics::matcher::{LyricMatcher, MatchRequest, RankedMatch};
use crate::lyrics::parser::{self, LyricLine};
use crate::lyrics::sanitize;
use crate::lyrics::{FetchedLyrics, LyricSource};
use crate::state::AppState;
use std::time::Instant;
use tauri::State;

/// 自动识别 LRC / YRC(逐字), 支持逐字歌词编辑往返
///
/// 传了歌名时按匹配歌词处理：同步来的歌词多是 Android 从平台匹配的，同样去掉制作信息和标题行
#[tauri::command]
pub async fn parse_lrc_content(
    content: String,
    title: Option<String>,
    artist: Option<String>,
) -> AppResult<Vec<LyricLine>> {
    let lines = parser::parse_auto(&content);
    Ok(match title {
        Some(title) => sanitize::sanitize_matched_lines(lines, &title, artist.as_deref().unwrap_or(""), ""),
        None => lines,
    })
}

#[tauri::command]
pub async fn fetch_word_timed_lyrics(
    title: String,
    artist: String,
    duration_ms: u64,
    state: State<'_, AppState>,
) -> AppResult<FetchedLyrics> {
    let manager = LyricsManager::with_transport(
        state.transport("lyrics"),
        crate::auth::cookies::read_netease_csrf(&state.cookie_jar),
    );
    let fetched = manager.fetch_word_timed_lyrics(&title, &artist, duration_ms).await?;
    Ok(sanitize_fetched(fetched, &title, &artist))
}

/// 在线取到的歌词去掉制作信息、歌名歌手标题行；用户自己的本地歌词文件原样保留
fn sanitize_fetched(fetched: FetchedLyrics, title: &str, artist: &str) -> FetchedLyrics {
    if fetched.source == Some(LyricSource::Local) {
        return fetched;
    }
    FetchedLyrics {
        source: fetched.source,
        lines: sanitize::sanitize_matched_lines(fetched.lines, title, artist, ""),
    }
}

/// 网易云音译轨原文（romalrc）；同步来的歌词缺音译时补上，对齐 Android loadNeteaseRomanizedFallback
#[tauri::command]
pub async fn fetch_netease_romanized_lyric(song_id: u64, state: State<'_, AppState>) -> AppResult<Option<String>> {
    let lyrics = state.netease().get_lyrics(song_id).await?;
    Ok(lyrics.romalrc.filter(|text| !text.trim().is_empty()))
}

/// 歌词编辑器「匹配」：按所选平台搜索并排序候选，每个候选带完整歌词行
#[tauri::command]
pub async fn match_lyrics(request: MatchRequest, state: State<'_, AppState>) -> AppResult<Vec<RankedMatch>> {
    let started = Instant::now();
    let sources = request.sources.clone();
    let matcher = LyricMatcher::new(state.transport("lyrics"), state.netease());
    let results = matcher.find(request).await;
    log::info!(
        target: "lyrics-command",
        "match sources={sources:?} results={} top={:?} elapsed_ms={}",
        results.len(),
        results.first().map(|result| (result.source, result.confidence, result.score)),
        started.elapsed().as_millis(),
    );
    Ok(results)
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
) -> AppResult<FetchedLyrics> {
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
        "end ok={}, source={:?}, lines={}, elapsed_ms={}",
        result.is_ok(),
        result.as_ref().ok().and_then(|fetched| fetched.source),
        result.as_ref().map_or(0, |fetched| fetched.lines.len()),
        started.elapsed().as_millis(),
    );
    result.map(|fetched| sanitize_fetched(fetched, &title, &artist))
}
