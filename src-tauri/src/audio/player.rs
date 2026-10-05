use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
use std::collections::VecDeque;
use std::io::Cursor;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::audio::analyzer::{AudioAnalyzer, SharedAudioLevel};
use crate::audio::buffered::PcmRing;
use crate::audio::effects::{AudioEffectsParams, EqualizerSource, LoudnessSource};
use crate::audio::growing::GrowingAudioReader;
use crate::audio::pcm::PcmSource;
use crate::audio::remote::{
    RemoteAudioSource, RemoteReadCancellation, SymphoniaAudioDecoder,
};
use crate::error::{AppError, AppResult};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
// 远程/超长媒体首帧准备超时：超时必须取消读，否则音频线程卡死会堵住后续 fallback
const REMOTE_PREPARE_TIMEOUT: Duration = Duration::from_secs(12);
// 超长远程 seek 重建 demuxer 可能需要拉尾部 moov，单独放宽
const REMOTE_SEEK_TIMEOUT: Duration = Duration::from_secs(45);
const LOCAL_PREBUFFER: Duration = Duration::from_millis(80);
const GROWING_PREBUFFER: Duration = Duration::from_millis(800);
const REMOTE_PREBUFFER: Duration = Duration::from_millis(400);
const REBUFFER_TARGET: Duration = Duration::from_millis(1_500);
// 音效/倍速切换后的短恢复目标：避免 underrun 后卡 1.5s
const EFFECTS_RECOVER_TARGET: Duration = Duration::from_millis(120);
const PCM_CAPACITY: Duration = Duration::from_secs(4);
const DECODE_IDLE_SLEEP: Duration = Duration::from_millis(2);
const READY_POLL: Duration = Duration::from_millis(20);
const FADE_STEP: Duration = Duration::from_millis(10);
const ANALYSIS_FRAME_SIZE: usize = 2_048;
const PLAYBACK_SUPERSEDED: &str = "Playback request superseded";
const SEEK_SUPERSEDED: &str = "Seek request superseded";
// 旧 seek 被更新 seek 的代际递增打断时，等待新 Seek 命令入队的宽限：
// request_seek 里递增代际与发送命令之间只隔几条语句，正常几微秒内可达；
// 宽限只兜发送线程被调度延迟的极端情况，超时则回滚旧位置，绝不悬空
const SEEK_ADOPT_GRACE: Duration = Duration::from_millis(200);
const OUTPUT_DEVICE_POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone)]
struct GenerationToken {
    generation: Arc<AtomicU64>,
    expected: u64,
}

impl GenerationToken {
    fn is_current(&self) -> bool {
        self.generation.load(Ordering::Acquire) == self.expected
    }
}

struct GrowingSourceLifetime(GrowingAudioReader);

impl Drop for GrowingSourceLifetime {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Clone)]
enum AudioSource {
    Bytes(Arc<[u8]>, u64),
    File(String, u64),
    Growing(GrowingAudioReader, u64, Arc<GrowingSourceLifetime>),
    Remote(RemoteAudioSource, u64),
}

impl AudioSource {
    fn growing(reader: GrowingAudioReader, hint: u64) -> Self {
        // seek 重建会共享下载缓冲，只有最后一个源释放时才停止生产者
        let lifetime = Arc::new(GrowingSourceLifetime(reader.clone()));
        Self::Growing(reader, hint, lifetime)
    }

    fn duration_hint_ms(&self) -> u64 {
        match self {
            Self::Bytes(_, hint)
            | Self::File(_, hint)
            | Self::Growing(_, hint, _)
            | Self::Remote(_, hint) => *hint,
        }
    }

    fn prebuffer_duration(&self) -> Duration {
        match self {
            Self::Growing(_, _, _) => GROWING_PREBUFFER,
            Self::Remote(_, _) => REMOTE_PREBUFFER,
            Self::Bytes(_, _) | Self::File(_, _) => LOCAL_PREBUFFER,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Bytes(_, _) => "bytes",
            Self::File(_, _) => "file",
            Self::Growing(_, _, _) => "growing",
            Self::Remote(_, _) => "remote",
        }
    }

    fn decoder_source_for_position(
        &self,
        position_ms: u64,
        read_cancellation: RemoteReadCancellation,
    ) -> Self {
        match self {
            Self::Growing(reader, hint, lifetime) => {
                let mut reader = reader.clone();
                reader.set_read_cancellation(read_cancellation);
                Self::Growing(reader, *hint, Arc::clone(lifetime))
            }
            Self::Remote(reader, hint) => {
                let reader = if position_ms == 0 {
                    reader.clone()
                } else {
                    reader.seekable_clone()
                };
                Self::Remote(reader.with_read_cancellation(read_cancellation), *hint)
            }
            _ => self.clone(),
        }
    }

    fn prefers_remote_virtual_body_seek(&self) -> bool {
        matches!(self, Self::Remote(reader, _) if reader.prefers_virtual_body_seek())
    }
}

#[derive(Clone, Copy)]
enum PlayTransition {
    Replace,
    Crossfade { fade_out_ms: u32, fade_in_ms: u32 },
}

enum AudioCmd {
    Play {
        source: AudioSource,
        start_position_ms: u64,
        transition: PlayTransition,
        playback_generation: u64,
        transition_generation: u64,
        /// 外部等待超时后置位，打断 remote make_decoder / prebuffer
        prepare_cancel: Arc<AtomicBool>,
        reply: mpsc::Sender<Result<PlaybackStarted, String>>,
    },
    Pause,
    Resume,
    Stop,
    SetVolume(f32),
    SetOutputDevice {
        name: Option<String>,
        cancel: Arc<AtomicBool>,
        reply: mpsc::Sender<Result<(), String>>,
    },
    SetSpeed(f32),
    Seek {
        position_ms: u64,
        playback_generation: u64,
        seek_generation: u64,
        reply: mpsc::Sender<Result<(), String>>,
    },
    QueryEmpty { reply: mpsc::Sender<bool> },
    /// 丢掉 ring 中过期的已处理样本，使 EQ/响度/倍速近似实时
    InvalidateProcessedBuffer,
    FadeOutPause {
        duration_ms: u32,
        transition_generation: u64,
        reply: mpsc::Sender<Result<(), String>>,
    },
    FadeInResume {
        duration_ms: u32,
        transition_generation: u64,
        reply: mpsc::Sender<Result<(), String>>,
    },
    /// 输出设备失效（拔掉耳机/蓝牙断连等，由 cpal 流错误回调上报）。
    /// 控制线程收到后丢弃缓存的设备档案并以当前默认设备原地重建会话
    DeviceLost {
        playback_generation: u64,
        session_cancelled: Arc<AtomicBool>,
    },
}


#[derive(Clone)]
pub struct PlaybackStarted {
    pub duration_ms: u64,
    pub clock: Arc<PlaybackClock>,
}

pub struct PlaybackClock {
    position_us: AtomicU64,
}

impl PlaybackClock {
    fn new(position_ms: u64) -> Self {
        Self {
            position_us: AtomicU64::new(position_ms.saturating_mul(1_000)),
        }
    }

    fn position_ms(&self) -> u64 {
        self.position_us.load(Ordering::Acquire) / 1_000
    }

    fn store_ms(&self, position_ms: u64) {
        self.position_us
            .store(position_ms.saturating_mul(1_000), Ordering::Release);
    }
}

struct AtomicF32(AtomicU32);

impl AtomicF32 {
    fn new(value: f32) -> Self {
        Self(AtomicU32::new(value.to_bits()))
    }

    fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Acquire))
    }

    fn store(&self, value: f32) {
        self.0.store(value.to_bits(), Ordering::Release);
    }
}

