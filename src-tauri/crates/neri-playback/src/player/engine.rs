//! PlayerEngine 对外接口：向音频控制线程投递命令并等待结果

use super::*;

impl PlayerEngine {
    pub fn new() -> Self {
        Self::with_playback_generation(Arc::new(AtomicU64::new(0)))
    }

    pub fn with_playback_generation(playback_generation: Arc<AtomicU64>) -> Self {
        // 先定下单调时钟原点，回调里就不会走到初始化分支
        metrics::monotonic_ns();
        let shared_audio_level = SharedAudioLevel::new();
        let effects = EffectsControl::new_shared();
        let seek_generation = Arc::new(AtomicU64::new(0));
        let transition_generation = Arc::new(AtomicU64::new(0));
        let (cmd_tx, thread_alive) = spawn_audio_thread(
            Arc::clone(&shared_audio_level),
            Arc::clone(&effects),
            Arc::clone(&playback_generation),
            Arc::clone(&seek_generation),
            Arc::clone(&transition_generation),
        );
        Self {
            cmd_tx,
            thread_alive,
            is_playing: false,
            volume: 1.0,
            speed: 1.0,
            current_path: None,
            duration_ms: 0,
            shared_audio_level,
            effects,
            playback_generation,
            seek_generation,
            transition_generation,
            loaded_generation: None,
            ended_transition: None,
            clock: None,
            audio_info: None,
        }
    }

    fn ensure_alive(&mut self) {
        if self.thread_alive.load(Ordering::Acquire) {
            return;
        }
        log::warn!(target: "cpal-output", "audio thread stopped, restarting");
        let (cmd_tx, thread_alive) = spawn_audio_thread(
            Arc::clone(&self.shared_audio_level),
            Arc::clone(&self.effects),
            Arc::clone(&self.playback_generation),
            Arc::clone(&self.seek_generation),
            Arc::clone(&self.transition_generation),
        );
        self.cmd_tx = cmd_tx;
        self.thread_alive = thread_alive;
        let _ = self.cmd_tx.send(AudioCmd::SetVolume(self.volume));
        let _ = self.cmd_tx.send(AudioCmd::SetSpeed(self.speed));
    }

    fn ensure_request_current(&self, expected: u64) -> AppResult<()> {
        if self.playback_generation.load(Ordering::Acquire) == expected {
            Ok(())
        } else {
            Err(AppError::Audio(PLAYBACK_SUPERSEDED.into()))
        }
    }

    /// 发送播放命令到音频线程，不等待结果。返回 PlayRequest 供锁外等待。
    fn request_start_source(
        &mut self,
        source: AudioSource,
        start_position_ms: u64,
        transition: PlayTransition,
        expected_generation: u64,
        current_path: String,
    ) -> AppResult<PlayRequest> {
        self.ensure_alive();
        self.ensure_request_current(expected_generation)?;
        let transition_generation = self.transition_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let prepare_cancel = Arc::new(AtomicBool::new(false));
        let is_remote = matches!(
            source,
            AudioSource::Remote(_, _) | AudioSource::Growing(_, _, _)
        );
        let (reply_tx, reply_rx) = mpsc::channel();
        self.cmd_tx
            .send(AudioCmd::Play {
                source,
                start_position_ms,
                transition,
                playback_generation: expected_generation,
                transition_generation,
                prepare_cancel: Arc::clone(&prepare_cancel),
                issued_ns: metrics::monotonic_ns(),
                reply: reply_tx,
            })
            .map_err(|_| AppError::Audio("Audio thread disconnected".into()))?;

        let fade_ms = match transition {
            PlayTransition::Replace => 0,
            PlayTransition::Crossfade {
                fade_out_ms,
                fade_in_ms,
            } => fade_out_ms.max(fade_in_ms),
        };
        Ok(PlayRequest {
            reply_rx,
            expected_generation,
            current_path,
            fade_ms,
            prepare_cancel,
            is_remote,
        })
    }

    /// 用音频线程返回的结果更新播放器状态
    pub fn complete_start(
        &mut self,
        result: PlaybackStarted,
        expected_generation: u64,
        current_path: String,
    ) -> AppResult<u64> {
        self.ensure_request_current(expected_generation)?;
        self.is_playing = true;
        self.current_path = Some(current_path);
        self.duration_ms = result.duration_ms;
        self.loaded_generation = Some(expected_generation);
        self.ended_transition = None;
        self.clock = Some(result.clock);
        self.audio_info = Some(result.audio_info);
        Ok(result.duration_ms)
    }

