//! 播放会话的准备、原地重建与解码线程

use super::*;

// 远程 worker 可能阻塞在网络读上，交给后台回收避免拖住音频控制线程
fn reap_worker_handle(handle: JoinHandle<()>) {
    use std::sync::OnceLock;
    static REAPER: OnceLock<Option<mpsc::Sender<JoinHandle<()>>>> = OnceLock::new();
    let sender = REAPER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<JoinHandle<()>>();
        thread::Builder::new()
            .name("audio-worker-reaper".into())
            .spawn(move || {
                for handle in rx {
                    let _ = handle.join();
                }
            })
            .ok()
            .map(|_| tx)
    });
    if let Some(sender) = sender {
        // 发送失败（收尸线程已退出）时退化为旧行为：丢弃句柄
        let _ = sender.send(handle);
    }
}

pub(super) fn finish_decode_worker(
    source: &AudioSource,
    shared: &PlaybackShared,
    worker: JoinHandle<()>,
) {
    shared.cancelled.store(true, Ordering::Release);
    shared.wake.notify_all();
    // 文件操作必须等解码器关闭句柄，远程读仍在后台回收
    if matches!(source, AudioSource::File(_, _)) || worker.is_finished() {
        let _ = worker.join();
    } else {
        reap_worker_handle(worker);
    }
}

pub(super) fn silence_stale_session(current: Option<&PlaybackSession>, before_generation: u64) {
    if let Some(session) = current.filter(|session| session.playback_generation < before_generation)
    {
        session.pause();
    }
}

pub(super) fn matches_local_file(source: &AudioSource, target: &Path) -> bool {
    let AudioSource::File(path, _) = source else {
        return false;
    };
    same_local_file(Path::new(path), target)
}

