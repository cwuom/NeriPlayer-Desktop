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

use crate::audio::analyzer::{AudioAnalyzer, SharedAudioLevel};
use crate::audio::buffered::PcmRing;
use crate::audio::decoder::{self, AudioDecoder};
use crate::audio::effects::{
    gain_to_millibels, EffectsControl, EffectsProcessor, LoudnessMeter, LoudnessScanner,
    LoudnessStats, TrackLoudness, NORMALIZE_WARMUP,
};
use crate::audio::ffmpeg::ByteInput;
use crate::audio::growing::GrowingAudioReader;
use crate::audio::metrics::{
    self, CommandStamps, FirstFrameProbe, MetricsReporter, OutputMetrics, StartKind,
};
use crate::audio::pcm::PcmSource;
use crate::audio::remote::{
    RemoteAudioSource, RemoteReadCancellation, SourceAudioInfo, SymphoniaAudioDecoder,
};
use crate::audio::stretch::Stretcher;
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
        let Self::Remote(reader, _) = self else { return false };
        reader.header_bytes().and_then(|header| decoder::sniff(&header)).is_some()
            && crate::audio::ffmpeg::runtime().is_ok()
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
    SilenceStale { before_generation: u64 },
    ReleaseFile {
        path: String,
        reply: mpsc::Sender<Option<u64>>,
    },
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
        issued_ns: u64,
        reply: mpsc::Sender<Result<(), String>>,
    },
    QueryEmpty { reply: mpsc::Sender<bool> },
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
    /// 变速器已从 ring 取出、还没送到设备的帧数：判断「播完」时要算上
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
    audio_info: SourceAudioInfo,
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

/// 能输出的设备，按默认混音格式判断
///
/// 不用 cpal 的 `output_devices()`：它对每个设备试探几十种格式，cpal 0.15 在 WASAPI 上
/// 不释放 `IsFormatSupported` 返回的近似格式，每次枚举泄漏约 23 KB（7 个设备实测）。
/// 设备监视每 2 秒枚举一次，一小时就是三十多 MB。
fn render_devices(host: &cpal::Host) -> Result<impl Iterator<Item = Device>, cpal::DevicesError> {
    Ok(host.devices()?.filter(|device| device.default_output_config().is_ok()))
}

