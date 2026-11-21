//! 整首响度扫描与快速估算

use super::*;

pub(super) const LOUDNESS_SCAN_CANCELLED: &str = "loudness scan cancelled";
/// 整首扫描每解出这么多帧检查一次取消
const LOUDNESS_SCAN_CHECK_FRAMES: u64 = 4_096;
/// 快速估计在整首上均匀取这么多个窗口，合计只解码几十分之一的内容
const QUICK_ESTIMATE_WINDOWS: u32 = 12;
const QUICK_ESTIMATE_WINDOW: Duration = Duration::from_secs(1);
/// 新曲目起播时最多等快速估计到这个时刻（从开始准备会话算起）；本地起播目标 300 ms
pub(super) const QUICK_ESTIMATE_BUDGET: Duration = Duration::from_millis(200);

/// 正在整首扫描的本地文件
///
/// 释放文件（删除、改写）前要等这些扫描线程关掉句柄，包括刚切走、还没来得及退出的上一首。
static FILE_SCANS: Mutex<Vec<FileScan>> = Mutex::new(Vec::new());

struct FileScan {
    path: PathBuf,
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

pub(super) fn register_file_scan(path: &str, cancel: Arc<AtomicBool>, handle: JoinHandle<()>) {
    let mut scans = FILE_SCANS.lock().unwrap_or_else(PoisonError::into_inner);
    scans.retain(|scan| !scan.handle.is_finished());
    scans.push(FileScan {
        path: PathBuf::from(path),
        cancel,
        handle,
    });
}

/// 停下这个文件上的整首扫描，返回前扫描线程都已退出
pub(super) fn stop_file_scans(target: &Path) {
    let stopping: Vec<FileScan> = {
        let mut scans = FILE_SCANS.lock().unwrap_or_else(PoisonError::into_inner);
        let (matching, rest) = std::mem::take(&mut *scans)
            .into_iter()
            .partition(|scan| same_local_file(&scan.path, target));
        *scans = rest;
        matching
    };
    for scan in stopping {
        scan.cancel.store(true, Ordering::Release);
        if scan.handle.join().is_err() {
            log::warn!(target: "audio-loudness", "loudness scan thread panicked");
        }
    }
}

/// 能整首扫描响度的源：本地文件、内存音频和边下边播的流
///
/// 按需拉区间的在线流不扫，免得为了统计把整首重新下载一遍。
pub(super) fn loudness_scan_source(
    source: &AudioSource,
    cancel: &Arc<AtomicBool>,
) -> Option<AudioSource> {
    match source {
        AudioSource::Bytes(_, _) | AudioSource::File(_, _) => Some(source.clone()),
        AudioSource::Growing(reader, hint, lifetime) => {
            // 读位置和取消标志独立，与播放共享同一份下载
            let mut reader = reader.clone();
            reader.set_prepare_cancel(Arc::clone(cancel));
            Some(AudioSource::Growing(reader, *hint, Arc::clone(lifetime)))
        }
        AudioSource::Remote(_, _) => None,
    }
}

/// 开着音量均衡时，另开一个解码器把整首的响度算出来
///
/// 播放时的统计只领先 ring 里的几秒，开头几十秒目标增益会随统计收敛漂移好几 dB，
/// 第一次分析到接近满幅的峰时还会被峰值封顶突然压低。能随机读取的本地源先在整首上
/// 抽样做快速估计（起播会稍等它），再完整扫一遍；扫完后整首歌只用一个增益。
/// 测量的帧格式（输出采样率、声道布局）与播放时一致。
pub(super) fn spawn_loudness_scan(
    source: &AudioSource,
    loudness: &Arc<TrackLoudness>,
    output_rate: u32,
    output_channels: usize,
    generation: u64,
) {
    let cancel = loudness.scan_cancel();
    let Some(source) = loudness_scan_source(source, &cancel) else {
        return;
    };
    let (file_path, quick) = match &source {
        AudioSource::File(path, _) => (Some(path.clone()), true),
        AudioSource::Bytes(_, _) => (None, true),
        // 边下边播只能顺着下载读，抽样会卡在还没到的位置上
        AudioSource::Growing(_, _, _) | AudioSource::Remote(_, _) => (None, false),
    };
    loudness.set_estimate_pending(quick);
    let track = Arc::downgrade(loudness);
    let thread_cancel = Arc::clone(&cancel);
    let spawned = thread::Builder::new()
        .name("loudness-scan".into())
        .spawn(move || {
            run_loudness_scan(
                &source,
                output_rate,
                output_channels,
                &thread_cancel,
                &track,
                generation,
                quick,
            )
        });
    match spawned {
        Ok(handle) => {
            if let Some(path) = file_path {
                register_file_scan(&path, cancel, handle);
            }
        }
        Err(error) => {
            loudness.set_estimate_pending(false);
            log::warn!(
                target: "audio-loudness",
                "loudness scan not started generation={generation}, keeping the running estimate: {error}",
            );
        }
    }
}

pub(super) fn run_loudness_scan(
    source: &AudioSource,
    output_rate: u32,
    output_channels: usize,
    cancel: &AtomicBool,
    track: &Weak<TrackLoudness>,
    generation: u64,
    quick: bool,
) {
    let started = Instant::now();
    if quick {
        let estimate = estimate_track_loudness(source, output_rate, output_channels, cancel);
        let elapsed_ms = started.elapsed().as_millis();
        let Some(track) = track.upgrade() else {
            return;
        };
        match estimate {
            Ok(Some(stats)) if !track.estimate(stats) => log::info!(
                target: "audio-loudness",
                "loudness estimate not used generation={generation}: no audible samples or the full scan already finished",
            ),
            Ok(Some(stats)) => {
                log::info!(
                    target: "audio-loudness",
                    "loudness estimate ready source={} generation={} windows={} target_db={:.2} elapsed_ms={}",
                    source.label(),
                    generation,
                    QUICK_ESTIMATE_WINDOWS,
                    stats.target_gain().map_or(f64::NAN, |gain| 20.0 * gain.log10()),
                    elapsed_ms,
                );
            }
            Ok(None) => log::info!(
                target: "audio-loudness",
                "loudness estimate skipped generation={generation}: track too short or not seekable",
            ),
            Err(error) if error == LOUDNESS_SCAN_CANCELLED => {}
            Err(error) => log::warn!(
                target: "audio-loudness",
                "loudness estimate failed generation={generation}, waiting for the full scan: {error}",
            ),
        }
        track.set_estimate_pending(false);
    }
    let result = scan_track_loudness(source, output_rate, output_channels, cancel);
    let elapsed_ms = started.elapsed().as_millis();
    match result {
        Ok((stats, frames)) => {
            let Some(track) = track.upgrade() else {
                log::info!(
                    target: "audio-loudness",
                    "loudness scan finished after the track was released generation={generation} elapsed_ms={elapsed_ms}",
                );
                return;
            };
            track.complete(stats);
            log::info!(
                target: "audio-loudness",
                "loudness scan complete source={} generation={} decoded_s={:.1} rms_dbfs={:.1} peak_dbfs={:.1} target_db={:.2} elapsed_ms={}",
                source.label(),
                generation,
                frames as f64 / f64::from(output_rate.max(1)),
                stats.rms_dbfs(),
                stats.peak_dbfs(),
                stats.target_gain().map_or(f64::NAN, |gain| 20.0 * gain.log10()),
                elapsed_ms,
            );
        }
        Err(error) if error == LOUDNESS_SCAN_CANCELLED => log::info!(
            target: "audio-loudness",
            "loudness scan stopped generation={generation} elapsed_ms={elapsed_ms}",
        ),
        Err(error) => log::warn!(
            target: "audio-loudness",
            "loudness scan failed generation={generation}, keeping the running estimate: {error}",
        ),
    }
}

/// 从头解码整首，按播放时的帧格式统计响度；返回统计和解出的帧数
pub(super) fn scan_track_loudness(
    source: &AudioSource,
    output_rate: u32,
    output_channels: usize,
    cancel: &AtomicBool,
) -> Result<(LoudnessStats, u64), String> {
    let decoder = make_decoder_for_position(source, 0, false)?;
    let mut converter = FrameResampler::new(decoder);
    let channels = output_channels.max(1);
    let measured_channels = loudness_channels(converter.source_channels(), channels);
    let mut frame = vec![0.0; channels];
    let mut scanner = LoudnessScanner::default();
    let mut frames = 0u64;
    while converter.next_frame(output_rate, &mut frame) {
        scanner.observe(&frame[..measured_channels]);
        frames += 1;
        if frames.is_multiple_of(LOUDNESS_SCAN_CHECK_FRAMES) && cancel.load(Ordering::Acquire) {
            return Err(LOUDNESS_SCAN_CANCELLED.into());
        }
    }
    if cancel.load(Ordering::Acquire) {
        return Err(LOUDNESS_SCAN_CANCELLED.into());
    }
    // 下载中断时读取也会走到结尾，那时只统计到半首，宁可不用
    if let AudioSource::Growing(reader, _, _) = source {
        if reader.clone().byte_len().is_none() {
            return Err("stream download did not complete".into());
        }
    }
    Ok((scanner.finish(), frames))
}

/// 在整首上均匀取若干 1 秒窗口统计响度，作为完整扫描之前的估计
///
/// 时长未知、太短或不能 seek 时返回 None：短曲完整扫描本身就很快，抽样反而不准。
pub(super) fn estimate_track_loudness(
    source: &AudioSource,
    output_rate: u32,
    output_channels: usize,
    cancel: &AtomicBool,
) -> Result<Option<LoudnessStats>, String> {
    let decoder = make_decoder_for_position(source, 0, false)?;
    let Some(duration) = decoder.total_duration() else {
        return Ok(None);
    };
    if duration < QUICK_ESTIMATE_WINDOW * QUICK_ESTIMATE_WINDOWS * 2 {
        return Ok(None);
    }
    let mut converter = FrameResampler::new(decoder);
    let channels = output_channels.max(1);
    let measured_channels = loudness_channels(converter.source_channels(), channels);
    let window_frames = duration_to_frames(QUICK_ESTIMATE_WINDOW, output_rate);
    let mut frame = vec![0.0; channels];
    let mut scanner = LoudnessScanner::default();
    for window in 0..QUICK_ESTIMATE_WINDOWS {
        if cancel.load(Ordering::Acquire) {
            return Err(LOUDNESS_SCAN_CANCELLED.into());
        }
        let center =
            duration.mul_f64((f64::from(window) + 0.5) / f64::from(QUICK_ESTIMATE_WINDOWS));
        if converter
            .source_mut()
            .try_seek(center.saturating_sub(QUICK_ESTIMATE_WINDOW / 2))
            .is_err()
        {
            return Ok(None);
        }
        converter.restart();
        for _ in 0..window_frames {
            if !converter.next_frame(output_rate, &mut frame) {
                break;
            }
            scanner.observe(&frame[..measured_channels]);
        }
    }
    Ok(Some(scanner.finish()))
}

/// 新曲目起播前稍等整首的快速估计，第一个可闻样本就用接近整首的增益
///
/// 最多等到 `deadline`；等不到就按播放时的统计起播，估计到了再平滑过去。
pub(super) fn wait_for_loudness_estimate(
    loudness: &TrackLoudness,
    deadline: Instant,
    playback_generation: &AtomicU64,
    expected_generation: u64,
    operation_generation: Option<&GenerationToken>,
    prepare_cancel: &AtomicBool,
) -> Result<(), String> {
    while loudness.estimate_pending() && Instant::now() < deadline {
        ensure_preparation_current(
            playback_generation,
            expected_generation,
            operation_generation,
        )?;
        if prepare_cancel.load(Ordering::Acquire) {
            return Err("Timed out waiting for decoded audio".into());
        }
        thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}