    pub fn playback_audio_info(&self, request_generation: u64) -> Option<SourceAudioInfo> {
        if self.loaded_generation == Some(request_generation)
            && self.playback_generation.load(Ordering::Acquire) == request_generation
        {
            self.audio_info.clone()
        } else {
            None
        }
    }

    /// 便捷方法：发送命令并阻塞等待（持锁整个过程，仅限内部无并发要求场景）
    fn start_source(
        &mut self,
        source: AudioSource,
        start_position_ms: u64,
        transition: PlayTransition,
        expected_generation: u64,
        current_path: String,
    ) -> AppResult<u64> {
        let request = self.request_start_source(
            source,
            start_position_ms,
            transition,
            expected_generation,
            current_path,
        )?;
        let started = wait_for_play_result(&request)?;
        self.complete_start(started, request.expected_generation, request.current_path)
    }

    pub fn play_file(&mut self, path: &str, generation: u64) -> AppResult<u64> {
        self.play_file_at_with_hint(path, 0, 0, generation)
    }

    pub fn play_file_with_hint(
        &mut self,
        path: &str,
        duration_hint_ms: u64,
        generation: u64,
    ) -> AppResult<u64> {
        self.play_file_at_with_hint(path, duration_hint_ms, 0, generation)
    }

    pub fn play_file_at(
        &mut self,
        path: &str,
        start_position_ms: u64,
        generation: u64,
    ) -> AppResult<u64> {
        self.play_file_at_with_hint(path, 0, start_position_ms, generation)
    }

    pub fn play_file_at_with_hint(
        &mut self,
        path: &str,
        duration_hint_ms: u64,
        start_position_ms: u64,
        generation: u64,
    ) -> AppResult<u64> {
        self.start_source(
            AudioSource::File(path.into(), duration_hint_ms),
            start_position_ms,
            PlayTransition::Replace,
            generation,
            path.into(),
        )
    }

    pub fn play_bytes(
        &mut self,
        data: Vec<u8>,
        duration_hint_ms: u64,
        generation: u64,
    ) -> AppResult<u64> {
        self.play_bytes_at(data, duration_hint_ms, 0, generation)
    }

    pub fn play_bytes_at(
        &mut self,
        data: Vec<u8>,
        duration_hint_ms: u64,
        start_position_ms: u64,
        generation: u64,
    ) -> AppResult<u64> {
        self.start_source(
            AudioSource::Bytes(Arc::<[u8]>::from(data), duration_hint_ms),
            start_position_ms,
            PlayTransition::Replace,
            generation,
            "__bytes__".into(),
        )
    }

    pub fn play_stream(
        &mut self,
        reader: GrowingAudioReader,
        duration_hint_ms: u64,
        generation: u64,
    ) -> AppResult<u64> {
        self.play_stream_at(reader, duration_hint_ms, 0, generation)
    }

    pub fn play_stream_at(
        &mut self,
        reader: GrowingAudioReader,
        duration_hint_ms: u64,
        start_position_ms: u64,
        generation: u64,
    ) -> AppResult<u64> {
        self.start_source(
            AudioSource::growing(reader, duration_hint_ms),
            start_position_ms,
            PlayTransition::Replace,
            generation,
            "__growing__".into(),
        )
    }

    pub fn play_remote_at(
        &mut self,
        reader: RemoteAudioSource,
        duration_hint_ms: u64,
        start_position_ms: u64,
        generation: u64,
    ) -> AppResult<u64> {
        self.start_source(
            AudioSource::Remote(reader, duration_hint_ms),
            start_position_ms,
            PlayTransition::Replace,
            generation,
            "__remote__".into(),
        )
    }

    pub fn position_ms(&self) -> u64 {
        let pos = self
            .clock
            .as_ref()
            .map(|clock| clock.position_ms())
            .unwrap_or(0);
        // duration 未知(=0)时不钳制：否则位置被永久钉在 1ms，进度条冻结、
        // near_end / begin_finished_query 永不满足导致曲目播完不推进队列（PB-04）
        if self.duration_ms > 0 {
            pos.min(self.duration_ms)
        } else {
            pos
        }
    }

