use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
use std::collections::VecDeque;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, PoisonError, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::analyzer::{AudioAnalyzer, SharedAudioLevel};
use crate::buffered::PcmRing;
use crate::decoder::{self, AudioDecoder};
use crate::dsp::{AudioEffectsStats, DspProcessor};
use crate::dsp_settings::AudioEffectsSettings;
use crate::effects::{
    gain_to_millibels, EffectsControl, EffectsProcessor, LoudnessMeter, LoudnessScanner,
    LoudnessStats, TrackLoudness, NORMALIZE_WARMUP,
};
use crate::ffmpeg::ByteInput;
use crate::growing::GrowingAudioReader;
use crate::metrics::{
    self, CommandStamps, FirstFrameProbe, MetricsReporter, OutputMetrics, StartKind,
};
use crate::pcm::PcmSource;
use crate::remote::{
    RemoteAudioSource, RemoteReadCancellation, SourceAudioInfo, SymphoniaAudioDecoder,
};
use crate::stretch::PitchShifter;
use neri_common::error::{AppError, AppResult};

// 按职责拆分的子模块，通过 `use super::*` 共享本模块的类型与常量
mod control_loop;
mod engine;
mod exclusive_format;
mod loudness;
mod output;
mod output_device;
mod pipeline;
mod resample;
mod transition;
#[cfg(target_os = "windows")]
mod wasapi_exclusive;
use control_loop::*;
pub use exclusive_format::{
    exclusive_output_requested, set_exclusive_output, EXCLUSIVE_OUTPUT_SUPPORTED,
};
use loudness::*;
use output::*;
use output_device::*;
pub use output_device::{list_audio_output_devices, AudioOutputDevice, OutputDeviceChangeRequest};
use pipeline::*;
use resample::*;
use transition::*;
pub use transition::{receive_fade_result, wait_for_play_result, wait_for_seek_result};

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
const DECODE_FULL_WAIT: Duration = Duration::from_millis(10);
const DECODE_PAUSED_WAIT: Duration = Duration::from_millis(20);
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
const OUTPUT_DEVICE_IDLE_POLL_INTERVAL: Duration = Duration::from_secs(15);
/// 回调一次最多经变速器处理的帧数；更大的设备缓冲按这个大小分块
const RENDER_CHUNK_FRAMES: usize = 4096;
/// 字节跳转后最多丢掉这么长的分片开头；超过说明落点不对，宁可不丢
const MAX_VIRTUAL_BODY_LEAD_MS: u64 = 20_000;

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

    fn encoded_byte_length(&self) -> Option<u64> {
        match self {
            Self::Bytes(bytes, _) => u64::try_from(bytes.len()).ok(),
            Self::File(path, _) => std::fs::metadata(path).ok().map(|metadata| metadata.len()),
            Self::Growing(reader, _, _) => symphonia::core::io::MediaSource::byte_len(reader),
            Self::Remote(source, _) => Some(source.byte_len()),
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

    /// 虚拟 body 只能交给 symphonia 解；需要 FFmpeg 的流（如 B 站 E-AC-3）改用 FFmpeg 按 sidx 定位
    fn prefers_remote_virtual_body_seek(&self) -> bool {
        matches!(self, Self::Remote(reader, _) if reader.prefers_virtual_body_seek())
            && !self.decodes_with_ffmpeg()
    }

    fn decodes_with_ffmpeg(&self) -> bool {
        let Self::Remote(reader, _) = self else {
            return false;
        };
        reader
            .header_bytes()
            .and_then(|header| decoder::sniff(&header))
            .is_some()
            && crate::ffmpeg::runtime().is_ok()
    }
}

/// 读本地文件开头一段用于嗅探编码；读不了时交给后续的打开流程报告错误
fn sniff_file(path: &Path) -> Option<decoder::SniffedCodec> {
    let mut header = Vec::with_capacity(decoder::SNIFF_BYTES);
    std::fs::File::open(path)
        .ok()?
        .take(decoder::SNIFF_BYTES as u64)
        .read_to_end(&mut header)
        .ok()?;
    decoder::sniff(&header)
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
        /// 命令入队时刻（`metrics::monotonic_ns`），首帧耗时从这里算起
        issued_ns: u64,
        reply: mpsc::Sender<Result<PlaybackStarted, String>>,
    },
    Pause,
    Resume,
    Stop,
    /// 切歌时立刻静音上一首：只暂停代际早于 `before_generation` 的会话，不推进任何代际，
    /// 晚于新会话到达也不会误伤它，也不会作废排在后面的 Play
    SilenceStale {
        before_generation: u64,
    },
    ReleaseFile {
        path: String,
        reply: mpsc::Sender<Option<u64>>,
    },
    SetVolume(f32),
    SetOutputDevice {
        name: Option<String>,
        /// User-visible sink name, kept as the DSP profile key when the ALSA profile is default.
        display_name: Option<String>,
        force: bool,
        cancel: Arc<AtomicBool>,
        reply: mpsc::Sender<Result<(), String>>,
    },
    SetSpeed(f32),
    Seek {
        position_ms: u64,
        playback_generation: u64,
        seek_generation: u64,
        issued_ns: u64,
        reply: mpsc::Sender<Result<(), String>>,
    },
    QueryEmpty {
        reply: mpsc::Sender<bool>,
    },
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
    /// 设备监视线程每轮枚举的结果；控制线程只比较名字，需要时才切换设备
    OutputDevicesListed {
        available: Vec<String>,
        default: Option<String>,
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
    pub audio_info: SourceAudioInfo,
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
    /// 首帧交给设备的时刻（`metrics::monotonic_ns`），0 表示还没出过帧
    first_frame_ns: Arc<AtomicU64>,
    /// 变速和音调采样已从 ring 取出、还没送到设备的帧数，判断播完时要算上
    stretch_buffered: AtomicUsize,
    effects: Arc<EffectsControl>,
    /// 这首歌的响度统计：解码线程写，输出回调读；同一首歌重建会话时沿用
    loudness: Arc<TrackLoudness>,
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
        self.buffer_target_frames.store(target, Ordering::Release);
        self.buffering.store(true, Ordering::Release);
        self.wake.notify_all();
    }
}