struct PlaybackShared {
    ring: Arc<PcmRing>,
    channels: usize,
    sample_rate: u32,
    paused: AtomicBool,
    buffering: AtomicBool,
    finished: AtomicBool,
    cancelled: Arc<AtomicBool>,
    volume: AtomicF32,
    fade_gain: AtomicF32,
    speed: AtomicF32,
    buffer_target_frames: AtomicUsize,
    clock: Arc<PlaybackClock>,
    wake_lock: Mutex<()>,
    wake: Condvar,
    /// 设备失效只上报一次：cpal 错误回调可能连续触发多次
    device_lost: AtomicBool,
}

impl PlaybackShared {
    fn buffered_frames(&self) -> usize {
        self.ring.readable_samples() / self.channels.max(1)
    }

    fn rebuffer_frames(&self) -> usize {
        duration_to_frames(REBUFFER_TARGET, self.sample_rate)
    }

    fn soft_recover_frames(&self) -> usize {
        duration_to_frames(EFFECTS_RECOVER_TARGET, self.sample_rate).max(1)
    }

    fn update_buffering_state(&self) {
        if !self.buffering.load(Ordering::Acquire) {
            return;
        }
        let target = self.buffer_target_frames.load(Ordering::Acquire);
        if self.buffered_frames() >= target || self.finished.load(Ordering::Acquire) {
            self.buffering.store(false, Ordering::Release);
            // 软恢复结束后恢复正常 underrun 目标，避免后续网络卡顿只缓冲 120ms
            let soft = self.soft_recover_frames();
            if target > 0 && target <= soft.saturating_mul(3) {
                self.buffer_target_frames
                    .store(self.rebuffer_frames(), Ordering::Release);
            }
            self.wake.notify_all();
        }
    }

    fn begin_rebuffering(&self) {
        if self.finished.load(Ordering::Acquire) {
            return;
        }
        // 音效/倍速刚切过（目标已是软恢复量级）时，underrun 不要拉回 1.5s
        let soft = self.soft_recover_frames();
        let current = self.buffer_target_frames.load(Ordering::Acquire);
        let target = if current > 0 && current <= soft.saturating_mul(3) {
            soft
        } else {
            self.rebuffer_frames()
        };
        self.buffer_target_frames
            .store(target, Ordering::Release);
        self.buffering.store(true, Ordering::Release);
        self.wake.notify_all();
    }

}

struct PlaybackSession {
    source: AudioSource,
    stream: Stream,
    shared: Arc<PlaybackShared>,
    worker: Option<JoinHandle<()>>,
    duration_ms: u64,
    playback_generation: u64,
}

struct OutputDeviceProfile {
    device: Device,
    config: StreamConfig,
    sample_format: SampleFormat,
    name: String,
}

impl OutputDeviceProfile {
    fn open_default() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "No default audio output device".to_string())?;
        Self::from_device(device)
    }

    fn open_named(name: &str) -> Result<Self, String> {
        let device = cpal::default_host()
            .output_devices()
            .map_err(|error| format!("Could not list audio output devices: {error}"))?
            .find(|device| device.name().is_ok_and(|candidate| candidate == name))
            .ok_or_else(|| "Selected audio output device is unavailable".to_string())?;
        Self::from_device(device)
    }

    fn from_device(device: Device) -> Result<Self, String> {
        let name = device
            .name()
            .unwrap_or_else(|_| "default output".to_string());
        let supported = device
            .default_output_config()
            .map_err(|error| format!("Could not query output format: {error}"))?;
        let sample_format = supported.sample_format();
        let config = supported.into();
        Ok(Self {
            device,
            config,
            sample_format,
            name,
        })
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioOutputDevice {
    pub name: String,
    pub is_default: bool,
}

pub struct OutputDeviceChangeRequest {
    receiver: mpsc::Receiver<Result<(), String>>,
    cancel: Arc<AtomicBool>,
    completed: bool,
}

impl OutputDeviceChangeRequest {
    pub fn wait(mut self) -> AppResult<()> {
        let result = self.receiver.recv_timeout(COMMAND_TIMEOUT);
        self.completed = result.is_ok();
        result.map_err(|error| AppError::Audio(error.to_string()))?.map_err(AppError::Audio)
    }
}

impl Drop for OutputDeviceChangeRequest {
    fn drop(&mut self) {
        if !self.completed {
            self.cancel.store(true, Ordering::Release);
        }
    }
}

pub fn list_audio_output_devices() -> AppResult<Vec<AudioOutputDevice>> {
    let host = cpal::default_host();
    let default_name = host.default_output_device().and_then(|device| device.name().ok());
    let devices = host.output_devices().map_err(|error| AppError::Audio(error.to_string()))?;
    let mut result = Vec::new();
    for device in devices {
        let Ok(name) = device.name() else { continue };
        if !result.iter().any(|item: &AudioOutputDevice| item.name == name) {
            result.push(AudioOutputDevice {
                is_default: default_name.as_deref() == Some(name.as_str()),
                name,
            });
        }
    }
    Ok(result)
}

struct OutputDeviceState {
    preferred_name: Option<String>,
    profile: Option<OutputDeviceProfile>,
}

fn effective_output_name<'a>(
    preferred: Option<&'a str>,
    available: &[&str],
    default: Option<&'a str>,
) -> Option<&'a str> {
    preferred.filter(|name| available.contains(name)).or(default)
}

fn matches_output_session(
    current_generation: u64,
    current_cancel: &Arc<AtomicBool>,
    reported_generation: u64,
    reported_cancel: &Arc<AtomicBool>,
) -> bool {
    current_generation == reported_generation && Arc::ptr_eq(current_cancel, reported_cancel)
}

/// 全局收尸线程：接管 stop 时尚未退出的解码 worker 句柄，在后台 join。
/// stop 的调用者（音频控制线程/命令路径）绝不能被 join 阻塞，
/// 而直接丢弃句柄又会让阻塞中的孤儿线程无人回收——移交给专用线程两全
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

impl PlaybackSession {
    fn play(&self) -> Result<(), String> {
        self.shared.paused.store(false, Ordering::Release);
        // 唤醒可能在暂停态长睡的解码线程（见 spawn_decode_worker 的暂停等待）
        self.shared.wake.notify_all();
        self.stream
            .play()
            .map_err(|error| format!("Could not start audio output: {error}"))
    }

    fn pause(&self) {
        self.shared.paused.store(true, Ordering::Release);
        if let Err(error) = self.stream.pause() {
            log::warn!(target: "cpal-output", "pause failed: {error}");
        }
    }

