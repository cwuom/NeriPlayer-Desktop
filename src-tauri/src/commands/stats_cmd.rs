// 播放统计命令：与 Android 播放统计页对齐
use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::state::{AppState, TrackSource};
use crate::stats::{self, PlaybackSession, PlaybackStatsSummary, StatsPeriod};
use crate::sync::manager;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// 前端上报的一次收听增量
///
/// 身份键由后端从 TrackInfo 派生，前端不需要重复实现各平台的哈希规则，
/// 也就不会出现两端算出不同 key 导致统计分裂。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackSessionInput {
    pub track: crate::state::TrackInfo,
    #[serde(default)]
    pub listened_ms: i64,
    #[serde(default)]
    pub play_count_increment: i32,
    /// 这首歌是从哪个本地歌单开始的队列里播的
    #[serde(default)]
    pub local_playlist_id: Option<String>,
}

/// 计入播放的本地歌单 id（对齐 Android countedLocalPlaylistId：只有算一次播放时才计歌单）
fn counted_local_playlist_id(input: &PlaybackSessionInput) -> Option<i64> {
    if input.play_count_increment <= 0 {
        return None;
    }
    input.local_playlist_id.as_deref()?.trim().parse().ok().filter(|id| *id != 0)
}

/// 本地歌单播放计数写进同步扩展段（对齐 Android LocalPlaylistPlaybackStatsRepository.recordPlayNow）
async fn record_local_playlist_plays(device_id: String, playlist_ids: Vec<i64>, played_at: i64) {
    if playlist_ids.is_empty() {
        return;
    }
    let result = tokio::task::spawn_blocking(move || -> AppResult<()> {
        // 与同步回写共用歌单锁并推进写入版本：进行中的同步会推迟，而不是用旧快照的扩展段覆盖这次计数
        let _guard = crate::library::playlist::lock_io();
        let day_start_at = stats::day_start_at(played_at);
        crate::db::user_db()?.write(|transaction| {
            crate::sync::storage::update_archive_extensions(transaction, |extensions| {
                for &playlist_id in &playlist_ids {
                    crate::sync::merge::record_local_playlist_play(extensions, playlist_id, day_start_at, &device_id, played_at);
                }
                Ok(())
            })
        })?;
        crate::library::playlist::mark_io_changed();
        Ok(())
    })
    .await;
    // 统计失败不能影响播放
    match result {
        Ok(Err(error)) => log::warn!("record local playlist play failed: {error}"),
        Err(error) => log::warn!("record local playlist play task failed: {error}"),
        Ok(Ok(())) => {}
    }
}

fn to_session(input: &PlaybackSessionInput) -> PlaybackSession {
    let track = &input.track;
    // 对齐 Android：本地歌曲在统计里记下文件位置，统计页才能直接播放
    let media_uri = (track.source == TrackSource::Local && !track.url.trim().is_empty())
        .then(|| track.url.clone());
    PlaybackSession {
        identity_key: manager::playback_stats_identity_key_pub(track),
        id: track.id.clone(),
        name: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        album_id: String::new(),
        cover_url: track.cover_url.clone(),
        media_uri,
        duration_ms: track.duration_ms as i64,
        listened_ms: input.listened_ms,
        play_count_increment: input.play_count_increment,
    }
}

/// 上报一次收听增量（由前端 PlaybackStatsTracker 产出）
#[tauri::command]
pub async fn record_playback_session(
    session: PlaybackSessionInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let device_id = manager::get_or_create_device_id_pub(&app);
    let played_at = now_ms();
    {
        let mut store = state.stats.lock();
        store.record(&to_session(&session), &device_id, played_at);
        stats::save(&store);
    }
    let playlists = counted_local_playlist_id(&session).into_iter().collect();
    record_local_playlist_plays(device_id, playlists, played_at).await;
    Ok(())
}

/// 批量上报，退出前一次性 flush 用
#[tauri::command]
pub async fn record_playback_sessions(
    sessions: Vec<PlaybackSessionInput>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    if sessions.is_empty() {
        return Ok(());
    }
    let device_id = manager::get_or_create_device_id_pub(&app);
    let played_at = now_ms();
    {
        let mut store = state.stats.lock();
        for session in &sessions {
            store.record(&to_session(session), &device_id, played_at);
        }
        stats::save(&store);
    }
    let playlists = sessions.iter().filter_map(counted_local_playlist_id).collect();
    record_local_playlist_plays(device_id, playlists, played_at).await;
    Ok(())
}

