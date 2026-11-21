//! 音频控制线程：命令循环与会话切换

use super::*;

pub(super) fn spawn_audio_thread(
    shared_level: Arc<Mutex<SharedAudioLevel>>,
    effects: Arc<EffectsControl>,
    playback_generation: Arc<AtomicU64>,
    seek_generation: Arc<AtomicU64>,
    transition_generation: Arc<AtomicU64>,
) -> (mpsc::Sender<AudioCmd>, Arc<AtomicBool>) {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let alive = Arc::new(AtomicBool::new(true));
    let alive_for_thread = Arc::clone(&alive);
    // 回传句柄：cpal 错误回调用它向控制线程上报 DeviceLost
    let loopback_tx = cmd_tx.clone();
    thread::Builder::new()
        .name("cpal-playback-control".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                audio_control_loop(
                    cmd_rx,
                    loopback_tx,
                    shared_level,
                    effects,
                    playback_generation,
                    seek_generation,
                    transition_generation,
                );
            }));
            if let Err(error) = result {
                log::error!(target: "cpal-output", "control thread panicked: {error:?}");
            }
            alive_for_thread.store(false, Ordering::Release);
        })
        .expect("Could not start CPAL playback control thread");
    (cmd_tx, alive)
}

fn audio_control_loop(
    receiver: mpsc::Receiver<AudioCmd>,
    loopback_tx: mpsc::Sender<AudioCmd>,
    shared_level: Arc<Mutex<SharedAudioLevel>>,
    effects: Arc<EffectsControl>,
    playback_generation: Arc<AtomicU64>,
    seek_generation: Arc<AtomicU64>,
    transition_generation: Arc<AtomicU64>,
) {
    let mut current: Option<PlaybackSession> = None;
    let output_watch = spawn_output_device_watch(loopback_tx.clone());
    let mut output_was_in_use = false;
    let mut volume = 1.0f32;
    let mut speed = 1.0f32;
    // 被 seek 折叠/接管路径暂存的命令队列：必须保序回放，
    // 单槽 Option 会让 ticker 的 QueryEmpty 直接打断 seek 接管等待
    let mut deferred: VecDeque<AudioCmd> = VecDeque::new();
    let profile = match OutputDeviceProfile::open_default() {
        Ok(profile) => {
            log::info!(
                target: "cpal-output",
                "prewarmed {}: {} Hz, {} ch",
                profile.name,
                profile.config.sample_rate.0,
                profile.config.channels
            );
            Some(profile)
        }
        Err(error) => {
            log::warn!(target: "cpal-output", "device prewarm deferred: {error}");
            None
        }
    };

    if let Some(profile) = &profile {
        effects.dsp.select_output(&profile.name);
    }
    let mut output_profile = OutputDeviceState {
        preferred_name: None,
        profile,
        dsp_output_name: None,
    };
    let mut reporter = MetricsReporter::new();
    loop {
        let output_in_use = current
            .as_ref()
            .is_some_and(|session| !session.shared.paused.load(Ordering::Acquire));
        if output_in_use != output_was_in_use {
            if let Some(watch) = &output_watch {
                let _ = watch.send(output_in_use);
            }
            output_was_in_use = output_in_use;
        }
        let idle_wait = if reporter.has_pending_probes() {
            metrics::PROBE_POLL_INTERVAL
        } else {
            OUTPUT_DEVICE_POLL_INTERVAL
        };
        let command = match deferred.pop_front() {
            Some(command) => Some(command),
            None => match receiver.recv_timeout(idle_wait) {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            },
        };
        reporter.poll(
            current.as_ref().map(|session| session.playback_generation),
            output_in_use,
        );
        let Some(command) = command else { continue };
        match command {
            AudioCmd::OutputDevicesListed { available, default } => {
                let Some(name) = output_device_to_switch(
                    output_profile.preferred_name.as_deref(),
                    &available,
                    default.as_deref(),
                    output_profile
                        .profile
                        .as_ref()
                        .map(|profile| profile.name.as_str()),
                ) else {
                    continue;
                };
                let result = OutputDeviceProfile::open_named(&name).and_then(|candidate| {
                    switch_output_device_in_place(
                        &mut current,
                        candidate,
                        &mut output_profile,
                        volume,
                        speed,
                        &shared_level,
                        &effects,
                        &playback_generation,
                        &loopback_tx,
                        Arc::new(AtomicBool::new(false)),
                    )
                });
                if result.is_ok() {
                    if let Some(display_name) = &output_profile.dsp_output_name {
                        effects.dsp.select_output(display_name);
                    }
                } else if let Err(error) = result {
                    log::warn!(target: "cpal-output", "output device change deferred: {error}");
                }
            }
            AudioCmd::Play {
                source,
                start_position_ms,
                transition,
                playback_generation: expected,
                transition_generation: expected_transition,
                prepare_cancel,
                issued_ns,
                reply,
            } => {
                let received_ns = metrics::monotonic_ns();
                let source_label = source.label();
                let transition_label = match transition {
                    PlayTransition::Replace => "replace",
                    PlayTransition::Crossfade { .. } => "crossfade",
                };
                let command_started = Instant::now();
                log::info!(
                    target: "cpal-output",
                    "play command received source={}, generation={}, start_ms={}, transition={}",
                    source_label,
                    expected,
                    start_position_ms,
                    transition_label,
                );
                let prepared = (|| {
                    if prepare_cancel.load(Ordering::Acquire) {
                        return Err("Timed out waiting for decoded audio".into());
                    }
                    let output = ensure_output_profile(&mut output_profile)?;
                    prepare_session(
                        source,
                        start_position_ms,
                        volume,
                        speed,
                        Arc::clone(&shared_level),
                        Arc::clone(&effects),
                        // 新曲目：响度从头统计
                        TrackLoudness::new_shared(),
                        Arc::clone(&playback_generation),
                        expected,
                        None,
                        None,
                        output,
                        Arc::clone(&prepare_cancel),
                        &loopback_tx,
                    )
                })();
                if prepared
                    .as_ref()
                    .is_err_and(|error| error.starts_with("Could not build audio output"))
                {
                    output_profile.profile = None;
                }
                restore_dsp_output_identity(&effects, &output_profile);
                let next = match prepared {
                    Ok(next) => next,
                    Err(error) => {
                        log::error!(
                            target: "cpal-output",
                            "failed to prepare {} generation={} elapsed_ms={}: {}",
                            source_label,
                            expected,
                            command_started.elapsed().as_millis(),
                            error,
                        );
                        let _ = reply.send(Err(error));
                        continue;
                    }
                };
                if let Err(error) = ensure_generation(&playback_generation, expected)
                    .and_then(|_| ensure_generation(&transition_generation, expected_transition))
                {
                    let _ = reply.send(Err(error));
                    continue;
                }

                next.shared.fade_gain.store(match transition {
                    PlayTransition::Crossfade { fade_in_ms, .. } if fade_in_ms > 0 => 0.0,
                    PlayTransition::Replace | PlayTransition::Crossfade { .. } => 1.0,
                });
                if let Err(error) = next.play() {
                    log::error!(
                        target: "cpal-output",
                        "failed to start output source={} generation={} elapsed_ms={}: {}",
                        source_label,
                        expected,
                        command_started.elapsed().as_millis(),
                        error,
                    );
                    let _ = reply.send(Err(error));
                    continue;
                }

                let started = PlaybackStarted {
                    duration_ms: next.duration_ms,
                    clock: Arc::clone(&next.shared.clock),
                    audio_info: next.audio_info.clone(),
                };
                reporter.track(FirstFrameProbe::new(
                    StartKind::Play,
                    source_label,
                    expected,
                    start_position_ms,
                    CommandStamps {
                        issued_ns,
                        received_ns,
                    },
                    Arc::clone(&next.shared.first_frame_ns),
                ));
                let mut previous = current.take();
                current = Some(next);
                if let Some(watch) = &output_watch {
                    let _ = watch.send(true);
                }
                output_was_in_use = true;
                log::info!(
                    target: "cpal-output",
                    "play session started source={} generation={} duration_ms={} elapsed_ms={}",
                    source_label,
                    expected,
                    started.duration_ms,
                    command_started.elapsed().as_millis(),
                );

                match transition {
                    PlayTransition::Replace => {
                        if let Some(mut previous) = previous.take() {
                            previous.stop();
                        }
                        let _ = reply.send(Ok(started));
                    }
                    PlayTransition::Crossfade {
                        fade_out_ms,
                        fade_in_ms,
                    } => {
                        // 新会话已可输出，先提交播放结果；淡化不再阻塞首播完成
                        let _ = reply.send(Ok(started));
                        let fade_result = crossfade_sessions(
                            previous.as_ref(),
                            current.as_ref().expect("committed playback session"),
                            fade_out_ms,
                            fade_in_ms,
                            &playback_generation,
                            expected,
                            &transition_generation,
                            expected_transition,
                        );
                        if let Some(session) = current.as_ref() {
                            session.shared.fade_gain.store(1.0);
                        }
                        if let Some(mut previous) = previous.take() {
                            previous.stop();
                        }
                        if let Err(error) = fade_result {
                            log::warn!(target: "cpal-output", "crossfade interrupted: {error}");
                        }
                    }
                }
            }
            AudioCmd::Pause => {
                if let Some(session) = &current {
                    session.pause();
                }
            }
            AudioCmd::Resume => {
                if let Some(session) = &current {
                    let _ = session.play();
                }
            }
            AudioCmd::Stop => {
                if let Some(mut session) = current.take() {
                    session.stop();
                }
                SharedAudioLevel::reset(&shared_level);
            }
            AudioCmd::SilenceStale { before_generation } => {
                silence_stale_session(current.as_ref(), before_generation);
            }
            AudioCmd::ReleaseFile { path, reply } => {
                let released = current
                    .as_ref()
                    .filter(|session| matches_local_file(&session.source, Path::new(&path)))
                    .map(|session| session.playback_generation);
                if let Some(released) = released {
                    invalidate_released_generation(&playback_generation, released);
                    if let Some(mut session) = current.take() {
                        session.stop();
                    }
                    SharedAudioLevel::reset(&shared_level);
                }
                // 整首扫描也开着文件（可能属于刚切走的上一首），一并停下
                stop_file_scans(Path::new(&path));
                // 本地 worker 与扫描线程都 join 完成后才允许调用方删除或改写文件
                let _ = reply.send(released);
            }
            AudioCmd::SetVolume(next_volume) => {
                volume = next_volume.clamp(0.0, 1.0);
                if let Some(session) = &current {
                    session.shared.volume.store(volume);
                }
            }
            AudioCmd::SetOutputDevice {
                name,
                display_name,
                force,
                cancel,
                reply,
            } => {
                if cancel.load(Ordering::Acquire) {
                    let _ = reply.send(Err("Audio output change cancelled".into()));
                    continue;
                }
                let name = name
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty());
                let result = match name.as_deref() {
                    Some(name) => OutputDeviceProfile::open_named(name),
                    None => OutputDeviceProfile::open_default(),
                }
                .and_then(|candidate| {
                    if cancel.load(Ordering::Acquire) {
                        return Err("Audio output change cancelled".into());
                    }
                    if !force
                        && output_profile
                            .profile
                            .as_ref()
                            .is_some_and(|profile| profile.name == candidate.name)
                    {
                        Ok(())
                    } else {
                        switch_output_device_in_place(
                            &mut current,
                            candidate,
                            &mut output_profile,
                            volume,
                            speed,
                            &shared_level,
                            &effects,
                            &playback_generation,
                            &loopback_tx,
                            cancel,
                        )
                    }
                });
                if result.is_ok() {
                    output_profile.preferred_name = name;
                    output_profile.dsp_output_name = display_name.clone();
                    let dsp_name = display_name.as_deref().or_else(|| {
                        output_profile
                            .profile
                            .as_ref()
                            .map(|profile| profile.name.as_str())
                    });
                    if let Some(dsp_name) = dsp_name {
                        effects.dsp.select_output(dsp_name);
                    }
                }
                let _ = reply.send(result);
            }
            AudioCmd::SetSpeed(next_speed) => {
                speed = next_speed.clamp(0.1, 4.0);
                // 输出端的变速器在下一跳（约 15 ms 内）就按新倍速、保持音调地播放，
                // ring 里的内容与倍速无关，不用停下会话重新解码
                if let Some(session) = &current {
                    session.shared.speed.store(speed);
                }
            }
            AudioCmd::Seek {
                position_ms,
                playback_generation: expected,
                seek_generation: expected_seek,
                issued_ns,
                reply,
            } => {
                let mut latest = position_ms;
                let mut latest_generation = expected;
                let mut latest_seek_generation = expected_seek;
                let mut latest_issued_ns = issued_ns;
                let mut latest_received_ns = metrics::monotonic_ns();
                let mut latest_reply = reply;
                take_latest_seek(
                    &mut latest,
                    &mut latest_generation,
                    &mut latest_seek_generation,
                    &mut latest_issued_ns,
                    &mut latest_reply,
                    &receiver,
                    &mut deferred,
                );
                let (source, paused, rollback_position_ms, clock, loudness) = {
                    let Some(session) = current.as_ref() else {
                        let _ = latest_reply.send(Err("Nothing is playing".into()));
                        continue;
                    };
                    if session.playback_generation != latest_generation
                        || ensure_generation(&playback_generation, latest_generation).is_err()
                    {
                        let _ = latest_reply.send(Err(PLAYBACK_SUPERSEDED.into()));
                        continue;
                    }
                    let seek_token = GenerationToken {
                        generation: Arc::clone(&seek_generation),
                        expected: latest_seek_generation,
                    };
                    if !seek_token.is_current() {
                        let _ = latest_reply.send(Err(SEEK_SUPERSEDED.into()));
                        continue;
                    }
                    (
                        session.source.clone(),
                        session.shared.paused.load(Ordering::Acquire),
                        session.shared.clock.position_ms(),
                        Arc::clone(&session.shared.clock),
                        // 同一首歌：响度统计和已施加的增益接着用，seek 后音量不跳
                        Arc::clone(&session.shared.loudness),
                    )
                };
                let mut previous = match current.take() {
                    Some(session) => session,
                    None => {
                        let _ = latest_reply.send(Err("Nothing is playing".into()));
                        continue;
                    }
                };
                // 抢占-接管循环：本次 seek 输给更新的 seek 时，接过最新目标重跑；
                // 接不到（宽限超时）就回滚旧位置重建。任何出口都不允许把会话悬空丢掉
                // ——否则会出现「旧 seek 被作废丢弃会话、新 seek 到达时 Nothing is
                // playing」的双亡死局（实机日志：seek 后卡死、预取仍在跑但无 decoder）
                loop {
                    // 新 Play 已接管（或接管来的 seek 不属于本会话）：
                    // 会话交接归 Play 命令处理，这里只需退出
                    if previous.playback_generation != latest_generation
                        || ensure_generation(&playback_generation, latest_generation).is_err()
                    {
                        let _ = latest_reply.send(Err(PLAYBACK_SUPERSEDED.into()));
                        break;
                    }
                    let seek_token = GenerationToken {
                        generation: Arc::clone(&seek_generation),
                        expected: latest_seek_generation,
                    };
                    if !seek_token.is_current() {
                        // 被更新的 seek 超越：旧请求让位，但最新的 seek 必须完整执行
                        let _ = latest_reply.send(Err(SEEK_SUPERSEDED.into()));
                        if let Some(adopted) =
                            wait_for_newer_seek(&receiver, &mut deferred, SEEK_ADOPT_GRACE)
                        {
                            latest = adopted.position_ms;
                            latest_generation = adopted.playback_generation;
                            latest_seek_generation = adopted.seek_generation;
                            latest_issued_ns = adopted.issued_ns;
                            latest_received_ns = metrics::monotonic_ns();
                            latest_reply = adopted.reply;
                            continue;
                        }
                        // 等不到新 Seek 命令（极端调度竞态）：回滚旧位置，
                        // 迟到的 Seek 会在恢复出的会话上正常执行
                        current = rebuild_after_failed_seek(
                            &previous,
                            rollback_position_ms,
                            paused,
                            volume,
                            speed,
                            &shared_level,
                            &effects,
                            &playback_generation,
                            latest_generation,
                            &clock,
                            &mut output_profile,
                            &loopback_tx,
                        );
                        break;
                    }
                    // 乐观置位目标时间：stop 到 decoder 打开完成之间可达秒级，
                    // 不置位的话 ticker 会把旧位置事件发给前端，进度条先回跳
                    // 再跳回目标（回流）。失败路径由 rollback 恢复快照位置
                    clock.store_ms(latest);
                    // 先停旧会话：取消旧 decode worker 的 remote 读，
                    // 避免和 seekable 重建抢 in_flight（重复 stop 幂等）
                    previous.stop();
                    let prepared = (|| {
                        let output = ensure_output_profile(&mut output_profile)?;
                        prepare_session(
                            source.clone(),
                            latest,
                            volume,
                            speed,
                            Arc::clone(&shared_level),
                            Arc::clone(&effects),
                            Arc::clone(&loudness),
                            Arc::clone(&playback_generation),
                            latest_generation,
                            Some(seek_token.clone()),
                            Some(Arc::clone(&clock)),
                            output,
                            Arc::new(AtomicBool::new(false)),
                            &loopback_tx,
                        )
                    })();
                    if prepared
                        .as_ref()
                        .is_err_and(|error| error.starts_with("Could not build audio output"))
                    {
                        output_profile.profile = None;
                    }
                    restore_dsp_output_identity(&effects, &output_profile);
                    match prepared {
                        Ok(next) => {
                            if ensure_generation(&playback_generation, latest_generation).is_err() {
                                let _ = latest_reply.send(Err(PLAYBACK_SUPERSEDED.into()));
                                break;
                            }
                            if !seek_token.is_current() {
                                // 会话已就绪但代际又被更新 seek 抬走：
                                // 丢弃本次结果，回到循环顶部接管最新目标
                                drop(next);
                                continue;
                            }
                            if paused {
                                // 暂停态 seek：新会话默认非暂停且输出流在跑，
                                // 必须显式暂停，否则 UI 显示暂停但歌曲已出声
                                next.pause();
                            } else {
                                if let Err(error) = next.play() {
                                    log::warn!(
                                        target: "cpal-output",
                                        "seek play failed: {error}"
                                    );
                                    drop(next);
                                    current = rebuild_after_failed_seek(
                                        &previous,
                                        rollback_position_ms,
                                        paused,
                                        volume,
                                        speed,
                                        &shared_level,
                                        &effects,
                                        &playback_generation,
                                        latest_generation,
                                        &clock,
                                        &mut output_profile,
                                        &loopback_tx,
                                    );
                                    let _ = latest_reply.send(Err(error));
                                    break;
                                }
                                // 暂停态 seek 要等用户恢复才出声，那段等待不算 seek 耗时
                                reporter.track(FirstFrameProbe::new(
                                    StartKind::Seek,
                                    previous.source.label(),
                                    latest_generation,
                                    latest,
                                    CommandStamps {
                                        issued_ns: latest_issued_ns,
                                        received_ns: latest_received_ns,
                                    },
                                    Arc::clone(&next.shared.first_frame_ns),
                                ));
                            }
                            current = Some(next);
                            let _ = latest_reply.send(Ok(()));
                            break;
                        }
                        Err(error) => {
                            if ensure_generation(&playback_generation, latest_generation).is_err() {
                                log::warn!(target: "cpal-output", "seek failed: {error}");
                                let _ = latest_reply.send(Err(PLAYBACK_SUPERSEDED.into()));
                                break;
                            }
                            if !seek_token.is_current() {
                                // prepare 被更新 seek 的代际递增打断（remote read
                                // superseded）：回到循环顶部接管最新目标重跑
                                log::warn!(
                                    target: "cpal-output",
                                    "seek superseded mid-prepare: {error}"
                                );
                                continue;
                            }
                            log::warn!(target: "cpal-output", "seek failed: {error}");
                            // 回滚时钟并重建旧位置，绝不静默卡死
                            current = rebuild_after_failed_seek(
                                &previous,
                                rollback_position_ms,
                                paused,
                                volume,
                                speed,
                                &shared_level,
                                &effects,
                                &playback_generation,
                                latest_generation,
                                &clock,
                                &mut output_profile,
                                &loopback_tx,
                            );
                            let _ = latest_reply.send(Err(error));
                            break;
                        }
                    }
                }
            }
            AudioCmd::QueryEmpty { reply } => {
                let _ = reply.send(current.as_ref().is_none_or(PlaybackSession::is_empty));
            }
            AudioCmd::FadeOutPause {
                duration_ms,
                transition_generation: expected_transition,
                reply,
            } => {
                let result = if let Some(session) = &current {
                    fade_gain(
                        &session.shared,
                        session.shared.fade_gain.load(),
                        0.0,
                        duration_ms,
                        &playback_generation,
                        session.playback_generation,
                        &transition_generation,
                        expected_transition,
                    )
                    .map(|_| session.pause())
                } else {
                    Err("Nothing is playing".into())
                };
                let _ = reply.send(result);
            }
            AudioCmd::FadeInResume {
                duration_ms,
                transition_generation: expected_transition,
                reply,
            } => {
                let result = if let Some(session) = &current {
                    session.shared.fade_gain.store(0.0);
                    session.play().and_then(|_| {
                        fade_gain(
                            &session.shared,
                            0.0,
                            1.0,
                            duration_ms,
                            &playback_generation,
                            session.playback_generation,
                            &transition_generation,
                            expected_transition,
                        )
                    })
                } else {
                    Err("Nothing is playing".into())
                };
                let _ = reply.send(result);
            }
            AudioCmd::DeviceLost {
                playback_generation: lost_generation,
                session_cancelled,
            } => {
                // 只处理当前会话的失效上报：会话切换后旧流的错误回调
                // 可能还会补发一条陈旧的 DeviceLost
                let matches_current = current.as_ref().is_some_and(|session| {
                    matches_output_session(
                        session.playback_generation,
                        &session.shared.cancelled,
                        lost_generation,
                        &session_cancelled,
                    )
                });
                if !matches_current {
                    log::info!(
                        target: "cpal-output",
                        "stale DeviceLost ignored generation={lost_generation}",
                    );
                    continue;
                }
                log::warn!(
                    target: "cpal-output",
                    "output device lost generation={lost_generation}, rebuilding on default device",
                );
                // 丢弃缓存的设备档案，强制按当前系统默认设备重开
                output_profile.profile = None;
                rebuild_session_in_place(
                    &mut current,
                    volume,
                    speed,
                    &shared_level,
                    &effects,
                    &playback_generation,
                    &mut output_profile,
                    &loopback_tx,
                );
                if current.is_none() {
                    // 重建失败：会话已置空，QueryEmpty 将返回 true，
                    // ticker 会据此发出结束事件推进队列，不会静默卡死
                    log::error!(
                        target: "cpal-output",
                        "device-lost rebuild failed generation={lost_generation}, session dropped",
                    );
                }
            }
        }
    }

    if let Some(mut session) = current {
        session.stop();
    }
}