    pub fn pause(&mut self) {
        if self.is_playing || self.ended_transition().is_some() {
            self.transition_generation.fetch_add(1, Ordering::AcqRel);
            let _ = self.cmd_tx.send(AudioCmd::Pause);
            self.is_playing = false;
        }
    }

    pub fn resume(&mut self) {
        if !self.is_playing && self.current_path.is_some() {
            self.transition_generation.fetch_add(1, Ordering::AcqRel);
            let _ = self.cmd_tx.send(AudioCmd::Resume);
            self.is_playing = true;
        }
    }

    pub fn stop(&mut self) {
        self.transition_generation.fetch_add(1, Ordering::AcqRel);
        let _ = self.cmd_tx.send(AudioCmd::Stop);
        self.clear_playback_state();
    }

    /// 新播放请求一开始就静音上一首（交叉淡化除外），不必等新音源解析、缓冲完
    pub fn silence_stale_sessions(&self, before_generation: u64) {
        let _ = self
            .cmd_tx
            .send(AudioCmd::SilenceStale { before_generation });
    }

    fn clear_playback_state(&mut self) {
        self.is_playing = false;
        self.current_path = None;
        self.duration_ms = 0;
        self.loaded_generation = None;
        self.ended_transition = None;
        self.clock = None;
        self.audio_info = None;
        SharedAudioLevel::reset(&self.shared_audio_level);
    }

    pub fn request_file_release(&mut self, path: String) -> AppResult<FileReleaseRequest> {
        self.ensure_alive();
        let (reply, receiver) = mpsc::channel();
        self.cmd_tx
            .send(AudioCmd::ReleaseFile { path, reply })
            .map_err(|error| AppError::Audio(error.to_string()))?;
        Ok(FileReleaseRequest { receiver })
    }

    pub fn complete_file_release(&mut self, released_generation: Option<u64>) -> bool {
        if released_generation.is_some() && self.loaded_generation == released_generation {
            self.clear_playback_state();
        }
        released_generation.is_some()
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        let _ = self.cmd_tx.send(AudioCmd::SetVolume(self.volume));
        self.effects.dsp.set_volume(self.volume);
    }

    pub fn request_output_device(
        &self,
        name: Option<String>,
    ) -> AppResult<OutputDeviceChangeRequest> {
        self.request_output_device_inner(name, None, false)
    }

    /// Rebuild the output stream even when the selected profile name is unchanged.
    /// Used after switching the sound server's default sink, where the ALSA profile
    /// (`default`) stays the same but the actual route moved.
    /// Rebuild even when the profile name matches, for a switched server default.
    pub fn request_output_device_force(
        &self,
        name: Option<String>,
        display_name: Option<String>,
    ) -> AppResult<OutputDeviceChangeRequest> {
        self.request_output_device_inner(name, display_name, true)
    }