#[tauri::command]
pub async fn get_playback_stats(
    period: StatsPeriod,
    state: State<'_, AppState>,
) -> AppResult<PlaybackStatsSummary> {
    let store = state.stats.lock();
    Ok(store.summarize(period, now_ms()))
}

/// 一次性返回全部区间，避免切换 tab 时来回 IPC
#[tauri::command]
pub async fn get_playback_stats_overview(
    state: State<'_, AppState>,
) -> AppResult<Vec<PlaybackStatsSummary>> {
    let now = now_ms();
    let store = state.stats.lock();
    Ok([
        StatsPeriod::Day,
        StatsPeriod::Week,
        StatsPeriod::Month,
        StatsPeriod::Year,
        StatsPeriod::All,
    ]
    .into_iter()
    .map(|period| store.summarize(period, now))
    .collect())
}

#[tauri::command]
pub async fn clear_playback_stats(state: State<'_, AppState>) -> AppResult<()> {
    let mut store = state.stats.lock();
    store.clear(now_ms());
    stats::save(&store);
    Ok(())
}

#[tauri::command]
pub async fn remove_playback_stats(
    identity_keys: Vec<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let mut store = state.stats.lock();
    store.remove_tracks(&identity_keys);
    stats::save(&store);
    Ok(())
}

/// 计算曲目的统计身份键，供前端上报时使用
#[tauri::command]
pub async fn playback_stats_identity_key(
    track: crate::state::TrackInfo,
) -> AppResult<String> {
    Ok(manager::playback_stats_identity_key_pub(&track))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 入参契约：track 必须是 toBackendTrack 产出的 snake_case 结构
    /// （TrackInfo 的 duration_ms/url 必填且无 camelCase 别名）
    #[test]
    fn session_input_accepts_to_backend_track_shape() {
        let payload = serde_json::json!({
            "track": {
                "id": "netease:2161589xxx",
                "title": "Song",
                "artist": "Artist",
                "album": "Album",
                "duration_ms": 200_000,
                "cover_url": "https://cover",
                "url": "",
                "source": "netease",
                "added_at": 0,
                "sync_payload": null,
                "playlist_key": null,
            },
            "listenedMs": 15_000,
            "playCountIncrement": 1,
        });

        let input: PlaybackSessionInput =
            serde_json::from_value(payload).expect("snake_case track 必须可反序列化");
        let session = to_session(&input);
        assert!(!session.identity_key.trim().is_empty(), "身份键不得为空");
        assert_eq!(session.listened_ms, 15_000);
        assert_eq!(session.play_count_increment, 1);
        assert_eq!(session.duration_ms, 200_000);
        assert_eq!(session.media_uri, None, "在线歌曲不记录播放地址");
    }

    #[test]
    fn local_session_keeps_file_path_for_replay() {
        let payload = serde_json::json!({
            "track": {
                "id": "local:D:\\Music\\song.flac",
                "title": "Song",
                "artist": "Artist",
                "album": "",
                "duration_ms": 180_000,
                "cover_url": null,
                "url": "D:\\Music\\song.flac",
                "source": "local",
            },
            "listenedMs": 30_000,
            "playCountIncrement": 1,
        });
        let input: PlaybackSessionInput = serde_json::from_value(payload).unwrap();
        assert_eq!(to_session(&input).media_uri.as_deref(), Some("D:\\Music\\song.flac"));
    }

    /// 回归：前端曾直接上送 camelCase TrackInfo，反序列化失败被前端
    /// catch 静默吞掉，导致播放统计从未落盘（全 0）。此测试钉死该形状
    /// 必须被拒绝——若未来给 TrackInfo 加别名放行，请同步删除本测试
    #[test]
    fn session_input_rejects_frontend_camelcase_track() {
        let payload = serde_json::json!({
            "track": {
                "id": "netease:2161589xxx",
                "title": "Song",
                "artist": "Artist",
                "album": "Album",
                "durationMs": 200_000,
                "coverUrl": "https://cover",
                "audioUrl": "",
                "source": "netease",
            },
            "listenedMs": 15_000,
            "playCountIncrement": 1,
        });

        let error = serde_json::from_value::<PlaybackSessionInput>(payload)
            .expect_err("camelCase track 形状必须被拒绝");
        assert!(
            error.to_string().contains("missing field"),
            "失败原因应为缺字段, 实际: {error}"
        );
    }
}