pub(super) fn same_local_file(source: &Path, target: &Path) -> bool {
    if let (Ok(source), Ok(target)) = (source.canonicalize(), target.canonicalize()) {
        return source == target;
    }
    #[cfg(windows)]
    {
        source
            .to_string_lossy()
            .eq_ignore_ascii_case(&target.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        source == target
    }
}

pub(super) fn invalidate_released_generation(generation: &AtomicU64, released: u64) {
    // 用户已切歌时不改变新请求，只有被释放的会话仍当前时才作废首播回包
    let _ = generation.compare_exchange(
        released,
        released.saturating_add(1),
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn switch_output_device_in_place(
    current: &mut Option<PlaybackSession>,
    candidate: OutputDeviceProfile,
    output: &mut OutputDeviceState,
    volume: f32,
    speed: f32,
    shared_level: &Arc<Mutex<SharedAudioLevel>>,
    effects: &Arc<EffectsControl>,
    playback_generation: &Arc<AtomicU64>,
    loopback_tx: &mpsc::Sender<AudioCmd>,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Audio output change cancelled".into());
    }
    let Some(previous) = current.as_mut() else {
        effects.dsp.select_output(&candidate.name);
        output.profile = Some(candidate);
        return Ok(());
    };
    ensure_generation(playback_generation, previous.playback_generation)?;
    let paused = previous.shared.paused.load(Ordering::Acquire);
    previous.pause();
    // 旧流独占着设备，或新流要独占而旧共享流还开着时，新流都打不开：先让出设备。
    // 让出后旧流不能再恢复，失败时改为在原设备、原位置重建会话
    let released = previous.release_output_before_reopen(candidate.exclusive);
    let previous = &*previous;
    // 保留旧流和源，目标设备准备失败时还能回到原位置
    let prepared = prepare_session(
        previous.source.clone(),
        previous.shared.clock.position_ms(),
        volume,
        speed,
        Arc::clone(shared_level),
        Arc::clone(effects),
        Arc::clone(&previous.shared.loudness),
        Arc::clone(playback_generation),
        previous.playback_generation,
        None,
        Some(Arc::clone(&previous.shared.clock)),
        &candidate,
        Arc::clone(&cancel),
        loopback_tx,
    )
    .and_then(|next| {
        if cancel.load(Ordering::Acquire) {
            return Err("Audio output change cancelled".into());
        }
        if paused {
            next.pause();
        } else {
            next.play()?;
        }
        ensure_generation(playback_generation, previous.playback_generation)?;
        if cancel.load(Ordering::Acquire) {
            return Err("Audio output change cancelled".into());
        }
        Ok(next)
    });
    match prepared {
        Ok(next) => {
            if let Some(mut previous) = current.replace(next) {
                previous.stop();
            }
            output.profile = Some(candidate);
            if let Some(display_name) = &output.dsp_output_name {
                effects.dsp.select_output(display_name);
            } else {
                effects
                    .dsp
                    .select_output(&output.profile.as_ref().unwrap().name);
            }
            Ok(())
        }
        Err(error) => {
            restore_dsp_output_identity(effects, output);
            if released {
                restore_released_session(
                    current,
                    paused,
                    volume,
                    speed,
                    shared_level,
                    effects,
                    playback_generation,
                    output,
                    loopback_tx,
                );
            } else if !paused
                && ensure_generation(playback_generation, previous.playback_generation).is_ok()
            {
                let _ = previous.play();
            }
            Err(error)
        }
    }
}

/// 旧流已让出设备、新设备又没建成：按原档案在原位置重建会话，避免停在无声状态
#[allow(clippy::too_many_arguments)]
fn restore_released_session(
    current: &mut Option<PlaybackSession>,
    paused: bool,
    volume: f32,
    speed: f32,
    shared_level: &Arc<Mutex<SharedAudioLevel>>,
    effects: &Arc<EffectsControl>,
    playback_generation: &Arc<AtomicU64>,
    output: &OutputDeviceState,
    loopback_tx: &mpsc::Sender<AudioCmd>,
) {
    let (Some(previous), Some(profile)) = (current.as_ref(), output.profile.as_ref()) else {
        return;
    };
    if ensure_generation(playback_generation, previous.playback_generation).is_err() {
        return;
    }
    let restored = prepare_session(
        previous.source.clone(),
        previous.shared.clock.position_ms(),
        volume,
        speed,
        Arc::clone(shared_level),
        Arc::clone(effects),
        Arc::clone(&previous.shared.loudness),
        Arc::clone(playback_generation),
        previous.playback_generation,
        None,
        Some(Arc::clone(&previous.shared.clock)),
        profile,
        Arc::new(AtomicBool::new(false)),
        loopback_tx,
    )
    .and_then(|next| {
        if paused {
            next.pause();
        } else {
            next.play()?;
        }
        Ok(next)
    });
    match restored {
        Ok(next) => {
            if let Some(mut released) = current.replace(next) {
                released.stop();
            }
        }
        Err(error) => {
            log::warn!(target: "cpal-output", "could not restore output after releasing exclusive device: {error}");
        }
    }
}

/// 输出设备失效后按当前位置原地重建会话
///
/// 同一首歌的响度统计沿用，重建前后音量一致。重建失败时再重试一次，
/// 两次都失败才放弃会话（由上层发出结束事件推进队列，不会静默卡住）。
#[allow(clippy::too_many_arguments)]
pub(super) fn rebuild_session_in_place(
    current: &mut Option<PlaybackSession>,
    volume: f32,
    speed: f32,
    shared_level: &Arc<Mutex<SharedAudioLevel>>,
    effects: &Arc<EffectsControl>,
    playback_generation: &Arc<AtomicU64>,
    output_profile: &mut OutputDeviceState,
    loopback_tx: &mpsc::Sender<AudioCmd>,
) {
    let Some(session) = current.as_ref() else {
        return;
    };
    let latest_generation = session.playback_generation;
    if ensure_generation(playback_generation, latest_generation).is_err() {
        return;
    }
    let position_ms = session.shared.clock.position_ms();
    let paused = session.shared.paused.load(Ordering::Acquire);
    let clock = Arc::clone(&session.shared.clock);
    let loudness = Arc::clone(&session.shared.loudness);
    let source = session.source.clone();

    let Some(mut previous) = current.take() else {
        return;
    };
    // 先停旧 worker：取消远程读，避免与重建后的解码器抢 in_flight
    previous.stop();
    drop(previous);

    let mut build = || {
        let output = ensure_output_profile(output_profile)?;
        prepare_session(
            source.clone(),
            position_ms,
            volume,
            speed,
            Arc::clone(shared_level),
            Arc::clone(effects),
            Arc::clone(&loudness),
            Arc::clone(playback_generation),
            latest_generation,
            None,
            Some(Arc::clone(&clock)),
            output,
            Arc::new(AtomicBool::new(false)),
            loopback_tx,
        )
    };
    let prepared = build().or_else(|error| {
        log::warn!(
            target: "cpal-output",
            "session rebuild failed once, retrying: {error}"
        );
        build()
    });
    restore_dsp_output_identity(effects, output_profile);
    match prepared {
        Ok(next) => {
            if !paused {
                if let Err(error) = next.play() {
                    log::warn!(target: "cpal-output", "session rebuild play failed: {error}");
                }
            } else {
                // 暂停态重建：保持静音，不能因为重建就出声
                next.pause();
            }
            *current = Some(next);
        }
        Err(error) => {
            log::warn!(
                target: "cpal-output",
                "session rebuild failed, playback session lost: {error}"
            );
        }
    }
}

pub(super) fn ensure_output_profile(
    output_profile: &mut OutputDeviceState,
) -> Result<&OutputDeviceProfile, String> {
    if output_profile.profile.is_none() {
        let selected = output_profile
            .preferred_name
            .as_deref()
            .map(OutputDeviceProfile::open_named);
        output_profile.profile = Some(match selected {
            Some(Ok(profile)) => profile,
            _ => OutputDeviceProfile::open_default()?,
        });
    }
    output_profile
        .profile
        .as_ref()
        .ok_or_else(|| "No default audio output device".to_string())
}

/// seek 失败/让位后的兜底：回拨时钟并在失败前位置重建旧会话
///
/// 返回 Some(session) 表示已恢复出可播放的会话；None 表示重建也失败
/// （只剩日志与上层错误回复，调用方不得再依赖 current 存在）
#[allow(clippy::too_many_arguments)]
pub(super) fn rebuild_after_failed_seek(
    previous: &PlaybackSession,
    rollback_position_ms: u64,
    paused: bool,
    volume: f32,
    speed: f32,
    shared_level: &Arc<Mutex<SharedAudioLevel>>,
    effects: &Arc<EffectsControl>,
    playback_generation: &Arc<AtomicU64>,
    expected_generation: u64,
    clock: &Arc<PlaybackClock>,
    output_profile: &mut OutputDeviceState,
    loopback_tx: &mpsc::Sender<AudioCmd>,
) -> Option<PlaybackSession> {
    clock.store_ms(rollback_position_ms);
    let restored = (|| {
        let output = ensure_output_profile(output_profile)?;
        prepare_session(
            previous.source.clone(),
            rollback_position_ms,
            volume,
            speed,
            Arc::clone(shared_level),
            Arc::clone(effects),
            Arc::clone(&previous.shared.loudness),
            Arc::clone(playback_generation),
            expected_generation,
            None,
            Some(Arc::clone(clock)),
            output,
            Arc::new(AtomicBool::new(false)),
            loopback_tx,
        )
    })();
    restore_dsp_output_identity(effects, output_profile);
    match restored {
        Ok(restored) => {
            if !paused {
                if let Err(play_error) = restored.play() {
                    log::warn!(
                        target: "cpal-output",
                        "seek rollback play failed: {play_error}"
                    );
                }
            } else {
                // 新会话的 paused 初始为 false 且输出流建好即在跑：
                // 暂停态下必须显式暂停，否则「UI 显示暂停但已出声」
                restored.pause();
            }
            Some(restored)
        }
        Err(restore_error) => {
            log::warn!(
                target: "cpal-output",
                "seek rollback failed: {restore_error}"
            );
            None
        }
    }
}

// 播放/下载编排函数的参数都是相互独立的运行时上下文，聚成结构体只是换个地方堆字段
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_session(
    source: AudioSource,
    start_position_ms: u64,
    volume: f32,
    speed: f32,
    shared_level: Arc<Mutex<SharedAudioLevel>>,
    effects: Arc<EffectsControl>,
    loudness: Arc<TrackLoudness>,
    playback_generation: Arc<AtomicU64>,
    expected_generation: u64,
    operation_generation: Option<GenerationToken>,
    clock: Option<Arc<PlaybackClock>>,
    output: &OutputDeviceProfile,
    prepare_cancel: Arc<AtomicBool>,
    loopback_tx: &mpsc::Sender<AudioCmd>,
) -> Result<PlaybackSession, String> {
    let prepare_started = Instant::now();
    ensure_preparation_current(
        &playback_generation,
        expected_generation,
        operation_generation.as_ref(),
    )?;
    if prepare_cancel.load(Ordering::Acquire) {
        return Err("Timed out waiting for decoded audio".into());
    }
    effects.dsp.select_output(&output.name);
    let decoder_started = Instant::now();
    let session_cancelled = Arc::new(AtomicBool::new(false));
    // 外部 prepare_cancel 挂到读取消链：等待超时后 remote read 可立刻打断
    let read_cancellation = RemoteReadCancellation::new(
        Arc::clone(&session_cancelled),
        operation_generation
            .as_ref()
            .map(|token| (Arc::clone(&token.generation), token.expected)),
    )
    .with_external_cancel(Arc::clone(&prepare_cancel));
    // 留一份句柄：prepare 成功后解除代际守卫（见 disarm_operation_guard 注释）
    let operation_guard = read_cancellation.clone();
    log::info!(
        target: "cpal-output",
        "decoder begin source={}, generation={}, start_ms={}",
        source.label(),
        expected_generation,
        start_position_ms,
    );
    // 必须先 clone/升级 access_mode，再判断 virtual-body：
    // LongFormProgressive 源在 clone 前也要能选中该路径（prefers 已兼容），
    // 但最终以 decoder_source 上的状态为准，避免误走 format.seek。
    let decoder_source = source.decoder_source_for_position(start_position_ms, read_cancellation);
    let use_byte_seek = start_position_ms > 0 && decoder_source.prefers_remote_virtual_body_seek();
    if start_position_ms > 0 {
        log::info!(
            target: "cpal-output",
            "decoder path generation={}, start_ms={}, use_byte_seek={}, prefers_virtual={}",
            expected_generation,
            start_position_ms,
            use_byte_seek,
            decoder_source.prefers_remote_virtual_body_seek(),
        );
    }
    let (mut decoder, segment_start_ms) =
        match make_decoder_with_start(&decoder_source, start_position_ms, use_byte_seek) {
            Ok(opened) => opened,
            Err(error) => {
                session_cancelled.store(true, Ordering::Release);
                return Err(error);
            }
        };
    if prepare_cancel.load(Ordering::Acquire) {
        session_cancelled.store(true, Ordering::Release);
        return Err("Timed out waiting for decoded audio".into());
    }
    let decoder_ms = decoder_started.elapsed().as_millis();
    let duration_ms = decoder
        .total_duration()
        .map(|duration| duration.as_millis() as u64)
        .filter(|duration| *duration > 0)
        .unwrap_or_else(|| source.duration_hint_ms());
    let audio_info = decoder
        .source_audio_info()
        .with_encoded_bitrate(source.encoded_byte_length(), decoder.total_duration());
    let start_position_ms = clamp_position(start_position_ms, duration_ms);
    // 字节跳转路径已经在目标附近顺序打开，不再走 format.seek（它会扫全文件）
    if start_position_ms > 0 && !use_byte_seek {
        decoder
            .try_seek(Duration::from_millis(start_position_ms))
            .map_err(|error| format!("Could not seek decoder: {error}"))?;
    } else if start_position_ms > 0 && use_byte_seek {
        let discarded_ms = segment_start_ms.map_or(0, |segment_start| {
            discard_until_target(decoder.as_mut(), segment_start, start_position_ms)
        });
        log::info!(
            target: "cpal-output",
            "virtual-body open ready generation={}, start_ms={}, segment_start_ms={:?}, discarded_ms={} skip_format_seek=true virtual_body=true",
            expected_generation,
            start_position_ms,
            segment_start_ms,
            discarded_ms,
        );
    }

    let config = &output.config;
    let channels = usize::from(config.channels.max(1));
    let sample_rate = config.sample_rate.0.max(1);
    let capacity_samples = duration_to_frames(PCM_CAPACITY, sample_rate)
        .saturating_mul(channels)
        .max(channels * 2);
    let normalize = effects.snapshot().normalize_volume;
    // 同一首歌重建会话时增益已经落定，不必再预热、再等估计
    let fresh_track = !loudness.is_seeded();
    // 新曲目开着音量均衡：先分析够预热时长再出声，第一个可闻样本就是正确的响度
    let prebuffer = if normalize && fresh_track {
        source.prebuffer_duration().max(NORMALIZE_WARMUP)
    } else {
        source.prebuffer_duration()
    };
    let initial_target = duration_to_frames(prebuffer, sample_rate);
    let clock = clock.unwrap_or_else(|| Arc::new(PlaybackClock::new(start_position_ms)));
    clock
        .position_us
        .store(start_position_ms.saturating_mul(1_000), Ordering::Release);
    let shared = Arc::new(PlaybackShared {
        ring: Arc::new(PcmRing::new(capacity_samples)),
        channels,
        sample_rate,
        paused: AtomicBool::new(false),
        buffering: AtomicBool::new(true),
        finished: AtomicBool::new(false),
        cancelled: Arc::clone(&session_cancelled),
        volume: AtomicF32::new(volume),
        fade_gain: AtomicF32::new(1.0),
        speed: AtomicF32::new(speed),
        buffer_target_frames: AtomicUsize::new(initial_target),
        clock,
        wake_lock: Mutex::new(()),
        wake: Condvar::new(),
        device_lost: AtomicBool::new(false),
        first_frame_ns: Arc::new(AtomicU64::new(0)),
        stretch_buffered: AtomicUsize::new(0),
        effects,
        loudness,
    });
    // 越早开始，起播前等到整首估计的机会越大；音量均衡关着时不花这份 CPU
    if normalize && shared.loudness.begin_scan() {
        spawn_loudness_scan(
            &source,
            &shared.loudness,
            sample_rate,
            channels,
            expected_generation,
        );
    }

    let worker = spawn_decode_worker(
        decoder,
        Arc::clone(&shared),
        shared_level,
        Arc::clone(&playback_generation),
        expected_generation,
        operation_guard.clone(),
    )?;
    let output_started = Instant::now();
    let stream = match open_output_stream(
        output,
        Arc::clone(&shared),
        loopback_tx.clone(),
        expected_generation,
    ) {
        Ok(stream) => stream,
        Err(error) => {
            finish_decode_worker(&source, &shared, worker);
            return Err(error);
        }
    };
    let output_ms = output_started.elapsed().as_millis();
    let ready_started = Instant::now();
    let mut ready_wait_ms = 0;
    let ready = wait_until_ready(
        &shared,
        &playback_generation,
        expected_generation,
        operation_generation.as_ref(),
        &prepare_cancel,
    )
    .and_then(|()| {
        ready_wait_ms = ready_started.elapsed().as_millis();
        if !(normalize && fresh_track) {
            return Ok(());
        }
        wait_for_loudness_estimate(
            &shared.loudness,
            prepare_started + QUICK_ESTIMATE_BUDGET,
            &playback_generation,
            expected_generation,
            operation_generation.as_ref(),
            &prepare_cancel,
        )
    });
    if let Err(error) = ready {
        shared.cancelled.store(true, Ordering::Release);
        shared.wake.notify_all();
        let _ = stream.pause();
        finish_decode_worker(&source, &shared, worker);
        return Err(error);
    }
    let loudness_wait_ms = ready_started.elapsed().as_millis() - ready_wait_ms;

    log::info!(
        target: "cpal-output",
        "prepared {} on {}{}: {} Hz, {} ch, target={}ms, decoder={}ms, output={}ms, wait={}ms, loudness_wait={}ms, total={}ms",
        source.label(),
        output.name,
        if output.exclusive { " (exclusive requested)" } else { "" },
        sample_rate,
        channels,
        prebuffer.as_millis(),
        decoder_ms,
        output_ms,
        ready_wait_ms,
        loudness_wait_ms,
        prepare_started.elapsed().as_millis()
    );
    // 会话已就绪，即将交给调用方提交：解除代际守卫。
    // 此后哪怕用户马上再 seek，本会话也由 previous.stop() 干净收尾，
    // 而不是被瞬间递增的代际把按需读掐死在半路
    operation_guard.disarm_operation_guard();
    Ok(PlaybackSession {
        source,
        stream,
        shared,
        worker: Some(worker),
        duration_ms,
        audio_info,
        playback_generation: expected_generation,
    })
}

pub(super) fn make_decoder_for_position(
    source: &AudioSource,
    start_position_ms: u64,
    use_byte_seek: bool,
) -> Result<Box<dyn AudioDecoder>, String> {
    make_decoder_with_start(source, start_position_ms, use_byte_seek).map(|(decoder, _)| decoder)
}

/// 同 make_decoder_for_position，另带解码实际出声的起点：字节跳转从目标所在分片开头解码，
/// 不是目标本身；为 None 时解码已经停在目标上，或者分片起点不可靠
fn make_decoder_with_start(
    source: &AudioSource,
    start_position_ms: u64,
    use_byte_seek: bool,
) -> Result<(Box<dyn AudioDecoder>, Option<u64>), String> {
    if let AudioSource::Remote(reader, _) = source {
        if use_byte_seek && start_position_ms > 0 {
            let segment_start_ms = reader
                .configure_virtual_body_for_time(start_position_ms)
                .map_err(|error| format!("Could not prepare remote virtual body: {error}"))?;
            let decoder = SymphoniaAudioDecoder::new_remote_virtual(reader.clone(), true)?;
            return Ok((Box::new(decoder), segment_start_ms));
        }
    }
    let decoder = match source {
        AudioSource::Bytes(data, _) => decoder::open_decoder(
            decoder::sniff(&data[..data.len().min(decoder::SNIFF_BYTES)]),
            || SymphoniaAudioDecoder::new(Box::new(Cursor::new(Arc::clone(data))), None),
            || Ok(Box::new(Cursor::new(Arc::clone(data))) as Box<dyn ByteInput>),
        ),
        AudioSource::File(path, _) => decoder::open_decoder(
            sniff_file(Path::new(path)),
            || SymphoniaAudioDecoder::new_file(Path::new(path)),
            || std::fs::File::open(path).map(|file| Box::new(file) as Box<dyn ByteInput>),
        ),
        AudioSource::Growing(reader, _, _) => {
            // 边下边播只嗅探已经到手的部分：开头一次读不会等待后续下载
            let mut header = vec![0u8; decoder::SNIFF_BYTES];
            let available = reader.clone().read(&mut header).unwrap_or(0);
            decoder::open_decoder(
                decoder::sniff(&header[..available]),
                || SymphoniaAudioDecoder::new(Box::new(reader.clone()), None),
                || Ok(Box::new(reader.clone()) as Box<dyn ByteInput>),
            )
        }
        AudioSource::Remote(reader, _) => {
            // 普通远程：demuxer open 可隐藏 seekable；无虚拟 body
            reader.clear_virtual_body();
            decoder::open_decoder(
                reader
                    .header_bytes()
                    .and_then(|header| decoder::sniff(&header)),
                || SymphoniaAudioDecoder::new_remote(reader.clone()),
                || {
                    // 分片 MP4 打开期间暂停了预取（防 symphonia 顺着 moof 链读完整个文件），
                    // FFmpeg 按 sidx 定位不需要这个限制，交出去之前恢复
                    reader.finish_demuxer_open();
                    Ok(Box::new(reader.clone()) as Box<dyn ByteInput>)
                },
            )
        }
    }?;
    Ok((decoder, None))
}

/// 字节跳转落在分片开头：解码并丢掉分片起点到目标之间的音频，让第一个出声的样本就是目标位置，
/// 和设成目标的播放时钟对上。分片约 5 秒，不丢的话时钟（以及歌词）会领先声音最多一个分片
pub(super) fn discard_until_target(
    decoder: &mut dyn AudioDecoder,
    segment_start_ms: u64,
    target_ms: u64,
) -> u64 {
    let lead_ms = target_ms.saturating_sub(segment_start_ms);
    if lead_ms == 0 || lead_ms > MAX_VIRTUAL_BODY_LEAD_MS {
        return 0;
    }
    let samples = lead_ms.saturating_mul(u64::from(decoder.sample_rate())) / 1_000
        * u64::from(decoder.channels().max(1));
    let mut discarded = 0u64;
    while discarded < samples && decoder.next().is_some() {
        discarded += 1;
    }
    lead_ms
}

pub(super) fn spawn_decode_worker(
    source: Box<dyn PcmSource>,
    shared: Arc<PlaybackShared>,
    shared_level: Arc<Mutex<SharedAudioLevel>>,
    playback_generation: Arc<AtomicU64>,
    expected_generation: u64,
    read_cancellation: RemoteReadCancellation,
) -> Result<JoinHandle<()>, String> {
    thread::Builder::new()
        .name("audio-decode".into())
        .spawn(move || {
            let mut converter = FrameResampler::new(source);
            let mut frame = vec![0.0; shared.channels];
            let mut analyzer = AudioAnalyzer::new();
            analyzer.configure(shared.sample_rate, ANALYSIS_FRAME_SIZE);
            let mut analysis = Vec::with_capacity(ANALYSIS_FRAME_SIZE);
            let mut loudness = LoudnessMeter::new(Arc::clone(&shared.loudness));
            let measured_channels = loudness_channels(converter.source_channels(), shared.channels);

            let mut frames_pushed = 0u64;
            let mut exit_reason = "source_eof";
            while !shared.cancelled.load(Ordering::Acquire)
                && playback_generation.load(Ordering::Acquire) == expected_generation
                && !read_cancellation.is_cancelled()
            {
                if shared.ring.writable_samples() < shared.channels {
                    // ring 满了就等：暂停时输出回调不消费，播放时每 10ms 只腾出 10ms 的空间，
                    // 2ms 忙眠一秒唤醒 500 次纯耗电。condvar 有界等待，resume/stop 会 notify 立即唤醒；
                    // ring 有 4 秒容量，这点解码延迟不可能造成欠载
                    let wait = if shared.paused.load(Ordering::Acquire) {
                        DECODE_PAUSED_WAIT
                    } else {
                        DECODE_FULL_WAIT
                    };
                    if let Ok(guard) = shared.wake_lock.lock() {
                        let _ = shared.wake.wait_timeout(guard, wait);
                    } else {
                        thread::sleep(DECODE_IDLE_SLEEP);
                    }
                    continue;
                }
                if !converter.next_frame(shared.sample_rate, &mut frame) {
                    exit_reason = "decoder_exhausted";
                    loudness.flush();
                    break;
                }
                if !shared.ring.try_push_frame(&frame) {
                    thread::yield_now();
                    continue;
                }
                frames_pushed = frames_pushed.saturating_add(1);
                loudness.observe(&frame[..measured_channels]);

                analysis.push(frame[0]);
                if analysis.len() >= ANALYSIS_FRAME_SIZE {
                    let result = analyzer.analyze_frame(&analysis);
                    SharedAudioLevel::try_update(&shared_level, result.level, result.beat_impulse);
                    analysis.clear();
                }
                shared.update_buffering_state();
            }
            if shared.cancelled.load(Ordering::Acquire) {
                exit_reason = "cancelled";
            } else if playback_generation.load(Ordering::Acquire) != expected_generation {
                exit_reason = "generation_changed";
            } else if read_cancellation.is_cancelled() {
                exit_reason = "operation_superseded";
            }
            log::info!(
                target: "cpal-output",
                "decode worker exit reason={}, frames={}, generation={}",
                exit_reason,
                frames_pushed,
                expected_generation,
            );
            shared.finished.store(true, Ordering::Release);
            shared.update_buffering_state();
            shared.wake.notify_all();
        })
        .map_err(|error| format!("Could not start decoder worker: {error}"))
}
