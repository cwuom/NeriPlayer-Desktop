//! 播放控制命令：暂停/继续、seek、音量、输出设备、音效参数与淡入淡出

use super::*;

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
pub async fn list_audio_output_devices() -> AppResult<Vec<neri_playback::player::AudioOutputDevice>>
{
    tokio::task::spawn_blocking(neri_playback::player::list_audio_output_devices)
        .await
        .map_err(|error| AppError::Other(error.to_string()))?
}

#[tauri::command]
pub async fn set_audio_output_device(
    name: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    // Linux sound server sinks: playback already goes through the server's default
    // node, so selecting a sink means making it the server default and reopening
    // that stream at the current position.
    #[cfg(target_os = "linux")]
    if let Some(sink) = name.clone() {
        // pactl is blocking and has no timeout; keep both the lookup and the
        // set-default call off the Tokio worker thread.
        let pactl_sink = sink.clone();
        let switched = tokio::task::spawn_blocking(move || {
            let known = neri_playback::system_sinks::list()
                .is_some_and(|sinks| sinks.iter().any(|item| item.name == pactl_sink));
            if known {
                neri_playback::system_sinks::set_default(&pactl_sink).map(|_| true)
            } else {
                Ok(false)
            }
        })
        .await
        .map_err(|error| AppError::Other(error.to_string()))?
        .map_err(AppError::Audio)?;
        if switched {
            // The sink is now the server default, but the ALSA profile name is still
            // `default`; force a rebuild so the running stream follows the new route.
            let request = state
                .player
                .lock()
                .request_output_device_force(None, Some(sink))?;
            return tokio::task::spawn_blocking(move || request.wait())
                .await
                .map_err(|error| AppError::Other(error.to_string()))?;
        }
    }
    let request = state.player.lock().request_output_device(name)?;
    tokio::task::spawn_blocking(move || request.wait())
        .await
        .map_err(|error| AppError::Other(error.to_string()))?
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
    })
    .await
    .map_err(|error| AppError::Other(error.to_string()))?
}

#[tauri::command]
pub async fn release_audio_file(path: String, state: State<'_, AppState>) -> AppResult<bool> {
    release_player_file(Arc::clone(&state.player), path).await
}

#[tauri::command]
pub async fn get_playback_audio_info(
    request_generation: u64,
    state: State<'_, AppState>,
) -> AppResult<Option<neri_playback::remote::SourceAudioInfo>> {
    run_player_blocking(Arc::clone(&state.player), move |player| {
        Ok(player.playback_audio_info(request_generation))
    })
    .await
}

/// 当前能解码的编码：前端据此决定能否选择 Opus、杜比等音轨
#[tauri::command]
pub async fn get_decoder_capabilities() -> AppResult<neri_playback::decoder::DecoderCapabilities> {
    // 首次调用可能要加载 FFmpeg 动态库，不能占用异步运行时的线程
    tokio::task::spawn_blocking(neri_playback::decoder::decoder_capabilities)
        .await
        .map_err(|error| AppError::Audio(format!("Could not query decoder capabilities: {error}")))
}

/// 多声道（AC-3/E-AC-3）音轨是否保留码流自带的动态范围压缩；对之后打开的音轨生效
#[tauri::command]
pub async fn set_multichannel_drc(enabled: bool) -> AppResult<()> {
    neri_playback::decoder::set_keep_dynamic_range_compression(enabled);
    log::info!(
        target: "audio-decoder",
        "multichannel dynamic range compression {}",
        if enabled { "kept" } else { "off" },
    );
    Ok(())
}

#[tauri::command]
pub async fn set_speed(speed: f32, state: State<'_, AppState>) -> AppResult<()> {
    require_finite_ratio(speed, "speed")?;
    state.player.lock().set_speed(speed);
    Ok(())
}

#[tauri::command]
pub async fn set_pitch(pitch: f32, state: State<'_, AppState>) -> AppResult<()> {
    require_finite_ratio(pitch, "pitch")?;
    state.player.lock().set_pitch(pitch);
    Ok(())
}

#[tauri::command]
pub async fn set_vinyl_effect(enabled: bool, state: State<'_, AppState>) -> AppResult<()> {
    state.player.lock().set_vinyl_effect(enabled);
    Ok(())
}

#[tauri::command]
pub async fn set_audio_effects(
    settings: neri_playback::dsp_settings::AudioEffectsSettings,
    output_device: String,
    state: State<'_, AppState>,
) -> AppResult<neri_playback::dsp_settings::AudioEffectsSettings> {
    Ok(state
        .player
        .lock()
        .set_audio_effects(settings, &output_device))
}

#[tauri::command]
pub async fn get_audio_effects_stats(
    state: State<'_, AppState>,
) -> AppResult<neri_playback::dsp::AudioEffectsStats> {
    Ok(state.player.lock().audio_effects_stats())
}

pub(super) fn require_finite_ratio(value: f32, name: &str) -> AppResult<()> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(AppError::Audio(format!("Invalid playback {name}: {value}")))
    }
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

pub(super) fn balance_centi(balance: f32) -> AppResult<i32> {
    if !balance.is_finite() {
        return Err(AppError::Audio(format!(
            "Invalid channel balance: {balance}"
        )));
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
