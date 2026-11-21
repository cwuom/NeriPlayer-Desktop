//! 淡入淡出、命令结果等待与 seek 合并

use super::*;

// 播放/下载编排函数的参数都是相互独立的运行时上下文，聚成结构体只是换个地方堆字段
#[allow(clippy::too_many_arguments)]
pub(super) fn crossfade_sessions(
    previous: Option<&PlaybackSession>,
    next: &PlaybackSession,
    fade_out_ms: u32,
    fade_in_ms: u32,
    playback_generation: &AtomicU64,
    expected_generation: u64,
    transition_generation: &AtomicU64,
    expected_transition: u64,
) -> Result<(), String> {
    let duration_ms = fade_out_ms.max(fade_in_ms);
    if duration_ms == 0 {
        next.shared.fade_gain.store(1.0);
        return Ok(());
    }
    let started = Instant::now();
    loop {
        ensure_generation(playback_generation, expected_generation)?;
        ensure_generation(transition_generation, expected_transition)?;
        let elapsed_ms = started.elapsed().as_secs_f32() * 1_000.0;
        if let Some(previous) = previous {
            let progress = if fade_out_ms == 0 {
                1.0
            } else {
                (elapsed_ms / fade_out_ms as f32).min(1.0)
            };
            previous.shared.fade_gain.store(1.0 - progress);
        }
        let progress = if fade_in_ms == 0 {
            1.0
        } else {
            (elapsed_ms / fade_in_ms as f32).min(1.0)
        };
        next.shared.fade_gain.store(progress);
        if elapsed_ms >= duration_ms as f32 {
            break;
        }
        thread::sleep(FADE_STEP);
    }
    next.shared.fade_gain.store(1.0);
    Ok(())
}

// 播放/下载编排函数的参数都是相互独立的运行时上下文，聚成结构体只是换个地方堆字段
#[allow(clippy::too_many_arguments)]
pub(super) fn fade_gain(
    shared: &PlaybackShared,
    from: f32,
    to: f32,
    duration_ms: u32,
    playback_generation: &AtomicU64,
    expected_generation: u64,
    transition_generation: &AtomicU64,
    expected_transition: u64,
) -> Result<(), String> {
    if duration_ms == 0 {
        shared.fade_gain.store(to);
        return Ok(());
    }
    let started = Instant::now();
    loop {
        ensure_generation(playback_generation, expected_generation)?;
        ensure_generation(transition_generation, expected_transition)?;
        let progress = (started.elapsed().as_secs_f32() * 1_000.0 / duration_ms as f32).min(1.0);
        shared.fade_gain.store(from + (to - from) * progress);
        if progress >= 1.0 {
            break;
        }
        thread::sleep(FADE_STEP);
    }
    Ok(())
}

/// 等待淡化命令完成（两段式第二段，必须在释放 `player` 锁后调用）
pub fn receive_fade_result(
    receiver: mpsc::Receiver<Result<(), String>>,
    duration_ms: u32,
) -> AppResult<()> {
    receiver
        .recv_timeout(COMMAND_TIMEOUT + Duration::from_millis(u64::from(duration_ms) + 1_000))
        .map_err(|error| AppError::Audio(format!("Audio thread timeout: {error}")))?
        .map_err(AppError::Audio)
}

pub(super) fn ensure_generation(generation: &AtomicU64, expected: u64) -> Result<(), String> {
    if generation.load(Ordering::Acquire) == expected {
        Ok(())
    } else {
        Err(PLAYBACK_SUPERSEDED.into())
    }
}

pub(super) fn ensure_preparation_current(
    playback_generation: &AtomicU64,
    expected_playback_generation: u64,
    operation_generation: Option<&GenerationToken>,
) -> Result<(), String> {
    ensure_generation(playback_generation, expected_playback_generation)?;
    if operation_generation.is_none_or(GenerationToken::is_current) {
        Ok(())
    } else {
        Err(SEEK_SUPERSEDED.into())
    }
}

pub fn wait_for_seek_result(receiver: mpsc::Receiver<Result<(), String>>) -> AppResult<()> {
    // 超长媒体 seek 可能要等 tail moov + 目标位置数据
    receiver
        .recv_timeout(REMOTE_SEEK_TIMEOUT)
        .map_err(|error| AppError::Audio(format!("Seek command timeout: {error}")))?
        .map_err(AppError::Audio)
}