    fn request_output_device_inner(
        &self,
        name: Option<String>,
        display_name: Option<String>,
        force: bool,
    ) -> AppResult<OutputDeviceChangeRequest> {
        let (reply, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cmd_tx
            .send(AudioCmd::SetOutputDevice {
                name,
                display_name,
                force,
                cancel: Arc::clone(&cancel),
                reply,
            })
            .map_err(|error| AppError::Audio(error.to_string()))?;
        Ok(OutputDeviceChangeRequest {
            receiver,
            cancel,
            completed: false,
        })
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.clamp(0.1, 4.0);
        // 倍速在输出端实时生效，不需要重建会话
        let _ = self.cmd_tx.send(AudioCmd::SetSpeed(self.speed));
    }

    pub fn set_pitch(&self, pitch: f32) {
        self.effects.update(|settings| {
            settings.pitch_centi = (pitch.clamp(0.25, 2.0) * 100.0).round() as i32;
        });
    }

    pub fn set_vinyl_effect(&self, enabled: bool) {
        self.effects
            .update(|settings| settings.vinyl_effect_enabled = enabled);
    }

    pub fn set_audio_effects(
        &self,
        settings: AudioEffectsSettings,
        output_device: &str,
    ) -> AudioEffectsSettings {
        self.effects.update(|legacy| legacy.eq_enabled = false);
        self.effects.dsp.set_settings(settings, output_device)
    }

    pub fn audio_effects_stats(&self) -> AudioEffectsStats {
        self.effects.dsp.stats()
    }

    // 音效参数直接写进原子量，正在播放的回调下一块（约 10 ms）就平滑切过去，
    // 不用停下会话重新解码
    pub fn set_loudness_gain(&self, millibels: i32) {
        self.effects
            .update(|settings| settings.loudness_gain_mb = millibels);
    }

    pub fn set_normalize_volume(&self, enabled: bool) {
        self.effects
            .update(|settings| settings.normalize_volume = enabled);
    }

    /// 声道平衡，-100（只剩左声道）～100（只剩右声道）
    pub fn set_balance(&self, balance_centi: i32) {
        self.effects
            .update(|settings| settings.balance_centi = balance_centi);
    }

    pub fn set_equalizer(&self, enabled: bool, bands: &[i32]) {
        self.effects.update(|settings| {
            settings.eq_enabled = enabled;
            for (level, value) in settings.eq_band_levels_mb.iter_mut().zip(bands) {
                *level = *value;
            }
        });
    }

    pub fn reset_effects(&self) {
        self.effects
            .update(|settings| *settings = settings.reset_panel());
        self.effects.dsp.reset_preserving_user_presets();
    }

    pub fn request_seek(
        &mut self,
        position_ms: u64,
        request_generation: u64,
    ) -> AppResult<mpsc::Receiver<Result<(), String>>> {
        self.ensure_alive();
        self.ensure_request_current(request_generation)?;
        if self.loaded_generation != Some(request_generation) {
            return Err(AppError::Audio(PLAYBACK_SUPERSEDED.into()));
        }
        let seeking_after_end = self.ended_transition().is_some();
        let position_ms = clamp_position(position_ms, self.duration_ms);
        self.transition_generation.fetch_add(1, Ordering::AcqRel);
        let seek_generation = self.seek_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let (reply_tx, reply_rx) = mpsc::channel();
        self.cmd_tx
            .send(AudioCmd::Seek {
                position_ms,
                playback_generation: request_generation,
                seek_generation,
                issued_ns: metrics::monotonic_ns(),
                reply: reply_tx,
            })
            .map_err(|_| AppError::Audio("Audio thread disconnected".into()))?;
        // 自然结束只更新引擎状态，未暂停音频会话，seek 后输出会继续
        if seeking_after_end {
            self.is_playing = true;
        }
        Ok(reply_rx)
    }

    pub fn loaded_generation(&self) -> Option<u64> {
        self.loaded_generation
    }

    pub fn is_finished(&self) -> bool {
        if !self.is_playing || self.position_ms() < 500 {
            return false;
        }
        let (reply_tx, reply_rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(AudioCmd::QueryEmpty { reply: reply_tx })
            .is_err()
        {
            return true;
        }
        reply_rx
            .recv_timeout(Duration::from_millis(100))
            .unwrap_or(false)
    }

    /// 结束查询的非阻塞变体：发送 QueryEmpty 后立即返回接收端，供 ticker
    /// 在释放 `player` 锁之后再等待结果（避免持锁做 100ms 阻塞 recv）。
    ///
    /// - 返回 `None`：当前明确未结束（未在播放或播放不足 500ms），无需等待；
    /// - 返回 `Some(rx)`：调用方应在锁外 `recv_timeout` 等待，超时按未结束处理。
    ///   音频线程已断开时接收端会立刻给出 `true`，与 `is_finished` 语义一致。
    pub fn begin_finished_query(&self) -> Option<mpsc::Receiver<bool>> {
        if !self.is_playing || self.position_ms() < 500 {
            return None;
        }
        let (reply_tx, reply_rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(AudioCmd::QueryEmpty { reply: reply_tx })
            .is_err()
        {
            // 音频线程断开视为已结束：预填充一个结果通道保持返回类型统一
            let (tx, rx) = mpsc::channel();
            let _ = tx.send(true);
            return Some(rx);
        }
        Some(reply_rx)
    }

    pub fn transition_generation(&self) -> u64 {
        self.transition_generation.load(Ordering::Acquire)
    }

    pub fn ended_transition(&self) -> Option<u64> {
        self.ended_transition.filter(|&ended| {
            self.loaded_generation == Some(self.playback_generation.load(Ordering::Acquire))
                && ended == self.transition_generation()
        })
    }

    pub fn mark_ended(&mut self, transition: u64) {
        self.is_playing = false;
        self.ended_transition = Some(transition);
    }

    /// 两段式淡出暂停第一段：仅发送命令并返回结果接收端，不阻塞等待。
    /// 调用方应在释放 `player` 锁后用 [`receive_fade_result`] 等待完成
    pub fn request_pause_with_fade(
        &mut self,
        duration_ms: u32,
    ) -> AppResult<mpsc::Receiver<Result<(), String>>> {
        self.ensure_alive();
        let transition_generation = self.transition_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let (reply_tx, reply_rx) = mpsc::channel();
        self.cmd_tx
            .send(AudioCmd::FadeOutPause {
                duration_ms,
                transition_generation,
                reply: reply_tx,
            })
            .map_err(|_| AppError::Audio("Audio thread disconnected".into()))?;
        Ok(reply_rx)
    }

    /// 两段式淡入恢复第一段：仅发送命令并返回结果接收端，不阻塞等待
    pub fn request_resume_with_fade(
        &mut self,
        duration_ms: u32,
    ) -> AppResult<mpsc::Receiver<Result<(), String>>> {
        self.ensure_alive();
        let transition_generation = self.transition_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let (reply_tx, reply_rx) = mpsc::channel();
        self.cmd_tx
            .send(AudioCmd::FadeInResume {
                duration_ms,
                transition_generation,
                reply: reply_tx,
            })
            .map_err(|_| AppError::Audio("Audio thread disconnected".into()))?;
        Ok(reply_rx)
    }

    /// 淡化完成后的状态提交（两段式第二段收尾，短锁内调用）
    pub fn commit_fade_pause(&mut self) {
        self.is_playing = false;
    }

    /// 淡化完成后的状态提交（两段式第二段收尾，短锁内调用）
    pub fn commit_fade_resume(&mut self) {
        self.is_playing = true;
    }

    pub fn pause_with_fade(&mut self, duration_ms: u32) -> AppResult<()> {
        let reply_rx = self.request_pause_with_fade(duration_ms)?;
        let result = receive_fade_result(reply_rx, duration_ms);
        if result.is_ok() {
            self.is_playing = false;
        }
        result
    }

    pub fn resume_with_fade(&mut self, duration_ms: u32) -> AppResult<()> {
        let reply_rx = self.request_resume_with_fade(duration_ms)?;
        let result = receive_fade_result(reply_rx, duration_ms);
        if result.is_ok() {
            self.is_playing = true;
        }
        result
    }

    pub fn crossfade_bytes(
        &mut self,
        data: Vec<u8>,
        duration_hint_ms: u64,
        fade_out_ms: u32,
        fade_in_ms: u32,
        generation: u64,
    ) -> AppResult<u64> {
        self.start_source(
            AudioSource::Bytes(Arc::<[u8]>::from(data), duration_hint_ms),
            0,
            PlayTransition::Crossfade {
                fade_out_ms,
                fade_in_ms,
            },
            generation,
            "__bytes__".into(),
        )
    }

    pub fn crossfade_file(
        &mut self,
        path: &str,
        fade_out_ms: u32,
        fade_in_ms: u32,
        generation: u64,
    ) -> AppResult<u64> {
        self.crossfade_file_with_hint(path, 0, fade_out_ms, fade_in_ms, generation)
    }

    pub fn crossfade_file_with_hint(
        &mut self,
        path: &str,
        duration_hint_ms: u64,
        fade_out_ms: u32,
        fade_in_ms: u32,
        generation: u64,
    ) -> AppResult<u64> {
        self.start_source(
            AudioSource::File(path.into(), duration_hint_ms),
            0,
            PlayTransition::Crossfade {
                fade_out_ms,
                fade_in_ms,
            },
            generation,
            path.into(),
        )
    }

    pub fn crossfade_stream(
        &mut self,
        reader: GrowingAudioReader,
        duration_hint_ms: u64,
        fade_out_ms: u32,
        fade_in_ms: u32,
        generation: u64,
    ) -> AppResult<u64> {
        self.start_source(
            AudioSource::growing(reader, duration_hint_ms),
            0,
            PlayTransition::Crossfade {
                fade_out_ms,
                fade_in_ms,
            },
            generation,
            "__growing__".into(),
        )
    }

    pub fn crossfade_remote(
        &mut self,
        reader: RemoteAudioSource,
        duration_hint_ms: u64,
        fade_out_ms: u32,
        fade_in_ms: u32,
        generation: u64,
    ) -> AppResult<u64> {
        self.start_source(
            AudioSource::Remote(reader, duration_hint_ms),
            0,
            PlayTransition::Crossfade {
                fade_out_ms,
                fade_in_ms,
            },
            generation,
            "__remote__".into(),
        )
    }

    // request_* 变体：只发命令不等待，供 cmd 层锁外等待
    pub fn request_play_file_at_with_hint(
        &mut self,
        path: &str,
        duration_hint_ms: u64,
        start_position_ms: u64,
        generation: u64,
    ) -> AppResult<PlayRequest> {
        self.request_start_source(
            AudioSource::File(path.into(), duration_hint_ms),
            start_position_ms,
            PlayTransition::Replace,
            generation,
            path.into(),
        )
    }

    pub fn request_play_file_with_hint(
        &mut self,
        path: &str,
        duration_hint_ms: u64,
        generation: u64,
    ) -> AppResult<PlayRequest> {
        self.request_play_file_at_with_hint(path, duration_hint_ms, 0, generation)
    }

    pub fn request_play_bytes_at(
        &mut self,
        data: Vec<u8>,
        duration_hint_ms: u64,
        start_position_ms: u64,
        generation: u64,
    ) -> AppResult<PlayRequest> {
        self.request_start_source(
            AudioSource::Bytes(Arc::<[u8]>::from(data), duration_hint_ms),
            start_position_ms,
            PlayTransition::Replace,
            generation,
            "__bytes__".into(),
        )
    }

    pub fn request_play_stream_at(
        &mut self,
        reader: GrowingAudioReader,
        duration_hint_ms: u64,
        start_position_ms: u64,
        generation: u64,
    ) -> AppResult<PlayRequest> {
        self.request_start_source(
            AudioSource::growing(reader, duration_hint_ms),
            start_position_ms,
            PlayTransition::Replace,
            generation,
            "__growing__".into(),
        )
    }

    pub fn request_play_remote_at(
        &mut self,
        reader: RemoteAudioSource,
        duration_hint_ms: u64,
        start_position_ms: u64,
        generation: u64,
    ) -> AppResult<PlayRequest> {
        self.request_start_source(
            AudioSource::Remote(reader, duration_hint_ms),
            start_position_ms,
            PlayTransition::Replace,
            generation,
            "__remote__".into(),
        )
    }

    pub fn request_crossfade_file_with_hint(
        &mut self,
        path: &str,
        duration_hint_ms: u64,
        fade_out_ms: u32,
        fade_in_ms: u32,
        generation: u64,
    ) -> AppResult<PlayRequest> {
        self.request_start_source(
            AudioSource::File(path.into(), duration_hint_ms),
            0,
            PlayTransition::Crossfade {
                fade_out_ms,
                fade_in_ms,
            },
            generation,
            path.into(),
        )
    }

    pub fn request_crossfade_bytes(
        &mut self,
        data: Vec<u8>,
        duration_hint_ms: u64,
        fade_out_ms: u32,
        fade_in_ms: u32,
        generation: u64,
    ) -> AppResult<PlayRequest> {
        self.request_start_source(
            AudioSource::Bytes(Arc::<[u8]>::from(data), duration_hint_ms),
            0,
            PlayTransition::Crossfade {
                fade_out_ms,
                fade_in_ms,
            },
            generation,
            "__bytes__".into(),
        )
    }

    pub fn request_crossfade_stream(
        &mut self,
        reader: GrowingAudioReader,
        duration_hint_ms: u64,
        fade_out_ms: u32,
        fade_in_ms: u32,
        generation: u64,
    ) -> AppResult<PlayRequest> {
        self.request_start_source(
            AudioSource::growing(reader, duration_hint_ms),
            0,
            PlayTransition::Crossfade {
                fade_out_ms,
                fade_in_ms,
            },
            generation,
            "__growing__".into(),
        )
    }

    pub fn request_crossfade_remote(
        &mut self,
        reader: RemoteAudioSource,
        duration_hint_ms: u64,
        fade_out_ms: u32,
        fade_in_ms: u32,
        generation: u64,
    ) -> AppResult<PlayRequest> {
        self.request_start_source(
            AudioSource::Remote(reader, duration_hint_ms),
            0,
            PlayTransition::Crossfade {
                fade_out_ms,
                fade_in_ms,
            },
            generation,
            "__remote__".into(),
        )
    }
}