struct PlaybackSession {
    source: AudioSource,
    stream: OutputStream,
    shared: Arc<PlaybackShared>,
    worker: Option<JoinHandle<()>>,
    duration_ms: u64,
    audio_info: SourceAudioInfo,
    playback_generation: u64,
}

struct OutputDeviceProfile {
    device: Device,
    config: StreamConfig,
    sample_format: SampleFormat,
    name: String,
    /// 打开时是否请求了独占输出，切换开关后通过强制重建生效
    exclusive: bool,
}

impl OutputDeviceProfile {
    fn open_default() -> Result<Self, String> {
        super::quiet::silence_library_diagnostics();
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "No default audio output device".to_string())?;
        Self::from_device(device)
    }

    fn open_named(name: &str) -> Result<Self, String> {
        super::quiet::silence_library_diagnostics();
        let device = render_devices(&cpal::default_host())
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
            exclusive: EXCLUSIVE_OUTPUT_SUPPORTED && exclusive_output_requested(),
        })
    }
}

pub struct FileReleaseRequest {
    receiver: mpsc::Receiver<Option<u64>>,
}

impl FileReleaseRequest {
    pub fn wait(self) -> AppResult<Option<u64>> {
        self.receiver
            .recv_timeout(COMMAND_TIMEOUT)
            .map_err(|error| AppError::Audio(format!("File release timeout: {error}")))
    }
}

impl PlaybackSession {
    fn play(&self) -> Result<(), String> {
        self.shared.paused.store(false, Ordering::Release);
        // 唤醒可能在暂停态长睡的解码线程（见 spawn_decode_worker 的暂停等待）
        self.shared.wake.notify_all();
        self.stream.play()
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
        // stop 是会话终态，此后只会读它的 source 重建新会话（seek 回滚也是）。
        // 独占流不能等会话 drop 才归还设备，否则同一设备上的新会话打不开
        self.stream.release_exclusive();
        if let Some(worker) = self.worker.take() {
            finish_decode_worker(&self.source, &self.shared, worker);
        }
    }

    fn holds_output_exclusively(&self) -> bool {
        self.stream.holds_device_exclusively()
    }

    /// 为新流让出设备、保留会话本身（源、时钟、响度），失败时仍能据此重建
    fn release_output_before_reopen(&mut self, next_exclusive: bool) -> bool {
        self.stream.release_before_reopen(next_exclusive)
    }

    fn is_empty(&self) -> bool {
        self.shared.finished.load(Ordering::Acquire)
            && self.shared.ring.readable_samples() < self.shared.channels
            && self.shared.stretch_buffered.load(Ordering::Acquire) == 0
    }
}

impl Drop for PlaybackSession {
    fn drop(&mut self) {
        self.stop();
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
    effects: Arc<EffectsControl>,
    playback_generation: Arc<AtomicU64>,
    seek_generation: Arc<AtomicU64>,
    transition_generation: Arc<AtomicU64>,
    loaded_generation: Option<u64>,
    ended_transition: Option<u64>,
    clock: Option<Arc<PlaybackClock>>,
    audio_info: Option<SourceAudioInfo>,
}

impl Default for PlayerEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
