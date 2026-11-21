//! 播放器状态查询、媒体元数据与播放队列命令

use super::*;

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