    fn stop(&mut self) {
        self.shared.cancelled.store(true, Ordering::Release);
        self.shared.wake.notify_all();
        let _ = self.stream.pause();
        if let Some(worker) = self.worker.take() {
            if worker.is_finished() {
                let _ = worker.join();
            } else {
                // worker 尚未退出（可能阻塞在网络读上）：交给全局收尸线程
                // 后台 join，既不丢句柄也不阻塞 stop 的调用者
                reap_worker_handle(worker);
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.shared.finished.load(Ordering::Acquire)
            && self.shared.ring.readable_samples() < self.shared.channels
    }
}

impl Drop for PlaybackSession {
    fn drop(&mut self) {
        self.stop();
    }
}

struct FrameResampler {
    source: Box<dyn PcmSource>,
    source_rate: u32,
    current: Vec<f32>,
    next: Vec<f32>,
    phase: f64,
    initialized: bool,
    source_ended: bool,
    finished: bool,
}

impl FrameResampler {
    fn new(source: Box<dyn PcmSource>) -> Self {
        let source_channels = usize::from(source.channels().max(1));
        let source_rate = source.sample_rate().max(1);
        Self {
            source,
            source_rate,
            current: vec![0.0; source_channels],
            next: vec![0.0; source_channels],
            phase: 0.0,
            initialized: false,
            source_ended: false,
            finished: false,
        }
    }

    fn read_source_frame(source: &mut dyn PcmSource, frame: &mut [f32]) -> bool {
        for sample in frame {
            let Some(value) = source.next() else {
                return false;
            };
            *sample = value;
        }
        true
    }

    fn initialize(&mut self) -> bool {
        if self.initialized {
            return !self.finished;
        }
        if !Self::read_source_frame(self.source.as_mut(), &mut self.current) {
            self.finished = true;
            return false;
        }
        if !Self::read_source_frame(self.source.as_mut(), &mut self.next) {
            self.next.copy_from_slice(&self.current);
            self.source_ended = true;
        }
        self.initialized = true;
        true
    }

    fn next_frame(&mut self, output_rate: u32, speed: f32, output: &mut [f32]) -> bool {
        if self.finished || !self.initialize() {
            return false;
        }

        let phase = self.phase as f32;
        let output_channels = output.len();
        for (channel, sample) in output.iter_mut().enumerate() {
            let current = channel_sample(&self.current, channel, output_channels);
            let next = channel_sample(&self.next, channel, output_channels);
            *sample = current + (next - current) * phase;
        }

        self.phase += f64::from(self.source_rate) * f64::from(speed.clamp(0.25, 3.0))
            / f64::from(output_rate.max(1));
        while self.phase >= 1.0 {
            self.phase -= 1.0;
            if self.source_ended {
                self.finished = true;
                break;
            }
            self.current.copy_from_slice(&self.next);
            if !Self::read_source_frame(self.source.as_mut(), &mut self.next) {
                self.next.copy_from_slice(&self.current);
                self.source_ended = true;
            }
        }
        true
    }
}

fn channel_sample(frame: &[f32], output_channel: usize, output_channels: usize) -> f32 {
    match (frame.len(), output_channels) {
        (1, _) => frame[0],
        (_, 1) => frame.iter().copied().sum::<f32>() / frame.len() as f32,
        _ => frame.get(output_channel).copied().unwrap_or(0.0),
    }
}

/// 播放请求的中间状态，持有 reply 接收端，可在锁外等待
pub struct PlayRequest {
    reply_rx: mpsc::Receiver<Result<PlaybackStarted, String>>,
    pub expected_generation: u64,
    pub current_path: String,
    fade_ms: u32,
    /// 与音频线程 prepare 共享；等待超时后置位以释放音频线程
    prepare_cancel: Arc<AtomicBool>,
    is_remote: bool,
}

pub struct PlayerEngine {
    cmd_tx: mpsc::Sender<AudioCmd>,
    thread_alive: Arc<AtomicBool>,
    pub is_playing: bool,
    pub volume: f32,
    pub speed: f32,
    pub current_path: Option<String>,
    pub duration_ms: u64,
    pub shared_audio_level: Arc<Mutex<SharedAudioLevel>>,
    pub effects_params: Arc<Mutex<AudioEffectsParams>>,
    playback_generation: Arc<AtomicU64>,
    seek_generation: Arc<AtomicU64>,
    transition_generation: Arc<AtomicU64>,
    loaded_generation: Option<u64>,
    clock: Option<Arc<PlaybackClock>>,
}

impl Default for PlayerEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerEngine {
    pub fn new() -> Self {
        Self::with_playback_generation(Arc::new(AtomicU64::new(0)))
    }

    pub fn with_playback_generation(playback_generation: Arc<AtomicU64>) -> Self {
        let shared_audio_level = SharedAudioLevel::new();
        let effects_params = AudioEffectsParams::new_shared();
        let seek_generation = Arc::new(AtomicU64::new(0));
        let transition_generation = Arc::new(AtomicU64::new(0));
        let (cmd_tx, thread_alive) = spawn_audio_thread(
            Arc::clone(&shared_audio_level),
            Arc::clone(&effects_params),
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
            effects_params,
            playback_generation,
            seek_generation,
            transition_generation,
            loaded_generation: None,
            clock: None,
        }
    }

    fn ensure_alive(&mut self) {
        if self.thread_alive.load(Ordering::Acquire) {
            return;
        }
        log::warn!(target: "cpal-output", "audio thread stopped, restarting");
        let (cmd_tx, thread_alive) = spawn_audio_thread(
            Arc::clone(&self.shared_audio_level),
            Arc::clone(&self.effects_params),
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
        let transition_generation =
            self.transition_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let prepare_cancel = Arc::new(AtomicBool::new(false));
        let is_remote = matches!(source, AudioSource::Remote(_, _) | AudioSource::Growing(_, _, _));
        let (reply_tx, reply_rx) = mpsc::channel();
        self.cmd_tx
            .send(AudioCmd::Play {
                source,
                start_position_ms,
                transition,
                playback_generation: expected_generation,
                transition_generation,
                prepare_cancel: Arc::clone(&prepare_cancel),
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
    pub fn complete_start(&mut self, result: PlaybackStarted, expected_generation: u64, current_path: String) -> AppResult<u64> {
        self.ensure_request_current(expected_generation)?;
        self.is_playing = true;
        self.current_path = Some(current_path);
        self.duration_ms = result.duration_ms;
        self.loaded_generation = Some(expected_generation);
        self.clock = Some(result.clock);
        Ok(result.duration_ms)
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
            source, start_position_ms, transition, expected_generation, current_path,
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
        let pos = self.clock.as_ref().map(|clock| clock.position_ms()).unwrap_or(0);
        // duration 未知(=0)时不钳制：否则位置被永久钉在 1ms，进度条冻结、
        // near_end / begin_finished_query 永不满足导致曲目播完不推进队列（PB-04）
        if self.duration_ms > 0 {
            pos.min(self.duration_ms)
        } else {
            pos
        }
    }

    pub fn pause(&mut self) {
        if self.is_playing {
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
        self.is_playing = false;
        self.current_path = None;
        self.duration_ms = 0;
        self.loaded_generation = None;
        self.clock = None;
        SharedAudioLevel::reset(&self.shared_audio_level);
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        let _ = self.cmd_tx.send(AudioCmd::SetVolume(self.volume));
    }

    pub fn request_output_device(
        &self,
        name: Option<String>,
    ) -> AppResult<OutputDeviceChangeRequest> {
        let (reply, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cmd_tx.send(AudioCmd::SetOutputDevice { name, cancel: Arc::clone(&cancel), reply })
            .map_err(|error| AppError::Audio(error.to_string()))?;
        Ok(OutputDeviceChangeRequest { receiver, cancel, completed: false })
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.clamp(0.25, 3.0);
        // SetSpeed 内部按旧速丢 ring 并补时钟，无需再发 Invalidate
        let _ = self.cmd_tx.send(AudioCmd::SetSpeed(self.speed));
    }

    pub fn set_loudness_gain(&self, millibels: i32) {
        if let Ok(mut params) = self.effects_params.lock() {
            params.loudness_gain_mb = millibels.clamp(0, 1_500);
        }
        let _ = self.cmd_tx.send(AudioCmd::InvalidateProcessedBuffer);
    }

    pub fn set_normalize_volume(&self, enabled: bool) {
        if let Ok(mut params) = self.effects_params.lock() {
            params.normalize_volume = enabled;
        }
        let _ = self.cmd_tx.send(AudioCmd::InvalidateProcessedBuffer);
    }

    pub fn set_equalizer(&self, enabled: bool, bands: &[i32]) {
        if let Ok(mut params) = self.effects_params.lock() {
            params.eq_enabled = enabled;
            for (index, value) in bands.iter().copied().enumerate().take(5) {
                params.eq_band_levels_mb[index] = value.clamp(-1_500, 1_500);
            }
        }
        let _ = self.cmd_tx.send(AudioCmd::InvalidateProcessedBuffer);
    }

    pub fn reset_effects(&self) {
        if let Ok(mut params) = self.effects_params.lock() {
            params.reset();
        }
        let _ = self.cmd_tx.send(AudioCmd::InvalidateProcessedBuffer);
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
        let position_ms = clamp_position(position_ms, self.duration_ms);
        self.transition_generation.fetch_add(1, Ordering::AcqRel);
        let seek_generation = self.seek_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let (reply_tx, reply_rx) = mpsc::channel();
        self.cmd_tx
            .send(AudioCmd::Seek {
                position_ms,
                playback_generation: request_generation,
                seek_generation,
                reply: reply_tx,
            })
            .map_err(|_| AppError::Audio("Audio thread disconnected".into()))?;
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

    pub fn mark_ended(&mut self) {
        self.is_playing = false;
    }

    /// 两段式淡出暂停第一段：仅发送命令并返回结果接收端，不阻塞等待。
    /// 调用方应在释放 `player` 锁后用 [`receive_fade_result`] 等待完成
    pub fn request_pause_with_fade(
        &mut self,
        duration_ms: u32,
    ) -> AppResult<mpsc::Receiver<Result<(), String>>> {
        self.ensure_alive();
        let transition_generation =
            self.transition_generation.fetch_add(1, Ordering::AcqRel) + 1;
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
        let transition_generation =
            self.transition_generation.fetch_add(1, Ordering::AcqRel) + 1;
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
            PlayTransition::Crossfade { fade_out_ms, fade_in_ms },
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
            PlayTransition::Crossfade { fade_out_ms, fade_in_ms },
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
            PlayTransition::Crossfade { fade_out_ms, fade_in_ms },
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
            PlayTransition::Crossfade { fade_out_ms, fade_in_ms },
            generation,
            "__remote__".into(),
        )
    }
}

fn spawn_audio_thread(
    shared_level: Arc<Mutex<SharedAudioLevel>>,
    effects_params: Arc<Mutex<AudioEffectsParams>>,
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
                    effects_params,
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
    effects_params: Arc<Mutex<AudioEffectsParams>>,
    playback_generation: Arc<AtomicU64>,
    seek_generation: Arc<AtomicU64>,
    transition_generation: Arc<AtomicU64>,
) {
    let mut current: Option<PlaybackSession> = None;
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

    let mut output_profile = OutputDeviceState { preferred_name: None, profile };
    let mut last_output_poll = Instant::now();
    loop {
        let command = match deferred.pop_front() {
            Some(command) => Some(command),
            None => match receiver.recv_timeout(OUTPUT_DEVICE_POLL_INTERVAL) {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            },
        };
        if last_output_poll.elapsed() >= OUTPUT_DEVICE_POLL_INTERVAL {
            last_output_poll = Instant::now();
            if let Ok(devices) = list_audio_output_devices() {
                let available: Vec<&str> = devices.iter().map(|device| device.name.as_str()).collect();
                let default = devices.iter().find(|device| device.is_default).map(|device| device.name.as_str());
                let target = effective_output_name(output_profile.preferred_name.as_deref(), &available, default).map(str::to_string);
                if let Some(name) = target {
                    if output_profile.profile.as_ref().is_none_or(|profile| profile.name != name) {
                        let result = OutputDeviceProfile::open_named(&name).and_then(|candidate| {
                            switch_output_device_in_place(
                                &mut current, candidate, &mut output_profile, volume, speed,
                                &shared_level, &effects_params, &playback_generation, &loopback_tx,
                                Arc::new(AtomicBool::new(false)),
                            )
                        });
                        if let Err(error) = result {
                            log::warn!(target: "cpal-output", "output device change deferred: {error}");
                        }
                    }
                }
            }
        }
        let Some(command) = command else { continue };
        match command {
            AudioCmd::Play {
                source,
                start_position_ms,
                transition,
                playback_generation: expected,
                transition_generation: expected_transition,
                prepare_cancel,
                reply,
            } => {
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
                        Arc::clone(&effects_params),
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
                    .and_then(|_| {
                        ensure_generation(&transition_generation, expected_transition)
                    })
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
                };
                let mut previous = current.take();
                current = Some(next);
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
            AudioCmd::SetVolume(next_volume) => {
                volume = next_volume.clamp(0.0, 1.0);
                if let Some(session) = &current {
                    session.shared.volume.store(volume);
                }
            }
            AudioCmd::SetOutputDevice { name, cancel, reply } => {
                if cancel.load(Ordering::Acquire) {
                    let _ = reply.send(Err("Audio output change cancelled".into()));
                    continue;
                }
                let name = name.map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
                let result = match name.as_deref() {
                    Some(name) => OutputDeviceProfile::open_named(name),
                    None => OutputDeviceProfile::open_default(),
                }.and_then(|candidate| {
                    if cancel.load(Ordering::Acquire) {
                        return Err("Audio output change cancelled".into());
                    }
                    if output_profile.profile.as_ref().is_some_and(|profile| profile.name == candidate.name) {
                        Ok(())
                    } else {
                        switch_output_device_in_place(
                            &mut current, candidate, &mut output_profile, volume, speed,
                            &shared_level, &effects_params, &playback_generation, &loopback_tx,
                            cancel,
                        )
                    }
                });
                if result.is_ok() {
                    output_profile.preferred_name = name;
                }
                let _ = reply.send(result);
            }
            AudioCmd::SetSpeed(next_speed) => {
                speed = next_speed.clamp(0.25, 3.0);
                if let Some(session) = &current {
                    session.shared.speed.store(speed);
                }
                // 速度固化在 ring 的输出帧里，必须从当前位置重解码；
                // 旧做法「丢缓冲 + 按旧速补时钟」会把缓冲住的内容整段跳过
                rebuild_session_in_place(
                    &mut current,
                    volume,
                    speed,
                    &shared_level,
                    &effects_params,
                    &playback_generation,
                    &mut output_profile,
                    &loopback_tx,
                );
            }
            AudioCmd::Seek {
                position_ms,
                playback_generation: expected,
                seek_generation: expected_seek,
                reply,
            } => {
                let mut latest = position_ms;
                let mut latest_generation = expected;
                let mut latest_seek_generation = expected_seek;
                let mut latest_reply = reply;
                take_latest_seek(
                    &mut latest,
                    &mut latest_generation,
                    &mut latest_seek_generation,
                    &mut latest_reply,
                    &receiver,
                    &mut deferred,
                );
                let (
                    source,
                    paused,
                    rollback_position_ms,
                    clock,
                ) = {
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
                            &effects_params,
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
                            Arc::clone(&effects_params),
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
                    match prepared {
                        Ok(next) => {
                            if ensure_generation(&playback_generation, latest_generation)
                                .is_err()
                            {
                                let _ =
                                    latest_reply.send(Err(PLAYBACK_SUPERSEDED.into()));
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
                                        &effects_params,
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
                            current = Some(next);
                            let _ = latest_reply.send(Ok(()));
                            break;
                        }
                        Err(error) => {
                            if ensure_generation(&playback_generation, latest_generation)
                                .is_err()
                            {
                                log::warn!(target: "cpal-output", "seek failed: {error}");
                                let _ =
                                    latest_reply.send(Err(PLAYBACK_SUPERSEDED.into()));
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
                                &effects_params,
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
            AudioCmd::InvalidateProcessedBuffer => {
                // EQ/响度已改 params。ring 里最多缓着 4 秒按旧参数处理完的
                // 输出帧，旧做法「丢弃 + 前拨时钟」等于把这段内容直接跳过——
                // 用户拨一下均衡器歌就快进了。改为从当前位置重建，内容不丢，
                // 新参数立即可闻。
                rebuild_session_in_place(
                    &mut current,
                    volume,
                    speed,
                    &shared_level,
                    &effects_params,
                    &playback_generation,
                    &mut output_profile,
                    &loopback_tx,
                );
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
                let matches_current = current
                    .as_ref()
                    .is_some_and(|session| matches_output_session(
                        session.playback_generation, &session.shared.cancelled,
                        lost_generation, &session_cancelled,
                    ));
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
                    &effects_params,
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

#[allow(clippy::too_many_arguments)]
fn switch_output_device_in_place(
    current: &mut Option<PlaybackSession>,
    candidate: OutputDeviceProfile,
    output: &mut OutputDeviceState,
    volume: f32,
    speed: f32,
    shared_level: &Arc<Mutex<SharedAudioLevel>>,
    effects_params: &Arc<Mutex<AudioEffectsParams>>,
    playback_generation: &Arc<AtomicU64>,
    loopback_tx: &mpsc::Sender<AudioCmd>,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Audio output change cancelled".into());
    }
    let Some(previous) = current.as_ref() else {
        output.profile = Some(candidate);
        return Ok(());
    };
    ensure_generation(playback_generation, previous.playback_generation)?;
    let paused = previous.shared.paused.load(Ordering::Acquire);
    previous.pause();
    // 保留旧流和源，目标设备准备失败时还能回到原位置
    let prepared = prepare_session(
        previous.source.clone(), previous.shared.clock.position_ms(), volume, speed,
        Arc::clone(shared_level), Arc::clone(effects_params), Arc::clone(playback_generation),
        previous.playback_generation, None, Some(Arc::clone(&previous.shared.clock)),
        &candidate, Arc::clone(&cancel), loopback_tx,
    ).and_then(|next| {
        if cancel.load(Ordering::Acquire) {
            return Err("Audio output change cancelled".into());
        }
        if paused { next.pause(); } else { next.play()?; }
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
            Ok(())
        }
        Err(error) => {
            if !paused && ensure_generation(playback_generation, previous.playback_generation).is_ok() {
                let _ = previous.play();
            }
            Err(error)
        }
    }
}

/// EQ/响度/速度变化时按当前位置原地重建会话
///
/// 这些参数都固化在 ring 的输出帧里，改参数只有两种选择：跳过已缓冲的
/// 内容（旧做法——ring 容量 4 秒，用户拨一下均衡器歌就快进一大段），
/// 或者从当前位置重解码。这里选后者。重建失败时降级重试一次原会话，
/// 宁可参数晚生效，也不能把声音弄停。
#[allow(clippy::too_many_arguments)]
fn rebuild_session_in_place(
    current: &mut Option<PlaybackSession>,
    volume: f32,
    speed: f32,
    shared_level: &Arc<Mutex<SharedAudioLevel>>,
    effects_params: &Arc<Mutex<AudioEffectsParams>>,
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
            Arc::clone(effects_params),
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
            "effects rebuild failed once, retrying: {error}"
        );
        build()
    });
    match prepared {
        Ok(next) => {
            if !paused {
                if let Err(error) = next.play() {
                    log::warn!(target: "cpal-output", "effects rebuild play failed: {error}");
                }
            } else {
                // 暂停态重建：保持静音，避免调音效时突然出声
                next.pause();
            }
            *current = Some(next);
        }
        Err(error) => {
            log::warn!(
                target: "cpal-output",
                "effects rebuild failed, playback session lost: {error}"
            );
        }
    }
}

fn ensure_output_profile(
    output_profile: &mut OutputDeviceState,
) -> Result<&OutputDeviceProfile, String> {
    if output_profile.profile.is_none() {
        let selected = output_profile.preferred_name.as_deref()
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
fn rebuild_after_failed_seek(
    previous: &PlaybackSession,
    rollback_position_ms: u64,
    paused: bool,
    volume: f32,
    speed: f32,
    shared_level: &Arc<Mutex<SharedAudioLevel>>,
    effects_params: &Arc<Mutex<AudioEffectsParams>>,
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
            Arc::clone(effects_params),
            Arc::clone(playback_generation),
            expected_generation,
            None,
            Some(Arc::clone(clock)),
            output,
            Arc::new(AtomicBool::new(false)),
            loopback_tx,
        )
    })();
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
fn prepare_session(
    source: AudioSource,
    start_position_ms: u64,
    volume: f32,
    speed: f32,
    shared_level: Arc<Mutex<SharedAudioLevel>>,
    effects_params: Arc<Mutex<AudioEffectsParams>>,
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
    let decoder_started = Instant::now();
    let session_cancelled = Arc::new(AtomicBool::new(false));
    // 外部 prepare_cancel 挂到读取消链：等待超时后 remote read 可立刻打断
    let read_cancellation = RemoteReadCancellation::new(
        Arc::clone(&session_cancelled),
        operation_generation.as_ref().map(|token| {
            (Arc::clone(&token.generation), token.expected)
        }),
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
    let decoder_source =
        source.decoder_source_for_position(start_position_ms, read_cancellation);
    let use_byte_seek =
        start_position_ms > 0 && decoder_source.prefers_remote_virtual_body_seek();
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
    let mut decoder = match make_decoder_for_position(&decoder_source, start_position_ms, use_byte_seek)
    {
        Ok(decoder) => decoder,
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
    let start_position_ms = clamp_position(start_position_ms, duration_ms);
    // 字节跳转路径已经在目标附近顺序打开，不再走 format.seek（它会扫全文件）
    if start_position_ms > 0 && !use_byte_seek {
        decoder
            .try_seek(Duration::from_millis(start_position_ms))
            .map_err(|error| format!("Could not seek decoder: {error}"))?;
    } else if start_position_ms > 0 && use_byte_seek {
        log::info!(
            target: "cpal-output",
            "virtual-body open ready generation={}, start_ms={}, skip_format_seek=true virtual_body=true",
            expected_generation,
            start_position_ms,
        );
    }

    let config = &output.config;
    let channels = usize::from(config.channels.max(1));
    let sample_rate = config.sample_rate.0.max(1);
    let capacity_samples = duration_to_frames(PCM_CAPACITY, sample_rate)
        .saturating_mul(channels)
        .max(channels * 2);
    let initial_target = duration_to_frames(source.prebuffer_duration(), sample_rate);
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
    });

    let processed: Box<dyn PcmSource> = Box::new(LoudnessSource::new(
        EqualizerSource::new(decoder, Arc::clone(&effects_params)),
        effects_params,
    ));
    let worker = spawn_decode_worker(
        processed,
        Arc::clone(&shared),
        shared_level,
        Arc::clone(&playback_generation),
        expected_generation,
        operation_guard.clone(),
    )?;
    let output_started = Instant::now();
    let stream = match build_output_stream(
        &output.device,
        config,
        output.sample_format,
        Arc::clone(&shared),
        loopback_tx.clone(),
        expected_generation,
    ) {
        Ok(stream) => stream,
        Err(error) => {
            shared.cancelled.store(true, Ordering::Release);
            shared.wake.notify_all();
            return Err(error);
        }
    };
    let output_ms = output_started.elapsed().as_millis();
    let ready_started = Instant::now();
    if let Err(error) = wait_until_ready(
        &shared,
        &playback_generation,
        expected_generation,
        operation_generation.as_ref(),
        &prepare_cancel,
    ) {
        shared.cancelled.store(true, Ordering::Release);
        shared.wake.notify_all();
        let _ = stream.pause();
        return Err(error);
    }
    let ready_wait_ms = ready_started.elapsed().as_millis();

    log::info!(
        target: "cpal-output",
        "prepared {} on {}: {} Hz, {} ch, target={}ms, decoder={}ms, output={}ms, wait={}ms, total={}ms",
        source.label(),
        output.name,
        sample_rate,
        channels,
        source.prebuffer_duration().as_millis(),
        decoder_ms,
        output_ms,
        ready_wait_ms,
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
        playback_generation: expected_generation,
    })
}

fn make_decoder_for_position(
    source: &AudioSource,
    start_position_ms: u64,
    use_byte_seek: bool,
) -> Result<Box<dyn PcmSource>, String> {
    match source {
        AudioSource::Bytes(data, _) => SymphoniaAudioDecoder::new(
            Box::new(Cursor::new(Arc::clone(data))),
            None,
        )
        .map(|decoder| Box::new(decoder) as Box<dyn PcmSource>),
        AudioSource::File(path, _) => SymphoniaAudioDecoder::new_file(Path::new(path))
            .map(|decoder| Box::new(decoder) as Box<dyn PcmSource>),
        AudioSource::Growing(reader, _, _) => {
            SymphoniaAudioDecoder::new(Box::new(reader.clone()), None)
                .map(|decoder| Box::new(decoder) as Box<dyn PcmSource>)
        }
        AudioSource::Remote(reader, _) => {
            if use_byte_seek && start_position_ms > 0 {
                reader
                    .configure_virtual_body_for_time(start_position_ms)
                    .map_err(|error| format!("Could not prepare remote virtual body: {error}"))?;
                let decoder = SymphoniaAudioDecoder::new_remote_virtual(reader.clone(), true)?;
                Ok(Box::new(decoder) as Box<dyn PcmSource>)
            } else {
                // 普通远程：demuxer open 可隐藏 seekable；无虚拟 body
                reader.clear_virtual_body();
                let decoder = SymphoniaAudioDecoder::new_remote(reader.clone())?;
                Ok(Box::new(decoder) as Box<dyn PcmSource>)
            }
        }
    }
}

fn spawn_decode_worker(
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

            let mut frames_pushed = 0u64;
            let mut exit_reason = "source_eof";
            while !shared.cancelled.load(Ordering::Acquire)
                && playback_generation.load(Ordering::Acquire) == expected_generation
                && !read_cancellation.is_cancelled()
            {
                if shared.ring.writable_samples() < shared.channels {
                    if shared.paused.load(Ordering::Acquire) {
                        // 暂停期间输出回调不消费 ring，2ms 忙眠纯耗电；
                        // 改用 condvar 有界等待，resume/stop 会 notify 立即唤醒。
                        // 保守选 20ms 上限：即便错过通知，恢复时 ring 是满的
                        // （4 秒容量），20ms 的解码延迟不可能造成欠载
                        if let Ok(guard) = shared.wake_lock.lock() {
                            let _ = shared
                                .wake
                                .wait_timeout(guard, Duration::from_millis(20));
                        } else {
                            thread::sleep(DECODE_IDLE_SLEEP);
                        }
                    } else {
                        thread::sleep(DECODE_IDLE_SLEEP);
                    }
                    continue;
                }
                if !converter.next_frame(shared.sample_rate, shared.speed.load(), &mut frame) {
                    exit_reason = "decoder_exhausted";
                    break;
                }
                if !shared.ring.try_push_frame(&frame) {
                    thread::yield_now();
                    continue;
                }
                frames_pushed = frames_pushed.saturating_add(1);

                analysis.push(frame[0]);
                if analysis.len() >= ANALYSIS_FRAME_SIZE {
                    let result = analyzer.analyze_frame(&analysis);
                    SharedAudioLevel::try_update(
                        &shared_level,
                        result.level,
                        result.beat_impulse,
                    );
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

fn wait_until_ready(
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
        if shared.cancelled.load(Ordering::Acquire)
            || prepare_cancel.load(Ordering::Acquire)
        {
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

fn build_output_stream(
    device: &cpal::Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    shared: Arc<PlaybackShared>,
    loopback_tx: mpsc::Sender<AudioCmd>,
    playback_generation: u64,
) -> Result<Stream, String> {
    match sample_format {
        SampleFormat::I8 => build_typed_stream::<i8>(device, config, shared, loopback_tx, playback_generation),
        SampleFormat::I16 => build_typed_stream::<i16>(device, config, shared, loopback_tx, playback_generation),
        SampleFormat::I32 => build_typed_stream::<i32>(device, config, shared, loopback_tx, playback_generation),
        SampleFormat::I64 => build_typed_stream::<i64>(device, config, shared, loopback_tx, playback_generation),
        SampleFormat::U8 => build_typed_stream::<u8>(device, config, shared, loopback_tx, playback_generation),
        SampleFormat::U16 => build_typed_stream::<u16>(device, config, shared, loopback_tx, playback_generation),
        SampleFormat::U32 => build_typed_stream::<u32>(device, config, shared, loopback_tx, playback_generation),
        SampleFormat::U64 => build_typed_stream::<u64>(device, config, shared, loopback_tx, playback_generation),
        SampleFormat::F32 => build_typed_stream::<f32>(device, config, shared, loopback_tx, playback_generation),
        SampleFormat::F64 => build_typed_stream::<f64>(device, config, shared, loopback_tx, playback_generation),
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
    let channels = shared.channels;
    let mut frame = vec![0.0f32; channels];
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                let silence = T::from_sample(0.0);
                if callback_shared.paused.load(Ordering::Acquire)
                    || callback_shared.buffering.load(Ordering::Acquire)
                {
                    output.fill(silence);
                    return;
                }

                let mut rendered_frames = 0usize;
                let mut underflowed = false;
                let gain = callback_shared.volume.load() * callback_shared.fade_gain.load();
                for output_frame in output.chunks_mut(channels) {
                    if underflowed {
                        output_frame.fill(silence);
                        continue;
                    }
                    if !callback_shared.ring.try_pop_frame(&mut frame) {
                        output_frame.fill(silence);
                        callback_shared.begin_rebuffering();
                        underflowed = true;
                        continue;
                    }

                    for (target, sample) in output_frame.iter_mut().zip(frame.iter().copied()) {
                        *target = T::from_sample((sample * gain).clamp(-1.0, 1.0));
                    }
                    rendered_frames += 1;
                }
                if rendered_frames > 0 {
                    let speed = callback_shared.speed.load().clamp(0.25, 3.0);
                    let elapsed_us = ((rendered_frames as f64 * 1_000_000.0
                        / f64::from(callback_shared.sample_rate))
                        * f64::from(speed)) as u64;
                    callback_shared
                        .clock
                        .position_us
                        .fetch_add(elapsed_us, Ordering::AcqRel);
                }
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

// 播放/下载编排函数的参数都是相互独立的运行时上下文，聚成结构体只是换个地方堆字段
#[allow(clippy::too_many_arguments)]
fn crossfade_sessions(
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
fn fade_gain(
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
        let progress =
            (started.elapsed().as_secs_f32() * 1_000.0 / duration_ms as f32).min(1.0);
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

fn ensure_generation(generation: &AtomicU64, expected: u64) -> Result<(), String> {
    if generation.load(Ordering::Acquire) == expected {
        Ok(())
    } else {
        Err(PLAYBACK_SUPERSEDED.into())
    }
}

fn ensure_preparation_current(
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

pub fn wait_for_seek_result(
    receiver: mpsc::Receiver<Result<(), String>>,
) -> AppResult<()> {
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

fn take_latest_seek(
    position_ms: &mut u64,
    playback_generation: &mut u64,
    seek_generation: &mut u64,
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
                reply: next_reply,
            }) => {
                let _ = reply.send(Err(SEEK_SUPERSEDED.into()));
                *position_ms = next;
                *playback_generation = next_generation;
                *seek_generation = next_seek_generation;
                *reply = next_reply;
            }
            Ok(command) => deferred.push_back(command),
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
        }
    }
}

/// 从命令队列接管到的更新 Seek 载荷
struct AdoptedSeek {
    position_ms: u64,
    playback_generation: u64,
    seek_generation: u64,
    reply: mpsc::Sender<Result<(), String>>,
}

/// 旧 seek 被更新 seek 的代际递增打断后，接过队列里那条更新的 Seek 命令
///
/// 代际递增（request_seek）与命令入队之间只有微秒级窗口，因此在宽限内
/// 阻塞等待即可拿到。等待期间收到的非 Seek 命令保序暂存到 deferred、
/// 继续等——ticker 每 200ms 必发 QueryEmpty，第一条非 Seek 就放弃的话
/// 接管几乎必然失败，回滚重建会白跑一次完整 prepare（慢路径下秒级）
fn wait_for_newer_seek(
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
                reply,
            }) => {
                return Some(AdoptedSeek {
                    position_ms,
                    playback_generation,
                    seek_generation,
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

fn clamp_position(position_ms: u64, duration_ms: u64) -> u64 {
    if duration_ms > 0 {
        position_ms.min(duration_ms)
    } else {
        position_ms
    }
}

fn duration_to_frames(duration: Duration, sample_rate: u32) -> usize {
    let frames = duration
        .as_nanos()
        .saturating_mul(u128::from(sample_rate.max(1)))
        .saturating_add(999_999_999)
        / 1_000_000_000;
    usize::try_from(frames).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    #[test]
    fn android_alignment_stale_device_error_does_not_match_a_rebuilt_session() {
        let previous = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let current = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        assert!(!super::matches_output_session(7, &current, 7, &previous));
        assert!(super::matches_output_session(7, &current, 7, &current.clone()));
        assert!(!super::matches_output_session(8, &current, 7, &current));
    }

    #[test]
    fn android_alignment_abandoned_output_change_is_cancelled() {
        let (_sender, receiver) = std::sync::mpsc::channel();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        drop(super::OutputDeviceChangeRequest { receiver, cancel: cancel.clone(), completed: false });
        assert!(cancel.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn android_alignment_output_selection_follows_default_changes() {
        assert_eq!(super::effective_output_name(None, &["speaker", "headphones"], Some("headphones")), Some("headphones"));
        assert_eq!(super::effective_output_name(None, &["speaker", "headphones"], Some("speaker")), Some("speaker"));
    }

    #[test]
    fn android_alignment_output_selection_preserves_an_available_preference() {
        assert_eq!(super::effective_output_name(Some("speaker"), &["speaker", "headphones"], Some("headphones")), Some("speaker"));
    }

    #[test]
    fn android_alignment_output_selection_falls_back_and_recovers_after_disconnect() {
        assert_eq!(super::effective_output_name(Some("headphones"), &["speaker"], Some("speaker")), Some("speaker"));
        assert_eq!(super::effective_output_name(Some("headphones"), &["speaker", "headphones"], Some("speaker")), Some("headphones"));
        assert_eq!(super::effective_output_name(Some("headphones"), &[], None), None);
    }

    use super::{
        channel_sample, clamp_position, duration_to_frames, ensure_generation,
        ensure_preparation_current, take_latest_seek, wait_for_newer_seek, AudioCmd,
        make_decoder_for_position, AudioSource, GenerationToken, PlaybackClock, SEEK_SUPERSEDED,
    };
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn android_alignment_growing_seek_retains_buffer_until_last_source_releases() {
        let buffer = crate::audio::growing::GrowingAudioBuffer::new();
        buffer.append(include_bytes!("fixtures/hls-silence.aac"));
        buffer.finish();
        let previous = AudioSource::growing(buffer.reader(), 192);
        let seek_source = previous.clone();
        drop(previous);
        assert!(!buffer.is_aborted());
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancellation = crate::audio::remote::RemoteReadCancellation::new(Arc::clone(&cancelled), None);
        let old_decoder_source = seek_source.decoder_source_for_position(0, cancellation);
        let mut old_decoder = make_decoder_for_position(&old_decoder_source, 0, false).unwrap();
        cancelled.store(true, Ordering::Release);
        let _ = old_decoder.next();
        drop(old_decoder);
        drop(old_decoder_source);
        assert!(!buffer.is_aborted());
        let mut next_decoder = make_decoder_for_position(&seek_source, 0, false).unwrap();
        assert!(next_decoder.next().is_some());
        drop(next_decoder);
        drop(seek_source);
        assert!(buffer.is_aborted());
    }

    #[test]
    fn android_alignment_cancelled_growing_prepare_does_not_abort_rollback_source() {
        let buffer = crate::audio::growing::GrowingAudioBuffer::new();
        buffer.append(include_bytes!("fixtures/hls-silence.aac"));
        buffer.finish();
        let rollback = AudioSource::growing(buffer.reader(), 192);
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let failed = rollback.decoder_source_for_position(0, crate::audio::remote::RemoteReadCancellation::new(cancelled, None));
        assert!(make_decoder_for_position(&failed, 0, false).is_err());
        drop(failed);
        assert!(!buffer.is_aborted());
        assert!(make_decoder_for_position(&rollback, 0, false).unwrap().next().is_some());
        drop(rollback);
        assert!(buffer.is_aborted());
    }

    #[test]
    fn android_alignment_growing_reader_uses_session_and_disarmed_prepare_cancellation() {
        use std::io::Read;
        let buffer = crate::audio::growing::GrowingAudioBuffer::new();
        buffer.append(b"abc");
        buffer.finish();
        let source = AudioSource::growing(buffer.reader(), 0);
        let session = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let prepare = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancellation = crate::audio::remote::RemoteReadCancellation::new(Arc::clone(&session), None).with_external_cancel(Arc::clone(&prepare));
        let guard = cancellation.clone();
        let AudioSource::Growing(mut reader, _, _) = source.decoder_source_for_position(0, cancellation) else { panic!("expected growing source") };
        guard.disarm_operation_guard();
        prepare.store(true, Ordering::Release);
        assert_eq!(reader.read(&mut [0; 1]).unwrap(), 1);
        session.store(true, Ordering::Release);
        assert!(reader.read(&mut [0; 1]).is_err());
        assert!(!buffer.is_aborted());
    }

    #[test]
    fn android_alignment_committed_decode_worker_survives_the_next_seek_generation() {
        let generation = Arc::new(AtomicU64::new(7));
        let seek = Arc::new(AtomicU64::new(1));
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancellation = crate::audio::remote::RemoteReadCancellation::new(
            Arc::clone(&cancelled), Some((Arc::clone(&seek), 1)),
        );
        let shared = Arc::new(super::PlaybackShared {
            ring: Arc::new(crate::audio::buffered::PcmRing::new(8)),
            channels: 1, sample_rate: 48_000,
            paused: std::sync::atomic::AtomicBool::new(false),
            buffering: std::sync::atomic::AtomicBool::new(true),
            finished: std::sync::atomic::AtomicBool::new(false),
            cancelled,
            volume: super::AtomicF32::new(1.0), fade_gain: super::AtomicF32::new(1.0),
            speed: super::AtomicF32::new(1.0),
            buffer_target_frames: std::sync::atomic::AtomicUsize::new(1),
            clock: Arc::new(PlaybackClock::new(0)),
            wake_lock: std::sync::Mutex::new(()), wake: std::sync::Condvar::new(),
            device_lost: std::sync::atomic::AtomicBool::new(false),
        });
        let decoder = crate::audio::remote::SymphoniaAudioDecoder::new(
            Box::new(std::io::Cursor::new(include_bytes!("fixtures/hls-silence.aac").as_slice())), Some("aac"),
        ).unwrap();
        let worker = super::spawn_decode_worker(
            Box::new(decoder), Arc::clone(&shared),
            crate::audio::analyzer::SharedAudioLevel::new(),
            generation, 7, cancellation.clone(),
        ).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while shared.ring.readable_samples() < 8 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(shared.ring.readable_samples(), 8);
        cancellation.disarm_operation_guard();
        seek.store(2, Ordering::Release);
        std::thread::sleep(Duration::from_millis(40));
        let kept_running = !shared.finished.load(Ordering::Acquire);
        shared.cancelled.store(true, Ordering::Release);
        shared.wake.notify_all();
        worker.join().unwrap();
        assert!(kept_running, "a submitted worker must stop through its session token");
    }

    #[test]
    fn duration_to_frames_matches_android_buffer_thresholds() {
        assert_eq!(duration_to_frames(Duration::from_millis(800), 48_000), 38_400);
        assert_eq!(duration_to_frames(Duration::from_millis(1_500), 48_000), 72_000);
    }

    #[test]
    fn channel_conversion_handles_mono_and_downmix() {
        assert_eq!(channel_sample(&[0.25], 1, 2), 0.25);
        assert!((channel_sample(&[0.25, 0.75], 0, 1) - 0.5).abs() < f32::EPSILON);
        assert_eq!(channel_sample(&[0.25, 0.75], 1, 2), 0.75);
    }

    /// 回归（seek 双亡死局）：旧 seek 的 prepare 被更新 seek 的代际递增
    /// 打断后，必须能从命令队列接过那条更新的 Seek 完整重跑，
    /// 而不是双双失败留下无人重启的空会话
    #[test]
    fn wait_for_newer_seek_adopts_queued_seek() {
        let (sender, receiver) = mpsc::channel();
        let (reply, _result) = mpsc::channel();
        sender
            .send(AudioCmd::Seek {
                position_ms: 210_641,
                playback_generation: 7,
                seek_generation: 4,
                reply,
            })
            .expect("queue newer seek");
        let mut deferred = VecDeque::new();

        let adopted = wait_for_newer_seek(&receiver, &mut deferred, Duration::from_millis(200))
            .expect("newer seek must be adopted");

        assert_eq!(
            (
                adopted.position_ms,
                adopted.playback_generation,
                adopted.seek_generation,
            ),
            (210_641, 7, 4),
        );
        assert!(deferred.is_empty());
    }

    /// 非 Seek 命令不能打断接管等待：保序暂存后继续等——ticker 的
    /// QueryEmpty 会夹在两次拖动之间，第一条非 Seek 就放弃的话
    /// 接管几乎必然失败（实机日志：每次接管前多跑一次回滚 prepare）
    #[test]
    fn wait_for_newer_seek_defers_noise_and_still_adopts() {
        let (sender, receiver) = mpsc::channel();
        let (reply, _result) = mpsc::channel();
        sender
            .send(AudioCmd::InvalidateProcessedBuffer)
            .expect("queue non-seek command");
        sender
            .send(AudioCmd::Seek {
                position_ms: 4_606_826,
                playback_generation: 7,
                seek_generation: 9,
                reply,
            })
            .expect("queue newer seek behind noise");
        let mut deferred = VecDeque::new();

        let adopted = wait_for_newer_seek(&receiver, &mut deferred, Duration::from_millis(200))
            .expect("seek behind noise must still be adopted");

        assert_eq!(adopted.position_ms, 4_606_826);
        assert_eq!(deferred.len(), 1);
        assert!(matches!(
            deferred.front(),
            Some(AudioCmd::InvalidateProcessedBuffer)
        ));
    }

    /// 宽限超时（极端调度竞态下新 Seek 未入队）：返回 None，
    /// 调用方走回滚重建，迟到的 Seek 会在恢复出的会话上正常执行；
    /// 期间收到的非 Seek 命令仍保序暂存不丢
    #[test]
    fn wait_for_newer_seek_times_out_on_empty_queue() {
        let (sender, receiver) = mpsc::channel::<AudioCmd>();
        sender
            .send(AudioCmd::InvalidateProcessedBuffer)
            .expect("queue non-seek command");
        let mut deferred = VecDeque::new();

        let started = std::time::Instant::now();
        let adopted =
            wait_for_newer_seek(&receiver, &mut deferred, Duration::from_millis(30));

        assert!(adopted.is_none());
        assert_eq!(deferred.len(), 1);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// 已有暂存命令时继续等待新 Seek：暂存命令保序不丢、不被覆盖
    #[test]
    fn wait_for_newer_seek_preserves_existing_deferred_commands() {
        let (sender, receiver) = mpsc::channel();
        let (reply, _result) = mpsc::channel();
        sender
            .send(AudioCmd::Seek {
                position_ms: 1_000,
                playback_generation: 1,
                seek_generation: 2,
                reply,
            })
            .expect("queue seek behind deferred");
        let mut deferred = VecDeque::from([AudioCmd::Pause]);

        let adopted = wait_for_newer_seek(&receiver, &mut deferred, Duration::from_millis(200))
            .expect("existing deferred must not block adoption");

        assert_eq!(adopted.position_ms, 1_000);
        assert_eq!(deferred.len(), 1);
        assert!(matches!(deferred.front(), Some(AudioCmd::Pause)));
    }

    #[test]
    fn seek_queue_keeps_latest_consecutive_position() {
        let (sender, receiver) = mpsc::channel();
        let (first_reply, first_result) = mpsc::channel();
        let (second_reply, second_result) = mpsc::channel();
        let (third_reply, third_result) = mpsc::channel();
        assert!(sender
            .send(AudioCmd::Seek {
                position_ms: 100,
                playback_generation: 7,
                seek_generation: 1,
                reply: first_reply,
            })
            .is_ok());
        assert!(sender
            .send(AudioCmd::Seek {
                position_ms: 800,
                playback_generation: 7,
                seek_generation: 2,
                reply: second_reply,
            })
            .is_ok());
        assert!(sender
            .send(AudioCmd::Seek {
                position_ms: 1_600,
                playback_generation: 8,
                seek_generation: 3,
                reply: third_reply,
            })
            .is_ok());
        let mut latest = 0;
        let mut generation = 7;
        let mut seek_generation = 0;
        let (initial_reply, initial_result) = mpsc::channel();
        let mut reply = initial_reply;
        let mut deferred = VecDeque::new();

        take_latest_seek(
            &mut latest,
            &mut generation,
            &mut seek_generation,
            &mut reply,
            &receiver,
            &mut deferred,
        );

        assert_eq!(latest, 1_600);
        assert_eq!(generation, 8);
        assert_eq!(seek_generation, 3);
        assert_eq!(
            initial_result.recv().expect("initial result"),
            Err(SEEK_SUPERSEDED.into())
        );
        assert_eq!(
            first_result.recv().expect("first result"),
            Err(SEEK_SUPERSEDED.into())
        );
        assert_eq!(
            second_result.recv().expect("second result"),
            Err(SEEK_SUPERSEDED.into())
        );
        reply.send(Ok(())).expect("latest reply");
        assert_eq!(third_result.recv().expect("third result"), Ok(()));
        assert!(deferred.is_empty());
    }

    #[test]
    fn playback_generation_rejects_superseded_work() {
        let generation = AtomicU64::new(7);
        assert!(ensure_generation(&generation, 7).is_ok());
        generation.store(8, Ordering::Release);
        assert!(ensure_generation(&generation, 7).is_err());
    }

    #[test]
    fn newer_seek_cancels_only_the_old_seek_preparation() {
        let playback_generation = AtomicU64::new(7);
        let seek_generation = Arc::new(AtomicU64::new(2));
        let old_seek = GenerationToken {
            generation: Arc::clone(&seek_generation),
            expected: 1,
        };
        let current_seek = GenerationToken {
            generation: seek_generation,
            expected: 2,
        };

        assert!(ensure_preparation_current(
            &playback_generation,
            7,
            Some(&old_seek),
        )
        .is_err());
        assert!(ensure_preparation_current(
            &playback_generation,
            7,
            Some(&current_seek),
        )
        .is_ok());
    }

    #[test]
    fn playback_clock_starts_at_requested_position() {
        let clock = PlaybackClock::new(12_345);
        assert_eq!(clock.position_ms(), 12_345);
    }

    #[test]
    fn known_duration_clamps_seek() {
        assert_eq!(clamp_position(20_000, 10_000), 10_000);
        assert_eq!(clamp_position(20_000, 0), 20_000);
    }
}