pub fn list_audio_output_devices() -> AppResult<Vec<AudioOutputDevice>> {
    let host = cpal::default_host();
    let default_name = host.default_output_device().and_then(|device| device.name().ok());
    let devices = render_devices(&host).map_err(|error| AppError::Audio(error.to_string()))?;
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

/// 设备列表更新后要切到的输出设备；已经在用目标设备时返回 None
fn output_device_to_switch(
    preferred: Option<&str>,
    available: &[String],
    default: Option<&str>,
    current: Option<&str>,
) -> Option<String> {
    let names: Vec<&str> = available.iter().map(String::as_str).collect();
    let target = effective_output_name(preferred, &names, default)?;
    (current != Some(target)).then(|| target.to_string())
}

/// 在独立线程里定期枚举输出设备，把结果交给控制线程
///
/// Windows 上一次枚举要 250–300 ms。放在控制线程里时，这段时间到达的
/// 播放、seek、暂停命令都得排队等它。控制线程断开后发送失败，线程随之退出。
fn spawn_output_device_watch(commands: mpsc::Sender<AudioCmd>) {
    let spawned = thread::Builder::new()
        .name("audio-device-watch".into())
        .spawn(move || {
            let mut last_error: Option<String> = None;
            loop {
                thread::sleep(OUTPUT_DEVICE_POLL_INTERVAL);
                let devices = match list_audio_output_devices() {
                    Ok(devices) => {
                        if last_error.take().is_some() {
                            log::info!(target: "cpal-output", "output device enumeration recovered");
                        }
                        devices
                    }
                    Err(error) => {
                        let message = error.to_string();
                        if last_error.as_deref() != Some(message.as_str()) {
                            log::warn!(target: "cpal-output", "output device enumeration failed: {message}");
                        }
                        last_error = Some(message);
                        continue;
                    }
                };
                let default = devices
                    .iter()
                    .find(|device| device.is_default)
                    .map(|device| device.name.clone());
                let available = devices.into_iter().map(|device| device.name).collect();
                if commands
                    .send(AudioCmd::OutputDevicesListed { available, default })
                    .is_err()
                {
                    break;
                }
            }
        });
    if let Err(error) = spawned {
        log::error!(target: "cpal-output", "could not start the output device watcher: {error}");
    }
}

fn matches_output_session(
    current_generation: u64,
    current_cancel: &Arc<AtomicBool>,
    reported_generation: u64,
    reported_cancel: &Arc<AtomicBool>,
) -> bool {
    current_generation == reported_generation && Arc::ptr_eq(current_cancel, reported_cancel)
}

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

fn finish_decode_worker(
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

fn silence_stale_session(current: Option<&PlaybackSession>, before_generation: u64) {
    if let Some(session) = current.filter(|session| session.playback_generation < before_generation) {
        session.pause();
    }
}

fn matches_local_file(source: &AudioSource, target: &Path) -> bool {
    let AudioSource::File(path, _) = source else { return false };
    same_local_file(Path::new(path), target)
}

fn same_local_file(source: &Path, target: &Path) -> bool {
    if let (Ok(source), Ok(target)) = (source.canonicalize(), target.canonicalize()) {
        return source == target;
    }
    #[cfg(windows)]
    {
        source.to_string_lossy().eq_ignore_ascii_case(&target.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        source == target
    }
}

fn invalidate_released_generation(generation: &AtomicU64, released: u64) {
    // 用户已切歌时不改变新请求，只有被释放的会话仍当前时才作废首播回包
    let _ = generation.compare_exchange(
        released, released.saturating_add(1), Ordering::AcqRel, Ordering::Acquire,
    );
}

pub struct FileReleaseRequest {
    receiver: mpsc::Receiver<Option<u64>>,
}

impl FileReleaseRequest {
    pub fn wait(self) -> AppResult<Option<u64>> {
        self.receiver.recv_timeout(COMMAND_TIMEOUT)
            .map_err(|error| AppError::Audio(format!("File release timeout: {error}")))
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
            finish_decode_worker(&self.source, &self.shared, worker);
        }
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

struct FrameResampler {
    source: Box<dyn PcmSource>,
    source_rate: u32,
    current: Vec<f32>,
    next: Vec<f32>,
    phase: f64,
    initialized: bool,
    source_ended: bool,
    finished: bool,
    downmix: Option<Downmix>,
    /// 下混系数是按几路输出算的；0 表示还没算过
    downmix_output: usize,
}

/// 源声道多于输出时的下混系数（按 WAVE 标准声道顺序推断布局）
///
/// 旧做法只取前两个声道：5.1 的中置（对白）、环绕和低频全部丢失。
/// 这里按 ITU-R BS.775 取系数（低频不计入），再整体缩放到每路输出系数和不超过 1。
struct Downmix {
    /// 按输出声道排列，每路一组源声道系数
    weights: Vec<f32>,
}

impl Downmix {
    fn new(source_channels: usize, output_channels: usize) -> Option<Self> {
        if source_channels <= output_channels.max(2) || !(1..=2).contains(&output_channels) {
            return None;
        }
        const SIDE: f32 = std::f32::consts::FRAC_1_SQRT_2;
        // (左, 右) 系数；顺序与 WAVE/FLAC 的默认声道布局一致
        let roles: &[(f32, f32)] = match source_channels {
            3 => &[(1.0, 0.0), (0.0, 1.0), (SIDE, SIDE)],
            4 => &[(1.0, 0.0), (0.0, 1.0), (SIDE, 0.0), (0.0, SIDE)],
            5 => &[(1.0, 0.0), (0.0, 1.0), (SIDE, SIDE), (SIDE, 0.0), (0.0, SIDE)],
            6 => &[(1.0, 0.0), (0.0, 1.0), (SIDE, SIDE), (0.0, 0.0), (SIDE, 0.0), (0.0, SIDE)],
            7 => &[(1.0, 0.0), (0.0, 1.0), (SIDE, SIDE), (0.0, 0.0), (0.5, 0.5), (SIDE, 0.0), (0.0, SIDE)],
            _ => &[(1.0, 0.0), (0.0, 1.0), (SIDE, SIDE), (0.0, 0.0), (SIDE, 0.0), (0.0, SIDE), (SIDE, 0.0), (0.0, SIDE)],
        };
        let role = |channel: usize| roles.get(channel).copied().unwrap_or((0.5, 0.5));
        let mut weights = Vec::with_capacity(source_channels * output_channels);
        for output in 0..output_channels {
            for channel in 0..source_channels {
                let (left, right) = role(channel);
                // 单声道取立体声下混的平均，与立体声转单声道的处理一致
                weights.push(match (output_channels, output) {
                    (1, _) => (left + right) * 0.5,
                    (_, 0) => left,
                    _ => right,
                });
            }
        }
        let largest = weights
            .chunks(source_channels)
            .map(|row| row.iter().map(|weight| weight.abs()).sum::<f32>())
            .fold(0.0f32, f32::max);
        if largest > 1.0 {
            weights.iter_mut().for_each(|weight| *weight /= largest);
        }
        Some(Self { weights })
    }

    fn sample(&self, frame: &[f32], output_channel: usize) -> f32 {
        let source_channels = frame.len();
        self.weights
            .chunks(source_channels)
            .nth(output_channel)
            .map_or(0.0, |row| row.iter().zip(frame).map(|(weight, sample)| weight * sample).sum())
    }
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
            downmix: None,
            downmix_output: 0,
        }
    }

    fn source_channels(&self) -> usize {
        self.current.len()
    }

    fn source_mut(&mut self) -> &mut dyn PcmSource {
        self.source.as_mut()
    }

    /// 源被 seek 之后丢掉插值状态，从新位置重新开始
    fn restart(&mut self) {
        self.phase = 0.0;
        self.initialized = false;
        self.source_ended = false;
        self.finished = false;
    }

    fn mixed_sample(&self, frame: &[f32], channel: usize, output_channels: usize) -> f32 {
        match &self.downmix {
            Some(downmix) => downmix.sample(frame, channel),
            None => channel_sample(frame, channel, output_channels),
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

    /// 只做采样率转换；倍速在输出端由变速器完成，这样 ring 里的内容与倍速无关
    fn next_frame(&mut self, output_rate: u32, output: &mut [f32]) -> bool {
        if self.finished || !self.initialize() {
            return false;
        }

        let phase = self.phase as f32;
        let output_channels = output.len();
        if self.downmix_output != output_channels {
            self.downmix_output = output_channels;
            self.downmix = Downmix::new(self.current.len(), output_channels);
        }
        for (channel, sample) in output.iter_mut().enumerate() {
            let current = self.mixed_sample(&self.current, channel, output_channels);
            let next = self.mixed_sample(&self.next, channel, output_channels);
            *sample = current + (next - current) * phase;
        }

        self.phase += f64::from(self.source_rate) / f64::from(output_rate.max(1));
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

/// 输出帧里承载节目内容的前几个声道，响度只按它们统计
///
/// 声道少的源接到声道多的设备上（立体声接 7.1），多出来的声道补的是零；
/// 把它们算进去会把 RMS 拉低，音量均衡就会把每首歌都多抬几 dB。
/// 单声道源复制到了每个声道，统计其中一个就够。
fn loudness_channels(source_channels: usize, output_channels: usize) -> usize {
    source_channels.clamp(1, output_channels.max(1))
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
    effects: Arc<EffectsControl>,
    playback_generation: Arc<AtomicU64>,
    seek_generation: Arc<AtomicU64>,
    transition_generation: Arc<AtomicU64>,
    loaded_generation: Option<u64>,
    clock: Option<Arc<PlaybackClock>>,
    audio_info: Option<SourceAudioInfo>,
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
    pub fn complete_start(&mut self, result: PlaybackStarted, expected_generation: u64, current_path: String) -> AppResult<u64> {
        self.ensure_request_current(expected_generation)?;
        self.is_playing = true;
        self.current_path = Some(current_path);
        self.duration_ms = result.duration_ms;
        self.loaded_generation = Some(expected_generation);
        self.clock = Some(result.clock);
        self.audio_info = Some(result.audio_info);
        Ok(result.duration_ms)
    }

    pub fn playback_audio_info(&self, request_generation: u64) -> Option<SourceAudioInfo> {
        if self.loaded_generation == Some(request_generation)
            && self.playback_generation.load(Ordering::Acquire) == request_generation {
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
        self.clear_playback_state();
    }

    /// 新播放请求一开始就静音上一首（交叉淡化除外），不必等新音源解析、缓冲完
    pub fn silence_stale_sessions(&self, before_generation: u64) {
        let _ = self.cmd_tx.send(AudioCmd::SilenceStale { before_generation });
    }

    fn clear_playback_state(&mut self) {
        self.is_playing = false;
        self.current_path = None;
        self.duration_ms = 0;
        self.loaded_generation = None;
        self.clock = None;
        self.audio_info = None;
        SharedAudioLevel::reset(&self.shared_audio_level);
    }

    pub fn request_file_release(&mut self, path: String) -> AppResult<FileReleaseRequest> {
        self.ensure_alive();
        let (reply, receiver) = mpsc::channel();
        self.cmd_tx.send(AudioCmd::ReleaseFile { path, reply })
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
        // 倍速在输出端实时生效，不需要重建会话
        let _ = self.cmd_tx.send(AudioCmd::SetSpeed(self.speed));
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
                issued_ns: metrics::monotonic_ns(),
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
    spawn_output_device_watch(cmd_tx.clone());
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
    let mut reporter = MetricsReporter::new();
    loop {
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
            current
                .as_ref()
                .is_some_and(|session| !session.shared.paused.load(Ordering::Acquire)),
        );
        let Some(command) = command else { continue };
        match command {
            AudioCmd::OutputDevicesListed { available, default } => {
                let Some(name) = output_device_to_switch(
                    output_profile.preferred_name.as_deref(),
                    &available,
                    default.as_deref(),
                    output_profile.profile.as_ref().map(|profile| profile.name.as_str()),
                ) else {
                    continue;
                };
                let result = OutputDeviceProfile::open_named(&name).and_then(|candidate| {
                    switch_output_device_in_place(
                        &mut current, candidate, &mut output_profile, volume, speed,
                        &shared_level, &effects, &playback_generation, &loopback_tx,
                        Arc::new(AtomicBool::new(false)),
                    )
                });
                if let Err(error) = result {
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
                    audio_info: next.audio_info.clone(),
                };
                reporter.track(FirstFrameProbe::new(
                    StartKind::Play,
                    source_label,
                    expected,
                    start_position_ms,
                    CommandStamps { issued_ns, received_ns },
                    Arc::clone(&next.shared.first_frame_ns),
                ));
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
            AudioCmd::SilenceStale { before_generation } => {
                silence_stale_session(current.as_ref(), before_generation);
            }
            AudioCmd::ReleaseFile { path, reply } => {
                let released = current.as_ref()
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
                            &shared_level, &effects, &playback_generation, &loopback_tx,
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
                let (
                    source,
                    paused,
                    rollback_position_ms,
                    clock,
                    loudness,
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

#[allow(clippy::too_many_arguments)]
fn switch_output_device_in_place(
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
        Arc::clone(shared_level), Arc::clone(effects), Arc::clone(&previous.shared.loudness),
        Arc::clone(playback_generation),
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

/// 输出设备失效后按当前位置原地重建会话
///
/// 同一首歌的响度统计沿用，重建前后音量一致。重建失败时再重试一次，
/// 两次都失败才放弃会话（由上层发出结束事件推进队列，不会静默卡住）。
#[allow(clippy::too_many_arguments)]
fn rebuild_session_in_place(
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
    let (mut decoder, segment_start_ms) = match make_decoder_with_start(&decoder_source, start_position_ms, use_byte_seek)
    {
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
    let audio_info = decoder.source_audio_info()
        .with_encoded_bitrate(source.encoded_byte_length(), decoder.total_duration());
    let start_position_ms = clamp_position(start_position_ms, duration_ms);
    // 字节跳转路径已经在目标附近顺序打开，不再走 format.seek（它会扫全文件）
    if start_position_ms > 0 && !use_byte_seek {
        decoder
            .try_seek(Duration::from_millis(start_position_ms))
            .map_err(|error| format!("Could not seek decoder: {error}"))?;
    } else if start_position_ms > 0 && use_byte_seek {
        let discarded_ms = segment_start_ms
            .map_or(0, |segment_start| discard_until_target(decoder.as_mut(), segment_start, start_position_ms));
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
        spawn_loudness_scan(&source, &shared.loudness, sample_rate, channels, expected_generation);
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
        "prepared {} on {}: {} Hz, {} ch, target={}ms, decoder={}ms, output={}ms, wait={}ms, loudness_wait={}ms, total={}ms",
        source.label(),
        output.name,
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

fn make_decoder_for_position(
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
                reader.header_bytes().and_then(|header| decoder::sniff(&header)),
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
fn discard_until_target(decoder: &mut dyn AudioDecoder, segment_start_ms: u64, target_ms: u64) -> u64 {
    let lead_ms = target_ms.saturating_sub(segment_start_ms);
    if lead_ms == 0 || lead_ms > MAX_VIRTUAL_BODY_LEAD_MS {
        return 0;
    }
    let samples = lead_ms
        .saturating_mul(u64::from(decoder.sample_rate()))
        / 1_000
        * u64::from(decoder.channels().max(1));
    let mut discarded = 0u64;
    while discarded < samples && decoder.next().is_some() {
        discarded += 1;
    }
    lead_ms
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
            let mut loudness = LoudnessMeter::new(Arc::clone(&shared.loudness));
            let measured_channels =
                loudness_channels(converter.source_channels(), shared.channels);

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

const LOUDNESS_SCAN_CANCELLED: &str = "loudness scan cancelled";
/// 整首扫描每解出这么多帧检查一次取消
const LOUDNESS_SCAN_CHECK_FRAMES: u64 = 4_096;
/// 快速估计在整首上均匀取这么多个窗口，合计只解码几十分之一的内容
const QUICK_ESTIMATE_WINDOWS: u32 = 12;
const QUICK_ESTIMATE_WINDOW: Duration = Duration::from_secs(1);
/// 新曲目起播时最多等快速估计到这个时刻（从开始准备会话算起）；本地起播目标 300 ms
const QUICK_ESTIMATE_BUDGET: Duration = Duration::from_millis(200);

/// 正在整首扫描的本地文件
///
/// 释放文件（删除、改写）前要等这些扫描线程关掉句柄，包括刚切走、还没来得及退出的上一首。
static FILE_SCANS: Mutex<Vec<FileScan>> = Mutex::new(Vec::new());

struct FileScan {
    path: PathBuf,
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

fn register_file_scan(path: &str, cancel: Arc<AtomicBool>, handle: JoinHandle<()>) {
    let mut scans = FILE_SCANS.lock().unwrap_or_else(PoisonError::into_inner);
    scans.retain(|scan| !scan.handle.is_finished());
    scans.push(FileScan { path: PathBuf::from(path), cancel, handle });
}

/// 停下这个文件上的整首扫描，返回前扫描线程都已退出
fn stop_file_scans(target: &Path) {
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
fn loudness_scan_source(source: &AudioSource, cancel: &Arc<AtomicBool>) -> Option<AudioSource> {
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
fn spawn_loudness_scan(
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
                &source, output_rate, output_channels, &thread_cancel, &track, generation, quick,
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

fn run_loudness_scan(
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
fn scan_track_loudness(
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
        if frames % LOUDNESS_SCAN_CHECK_FRAMES == 0 && cancel.load(Ordering::Acquire) {
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
fn estimate_track_loudness(
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
        let center = duration.mul_f64((f64::from(window) + 0.5) / f64::from(QUICK_ESTIMATE_WINDOWS));
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
fn wait_for_loudness_estimate(
    loudness: &TrackLoudness,
    deadline: Instant,
    playback_generation: &AtomicU64,
    expected_generation: u64,
    operation_generation: Option<&GenerationToken>,
    prepare_cancel: &AtomicBool,
) -> Result<(), String> {
    while loudness.estimate_pending() && Instant::now() < deadline {
        ensure_preparation_current(playback_generation, expected_generation, operation_generation)?;
        if prepare_cancel.load(Ordering::Acquire) {
            return Err("Timed out waiting for decoded audio".into());
        }
        thread::sleep(Duration::from_millis(2));
    }
    Ok(())
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
    let mut chain = OutputChain::new(&shared);
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                render_output(&callback_shared, output, &mut chain, &metrics::OUTPUT_METRICS);
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
struct OutputChain {
    stretcher: Stretcher,
    effects: EffectsProcessor,
    scratch: Vec<f32>,
    /// 时钟按整微秒推进，余下的零头留到下一次回调，长时间播放不累积偏差
    clock_remainder_us: f64,
}

impl OutputChain {
    fn new(shared: &PlaybackShared) -> Self {
        let channels = shared.channels.max(1);
        Self {
            stretcher: Stretcher::new(channels, shared.sample_rate),
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
/// 倍速（保持音调）和音效参数都在这里实时生效，改它们不需要重建解码会话。
/// 播放中途取空 ring 记一次欠载；解码已结束后自然排空不算。
fn render_output<T>(
    shared: &PlaybackShared,
    output: &mut [T],
    chain: &mut OutputChain,
    counters: &OutputMetrics,
) where
    T: SizedSample + FromSample<f32>,
{
    let OutputChain { stretcher, effects, scratch, clock_remainder_us } = chain;
    let started_ns = metrics::monotonic_ns();
    let silence = T::from_sample(0.0);
    if shared.paused.load(Ordering::Acquire) || shared.buffering.load(Ordering::Acquire) {
        output.fill(silence);
        counters.record_callback(0, 0, false, metrics::monotonic_ns().saturating_sub(started_ns));
        return;
    }

    let channels = shared.channels.max(1);
    let speed = shared.speed.load();
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
        let rendered = stretcher.render(&mut scratch[..frames * channels], speed, draining, &mut pull);
        let filled = rendered.frames * channels;
        effects.process(&mut scratch[..filled]);
        for (target, sample) in chunk[..filled].iter_mut().zip(scratch[..filled].iter().copied()) {
            // 音效全关时样本原样直通；NaN 的 clamp 还是 NaN，不能交给设备
            let value = sample * gain;
            *target = T::from_sample(if value.is_finite() { value.clamp(-1.0, 1.0) } else { 0.0 });
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
    let elapsed_us = media_frames * 1_000_000.0 / f64::from(shared.sample_rate) + *clock_remainder_us;
    let whole_us = elapsed_us.floor().max(0.0);
    *clock_remainder_us = elapsed_us - whole_us;
    if whole_us > 0.0 {
        shared.clock.position_us.fetch_add(whole_us as u64, Ordering::AcqRel);
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
struct AdoptedSeek {
    position_ms: u64,
    playback_generation: u64,
    seek_generation: u64,
    issued_ns: u64,
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
    fn listed_devices_switch_output_only_when_the_target_changes() {
        let available = vec!["speaker".to_string(), "headphones".to_string()];
        assert_eq!(super::output_device_to_switch(None, &available, Some("headphones"), Some("headphones")), None);
        assert_eq!(
            super::output_device_to_switch(None, &available, Some("headphones"), Some("speaker")).as_deref(),
            Some("headphones"),
        );
        assert_eq!(super::output_device_to_switch(Some("speaker"), &available, Some("headphones"), Some("speaker")), None);
        assert_eq!(
            super::output_device_to_switch(None, &available, Some("speaker"), None).as_deref(),
            Some("speaker"),
            "还没有设备档案时直接打开目标设备",
        );
        assert_eq!(super::output_device_to_switch(None, &[], None, Some("speaker")), None);
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
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn local_file_release_waits_for_decoder_before_file_operations() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("playing.aac");
        std::fs::write(&path, include_bytes!("fixtures/hls-silence.aac")).unwrap();
        let source = AudioSource::File(path.to_string_lossy().into_owned(), 192);
        let mut decoder = make_decoder_for_position(&source, 0, false).unwrap();
        assert!(decoder.next().is_some());
        let shared = Arc::new(super::PlaybackShared {
            ring: Arc::new(crate::audio::buffered::PcmRing::new(8)),
            channels: 1, sample_rate: 48_000,
            paused: std::sync::atomic::AtomicBool::new(true),
            buffering: std::sync::atomic::AtomicBool::new(false),
            finished: std::sync::atomic::AtomicBool::new(false),
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            volume: super::AtomicF32::new(1.0), fade_gain: super::AtomicF32::new(1.0),
            speed: super::AtomicF32::new(1.0),
            buffer_target_frames: std::sync::atomic::AtomicUsize::new(1),
            clock: Arc::new(PlaybackClock::new(0)),
            wake_lock: std::sync::Mutex::new(()), wake: std::sync::Condvar::new(),
            device_lost: std::sync::atomic::AtomicBool::new(false),
            first_frame_ns: Arc::new(AtomicU64::new(0)),
            stretch_buffered: std::sync::atomic::AtomicUsize::new(0),
            effects: crate::audio::effects::EffectsControl::new_shared(),
            loudness: crate::audio::effects::TrackLoudness::new_shared(),
        });
        let exited = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_shared = Arc::clone(&shared);
        let worker_exited = Arc::clone(&exited);
        let worker = std::thread::spawn(move || {
            while !worker_shared.cancelled.load(Ordering::Acquire) {
                let guard = worker_shared.wake_lock.lock().unwrap();
                let _ = worker_shared.wake.wait_timeout(guard, Duration::from_millis(20)).unwrap();
            }
            std::thread::sleep(Duration::from_millis(80));
            drop(decoder);
            worker_exited.store(true, Ordering::Release);
        });
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            assert!(std::fs::OpenOptions::new().write(true).share_mode(0).open(&path).is_err());
        }
        super::finish_decode_worker(&source, &shared, worker);
        let released = exited.load(Ordering::Acquire);
        // 失败时也让临时文件在解码线程退出后清理
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !exited.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(released, "file release must wait until the decoder has dropped its handle");
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            drop(std::fs::OpenOptions::new().write(true).share_mode(0).open(&path).unwrap());
        }
        let renamed = path.with_file_name("released.aac");
        std::fs::rename(&path, &renamed).unwrap();
        std::fs::remove_file(renamed).unwrap();
    }

    #[test]
    fn local_file_release_matches_only_the_requested_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("playing.aac");
        let other = directory.path().join("other.aac");
        std::fs::write(&path, b"audio").unwrap();
        std::fs::write(&other, b"audio").unwrap();
        let source = AudioSource::File(path.to_string_lossy().into_owned(), 0);
        assert!(super::matches_local_file(&source, &directory.path().join(".").join("playing.aac")));
        assert!(!super::matches_local_file(&source, &other));
        assert!(!super::matches_local_file(&AudioSource::Bytes(Arc::from([]), 0), &path));
        #[cfg(windows)]
        assert!(super::matches_local_file(&source, &directory.path().join("PLAYING.AAC")));
    }

    #[test]
    fn local_file_release_does_not_wait_for_a_remote_worker() {
        let buffer = crate::audio::growing::GrowingAudioBuffer::new();
        let source = AudioSource::growing(buffer.reader(), 0);
        let shared = Arc::new(super::PlaybackShared {
            ring: Arc::new(crate::audio::buffered::PcmRing::new(8)),
            channels: 1, sample_rate: 48_000,
            paused: std::sync::atomic::AtomicBool::new(false),
            buffering: std::sync::atomic::AtomicBool::new(true),
            finished: std::sync::atomic::AtomicBool::new(false),
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            volume: super::AtomicF32::new(1.0), fade_gain: super::AtomicF32::new(1.0),
            speed: super::AtomicF32::new(1.0),
            buffer_target_frames: std::sync::atomic::AtomicUsize::new(1),
            clock: Arc::new(PlaybackClock::new(0)),
            wake_lock: std::sync::Mutex::new(()), wake: std::sync::Condvar::new(),
            device_lost: std::sync::atomic::AtomicBool::new(false),
            first_frame_ns: Arc::new(AtomicU64::new(0)),
            stretch_buffered: std::sync::atomic::AtomicUsize::new(0),
            effects: crate::audio::effects::EffectsControl::new_shared(),
            loudness: crate::audio::effects::TrackLoudness::new_shared(),
        });
        let (resume, blocked) = mpsc::channel();
        let (exited, done) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = blocked.recv_timeout(Duration::from_secs(1));
            let _ = exited.send(());
        });
        super::finish_decode_worker(&source, &shared, worker);
        assert!(matches!(done.try_recv(), Err(mpsc::TryRecvError::Empty)));
        assert!(shared.cancelled.load(Ordering::Acquire));
        let _ = resume.send(());
        done.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    fn player_without_output(generation: u64) -> (super::PlayerEngine, mpsc::Receiver<AudioCmd>) {
        let (cmd_tx, receiver) = mpsc::channel();
        (super::PlayerEngine {
            cmd_tx,
            thread_alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            is_playing: true, volume: 1.0, speed: 1.0,
            current_path: Some("other.aac".into()), duration_ms: 192,
            shared_audio_level: crate::audio::analyzer::SharedAudioLevel::new(),
            effects: crate::audio::effects::EffectsControl::new_shared(),
            playback_generation: Arc::new(AtomicU64::new(generation)),
            seek_generation: Arc::new(AtomicU64::new(0)),
            transition_generation: Arc::new(AtomicU64::new(0)),
            loaded_generation: Some(generation), clock: Some(Arc::new(PlaybackClock::new(42))),
            audio_info: None,
        }, receiver)
    }

    #[test]
    fn local_file_release_reply_does_not_clear_a_newer_playback_session() {
        let (mut player, commands) = player_without_output(8);
        let request = player.request_file_release("previous.aac".into()).unwrap();
        let AudioCmd::ReleaseFile { path, reply } = commands.recv().unwrap() else {
            panic!("expected a file release command");
        };
        assert_eq!(path, "previous.aac");
        reply.send(Some(7)).unwrap();
        assert!(player.complete_file_release(request.wait().unwrap()));
        assert!(player.is_playing);
        assert_eq!(player.current_path.as_deref(), Some("other.aac"));
        assert_eq!(player.position_ms(), 42);
        assert!(!player.complete_file_release(None));
        assert!(player.complete_file_release(Some(8)));
        assert!(!player.is_playing);
        assert!(player.current_path.is_none());
        assert_eq!(player.position_ms(), 0);
    }

    #[test]
    fn local_file_release_invalidates_stale_start_without_superseding_a_new_request() {
        let (mut player, _) = player_without_output(7);
        super::invalidate_released_generation(&player.playback_generation, 7);
        assert_eq!(player.playback_generation.load(Ordering::Acquire), 8);
        let started = super::PlaybackStarted {
            duration_ms: 192, clock: Arc::new(PlaybackClock::new(0)), audio_info: super::SourceAudioInfo::default(),
        };
        assert!(player.complete_start(started, 7, "released.aac".into()).is_err());
        assert_eq!(player.current_path.as_deref(), Some("other.aac"));
        super::invalidate_released_generation(&player.playback_generation, 7);
        assert_eq!(player.playback_generation.load(Ordering::Acquire), 8);
    }

    #[test]
    fn stream_audio_info_commits_only_for_the_current_playback_generation() {
        let (mut player, _) = player_without_output(8);
        let decoder = crate::audio::remote::SymphoniaAudioDecoder::new(
            Box::new(std::io::Cursor::new(include_bytes!("fixtures/hls-silence.aac").as_slice())), None,
        ).unwrap();
        let source_audio_info = decoder.source_audio_info();
        assert!(player.playback_audio_info(8).is_none());
        let started = super::PlaybackStarted {
            duration_ms: 192, clock: Arc::new(PlaybackClock::new(0)),
            audio_info: source_audio_info.clone(),
        };
        player.complete_start(started, 8, "__remote__".into()).unwrap();
        let info = player.playback_audio_info(8).unwrap();
        assert_eq!(info, source_audio_info);
        assert!(player.playback_audio_info(7).is_none());
        player.playback_generation.store(9, Ordering::Release);
        assert!(player.playback_audio_info(8).is_none());
        assert!(player.playback_audio_info(9).is_none());
    }

    #[test]
    fn stream_audio_info_stop_and_file_release_remove_loaded_properties() {
        let (mut player, _) = player_without_output(8);
        let decoder = crate::audio::remote::SymphoniaAudioDecoder::new(
            Box::new(std::io::Cursor::new(include_bytes!("fixtures/hls-silence.aac").as_slice())), None,
        ).unwrap();
        let started = super::PlaybackStarted {
            duration_ms: 192, clock: Arc::new(PlaybackClock::new(0)),
            audio_info: decoder.source_audio_info(),
        };
        player.complete_start(started.clone(), 8, "__remote__".into()).unwrap();
        assert!(player.playback_audio_info(8).is_some());
        player.stop();
        assert!(player.playback_audio_info(8).is_none());
        player.complete_start(started, 8, "cached.audio".into()).unwrap();
        assert!(player.complete_file_release(Some(8)));
        assert!(player.playback_audio_info(8).is_none());
    }

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
            first_frame_ns: Arc::new(AtomicU64::new(0)),
            stretch_buffered: std::sync::atomic::AtomicUsize::new(0),
            effects: crate::audio::effects::EffectsControl::new_shared(),
            loudness: crate::audio::effects::TrackLoudness::new_shared(),
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
    fn surround_downmix_keeps_center_dialogue_and_never_clips() {
        // 5.1 只有中置有信号：旧做法只取前两个声道，对白会整段消失
        let center_only = [0.0, 0.0, 0.5, 0.0, 0.0, 0.0];
        let stereo = super::Downmix::new(6, 2).expect("5.1 to stereo needs a downmix");
        let (left, right) = (stereo.sample(&center_only, 0), stereo.sample(&center_only, 1));
        assert!(left > 0.1 && (left - right).abs() < 1e-6, "L={left} R={right}");

        let lfe_only = [0.0, 0.0, 0.0, 0.9, 0.0, 0.0];
        assert_eq!(stereo.sample(&lfe_only, 0), 0.0, "ITU 默认下混不计入低频声道");

        let full_scale = [1.0f32; 6];
        for output in 0..2 {
            assert!(stereo.sample(&full_scale, output) <= 1.0 + 1e-6, "满幅输入下混后不得削波");
        }
        let mono = super::Downmix::new(6, 1).expect("5.1 to mono needs a downmix");
        assert!(mono.sample(&center_only, 0) > 0.1);
        assert!(mono.sample(&full_scale, 0) <= 1.0 + 1e-6);

        assert!(super::Downmix::new(2, 2).is_none(), "立体声不需要下混");
        assert!(super::Downmix::new(2, 1).is_none(), "立体声转单声道仍按平均处理");
        assert!(super::Downmix::new(6, 6).is_none(), "输出声道够多时直接映射");
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
                issued_ns: 0,
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
            .send(AudioCmd::SetVolume(0.5))
            .expect("queue non-seek command");
        sender
            .send(AudioCmd::Seek {
                position_ms: 4_606_826,
                playback_generation: 7,
                seek_generation: 9,
                issued_ns: 0,
                reply,
            })
            .expect("queue newer seek behind noise");
        let mut deferred = VecDeque::new();

        let adopted = wait_for_newer_seek(&receiver, &mut deferred, Duration::from_millis(200))
            .expect("seek behind noise must still be adopted");

        assert_eq!(adopted.position_ms, 4_606_826);
        assert_eq!(deferred.len(), 1);
        assert!(matches!(deferred.front(), Some(AudioCmd::SetVolume(_))));
    }

    /// 宽限超时（极端调度竞态下新 Seek 未入队）：返回 None，
    /// 调用方走回滚重建，迟到的 Seek 会在恢复出的会话上正常执行；
    /// 期间收到的非 Seek 命令仍保序暂存不丢
    #[test]
    fn wait_for_newer_seek_times_out_on_empty_queue() {
        let (sender, receiver) = mpsc::channel::<AudioCmd>();
        sender
            .send(AudioCmd::SetVolume(0.5))
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
                issued_ns: 0,
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
                issued_ns: 10,
                reply: first_reply,
            })
            .is_ok());
        assert!(sender
            .send(AudioCmd::Seek {
                position_ms: 800,
                playback_generation: 7,
                seek_generation: 2,
                issued_ns: 20,
                reply: second_reply,
            })
            .is_ok());
        assert!(sender
            .send(AudioCmd::Seek {
                position_ms: 1_600,
                playback_generation: 8,
                seek_generation: 3,
                issued_ns: 30,
                reply: third_reply,
            })
            .is_ok());
        let mut latest = 0;
        let mut generation = 7;
        let mut seek_generation = 0;
        let mut issued_ns = 0;
        let (initial_reply, initial_result) = mpsc::channel();
        let mut reply = initial_reply;
        let mut deferred = VecDeque::new();

        take_latest_seek(
            &mut latest,
            &mut generation,
            &mut seek_generation,
            &mut issued_ns,
            &mut reply,
            &receiver,
            &mut deferred,
        );

        assert_eq!(latest, 1_600);
        assert_eq!(generation, 8);
        assert_eq!(seek_generation, 3);
        assert_eq!(issued_ns, 30, "seek 耗时要从最后一次拖动算起");
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

    fn render_test_shared(channels: usize, capacity_samples: usize) -> Arc<super::PlaybackShared> {
        Arc::new(super::PlaybackShared {
            ring: Arc::new(crate::audio::buffered::PcmRing::new(capacity_samples)),
            channels, sample_rate: 48_000,
            paused: std::sync::atomic::AtomicBool::new(false),
            buffering: std::sync::atomic::AtomicBool::new(false),
            finished: std::sync::atomic::AtomicBool::new(false),
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            volume: super::AtomicF32::new(1.0), fade_gain: super::AtomicF32::new(1.0),
            speed: super::AtomicF32::new(1.0),
            buffer_target_frames: std::sync::atomic::AtomicUsize::new(1),
            clock: Arc::new(PlaybackClock::new(0)),
            wake_lock: std::sync::Mutex::new(()), wake: std::sync::Condvar::new(),
            device_lost: std::sync::atomic::AtomicBool::new(false),
            first_frame_ns: Arc::new(AtomicU64::new(0)),
            stretch_buffered: std::sync::atomic::AtomicUsize::new(0),
            effects: crate::audio::effects::EffectsControl::new_shared(),
            loudness: crate::audio::effects::TrackLoudness::new_shared(),
        })
    }

    /// 与 build_typed_stream 里一样的回调状态
    struct CallbackState {
        chain: super::OutputChain,
    }

    impl CallbackState {
        fn new(shared: &super::PlaybackShared) -> Self {
            Self { chain: super::OutputChain::new(shared) }
        }

        fn render(&mut self, shared: &super::PlaybackShared, output: &mut [f32], counters: &crate::audio::metrics::OutputMetrics) {
            super::render_output(shared, output, &mut self.chain, counters);
        }
    }

    #[test]
    fn render_output_counts_a_mid_buffer_underrun_once() {
        let shared = render_test_shared(2, 64);
        for index in 0..3 {
            let value = 0.1 * (index + 1) as f32;
            assert!(shared.ring.try_push_frame(&[value, -value]));
        }
        let counters = crate::audio::metrics::OutputMetrics::new();
        let mut callback = CallbackState::new(&shared);
        let mut output = [1.0f32; 16];

        callback.render(&shared, &mut output, &counters);

        assert!((output[4] - 0.3).abs() < 1e-6 && (output[5] + 0.3).abs() < 1e-6);
        assert!(output[6..].iter().all(|sample| *sample == 0.0), "取空后整块补静音");
        let snapshot = counters.snapshot();
        assert_eq!((snapshot.underruns, snapshot.underrun_frames, snapshot.rendered_frames), (1, 5, 3));
        assert!(shared.buffering.load(Ordering::Acquire), "欠载后进入重缓冲");
        assert_eq!(shared.clock.position_us.load(Ordering::Acquire), 62);

        callback.render(&shared, &mut output, &counters);
        assert_eq!(counters.snapshot().underruns, 1, "重缓冲期间的静音不重复计数");
    }

    #[test]
    fn render_output_does_not_count_the_drain_after_the_decoder_finished() {
        let shared = render_test_shared(2, 64);
        assert!(shared.ring.try_push_frame(&[0.5, 0.5]));
        shared.finished.store(true, Ordering::Release);
        let counters = crate::audio::metrics::OutputMetrics::new();
        let mut callback = CallbackState::new(&shared);
        let mut output = [0.0f32; 8];

        callback.render(&shared, &mut output, &counters);

        let snapshot = counters.snapshot();
        assert_eq!((snapshot.underruns, snapshot.underrun_frames, snapshot.rendered_frames), (0, 0, 1));
        assert_eq!(shared.stretch_buffered.load(Ordering::Acquire), 0, "播完时变速器里不能有残留");
    }

    #[test]
    fn render_output_stays_silent_and_uncounted_while_paused() {
        let shared = render_test_shared(2, 64);
        assert!(shared.ring.try_push_frame(&[0.5, 0.5]));
        shared.paused.store(true, Ordering::Release);
        let counters = crate::audio::metrics::OutputMetrics::new();
        let mut callback = CallbackState::new(&shared);
        let mut output = [1.0f32; 8];

        callback.render(&shared, &mut output, &counters);

        assert!(output.iter().all(|sample| *sample == 0.0));
        assert_eq!(shared.ring.readable_samples(), 2, "暂停时不消费 ring");
        assert_eq!(counters.snapshot().underruns, 0);
        assert_eq!(shared.first_frame_ns.load(Ordering::Acquire), 0);
    }

    #[test]
    fn render_output_records_only_the_first_frame_time() {
        let shared = render_test_shared(1, 64);
        for _ in 0..8 {
            assert!(shared.ring.try_push_frame(&[0.25]));
        }
        let counters = crate::audio::metrics::OutputMetrics::new();
        let mut callback = CallbackState::new(&shared);
        let mut output = [0.0f32; 4];

        callback.render(&shared, &mut output, &counters);
        let first = shared.first_frame_ns.load(Ordering::Acquire);
        assert_ne!(first, 0);
        std::thread::sleep(Duration::from_millis(2));
        callback.render(&shared, &mut output, &counters);

        assert_eq!(shared.first_frame_ns.load(Ordering::Acquire), first);
        assert_eq!(counters.snapshot().rendered_frames, 8);
    }

    /// 一起听的软同步把倍速调到 1.05：回调里立即按新倍速消费 ring、推进时钟，不停下会话
    #[test]
    fn speed_changes_take_effect_inside_the_callback_without_a_gap() {
        let shared = render_test_shared(1, 96_000);
        for index in 0..96_000 {
            let sample = (0.5 * (std::f64::consts::TAU * 440.0 * index as f64 / 48_000.0).sin()) as f32;
            assert!(shared.ring.try_push_frame(&[sample]));
        }
        let counters = crate::audio::metrics::OutputMetrics::new();
        let mut callback = CallbackState::new(&shared);
        let mut output = [0.0f32; 480];
        for _ in 0..20 {
            callback.render(&shared, &mut output, &counters);
        }
        let clock_before = shared.clock.position_us.load(Ordering::Acquire);
        let ring_before = shared.ring.readable_samples();
        assert_eq!(clock_before, 200_000, "1 倍速时 9 600 帧对应 200 ms");

        shared.speed.store(1.05);
        for _ in 0..100 {
            callback.render(&shared, &mut output, &counters);
        }
        let clock_advance = shared.clock.position_us.load(Ordering::Acquire) - clock_before;
        let consumed = ring_before - shared.ring.readable_samples();
        assert!((clock_advance as f64 - 1_050_000.0).abs() < 25_000.0, "1 秒输出应推进约 1.05 秒媒体时间: {clock_advance}");
        assert!(consumed > 48_000, "倍速后每秒应消费多于 48 000 帧: {consumed}");
        assert_eq!(counters.snapshot().underruns, 0, "变速不能造成断音");
        assert!(output.iter().any(|sample| sample.abs() > 0.1), "变速后仍在出声");
    }

    /// 变速中欠载：变速器里已取出的前瞻要保留，恢复后接着放；播完时时钟与输入时长一致
    #[test]
    fn an_underrun_while_stretching_keeps_the_buffered_audio() {
        let total = 48_000usize;
        let shared = render_test_shared(1, total);
        let sample = |index: usize| (0.5 * (std::f64::consts::TAU * 440.0 * index as f64 / 48_000.0).sin()) as f32;
        for index in 0..9_600 {
            assert!(shared.ring.try_push_frame(&[sample(index)]));
        }
        shared.speed.store(1.25);
        let counters = crate::audio::metrics::OutputMetrics::new();
        let mut callback = CallbackState::new(&shared);
        let mut output = [0.0f32; 480];
        for _ in 0..100 {
            if shared.buffering.load(Ordering::Acquire) {
                break;
            }
            callback.render(&shared, &mut output, &counters);
        }
        assert!(shared.buffering.load(Ordering::Acquire), "ring 取空后应进入重缓冲");
        assert!(shared.stretch_buffered.load(Ordering::Acquire) > 0, "欠载时变速器里还留着前瞻");

        for index in 9_600..total {
            assert!(shared.ring.try_push_frame(&[sample(index)]));
        }
        shared.buffering.store(false, Ordering::Release);
        shared.finished.store(true, Ordering::Release);
        for _ in 0..200 {
            callback.render(&shared, &mut output, &counters);
            if shared.ring.readable_samples() == 0 && shared.stretch_buffered.load(Ordering::Acquire) == 0 {
                break;
            }
        }
        assert_eq!(shared.stretch_buffered.load(Ordering::Acquire), 0, "播完时变速器里不能有残留");
        let position_us = shared.clock.position_us.load(Ordering::Acquire) as f64;
        assert!((position_us - 1_000_000.0).abs() < 1_000.0, "1 秒输入播完，时钟应走到 1 秒: {position_us}");
    }

    /// 每次回调的媒体时长不是整微秒（441 帧 = 9 187.5 µs）：零头要留到下次，不能逐次截掉
    #[test]
    fn the_clock_keeps_sub_microsecond_remainders_across_callbacks() {
        let shared = render_test_shared(1, 441 * 100);
        for _ in 0..441 * 100 {
            assert!(shared.ring.try_push_frame(&[0.25]));
        }
        let counters = crate::audio::metrics::OutputMetrics::new();
        let mut callback = CallbackState::new(&shared);
        let mut output = [0.0f32; 441];
        for _ in 0..100 {
            callback.render(&shared, &mut output, &counters);
        }
        assert_eq!(shared.clock.position_us.load(Ordering::Acquire), 918_750);
    }

    /// 播放中打开音量均衡：回调下一块就开始过渡，会话不重建、时钟连续、不断音
    #[test]
    fn normalization_toggles_inside_the_callback_without_a_rebuild() {
        let shared = render_test_shared(2, 2 * 48_000);
        let mut meter = crate::audio::effects::LoudnessMeter::new(Arc::clone(&shared.loudness));
        for _ in 0..48_000 {
            assert!(shared.ring.try_push_frame(&[0.01, 0.01]));
            meter.observe(&[0.01, 0.01]);
        }
        meter.flush();
        let counters = crate::audio::metrics::OutputMetrics::new();
        let mut callback = CallbackState::new(&shared);
        let mut output = [0.0f32; 960];
        callback.render(&shared, &mut output, &counters);
        assert!(output.iter().all(|sample| *sample == 0.01), "关着时逐位直通");

        shared.effects.update(|settings| settings.normalize_volume = true);
        for _ in 0..10 {
            callback.render(&shared, &mut output, &counters);
        }

        let boosted = output[output.len() - 1] / 0.01;
        assert!((boosted - 1.995).abs() < 0.01, "打开后应抬到 +6 dB: {boosted}");
        assert_eq!(shared.clock.position_us.load(Ordering::Acquire), 110_000, "11 块各 10 ms，时钟连续");
        let snapshot = counters.snapshot();
        assert_eq!(snapshot.underruns, 0);
        assert_eq!(snapshot.normalization_gain_mb, 600, "指标里能看到当前增益");
    }

    #[test]
    fn loudness_is_measured_only_on_channels_that_carry_the_source() {
        assert_eq!(super::loudness_channels(2, 8), 2, "立体声接 7.1：多出来的零声道不算");
        assert_eq!(super::loudness_channels(1, 2), 1, "单声道复制到两边，统计一份");
        assert_eq!(super::loudness_channels(6, 2), 2, "5.1 缩混成立体声后两个声道都有内容");
        assert_eq!(super::loudness_channels(8, 8), 8);
        assert_eq!(super::loudness_channels(0, 2), 1);
    }

    /// 16 bit 立体声 440 Hz WAV，由若干段（幅度, 秒数）拼成；同时返回解码后应得到的样本
    fn sine_sections_wav(sample_rate: u32, sections: &[(f64, u32)]) -> (Vec<u8>, Vec<f32>) {
        let mut samples = Vec::new();
        let mut index = 0usize;
        for &(amplitude, seconds) in sections {
            for _ in 0..(sample_rate * seconds) {
                let phase = std::f64::consts::TAU * 440.0 * index as f64 / f64::from(sample_rate);
                let value = (amplitude * phase.sin() * 32_767.0).round() as i16;
                samples.extend([value, value]);
                index += 1;
            }
        }
        let data_len = (samples.len() * 2) as u32;
        let mut bytes = Vec::with_capacity(44 + data_len as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(sample_rate * 4).to_le_bytes());
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for sample in &samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        let decoded = samples.iter().map(|sample| f32::from(*sample) / 32_768.0).collect();
        (bytes, decoded)
    }

    /// 前一半安静、后一半响
    fn quiet_then_loud_wav(sample_rate: u32, seconds_each: u32) -> (Vec<u8>, Vec<f32>) {
        sine_sections_wav(sample_rate, &[(0.05, seconds_each), (0.5, seconds_each)])
    }

    fn wav_file(directory: &tempfile::TempDir, name: &str, bytes: &[u8]) -> AudioSource {
        let path = directory.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        AudioSource::File(path.to_string_lossy().into_owned(), 0)
    }

    fn target_db(stats: &crate::audio::effects::LoudnessStats) -> f64 {
        20.0 * stats.target_gain().unwrap().log10()
    }

    /// 安静-响-安静的 40 秒曲子：抽样估计与完整扫描相差不到 0.5 dB，
    /// 而只看开头 10 秒的估计差了 6 dB 以上
    #[test]
    fn quick_estimate_lands_close_to_the_full_scan() {
        let (wav, decoded) = sine_sections_wav(24_000, &[(0.05, 10), (0.5, 20), (0.05, 10)]);
        let directory = tempfile::tempdir().unwrap();
        let source = wav_file(&directory, "sections.wav", &wav);
        let cancel = AtomicBool::new(false);

        let estimate = super::estimate_track_loudness(&source, 24_000, 2, &cancel)
            .unwrap()
            .expect("40 秒的本地文件应给出估计");
        let (full, _) = super::scan_track_loudness(&source, 24_000, 2, &cancel).unwrap();

        assert!(
            (target_db(&estimate) - target_db(&full)).abs() < 0.5,
            "估计 {:.2} dB，整首 {:.2} dB",
            target_db(&estimate),
            target_db(&full),
        );
        assert!(estimate.samples() < full.samples() / 3, "估计只解码一小部分");
        let mut opening = crate::audio::effects::LoudnessScanner::default();
        opening.observe(&decoded[..decoded.len() / 4]);
        assert!(target_db(&opening.finish()) - target_db(&full) > 6.0);
    }

    /// 在真实曲库上量快速估计与完整扫描的耗时和差距（发布构建才有参考价值）：
    /// `NERI_LOUDNESS_BENCH_DIR=<目录> cargo test --release --lib -- --ignored loudness_scan_timing --nocapture`
    #[test]
    #[ignore = "needs NERI_LOUDNESS_BENCH_DIR pointing at a music folder"]
    fn loudness_scan_timing_on_a_local_library() {
        let directory = std::env::var("NERI_LOUDNESS_BENCH_DIR")
            .expect("set NERI_LOUDNESS_BENCH_DIR to a folder with audio files");
        const AUDIO: [&str; 9] = ["mp3", "flac", "m4a", "aac", "ogg", "opus", "wav", "ape", "wv"];
        let mut files: Vec<_> = std::fs::read_dir(directory)
            .unwrap()
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| AUDIO.contains(&extension.to_ascii_lowercase().as_str()))
            })
            .collect();
        files.sort();
        let cancel = AtomicBool::new(false);
        for path in files.iter().take(8) {
            let source = AudioSource::File(path.to_string_lossy().into_owned(), 0);
            let started = std::time::Instant::now();
            let estimate = super::estimate_track_loudness(&source, 48_000, 2, &cancel);
            let estimate_ms = started.elapsed().as_millis();
            let started = std::time::Instant::now();
            let full = super::scan_track_loudness(&source, 48_000, 2, &cancel);
            let full_ms = started.elapsed().as_millis();
            let estimate_db = estimate.ok().flatten().map(|stats| target_db(&stats));
            let (full_db, seconds) = match &full {
                Ok((stats, frames)) => (Some(target_db(stats)), *frames as f64 / 48_000.0),
                Err(_) => (None, 0.0),
            };
            println!(
                "{:>6.1}s estimate={estimate_db:>8.2?} dB in {estimate_ms:>5} ms, full={full_db:>8.2?} dB in {full_ms:>6} ms  {}",
                seconds,
                path.file_name().unwrap_or_default().to_string_lossy(),
            );
        }
    }

    #[test]
    fn short_tracks_skip_the_quick_estimate() {
        let (wav, _) = quiet_then_loud_wav(24_000, 2);
        let directory = tempfile::tempdir().unwrap();
        let source = wav_file(&directory, "short.wav", &wav);

        let estimate =
            super::estimate_track_loudness(&source, 24_000, 2, &AtomicBool::new(false)).unwrap();

        assert!(estimate.is_none());
    }

    /// 起播等估计：估计一到就返回，等不到就在截止时刻放弃，播放请求作废时立刻退出
    #[test]
    fn waiting_for_the_estimate_is_bounded() {
        let generation = AtomicU64::new(3);
        let not_cancelled = AtomicBool::new(false);
        let track = crate::audio::effects::TrackLoudness::new_shared();
        track.set_estimate_pending(true);
        let landing = Arc::clone(&track);
        let started = std::time::Instant::now();
        let lander = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            landing.set_estimate_pending(false);
        });
        super::wait_for_loudness_estimate(
            &track, started + Duration::from_secs(5), &generation, 3, None, &not_cancelled,
        )
        .unwrap();
        lander.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(2), "估计到了就该返回");

        track.set_estimate_pending(true);
        let started = std::time::Instant::now();
        super::wait_for_loudness_estimate(
            &track, started + Duration::from_millis(40), &generation, 3, None, &not_cancelled,
        )
        .unwrap();
        assert!(started.elapsed() >= Duration::from_millis(40));

        generation.store(4, Ordering::Release);
        assert!(super::wait_for_loudness_estimate(
            &track,
            std::time::Instant::now() + Duration::from_secs(5),
            &generation,
            3,
            None,
            &not_cancelled,
        )
        .is_err());
    }

    /// 整首扫描读完整个文件，结果与按播放口径统计同样的样本一致，并把目标固定下来；
    /// 只看开头安静段的估计比整首高得多——这就是播放时统计在开头的漂移
    #[test]
    fn loudness_scan_measures_the_whole_file_like_playback() {
        let (wav, decoded) = quiet_then_loud_wav(48_000, 2);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("scan.wav");
        std::fs::write(&path, &wav).unwrap();
        let source = AudioSource::File(path.to_string_lossy().into_owned(), 0);

        let (stats, frames) =
            super::scan_track_loudness(&source, 48_000, 2, &AtomicBool::new(false)).unwrap();

        assert_eq!(frames, 192_000);
        let mut expected = crate::audio::effects::LoudnessScanner::default();
        expected.observe(&decoded);
        let target = stats.target_gain().unwrap();
        assert!((target - expected.finish().target_gain().unwrap()).abs() < 1e-4);
        let mut opening = crate::audio::effects::LoudnessScanner::default();
        opening.observe(&decoded[..decoded.len() / 2]);
        let early = opening.finish().target_gain().unwrap();
        assert!(early / target > 2.0, "开头的估计 {early:.3} 应远高于整首 {target:.3}");

        let track = crate::audio::effects::TrackLoudness::new_shared();
        track.set_estimate_pending(true);
        super::run_loudness_scan(
            &source, 48_000, 2, &AtomicBool::new(false), &Arc::downgrade(&track), 1, true,
        );
        assert!(track.is_complete());
        assert!(!track.estimate_pending(), "跳过估计时也要放行等待中的起播");
        assert_eq!(track.analyzed_samples(), stats.samples());
    }

    /// 字节跳转从分片开头解码：丢掉分片起点到目标之间的音频后，下一个样本正好是目标位置
    #[test]
    fn virtual_body_start_discards_audio_before_the_target() {
        let (wav, expected) = sine_sections_wav(8_000, &[(0.5, 3)]);
        let source = AudioSource::Bytes(Arc::from(wav), 0);
        let mut decoder = super::make_decoder_for_position(&source, 0, false).unwrap();

        // 分片从 1s 开始，目标 2.25s：丢 1.25s
        assert_eq!(super::discard_until_target(decoder.as_mut(), 1_000, 2_250), 1_250);

        let target_sample = 8_000 * 1_250 / 1_000 * 2;
        let next: Vec<f32> = decoder.by_ref().take(4).collect();
        assert_eq!(next, expected[target_sample..target_sample + 4]);
    }

    #[test]
    fn virtual_body_start_keeps_audio_when_the_segment_start_is_implausible() {
        let (wav, expected) = sine_sections_wav(8_000, &[(0.5, 1)]);
        let source = AudioSource::Bytes(Arc::from(wav), 0);
        let mut decoder = super::make_decoder_for_position(&source, 0, false).unwrap();

        assert_eq!(super::discard_until_target(decoder.as_mut(), 0, 60_000), 0, "领先超过 20s 说明落点不对");
        assert_eq!(super::discard_until_target(decoder.as_mut(), 5_000, 4_000), 0, "分片晚于目标时没有可丢的");
        assert_eq!(decoder.next(), Some(expected[0]));
    }

    #[test]
    fn loudness_scan_stops_when_cancelled() {
        let (wav, _) = quiet_then_loud_wav(48_000, 2);
        let source = AudioSource::Bytes(Arc::from(wav), 0);

        let result = super::scan_track_loudness(&source, 48_000, 2, &AtomicBool::new(true));

        assert_eq!(result.unwrap_err(), super::LOUDNESS_SCAN_CANCELLED);
    }

    /// 边下边播要等下载完整才算扫完；下载中断时读到的结尾不是曲尾，不能拿半首当整首
    #[test]
    fn growing_scan_finishes_only_with_the_whole_download() {
        let (wav, _) = quiet_then_loud_wav(48_000, 1);
        let cancel = Arc::new(AtomicBool::new(false));
        let complete = crate::audio::growing::GrowingAudioBuffer::new();
        complete.append(&wav);
        complete.finish();
        let source =
            super::loudness_scan_source(&AudioSource::growing(complete.reader(), 0), &cancel)
                .unwrap();
        let (_, frames) = super::scan_track_loudness(&source, 48_000, 2, &cancel).unwrap();
        assert_eq!(frames, 96_000);

        let interrupted = crate::audio::growing::GrowingAudioBuffer::new();
        interrupted.append(&wav[..wav.len() / 2]);
        let source =
            super::loudness_scan_source(&AudioSource::growing(interrupted.reader(), 0), &cancel)
                .unwrap();
        let aborter = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            interrupted.abort();
        });
        let result = super::scan_track_loudness(&source, 48_000, 2, &cancel);
        aborter.join().unwrap();

        assert!(result.is_err(), "下载中断时不能当成整首: {result:?}");
    }

    /// 释放文件要等这个文件上的扫描线程都退出，别的文件上的扫描不受影响
    #[test]
    fn releasing_a_file_waits_for_its_loudness_scans_only() {
        let directory = tempfile::tempdir().unwrap();
        let released = directory.path().join("released.flac");
        let kept = directory.path().join("kept.flac");
        std::fs::write(&released, b"audio").unwrap();
        std::fs::write(&kept, b"audio").unwrap();
        let spawn_scan = |path: &Path| {
            let cancel = Arc::new(AtomicBool::new(false));
            let exited = Arc::new(AtomicBool::new(false));
            let (thread_cancel, thread_exited) = (Arc::clone(&cancel), Arc::clone(&exited));
            let handle = std::thread::spawn(move || {
                while !thread_cancel.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                std::thread::sleep(Duration::from_millis(20));
                thread_exited.store(true, Ordering::Release);
            });
            super::register_file_scan(&path.to_string_lossy(), Arc::clone(&cancel), handle);
            (cancel, exited)
        };
        let (_, released_exited) = spawn_scan(&released);
        let (kept_cancel, kept_exited) = spawn_scan(&kept);

        super::stop_file_scans(&released);

        assert!(released_exited.load(Ordering::Acquire), "返回前扫描线程必须已经退出");
        assert!(!kept_cancel.load(Ordering::Acquire), "别的文件上的扫描不受影响");
        super::stop_file_scans(&kept);
        assert!(kept_exited.load(Ordering::Acquire));
    }
}