/// 在锁外等待播放命令的结果
pub fn wait_for_play_result(request: &PlayRequest) -> AppResult<PlaybackStarted> {
    // 远程源用更短超时；超时必须 cancel prepare，否则音频线程卡在 make_decoder 会堵死后续 fallback
    let base = if request.is_remote {
        REMOTE_PREPARE_TIMEOUT
    } else {
        COMMAND_TIMEOUT
    };
    let timeout = base + Duration::from_millis(u64::from(request.fade_ms) + 1_000);
    match request.reply_rx.recv_timeout(timeout) {
        Ok(result) => result.map_err(AppError::Audio),
        Err(_) => {
            request.prepare_cancel.store(true, Ordering::Release);
            // 给音频线程一点时间感知 cancel 并回包，避免泄漏阻塞
            match request.reply_rx.recv_timeout(Duration::from_millis(500)) {
                Ok(result) => result.map_err(AppError::Audio),
                Err(error) => Err(AppError::Audio(format!("Audio thread timeout: {error}"))),
            }
        }
    }
}

pub(super) fn take_latest_seek(
    position_ms: &mut u64,
    playback_generation: &mut u64,
    seek_generation: &mut u64,
    issued_ns: &mut u64,
    reply: &mut mpsc::Sender<Result<(), String>>,
    receiver: &mpsc::Receiver<AudioCmd>,
    deferred: &mut VecDeque<AudioCmd>,
) {
    // 把队列里已有的 Seek 全部折叠成最新一条；非 Seek 命令保序暂存，
    // 不能在第一条非 Seek 处停下——ticker 的 QueryEmpty 会夹在两次
    // 拖动之间，停下就折叠不到真正的最新目标
    loop {
        match receiver.try_recv() {
            Ok(AudioCmd::Seek {
                position_ms: next,
                playback_generation: next_generation,
                seek_generation: next_seek_generation,
                issued_ns: next_issued_ns,
                reply: next_reply,
            }) => {
                let _ = reply.send(Err(SEEK_SUPERSEDED.into()));
                *position_ms = next;
                *playback_generation = next_generation;
                *seek_generation = next_seek_generation;
                *issued_ns = next_issued_ns;
                *reply = next_reply;
            }
            Ok(command) => deferred.push_back(command),
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
        }
    }
}

/// 从命令队列接管到的更新 Seek 载荷
pub(super) struct AdoptedSeek {
    pub(super) position_ms: u64,
    pub(super) playback_generation: u64,
    pub(super) seek_generation: u64,
    pub(super) issued_ns: u64,
    pub(super) reply: mpsc::Sender<Result<(), String>>,
}

/// 旧 seek 被更新 seek 的代际递增打断后，接过队列里那条更新的 Seek 命令
///
/// 代际递增（request_seek）与命令入队之间只有微秒级窗口，因此在宽限内
/// 阻塞等待即可拿到。等待期间收到的非 Seek 命令保序暂存到 deferred、
/// 继续等——ticker 每 200ms 必发 QueryEmpty，第一条非 Seek 就放弃的话
/// 接管几乎必然失败，回滚重建会白跑一次完整 prepare（慢路径下秒级）
pub(super) fn wait_for_newer_seek(
    receiver: &mpsc::Receiver<AudioCmd>,
    deferred: &mut VecDeque<AudioCmd>,
    grace: Duration,
) -> Option<AdoptedSeek> {
    let deadline = Instant::now() + grace;
    loop {
        let remaining = deadline.checked_duration_since(Instant::now())?;
        match receiver.recv_timeout(remaining) {
            Ok(AudioCmd::Seek {
                position_ms,
                playback_generation,
                seek_generation,
                issued_ns,
                reply,
            }) => {
                return Some(AdoptedSeek {
                    position_ms,
                    playback_generation,
                    seek_generation,
                    issued_ns,
                    reply,
                })
            }
            Ok(command) => deferred.push_back(command),
            Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {
                return None
            }
        }
    }
}

pub(super) fn clamp_position(position_ms: u64, duration_ms: u64) -> u64 {
    if duration_ms > 0 {
        position_ms.min(duration_ms)
    } else {
        position_ms
    }
}

pub(super) fn duration_to_frames(duration: Duration, sample_rate: u32) -> usize {
    let frames = duration
        .as_nanos()
        .saturating_mul(u128::from(sample_rate.max(1)))
        .saturating_add(999_999_999)
        / 1_000_000_000;
    usize::try_from(frames).unwrap_or(usize::MAX)
}
