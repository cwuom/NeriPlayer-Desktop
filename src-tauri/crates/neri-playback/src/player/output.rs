//! cpal 输出流构建与渲染回调

use super::*;

pub(super) fn wait_until_ready(
    shared: &PlaybackShared,
    playback_generation: &AtomicU64,
    expected_generation: u64,
    operation_generation: Option<&GenerationToken>,
    prepare_cancel: &AtomicBool,
) -> Result<(), String> {
    let started = Instant::now();
    let mut guard = shared
        .wake_lock
        .lock()
        .map_err(|_| "Playback ready lock poisoned".to_string())?;
    while shared.buffering.load(Ordering::Acquire) {
        ensure_preparation_current(
            playback_generation,
            expected_generation,
            operation_generation,
        )?;
        if shared.cancelled.load(Ordering::Acquire) || prepare_cancel.load(Ordering::Acquire) {
            return Err(if prepare_cancel.load(Ordering::Acquire) {
                "Timed out waiting for decoded audio".into()
            } else {
                PLAYBACK_SUPERSEDED.into()
            });
        }
        if started.elapsed() >= COMMAND_TIMEOUT {
            return Err("Timed out waiting for decoded audio".into());
        }
        guard = shared
            .wake
            .wait_timeout(guard, READY_POLL)
            .map_err(|_| "Playback ready lock poisoned".to_string())?
            .0;
    }
    ensure_preparation_current(
        playback_generation,
        expected_generation,
        operation_generation,
    )?;
    if prepare_cancel.load(Ordering::Acquire) {
        return Err("Timed out waiting for decoded audio".into());
    }
    Ok(())
}

pub(super) fn build_output_stream(
    device: &cpal::Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    shared: Arc<PlaybackShared>,
    loopback_tx: mpsc::Sender<AudioCmd>,
    playback_generation: u64,
) -> Result<Stream, String> {
    match sample_format {
        SampleFormat::I8 => {
            build_typed_stream::<i8>(device, config, shared, loopback_tx, playback_generation)
        }
        SampleFormat::I16 => {
            build_typed_stream::<i16>(device, config, shared, loopback_tx, playback_generation)
        }
        SampleFormat::I32 => {
            build_typed_stream::<i32>(device, config, shared, loopback_tx, playback_generation)
        }
        SampleFormat::I64 => {
            build_typed_stream::<i64>(device, config, shared, loopback_tx, playback_generation)
        }
        SampleFormat::U8 => {
            build_typed_stream::<u8>(device, config, shared, loopback_tx, playback_generation)
        }
        SampleFormat::U16 => {
            build_typed_stream::<u16>(device, config, shared, loopback_tx, playback_generation)
        }
        SampleFormat::U32 => {
            build_typed_stream::<u32>(device, config, shared, loopback_tx, playback_generation)
        }
        SampleFormat::U64 => {
            build_typed_stream::<u64>(device, config, shared, loopback_tx, playback_generation)
        }
        SampleFormat::F32 => {
            build_typed_stream::<f32>(device, config, shared, loopback_tx, playback_generation)
        }
        SampleFormat::F64 => {
            build_typed_stream::<f64>(device, config, shared, loopback_tx, playback_generation)
        }
        other => Err(format!("Unsupported output sample format: {other}")),
    }
}

fn build_typed_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    shared: Arc<PlaybackShared>,
    loopback_tx: mpsc::Sender<AudioCmd>,
    playback_generation: u64,
) -> Result<Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let callback_shared = Arc::clone(&shared);
    let mut chain = OutputChain::new(&shared);
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                render_output(
                    &callback_shared,
                    output,
                    &mut chain,
                    &metrics::OUTPUT_METRICS,
                );
            },
            move |error| {
                log::error!(target: "cpal-output", "stream error: {error}");
                shared.begin_rebuffering();
                // 输出设备失效（拔耳机/蓝牙断连）恢复：向控制线程上报一次
                // DeviceLost，由它以默认设备原地重建会话。此回调运行在音频
                // 线程，仅做无阻塞 send（std mpsc 无界通道，不会等待）
                if !shared.device_lost.swap(true, Ordering::AcqRel) {
                    let _ = loopback_tx.send(AudioCmd::DeviceLost {
                        playback_generation,
                        session_cancelled: Arc::clone(&shared.cancelled),
                    });
                }
            },
            None,
        )
        .map_err(|error| format!("Could not build audio output: {error}"))
}

/// 输出回调独占的处理状态，建流时一次分配，回调里只复用
pub(super) struct OutputChain {
    stretcher: PitchShifter,
    effects: EffectsProcessor,
    dsp: DspProcessor,
    scratch: Vec<f32>,
    /// 时钟按整微秒推进，余下的零头留到下一次回调，长时间播放不累积偏差
    clock_remainder_us: f64,
}

impl OutputChain {
    pub(super) fn new(shared: &PlaybackShared) -> Self {
        let channels = shared.channels.max(1);
        Self {
            stretcher: PitchShifter::new(
                channels,
                shared.sample_rate,
                shared
                    .effects
                    .snapshot()
                    .effective_pitch(shared.speed.load()),
            ),
            dsp: DspProcessor::new(
                channels,
                shared.sample_rate,
                Arc::clone(&shared.effects.dsp),
            ),
            effects: EffectsProcessor::new(
                channels,
                shared.sample_rate,
                Arc::clone(&shared.effects),
                Arc::clone(&shared.loudness),
                RENDER_CHUNK_FRAMES,
            ),
            scratch: vec![0.0; RENDER_CHUNK_FRAMES * channels],
            clock_remainder_us: 0.0,
        }
    }
}

/// 输出回调主体：经变速器从 ring 取帧，过音效后写入设备缓冲，推进时钟并记录指标
///
/// 运行在设备回调线程：只用原子操作和预分配的状态，不加锁、不分配。
/// 倍速、独立音调和音效参数都在这里实时生效，改它们不需要重建解码会话
/// 播放中途取空 ring 记一次欠载；解码已结束后自然排空不算。
pub(super) fn render_output<T>(
    shared: &PlaybackShared,
    output: &mut [T],
    chain: &mut OutputChain,
    counters: &OutputMetrics,
) where
    T: SizedSample + FromSample<f32>,
{
    let OutputChain {
        stretcher,
        effects,
        dsp,
        scratch,
        clock_remainder_us,
    } = chain;
    let started_ns = metrics::monotonic_ns();
    let silence = T::from_sample(0.0);
    if shared.paused.load(Ordering::Acquire) || shared.buffering.load(Ordering::Acquire) {
        output.fill(silence);
        counters.record_callback(
            0,
            0,
            false,
            metrics::monotonic_ns().saturating_sub(started_ns),
        );
        return;
    }

    let channels = shared.channels.max(1);
    let speed = shared.speed.load();
    let pitch = shared.effects.snapshot().effective_pitch(speed);
    let draining = shared.finished.load(Ordering::Acquire);
    let gain = shared.volume.load() * shared.fade_gain.load();
    let mut pull = |target: &mut [f32]| shared.ring.pop_frames(target, channels);
    let mut rendered_frames = 0usize;
    let mut media_frames = 0.0f64;
    let mut silent_frames = 0usize;
    let chunk_samples = (scratch.len() / channels * channels).max(channels);
    for chunk in output.chunks_mut(chunk_samples) {
        let frames = chunk.len() / channels;
        if silent_frames > 0 {
            chunk.fill(silence);
            silent_frames += frames;
            continue;
        }
        let rendered = stretcher.render(
            &mut scratch[..frames * channels],
            speed,
            pitch,
            draining,
            &mut pull,
        );
        let filled = rendered.frames * channels;
        effects.process(&mut scratch[..filled]);
        dsp.process(&mut scratch[..filled]);
        for (target, sample) in chunk[..filled]
            .iter_mut()
            .zip(scratch[..filled].iter().copied())
        {
            // 音效全关时样本原样直通；NaN 的 clamp 还是 NaN，不能交给设备
            let value = sample * gain;
            *target = T::from_sample(if value.is_finite() {
                value.clamp(-1.0, 1.0)
            } else {
                0.0
            });
        }
        chunk[filled..].fill(silence);
        rendered_frames += rendered.frames;
        media_frames += rendered.media_frames;
        if rendered.frames < frames {
            // 欠载只是数据晚到，同一条流里 ring 的数据始终连续（seek、换歌都会重建流）：
            // 变速器里已经取出的帧和叠加状态照常保留，恢复后接着放
            silent_frames = frames - rendered.frames;
            shared.begin_rebuffering();
        }
    }
    shared
        .stretch_buffered
        .store(stretcher.buffered_frames(), Ordering::Release);
    if rendered_frames > 0 {
        let _ = shared.first_frame_ns.compare_exchange(
            0,
            started_ns.max(1),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
    // 变速退出时的位置校正可能是负的，先并进零头，攒够正的整微秒再推进时钟
    let elapsed_us =
        media_frames * 1_000_000.0 / f64::from(shared.sample_rate) + *clock_remainder_us;
    let whole_us = elapsed_us.floor().max(0.0);
    *clock_remainder_us = elapsed_us - whole_us;
    if whole_us > 0.0 {
        shared
            .clock
            .position_us
            .fetch_add(whole_us as u64, Ordering::AcqRel);
    }
    // 交叉淡化时两个会话都在出声，增益读数只取声音占主导的那个
    let dominant = shared.fade_gain.load() >= 0.5;
    counters.record_effects(
        dominant.then(|| gain_to_millibels(effects.normalization_gain())),
        effects.take_limited_frames(),
    );
    let underrun = silent_frames > 0 && !draining;
    counters.record_callback(
        rendered_frames,
        silent_frames,
        underrun,
        metrics::monotonic_ns().saturating_sub(started_ns),
    );
}
