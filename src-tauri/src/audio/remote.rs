use reqwest::header::{CONTENT_RANGE, RANGE, REFERER, USER_AGENT};
use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use symphonia::core::audio::{AudioBufferRef, SampleBuffer, SignalSpec};
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, SeekedTo};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::{self, Time, TimeBase};
use symphonia::default::{get_codecs, get_probe};

use crate::error::{AppError, AppResult};
use crate::audio::pcm::{PcmSeekError, PcmSource};

const REMOTE_INITIAL_BLOCK_BYTES: u64 = 128 * 1024;
/// 超长媒体首包加大，尽量一次拿齐 moov + 起始音频
const REMOTE_LONG_INITIAL_BLOCK_BYTES: u64 = 512 * 1024;
const REMOTE_FETCH_BLOCK_BYTES: u64 = 256 * 1024;
/// 预取专用块：慢链路上每个请求的往返/慢启动开销占大头（实机 256KB 块耗时
/// 600-4000ms），顺序预取用大块摊薄开销；需求读与 seek 仍用 256KB 保低延迟
const REMOTE_PREFETCH_BLOCK_BYTES: u64 = 512 * 1024;
/// 预取流水线并发数：单流有效吞吐可低于码率（实机 65-430KB/s < 356KB/s），
/// 多条并发请求掩盖单请求延迟；受 prefetch_target 窗口约束不会过量拉取
const REMOTE_PREFETCH_CONCURRENCY: usize = 3;
const REMOTE_FRAGMENTED_SEEK_BLOCK_BYTES: u64 = 8 * 1024 * 1024;
const REMOTE_DECODER_BUFFER_BYTES: usize = 128 * 1024;
/// virtual-body 远程 demuxer 缓冲：需覆盖单段 moof+mdat 回跳
const REMOTE_VIRTUAL_DECODER_BUFFER_BYTES: usize = 2 * 1024 * 1024;
const REMOTE_MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;
// 超长媒体按 bitrate 估算时 low/target 会落到 min；抬高下限保证首帧 moov+音频够解码
const REMOTE_PREFETCH_LOW_WATER_MS: u64 = 5_000;
const REMOTE_PREFETCH_TARGET_MS: u64 = 15_000;
const REMOTE_PREFETCH_MIN_LOW_WATER_BYTES: u64 = 256 * 1024;
const REMOTE_PREFETCH_MAX_LOW_WATER_BYTES: u64 = 8 * 1024 * 1024;
const REMOTE_PREFETCH_MIN_TARGET_BYTES: u64 = 1024 * 1024;
const REMOTE_PREFETCH_MAX_TARGET_BYTES: u64 = 16 * 1024 * 1024;
const REMOTE_PREFETCH_FALLBACK_LOW_WATER_BYTES: u64 = 1024 * 1024;
const REMOTE_PREFETCH_FALLBACK_TARGET_BYTES: u64 = 4 * 1024 * 1024;
/// ≥45 分钟按「长内容」处理：初始不向 demuxer 暴露 seekable，避免扫文件尾找 moov
const REMOTE_LONG_FORM_DURATION_MS: u64 = 45 * 60 * 1000;
/// 长内容额外预取尾部（moov 常在文件尾），seek 时 demuxer 才能秒开索引
const REMOTE_LONG_FORM_TAIL_BYTES: u64 = 1024 * 1024;
const MAX_DECODE_RETRIES: usize = 3;
/// virtual-body 中段打开后跳过坏包直到关键帧的上限
const MAX_VIRTUAL_BODY_SKIP_PACKETS: usize = 256;
/// virtual-body 拼接 demux：最多吞掉的 moov 假样本数（stco 指向文件头 mdat）
const MAX_VIRTUAL_BODY_SKIP_MOOV_SAMPLES: usize = 4096;
// 超长 YouTube 首包探测 2s 容易假失败，放宽到 6s
const REMOTE_PROBE_TIMEOUT: Duration = Duration::from_secs(6);
/// per-request 超时下限：15s 在实机上被 512KB 块贴脸打穿（成功请求已到
/// 12.4s），抬到 20s 并按块大小/吞吐向上缩放，避免"块越大越必超时"
const REMOTE_REQUEST_TIMEOUT_FLOOR: Duration = Duration::from_secs(20);
const REMOTE_REQUEST_TIMEOUT_CEILING: Duration = Duration::from_secs(60);
/// 自适应块下限/上限：慢网折半到 64KB 仍拉不动就该报错了；
/// 上限 1MB 防止快网把单请求撑到超时边缘
const REMOTE_MIN_BLOCK_BYTES: u64 = 64 * 1024;
const REMOTE_ADAPTIVE_MAX_BLOCK_BYTES: u64 = 1024 * 1024;
/// 自适应选块目标：按近期吞吐估计，块应能在 ~8s 内完成（远小于超时预算）
const REMOTE_BLOCK_FILL_TARGET_SECS: u64 = 8;
/// 吞吐 EWMA 新样本权重（百分数）：0.3 兼顾收敛速度与抗单样本抖动
const THROUGHPUT_EWMA_NEW_WEIGHT_PERCENT: u64 = 30;
/// 失败样本至少跑满该时长才计入吞吐估计：瞬时连接错误会注入天价上界样本
const THROUGHPUT_FAILURE_MIN_ELAPSED_MS: u64 = 3_000;
/// 需求读连续失败上限：超时/传输中断类错误折半重试，
/// 连续失败到该次数才向解码层报 fatal（播放侧表现为 rebuffering 而非死亡）
const REMOTE_DEMAND_MAX_ATTEMPTS: usize = 5;
/// 需求读重试退避基数（按次数线性放大）
const REMOTE_DEMAND_RETRY_BACKOFF_MS: u64 = 150;
const CACHE_MARKER_VERSION: &str = "v2";
const CACHE_MIN_DURATION_GAP_MS: u64 = 5_000;
const CACHE_MIN_DURATION_RATIO_PERCENT: u64 = 85;
const STALE_CACHE_PART_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_VALIDATED_CACHE_STAMPS: usize = 4_096;
const PLAYBACK_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// 按 URL 选择拉流 User-Agent.
/// YouTube googlevideo CDN 校验拉流 UA 与直链 `c=` 客户端一致, 不一致直接 403;
/// 因此 googlevideo 直链按铸造它的客户端选择匹配 UA (对齐 Android), 其它平台用桌面 Chrome UA.
fn playback_user_agent(url: &str) -> &'static str {
    if url.contains("googlevideo.com") || url.contains("youtube.com") {
        crate::api::youtube::playback::stream_user_agent_for_url(url)
    } else {
        PLAYBACK_USER_AGENT
    }
}

fn summarize_remote_error(error: &dyn std::fmt::Display) -> String {
    let mut message = error.to_string();
    for scheme in ["https://", "http://"] {
        while let Some(start) = message.find(scheme) {
            let tail = &message[start..];
            let end = tail
                .find(|value: char| value.is_whitespace() || matches!(value, ')' | ']' | '}'))
                .map(|offset| start + offset)
                .unwrap_or(message.len());
            message.replace_range(start..end, "[url]");
        }
    }
    message
}

#[derive(Clone)]
pub struct RemoteAudioCache {
    cache_root: PathBuf,
    cache_dir: PathBuf,
    digest: String,
    legacy_data_path: PathBuf,
    staging: Arc<Mutex<CacheStaging>>,
    ready_path: PathBuf,
    expected_content_length: Option<u64>,
    expected_content_md5: Option<String>,
    expected_duration_ms: Option<u64>,
    max_cache_bytes: u64,
    published_path: Arc<Mutex<Option<PathBuf>>>,
    bypass_ready: Arc<AtomicBool>,
}

struct CacheStaging {
    path: Option<PathBuf>,
}

impl Drop for CacheStaging {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[derive(Clone)]
pub struct RemoteAudioSource {
    inner: Arc<RemoteAudioInner>,
    pos: u64,
    access_mode: RemoteAccessMode,
    read_cancellation: Option<RemoteReadCancellation>,
}

#[derive(Clone)]
pub struct RemoteReadCancellation {
    session_cancelled: Arc<AtomicBool>,
    operation_generation: Option<(Arc<AtomicU64>, u64)>,
    /// prepare 期取消通道（代际检查 + 外部超时旗标）是否仍生效
    ///
    /// 两条通道的正当职责都只在 prepare 阶段：代际检查打断「输给更新
    /// seek 的 prepare」（seek 代际在发送命令的瞬间就递增，不解除的话
    /// 用户下一次 seek 会先把还在正常播放的会话的按需读杀死——实机日志
    /// seek 后 6 秒 next_packet fatal: remote read superseded）；外部超时
    /// 旗标解卡「等不到回包的 prepare」（回包超时与提交存在竞态窗口，
    /// 迟到的置位会无声杀死已提交的会话）。会话提交后一并解除，
    /// 生命周期只归 session_cancelled（previous.stop）管。
    operation_armed: Arc<AtomicBool>,
    /// 外部 prepare 超时取消（不改 generation，仅打断当前 prepare）
    external_cancel: Option<Arc<AtomicBool>>,
}

impl RemoteReadCancellation {
    pub fn new(
        session_cancelled: Arc<AtomicBool>,
        operation_generation: Option<(Arc<AtomicU64>, u64)>,
    ) -> Self {
        Self {
            session_cancelled,
            operation_generation,
            operation_armed: Arc::new(AtomicBool::new(true)),
            external_cancel: None,
        }
    }

    /// prepare 成功、会话即将提交时调用：
    /// 此后代际递增与迟到的 prepare 超时都不再影响本会话的读
    pub fn disarm_operation_guard(&self) {
        self.operation_armed.store(false, Ordering::Release);
    }

    pub fn with_external_cancel(mut self, flag: Arc<AtomicBool>) -> Self {
        self.external_cancel = Some(flag);
        self
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        if self.session_cancelled.load(Ordering::Acquire) {
            return true;
        }
        // prepare 期专属的两条取消通道，提交后一并失效
        if !self.operation_armed.load(Ordering::Acquire) {
            return false;
        }
        self.external_cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Acquire))
            || self
                .operation_generation
                .as_ref()
                .is_some_and(|(generation, expected)| {
                    generation.load(Ordering::Acquire) != *expected
                })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RemoteAccessMode {
    StandardSeekable,
    /// 长内容首播：顺序读，不向 demuxer 暴露长度/可 seek，避免扫尾部 moov
    LongFormProgressive,
    FragmentedProgressive,
    FragmentedSeekable,
}

impl RemoteAccessMode {
    fn for_url(url: &str, duration_hint_ms: u64) -> Self {
        if is_fragmented_mp4_url(url) {
            Self::FragmentedProgressive
        } else if duration_hint_ms >= REMOTE_LONG_FORM_DURATION_MS {
            Self::LongFormProgressive
        } else {
            Self::StandardSeekable
        }
    }

    fn is_seekable(self) -> bool {
        matches!(self, Self::StandardSeekable | Self::FragmentedSeekable)
    }

    fn demand_fetch_block_bytes(self) -> u64 {
        if matches!(self, Self::FragmentedSeekable) {
            REMOTE_FRAGMENTED_SEEK_BLOCK_BYTES
        } else {
            REMOTE_FETCH_BLOCK_BYTES
        }
    }

    /// 虚拟 body 生效时的按需块
    ///
    /// 虚拟 body 已经把起点对准目标 moof，不需要靠大块去「兜住」整段；
    /// 继续用 8MB 只会让一次 seek 卡在整块下完之前，与秒 seek 直接冲突。
    fn seek_fetch_block_bytes(self) -> u64 {
        REMOTE_FETCH_BLOCK_BYTES
    }
}

struct RemoteAudioInner {
    client: reqwest::Client,
    url: String,
    referer: String,
    total_len: u64,
    /// 首个 mdat/moof 起点；长内容虚拟 seek 时逻辑头长度
    header_end: AtomicU64,
    prefetch_window: PrefetchWindow,
    cache: Mutex<Vec<CachedSegment>>,
    disk_ranges: Mutex<Vec<CachedRange>>,
    /// 在途 range 请求注册表（key=起始物理偏移）：需求读与预取共享，
    /// 同一块只允许一个请求在途，后来者等待并共享缓存结果
    in_flight: Mutex<HashMap<u64, Arc<InflightFetch>>>,
    range_available: Condvar,
    /// 需求读在网计数：预取 lane 看到非零时暂缓发起新请求（需求读优先）
    demand_fetches: AtomicUsize,
    last_read_pos: Mutex<u64>,
    /// 保护尾部 moov/sidx：seek 重开 demuxer 时不可被 trim 掉
    protected_ranges: Mutex<Vec<CachedRange>>,
    disk_cache: Option<RemoteAudioCache>,
    cache_finalize_started: AtomicBool,
    /// 预取世代：seek 时 +1，旧预取任务看到后立刻停
    prefetch_epoch: AtomicU64,
    /// 远端已明确拒绝访问（HTTP 403/401，直链过期/风控）：
    /// 该状态对本直链是永久的，置位后停止一切新预取，
    /// 避免每次读都重新孵化注定 403 的后台请求
    remote_forbidden: AtomicBool,
    /// demuxer 打开阶段：对 isomp4 隐藏 seekable，避免扫完整 mdat
    demuxer_open_sequential: AtomicBool,
    /// 虚拟 body：逻辑 header_end 对应的远端 moof 起点；0 表示关闭
    virtual_body_origin: AtomicU64,
    /// 粗粒度时间→字节映射（来自 sidx 或线性估算），长内容 seek 用
    seek_index: Mutex<Option<RemoteSeekIndex>>,
    /// 近期实测下行吞吐 EWMA（bytes/s）；0 表示尚无样本。
    /// 成功请求按 bytes/elapsed 混入；超时失败只允许向下修正（上界样本）
    throughput_ewma_bps: AtomicU64,
    /// 媒体码率估算（bytes/s，total_len/duration_hint）；0 表示未知。
    /// 预取并发降级用：EWMA 低于码率说明链路喂不饱解码，带宽应让给需求读
    bitrate_bps: u64,
    playback_generation: Arc<AtomicU64>,
    expected_generation: u64,
}

/// 粗粒度 seek 索引：用 sidx 或线性比例估算目标字节
#[derive(Clone, Debug)]
struct RemoteSeekIndex {
    duration_ms: u64,
    entries: Vec<SeekIndexEntry>,
}

#[derive(Clone, Copy, Debug)]
struct SeekIndexEntry {
    time_ms: u64,
    byte_offset: u64,
}

impl RemoteSeekIndex {
    /// 返回 <= target_ms 的最近 segment 起点（moof 边界），供 virtual-body 跳转
    #[cfg(test)]
    fn estimate_segment_start(&self, target_ms: u64) -> u64 {
        self.estimate_segment(target_ms).0
    }

    /// (start, end_exclusive_hint)：用下一条 sidx 推段长，便于一次预满 moof+mdat
    fn estimate_segment(&self, target_ms: u64) -> (u64, Option<u64>) {
        if self.entries.is_empty() {
            return (0, None);
        }
        let mut best_idx = 0usize;
        for (idx, entry) in self.entries.iter().enumerate() {
            if entry.time_ms <= target_ms {
                best_idx = idx;
            } else {
                break;
            }
        }
        let start = self.entries[best_idx].byte_offset;
        let end = self.entries.get(best_idx + 1).map(|e| e.byte_offset);
        (start, end)
    }

    #[allow(dead_code)]
    fn estimate_byte(&self, target_ms: u64) -> u64 {

        if self.entries.is_empty() {
            return 0;
        }
        if target_ms == 0 {
            return self.entries[0].byte_offset;
        }
        // 找最后一个 time <= target 的 entry，再线性插值到下一个
        let mut lo = 0usize;
        for (idx, entry) in self.entries.iter().enumerate() {
            if entry.time_ms <= target_ms {
                lo = idx;
            } else {
                break;
            }
        }
        let left = self.entries[lo];
        if lo + 1 >= self.entries.len() {
            return left.byte_offset;
        }
        let right = self.entries[lo + 1];
        if right.time_ms <= left.time_ms {
            return left.byte_offset;
        }
        let span = right.time_ms - left.time_ms;
        let progress = target_ms.saturating_sub(left.time_ms).min(span);
        let byte_span = right.byte_offset.saturating_sub(left.byte_offset);
        left.byte_offset
            + (u128::from(byte_span) * u128::from(progress) / u128::from(span)) as u64
    }
}

impl RemoteAudioInner {
    fn throughput_bps(&self) -> u64 {
        self.throughput_ewma_bps.load(Ordering::Acquire)
    }

    /// 成功请求样本混入 EWMA。读-改-写竞态最多丢一个样本，无碍估计收敛
    fn record_fetch_success(&self, bytes: u64, elapsed_ms: u64) {
        let sample = throughput_sample_bps(bytes, elapsed_ms);
        let old = self.throughput_ewma_bps.load(Ordering::Acquire);
        self.throughput_ewma_bps
            .store(ewma_bps_after_success(old, sample), Ordering::Release);
    }

    /// 超时/中断失败：requested/elapsed 是真实吞吐的上界，只向下修正估计
    fn record_fetch_failure(&self, requested_bytes: u64, elapsed_ms: u64) {
        let old = self.throughput_ewma_bps.load(Ordering::Acquire);
        self.throughput_ewma_bps.store(
            ewma_bps_after_failure(old, requested_bytes, elapsed_ms),
            Ordering::Release,
        );
    }

    fn playback_cancelled(&self) -> bool {
        self.playback_generation.load(Ordering::Acquire) != self.expected_generation
    }

    fn ensure_playback_current(&self) -> io::Result<()> {
        if self.playback_cancelled() {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "playback request superseded",
            ))
        } else {
            Ok(())
        }
    }
}

struct CachedSegment {
    start: u64,
    data: Vec<u8>,
}

/// 单个在途 range 请求的登记项。
///
/// 为什么需要它：此前需求读与预取互不知道对方在途——需求读 claim 失败只等
/// 800ms 就强抢重发，同一 range 被拉两次，慢链路上带宽直接减半（实机日志
/// 同 range 两个响应）。登记后后来者改为等待首个请求完成并共享缓存结果。
struct InflightFetch {
    /// 请求覆盖的物理区间尾（含端点）：判断任意偏移是否已被在途请求覆盖。
    /// 原子化：需求读折半重试时收缩该边界，让 (新末尾, 旧末尾] 上的等待方
    /// 立即退出等待、自行 claim，不悬挂在别人的重试循环上
    end: AtomicU64,
    /// 请求结束（成功数据已入缓存 / 失败 / 被代际作废）后置位，
    /// 配合 range_available 唤醒共享等待方
    done: AtomicBool,
}

/// 需求读在网计数守卫：用 Drop 保证错误路径也能归零计数
struct DemandFetchGuard<'a> {
    inner: &'a RemoteAudioInner,
}

impl<'a> DemandFetchGuard<'a> {
    fn new(inner: &'a RemoteAudioInner) -> Self {
        inner.demand_fetches.fetch_add(1, Ordering::AcqRel);
        Self { inner }
    }
}

impl Drop for DemandFetchGuard<'_> {
    fn drop(&mut self) {
        self.inner.demand_fetches.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy)]
struct CachedRange {
    start: u64,
    end: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PrefetchWindow {
    low_water_bytes: u64,
    target_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PrefetchPlan {
    start: u64,
    target_end: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CacheMarker {
    Legacy {
        content_length: u64,
    },
    Validated {
        content_length: u64,
        sha256: String,
        file_name: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HttpByteRange {
    start: u64,
    end: u64,
    total: u64,
}

#[derive(Clone)]
struct ValidatedCacheFile {
    content_length: u64,
    sha256: String,
}

#[derive(Clone)]
struct ValidatedCacheStamp {
    content_length: u64,
    modified: SystemTime,
    sha256: String,
}

static VALIDATED_AUDIO_CACHE: OnceLock<Mutex<HashMap<PathBuf, ValidatedCacheStamp>>> =
    OnceLock::new();

impl RemoteAudioSource {
    pub async fn open(
        client: reqwest::Client,
        url: String,
        referer: String,
        disk_cache: Option<RemoteAudioCache>,
        duration_hint_ms: u64,
        playback_generation: Arc<AtomicU64>,
        expected_generation: u64,
    ) -> AppResult<Self> {
        let open_started = Instant::now();
        let host = url::Url::parse(&url)
            .ok()
            .and_then(|value| value.host_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".into());
        let access_mode = RemoteAccessMode::for_url(&url, duration_hint_ms);
        log::info!(
            target: "remote-audio",
            "open begin host={}, mode={:?}, generation={}, duration_hint_ms={}",
            host,
            access_mode,
            expected_generation,
            duration_hint_ms,
        );
        if playback_generation.load(Ordering::Acquire) != expected_generation {
            return Err(AppError::Audio("Playback request superseded".into()));
        }
        let initial_block = if duration_hint_ms >= REMOTE_LONG_FORM_DURATION_MS {
            REMOTE_LONG_INITIAL_BLOCK_BYTES
        } else {
            REMOTE_INITIAL_BLOCK_BYTES
        };
        let probe_started = Instant::now();
        let range_result = {
            let range_probe = probe_range_len(&client, &url, &referer, initial_block);
            tokio::pin!(range_probe);
            tokio::select! {
                result = &mut range_probe => result,
                () = wait_for_generation_change(
                    &playback_generation,
                    expected_generation,
                ) => {
                    return Err(AppError::Audio("Playback request superseded".into()));
                }
            }
        };
        log::info!(
            target: "remote-audio",
            "range probe finished host={}, ok={}, elapsed_ms={}, total_ms={}",
            host,
            range_result.is_ok(),
            probe_started.elapsed().as_millis(),
            open_started.elapsed().as_millis(),
        );
        let (total_len, initial_segment) = match range_result {
            Ok((len, data)) => {
                log::info!(
                    target: "remote-audio",
                    "range probe data host={}, total_len={}, initial_bytes={}",
                    host,
                    len,
                    data.len(),
                );
                (len, Some(CachedSegment { start: 0, data }))
            }
            Err(range_error) => {
                if playback_generation.load(Ordering::Acquire) != expected_generation {
                    return Err(AppError::Audio("Playback request superseded".into()));
                }
                log::warn!(
                    target: "remote-audio",
                    "range probe failed host={}, elapsed_ms={}, error={}",
                    host,
                    probe_started.elapsed().as_millis(),
                    summarize_remote_error(&range_error),
                );
                return Err(AppError::Audio(
                    "Remote source does not support seekable HTTP Range".into(),
                ));
            }
        };

        // 用首包探测播种吞吐 EWMA：冷启动第一个自适应块/超时预算就有依据。
        // 探测耗时含建连与 TTFB，样本偏保守（慢网首块更小），符合预期
        let initial_throughput_bps = throughput_sample_bps(
            initial_segment
                .as_ref()
                .map(|segment| segment.data.len() as u64)
                .unwrap_or(0),
            (probe_started.elapsed().as_millis().max(1)) as u64,
        );

        if let Some(cache) = disk_cache.as_ref().cloned() {
            let cache_started = Instant::now();
            let queued_at = Instant::now();
            tokio::task::spawn_blocking(move || {
                log::info!(
                    target: "remote-cache",
                    "prepare worker started queued_ms={}",
                    queued_at.elapsed().as_millis(),
                );
                cache.prepare(total_len)
            })
            .await
            .map_err(|error| AppError::Other(error.to_string()))?
            .map_err(|error| AppError::Other(error.to_string()))?;
            log::info!(
                target: "remote-cache",
                "prepare host={}, total_len={}, elapsed_ms={}",
                host,
                total_len,
                cache_started.elapsed().as_millis(),
            );
        }

        let initial_disk_segment = initial_segment
            .as_ref()
            .map(|segment| (segment.start, segment.data.clone()));
        let cache = initial_segment.into_iter().collect();
        let prefetch_window = prefetch_window(total_len, duration_hint_ms);
        let mut protected_ranges = Vec::new();
        if let Some((start, data)) = &initial_disk_segment {
            if !data.is_empty() {
                protected_ranges.push(CachedRange {
                    start: *start,
                    end: start.saturating_add(data.len() as u64),
                });
            }
        }
        let head_bytes = initial_disk_segment
            .as_ref()
            .map(|(_, data)| data.as_slice())
            .unwrap_or(&[]);
        let header_end = detect_mp4_header_end(head_bytes).unwrap_or(0);
        // URL 后缀嗅探不到 googlevideo 的分片直链，必须按首包内容再判一次。
        //
        // 嗅探结果只用于「打开阶段顺序读」这一件事，不改 access_mode：
        // 分片模式的按需块是 8MB，那是给 Bilibili 独立 .m4s 分片文件调的，
        // 套到 YouTube 的单文件上反而拖慢 seek。
        // 索引构建不看嗅探结果：所有分片流（嗅探出的 + .m4s 直链）
        // 统一在下方 needs_seek_index 里按首包 sidx 建索引
        let sniffed_fragmented = !matches!(access_mode, RemoteAccessMode::FragmentedProgressive)
            && detect_fragmented_mp4(head_bytes);
        let inner = Arc::new(RemoteAudioInner {
            client,
            url,
            referer,
            total_len,
            header_end: AtomicU64::new(header_end),
            prefetch_window,
            cache: Mutex::new(cache),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::new()),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(protected_ranges),
            disk_cache,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(0),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(initial_throughput_bps),
            bitrate_bps: total_len
                .saturating_mul(1000)
                .checked_div(duration_hint_ms)
                .unwrap_or(0),
            playback_generation,
            expected_generation,
        });
        if header_end > 0 {
            log::info!(
                target: "remote-audio",
                "mp4 header_end detected host={}, header_end={}",
                host,
                header_end,
            );
        }
        if sniffed_fragmented {
            // 打开阶段隐藏 seekable，避免 symphonia 顺着 moof 链把整个文件读完；
            // sidx 位于 header_end 之前，顺序读同样能拿到，不影响后续按 sidx seek。
            // finish_demuxer_open() 会在打开完成后恢复可 seek。
            inner
                .demuxer_open_sequential
                .store(true, Ordering::Release);
            log::info!(
                target: "remote-audio",
                "fragmented mp4 sniffed from payload host={}, mode={:?}, opening sequentially",
                host,
                access_mode,
            );
        }
        inner
            .ensure_playback_current()
            .map_err(|err| AppError::Audio(err.to_string()))?;
        if let Some((start, data)) = initial_disk_segment {
            write_disk_range(&inner, start, &data);
        }
        // 分片流必须建索引
        //
        // symphonia 0.5.5 的 isomp4 解析完 sidx 就丢掉（demuxer.rs 里只剩一句
        // "stream is segmented with a segment index" 日志），seek 时只能靠
        // try_read_more_segments 一个个 moof 往前爬，代价是 O(文件大小)：
        // 实测 68MB 的 70 分钟音频跳到 37 分钟要顺序读满 30MB。
        // 所以 sidx 必须由我们自己解析成索引，seek 时直接按字节落点开 demuxer。
        //
        // FragmentedProgressive（Bilibili .m4s 直链）同样必须建：它的 sidx
        // 一样在首包（header_end 之前），旧代码只认嗅探结果与 LongForm，
        // 导致 2 小时 118MB 的 fMP4 每次 seek 都顺序爬 moof ~12s
        let needs_seek_index = needs_head_seek_index(access_mode, sniffed_fragmented);
        // 分片 MP4 的 sidx 紧跟 moov 位于首包，先用首包建索引；
        // 建成了就不必再为长内容拉 1MB 尾部，直接省掉起播路径上的一次往返
        //
        // 不要求 duration_hint_ms > 0：sidx 自带 timescale 与分段时长，
        // 时长未知（YouTube 前端常传 hint=0）同样能建出精确索引；
        // 否则这类流会退化到 format.seek 顺序爬 moof，O(文件大小)——
        // 实测 58MB 一次 seek 要顺序下完全文件耗时 16.6s
        let head_index_ready = needs_seek_index
            && total_len > 0
            && build_seek_index_from_head(&inner, duration_hint_ms);

        // 长内容：尾部 moov/sidx 预取进缓存，seek 升级为 seekable 后 demuxer 能直接跳转
        if matches!(access_mode, RemoteAccessMode::LongFormProgressive)
            && total_len > 0
            && !head_index_ready
            // 分片流的 moov/sidx 都在首包，拉尾部只会白白拖慢起播
            && !sniffed_fragmented
        {
            let tail_len = REMOTE_LONG_FORM_TAIL_BYTES.min(total_len);
            let tail_start = total_len.saturating_sub(tail_len);
            // 与首包重叠时跳过（极短文件）
            if tail_start > 0 {
                let tail_end = total_len.saturating_sub(1);
                match fetch_range_block_async(
                    &inner,
                    tail_start,
                    tail_end,
                    None,
                )
                .await
                {
                    Ok(data) if !data.is_empty() => {
                        log::info!(
                            target: "remote-audio",
                            "long-form tail cached host={}, start={}, bytes={}",
                            host,
                            tail_start,
                            data.len(),
                        );
                        protect_cached_range(&inner, tail_start, data.len() as u64);
                        if let Ok(mut cache) = inner.cache.lock() {
                            if !cache_contains_position(&cache, tail_start) {
                                cache.push(CachedSegment {
                                    start: tail_start,
                                    data: data.clone(),
                                });
                                cache.sort_by_key(|segment| segment.start);
                                trim_cache(&inner, &mut cache, cache_center(&inner, 0));
                            }
                        }
                        write_disk_range(&inner, tail_start, &data);
                    }
                    Ok(_) => {}
                    Err(err) => {
                        log::warn!(
                            target: "remote-audio",
                            "long-form tail prefetch failed host={}, error={}",
                            host,
                            summarize_remote_error(&err),
                        );
                    }
                }
            }
        }
        // 首包没建成索引时再兜底：长内容可退到尾部 sidx / 线性估算，
        // 分片内容（嗅探出的 + .m4s 直链）只接受真实 sidx，解析不到就保持
        // 原 format.seek 顺序行为（函数内部会打 "seek index skipped" 日志）。
        // 线性估算依赖声明时长，hint=0（时长未知）时在函数内部自动禁用
        if needs_seek_index && total_len > 0 && !head_index_ready {
            let allow_linear_fallback =
                matches!(access_mode, RemoteAccessMode::LongFormProgressive);
            build_seek_index_for_long_form(&inner, duration_hint_ms, allow_linear_fallback);
        }
        replenish_prefetch_window(&inner, 0);

        log::info!(
            target: "remote-audio",
            "open ready host={}, total_len={}, prefetch_low={}, prefetch_target={}, total_ms={}",
            host,
            total_len,
            prefetch_window.low_water_bytes,
            prefetch_window.target_bytes,
            open_started.elapsed().as_millis(),
        );

        Ok(Self {
            inner,
            pos: 0,
            access_mode,
            read_cancellation: None,
        })
    }

    pub fn byte_len(&self) -> u64 {
        self.inner.total_len
    }

    pub fn seekable_clone(&self) -> Self {
        // 分片 m4s → FragmentedSeekable（大块按需）；
        // 长内容渐进 m4a → StandardSeekable（用 moov 表跳转，绝不能走分片 8MB 逻辑）
        let (access_mode, sequential_demuxer_open) = match self.access_mode {
            RemoteAccessMode::FragmentedProgressive => {
                (RemoteAccessMode::FragmentedSeekable, true)
            }
            RemoteAccessMode::LongFormProgressive => {
                (RemoteAccessMode::StandardSeekable, true)
            }
            mode => (mode, false),
        };
        // 作废旧位置的预取，避免 seek 重建 demuxer 时和旧 in_flight 撞车
        invalidate_prefetch_epoch(&self.inner);
        // 长内容/分片升级：demuxer 打开必须假装不可 seek，否则会扫完整 mdat
        // 短内容本身就是 StandardSeekable，moov 可能在尾部，不能强制顺序打开
        if sequential_demuxer_open {
            self.inner
                .demuxer_open_sequential
                .store(true, Ordering::Release);
        }
        Self {
            inner: Arc::clone(&self.inner),
            pos: 0,
            access_mode,
            read_cancellation: self.read_cancellation.clone(),
        }
    }

    pub fn with_read_cancellation(mut self, cancellation: RemoteReadCancellation) -> Self {
        self.read_cancellation = Some(cancellation);
        self
    }

    /// demuxer 打开完成：允许真实时间 seek 的字节跳转
    pub fn finish_demuxer_open(&self) {
        self.inner
            .demuxer_open_sequential
            .store(false, Ordering::Release);
    }

    fn is_demuxer_open_sequential(&self) -> bool {
        self.inner.demuxer_open_sequential.load(Ordering::Acquire)
    }

    /// 长内容 seek：用「虚拟 body」打开 demuxer，而不是 format.seek 扫全文件
    ///
    /// LongFormProgressive 也算：seek 时会先 seekable_clone 升级，再走虚拟 body。
    /// 判断必须在 clone 前后都能成立，否则会误走 format.seek 扫整文件 mdat。
    pub fn prefers_virtual_body_seek(&self) -> bool {
        let header_end = self.inner.header_end.load(Ordering::Acquire);
        if header_end == 0 {
            return false;
        }
        let mode_ok = matches!(
            self.access_mode,
            RemoteAccessMode::LongFormProgressive
                | RemoteAccessMode::StandardSeekable
                | RemoteAccessMode::FragmentedProgressive
                | RemoteAccessMode::FragmentedSeekable
        );
        if !mode_ok {
            return false;
        }
        self.inner
            .seek_index
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|index| !index.entries.is_empty()))
            .unwrap_or(false)
    }

    /// 取出文件头缓存（ftyp/moov/sidx），供 virtual-body 拼接 demux 使用
    pub fn header_bytes(&self) -> Option<Vec<u8>> {
        let header_end = self.inner.header_end.load(Ordering::Acquire);
        if header_end == 0 {
            return None;
        }
        let need = header_end as usize;
        let mut out = vec![0u8; need];
        // 优先内存缓存 start=0
        if let Ok(cache) = self.inner.cache.lock() {
            if let Some(seg) = cache.iter().find(|s| s.start == 0) {
                let n = seg.data.len().min(need);
                out[..n].copy_from_slice(&seg.data[..n]);
                if n == need {
                    return Some(out);
                }
            }
        }
        // 磁盘缓存兜底
        if let Some(disk) = &self.inner.disk_cache {
            if disk.read_cached(0, &mut out).unwrap_or(0) == need {
                return Some(out);
            }
        }
        None
    }

    /// 配置虚拟 body：逻辑 [0, header_end) = 文件头；逻辑 header_end.. = 目标 moof 起
    pub fn configure_virtual_body_for_time(&self, position_ms: u64) -> io::Result<u64> {
        self.ensure_read_current()?;
        let header_end = self.inner.header_end.load(Ordering::Acquire);
        if header_end == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mp4 header_end unknown",
            ));
        }
        let index = self
            .inner
            .seek_index
            .lock()
            .map_err(|_| io::Error::other("seek index lock poisoned"))?
            .clone();
        let Some(index) = index else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "seek index missing",
            ));
        };
        // 只算 sidx 落点；真正 body 拉取交给 splice 分块（避免这里一次拉过大超时）
        let (seg_start, _seg_end_hint) = index.estimate_segment(position_ms);
        let mut target = seg_start.max(header_end).min(self.inner.total_len.saturating_sub(1));

        // 轻量探测：最多拉 256KB 校验 moof 并对齐；失败也不致命，splice 会再 snap
        let probe_end = target
            .saturating_add(REMOTE_FETCH_BLOCK_BYTES)
            .saturating_sub(1)
            .min(self.inner.total_len.saturating_sub(1));
        let probe = if position_is_cached_range(&self.inner, target, probe_end) {
            None
        } else {
            // seek 关键路径同样属于需求读：探测期间预取让路
            let _demand_guard = DemandFetchGuard::new(&self.inner);
            match fetch_range_block(
                &self.inner,
                target,
                probe_end,
                self.read_cancellation.as_ref(),
            ) {
                Ok(data) if !data.is_empty() => Some(data),
                Ok(_) => None,
                // 403/401 对本直链是永久错误：继续配置虚拟 body 只会让
                // demuxer 打开阶段对同一 range 无限重试刷 403，
                // 立即终止本次 seek 让上层回滚/换直链
                Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
                    log::warn!(
                        target: "remote-audio",
                        "virtual-body probe forbidden at {}: {}",
                        target,
                        err,
                    );
                    return Err(err);
                }
                Err(err) => {
                    log::warn!(
                        target: "remote-audio",
                        "virtual-body probe fetch failed at {}: {}",
                        target,
                        err,
                    );
                    None
                }
            }
        };
        if let Some(data) = probe {
            if let Some(rel) = find_moof_offset_near(&data, 0, 64 * 1024) {
                if rel > 0 {
                    log::warn!(
                        target: "remote-audio",
                        "virtual-body sidx miss-aligned by {} bytes, snapping to moof",
                        rel,
                    );
                    target = target
                        .saturating_add(rel)
                        .min(self.inner.total_len.saturating_sub(1));
                }
                let store = data[rel as usize..].to_vec();
                if !store.is_empty() {
                    if let Ok(mut cache) = self.inner.cache.lock() {
                        if !cache_contains_position(&cache, target) {
                            cache.push(CachedSegment {
                                start: target,
                                data: store.clone(),
                            });
                            cache.sort_by_key(|segment| segment.start);
                            trim_cache(&self.inner, &mut cache, target);
                        }
                    }
                    write_disk_range(&self.inner, target, &store);
                    protect_cached_range(&self.inner, target, store.len() as u64);
                }
            } else {
                log::warn!(
                    target: "remote-audio",
                    "virtual-body probe {:#x} has no moof in first 64KiB (head={:02x?})",
                    target,
                    &data[..data.len().min(16)],
                );
            }
        }

        self.inner
            .virtual_body_origin
            .store(target, Ordering::Release);
        self.inner
            .demuxer_open_sequential
            .store(true, Ordering::Release);
        invalidate_prefetch_epoch(&self.inner);
        let logical_len = header_end.saturating_add(self.inner.total_len.saturating_sub(target));
        log::info!(
            target: "remote-audio",
            "virtual-body configured target_ms={}, header_end={}, body_origin={}, total_len={}, logical_len={}",
            position_ms,
            header_end,
            target,
            self.inner.total_len,
            logical_len,
        );
        Ok(target)
    }

    pub fn clear_virtual_body(&self) {
        self.inner.virtual_body_origin.store(0, Ordering::Release);
    }

    /// virtual-body **纯逻辑连续**坐标（AtomIterator / moof_base_pos 共用）：
    /// - [0, header_end) → 物理 header
    /// - [header_end + k) → 物理 body_origin + k
    ///
    /// 绝对 base_data_offset 的 sample Seek 在 `seek()` 里先折算成逻辑坐标，
    /// 禁止在 map 里对 pos≥origin 做恒等：否则 MSS pos 跳到物理 55MB，
    /// AtomIterator 仍在逻辑 ~header_end，下一 atom 直接 overread / EOF。
    fn map_logical_to_physical(&self, pos: u64) -> u64 {
        let origin = self.inner.virtual_body_origin.load(Ordering::Acquire);
        if origin == 0 {
            return pos.min(self.inner.total_len);
        }
        let header_end = self.inner.header_end.load(Ordering::Acquire);
        if pos < header_end {
            pos.min(self.inner.total_len)
        } else {
            origin
                .saturating_add(pos - header_end)
                .min(self.inner.total_len)
        }
    }

    fn logical_len(&self) -> u64 {
        let origin = self.inner.virtual_body_origin.load(Ordering::Acquire);
        if origin == 0 {
            return self.inner.total_len;
        }
        let header_end = self.inner.header_end.load(Ordering::Acquire);
        // 纯连续逻辑长度；绝对 sample 偏移在 seek 时折算，不靠拉长 logical_len
        header_end.saturating_add(self.inner.total_len.saturating_sub(origin))
    }

    /// 将 demuxer 的 Seek 目标折成逻辑坐标。
    /// - 相对 moof：目标已在 [0, logical_len)
    /// - 绝对 base_data_offset：目标 ≥ body_origin，折成 header_end + (phys - origin)
    fn normalize_seek_offset(&self, offset: u64) -> u64 {
        let origin = self.inner.virtual_body_origin.load(Ordering::Acquire);
        if origin == 0 {
            return offset.min(self.inner.total_len);
        }
        let header_end = self.inner.header_end.load(Ordering::Acquire);
        let logical_len = self.logical_len();
        if offset >= origin {
            // 绝对文件偏移 → 逻辑 body
            let logical = header_end.saturating_add(offset - origin);
            logical.min(logical_len)
        } else {
            offset.min(logical_len)
        }
    }

    fn read_cancelled(&self) -> bool {
        self.inner.playback_cancelled()
            || self
                .read_cancellation
                .as_ref()
                .is_some_and(RemoteReadCancellation::is_cancelled)
    }

    fn ensure_read_current(&self) -> io::Result<()> {
        if self.read_cancelled() {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "remote read superseded",
            ))
        } else {
            Ok(())
        }
    }

    /// 按物理偏移读内存缓存
    fn read_cached_at(&self, physical: u64, out: &mut [u8]) -> usize {
        let cache = match self.inner.cache.lock() {
            Ok(cache) => cache,
            Err(_) => return 0,
        };

        for segment in cache.iter() {
            let segment_end = segment.start + segment.data.len() as u64;
            if physical < segment.start || physical >= segment_end {
                continue;
            }

            let offset = (physical - segment.start) as usize;
            let available = segment.data.len().saturating_sub(offset);
            let len = available.min(out.len());
            out[..len].copy_from_slice(&segment.data[offset..offset + len]);
            return len;
        }

        0
    }

    fn read_disk_cached_at(&self, physical: u64, out: &mut [u8]) -> usize {
        let Some(cache) = &self.inner.disk_cache else {
            return 0;
        };
        let Some(len) = disk_cached_len(&self.inner, physical, out.len()) else {
            return 0;
        };
        cache.read_cached(physical, &mut out[..len]).unwrap_or(0)
    }

    fn has_cached_physical(&self, physical: u64) -> bool {
        let cache = match self.inner.cache.lock() {
            Ok(cache) => cache,
            Err(_) => return false,
        };

        cache.iter().any(|segment| {
            let segment_end = segment.start + segment.data.len() as u64;
            physical >= segment.start && physical < segment_end
        })
    }

    /// 按物理偏移拉取；全程不改 self.pos（self.pos 只存逻辑坐标）
    fn fetch_physical_range(&self, physical: u64, wanted_len: usize) -> io::Result<()> {
        self.ensure_read_current()?;
        if physical >= self.inner.total_len {
            return Ok(());
        }

        // 虚拟 body 生效时改用 seek 专用块，避免 8MB 拖垮跳转延迟
        let mode_block = if self.inner.virtual_body_origin.load(Ordering::Acquire) > 0 {
            self.access_mode.seek_fetch_block_bytes()
        } else {
            self.access_mode.demand_fetch_block_bytes()
        };
        // 慢网自适应：按近期吞吐缩块（8s 可完成），mode 块只作上限；
        // 无样本时保持 mode 默认块，冷启动行为不变
        let block = adaptive_block_bytes(self.inner.throughput_bps(), mode_block, mode_block);
        let wanted = wanted_len.max(block as usize) as u64;
        let end = physical
            .saturating_add(wanted)
            .saturating_sub(1)
            .min(self.inner.total_len.saturating_sub(1));

        // 「查缓存 → 等在途共享 → claim 亲拉」最多三轮：
        // 同一块已有请求在途时等待并共享其结果，绝不重复发起（重复只会把
        // 带宽劈半，数据不会更早到达）；claim 竞态失败则下一轮转入等待
        let mut claimed = None;
        for _ in 0..3 {
            self.ensure_read_current()?;
            if self.has_cached_physical(physical)
                || disk_cached_len(&self.inner, physical, 1).is_some()
            {
                return Ok(());
            }
            if self.wait_for_inflight(physical) {
                return Ok(());
            }
            self.ensure_read_current()?;
            if let Some(entry) = claim_prefetch_start(&self.inner, physical, end) {
                claimed = Some(entry);
                break;
            }
        }
        let Some(entry) = claimed else {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "remote range is already being fetched",
            ));
        };

        // 需求读在网标记：预取 lane 看到后暂缓开新请求，带宽让给解码急需的数据
        let demand_guard = DemandFetchGuard::new(&self.inner);
        // 超时/传输中断绝不直接 fatal：对同一起点折半块大小重试（短退避），
        // 连续 REMOTE_DEMAND_MAX_ATTEMPTS 次失败才把错误抛给解码层。
        // 播放侧表现为 rebuffering（read 阻塞在重试循环里）而非会话死亡
        let mut attempt_end = end;
        let mut attempt = 0usize;
        let fetched = loop {
            attempt += 1;
            match fetch_range_block(
                &self.inner,
                physical,
                attempt_end,
                self.read_cancellation.as_ref(),
            ) {
                Ok(data) => break Ok(data),
                Err(err)
                    if attempt < REMOTE_DEMAND_MAX_ATTEMPTS
                        && is_transient_fetch_error(&err) =>
                {
                    let current_len = attempt_end.saturating_sub(physical).saturating_add(1);
                    let next_len = halved_fetch_len(current_len);
                    attempt_end = physical
                        .saturating_add(next_len)
                        .saturating_sub(1)
                        .min(self.inner.total_len.saturating_sub(1));
                    // 收缩在途登记边界并唤醒等待方：落在收缩区外的等待方
                    // 立即改走自己的 claim，不悬挂在本重试循环上
                    entry.end.store(attempt_end, Ordering::Release);
                    self.inner.range_available.notify_all();
                    log::warn!(
                        target: "remote-range",
                        "demand fetch retry attempt={}/{} range={}..{} next_len={} error={}",
                        attempt,
                        REMOTE_DEMAND_MAX_ATTEMPTS,
                        physical,
                        attempt_end,
                        next_len,
                        summarize_remote_error(&err),
                    );
                    // 短退避（按次数放大），期间保持取消即时性
                    let backoff =
                        Duration::from_millis(REMOTE_DEMAND_RETRY_BACKOFF_MS * attempt as u64);
                    let backoff_started = Instant::now();
                    while backoff_started.elapsed() < backoff {
                        if self.ensure_read_current().is_err() {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    if let Err(cancelled) = self.ensure_read_current() {
                        break Err(cancelled);
                    }
                }
                Err(err) => break Err(err),
            }
        };
        drop(demand_guard);
        let data = match fetched {
            Ok(data) => data,
            Err(err) => {
                release_prefetch_start(&self.inner, physical, &entry);
                return Err(err);
            }
        };

        if data.is_empty() {
            release_prefetch_start(&self.inner, physical, &entry);
            return Ok(());
        }

        // 先入缓存再发布 done：共享等待方醒来即可命中，不会空等一场
        let insert_result = (|| -> io::Result<()> {
            let mut cache = self
                .inner
                .cache
                .lock()
                .map_err(|_| io::Error::other("remote cache lock poisoned"))?;
            if !cache_contains_position(&cache, physical) {
                cache.push(CachedSegment {
                    start: physical,
                    data,
                });
                cache.sort_by_key(|segment| segment.start);
                trim_cache(
                    &self.inner,
                    &mut cache,
                    cache_center(&self.inner, physical),
                );
            }
            Ok(())
        })();
        release_prefetch_start(&self.inner, physical, &entry);
        insert_result?;
        replenish_prefetch_window(&self.inner, physical);
        Ok(())
    }

    /// 等待覆盖 start 的在途请求完成并共享其缓存结果。
    ///
    /// 旧实现只等 800ms 就强抢重发同一 range，慢链路上必然双请求并发、
    /// 带宽减半；这里改为等到发布方置 done。seek 换代时
    /// invalidate_prefetch_epoch 会立刻置 done 解除等待，低延迟语义不变。
    fn wait_for_inflight(&self, start: u64) -> bool {
        let entry = {
            let Ok(in_flight) = self.inner.in_flight.lock() else {
                return false;
            };
            in_flight
                .iter()
                .find(|(entry_start, entry)| {
                    **entry_start <= start && start <= entry.end.load(Ordering::Acquire)
                })
                .map(|(_, entry)| Arc::clone(entry))
        };
        let Some(entry) = entry else {
            return false;
        };

        let started = Instant::now();
        // 防御上限：发布方的请求超时上限为 REMOTE_REQUEST_TIMEOUT_CEILING，
        // 正常路径必在此之前发布结果
        let wait_budget = REMOTE_REQUEST_TIMEOUT_CEILING + Duration::from_secs(5);
        while !entry.done.load(Ordering::Acquire) && started.elapsed() < wait_budget {
            if self.read_cancelled() {
                return false;
            }
            // 发布方折半重试收缩了区间、本偏移已不被覆盖：立即退出等待，
            // 走自己的 claim 拉取，不陪发布方的重试循环空等
            if start > entry.end.load(Ordering::Acquire) {
                break;
            }
            let Ok(guard) = self.inner.in_flight.lock() else {
                return false;
            };
            // 拿锁后复查，避免错过 done 置位与 notify 之间的竞态窗口
            if entry.done.load(Ordering::Acquire) {
                break;
            }
            match self
                .inner
                .range_available
                .wait_timeout(guard, Duration::from_millis(40))
            {
                Ok(_) | Err(_) => {}
            }
        }

        self.has_cached_physical(start) || disk_cached_len(&self.inner, start, 1).is_some()
    }
}

impl Read for RemoteAudioSource {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.ensure_read_current()?;
        let logical_len = self.logical_len();
        if out.is_empty() || self.pos >= logical_len {
            return Ok(0);
        }

        let mut total_read = 0usize;
        while total_read < out.len() && self.pos < logical_len {
            self.ensure_read_current()?;
            let logical_pos = self.pos;
            let physical = self.map_logical_to_physical(logical_pos);

            // 单次读绝不许跨过 header_end 的拼接边界
            //
            // 虚拟 body 的逻辑地址在 header_end 处不连续，而缓存段在物理空间
            // 是连续的：从头部起读时若不封顶，read_cached_at 会顺着物理地址
            // 一路读进「文件开头的第一个 moof」——它也是合法 moof，demuxer
            // 便按它的 trun 去取样本，解出来的就是真假混杂的 AAC。
            // 封顶后本轮只读到边界，循环下一轮重新映射，body 从正确落点续读。
            let mut limit = out.len() - total_read;
            let header_end = self.inner.header_end.load(Ordering::Acquire);
            if self.inner.virtual_body_origin.load(Ordering::Acquire) > 0
                && logical_pos < header_end
            {
                limit = limit.min((header_end - logical_pos) as usize);
            }
            let window = total_read + limit;

            // 全程不改 self.pos 为物理坐标：缓存按 physical 显式寻址
            let mut read = self.read_cached_at(physical, &mut out[total_read..window]);
            if read == 0 {
                read = self.read_disk_cached_at(physical, &mut out[total_read..window]);
            }
            if read == 0 {
                match self.fetch_physical_range(physical, limit) {
                    Ok(()) => {
                        read = self.read_cached_at(physical, &mut out[total_read..window]);
                        if read == 0 {
                            read = self.read_disk_cached_at(physical, &mut out[total_read..window]);
                        }
                    }
                    Err(err) if total_read > 0 && can_return_partial_read(&err) => {
                        return Ok(total_read)
                    }
                    Err(err) => return Err(err),
                }
            }
            if read == 0 {
                break;
            }
            self.pos = logical_pos.saturating_add(read as u64);
            total_read += read;
            remember_read_pos(&self.inner, physical.saturating_add(read as u64));
            if !self.is_demuxer_open_sequential() {
                replenish_prefetch_window(
                    &self.inner,
                    physical.saturating_add(read as u64),
                );
            }
        }

        Ok(total_read)
    }
}

impl Seek for RemoteAudioSource {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.ensure_read_current()?;
        let logical_len = self.logical_len();
        let raw_next = match pos {
            SeekFrom::Start(offset) => offset as i128,
            SeekFrom::Current(offset) => self.pos as i128 + offset as i128,
            SeekFrom::End(offset) => logical_len as i128 + offset as i128,
        };

        if raw_next < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid seek before start",
            ));
        }

        let previous = self.pos;
        // Start 可能带绝对 base_data_offset；先折成逻辑坐标再 clamp
        let next = match pos {
            SeekFrom::Start(offset) => self.normalize_seek_offset(offset),
            _ => (raw_next as u64).min(logical_len),
        };
        self.pos = next;
        if self.pos.abs_diff(previous) > REMOTE_FETCH_BLOCK_BYTES {
            invalidate_prefetch_epoch(&self.inner);
        }
        let physical = self.map_logical_to_physical(self.pos);
        if self.inner.virtual_body_origin.load(Ordering::Acquire) > 0
            && self.pos.abs_diff(previous) > 64
        {
            log::debug!(
                target: "remote-audio",
                "virtual-body seek logical={} physical={} header_end={} body_origin={} prev={} raw={}",
                self.pos,
                physical,
                self.inner.header_end.load(Ordering::Acquire),
                self.inner.virtual_body_origin.load(Ordering::Acquire),
                previous,
                raw_next,
            );
        }
        remember_read_pos(&self.inner, physical);
        // 返回逻辑 pos（MSS / AtomIterator 只认连续逻辑坐标）
        Ok(self.pos)
    }
}

impl RemoteAudioCache {
    pub fn new(
        root: PathBuf,
        cache_key: &str,
        max_cache_bytes: u64,
        expected_content_length: Option<u64>,
        expected_duration_ms: u64,
    ) -> AppResult<Self> {
        let digest = hex::encode(Sha256::digest(cache_key.as_bytes()));
        let shard = digest.get(0..2).unwrap_or("00");
        let dir = root.join(shard);
        Ok(Self {
            cache_root: root,
            cache_dir: dir.clone(),
            digest: digest.clone(),
            legacy_data_path: dir.join(format!("{}.audio", digest)),
            staging: Arc::new(Mutex::new(CacheStaging { path: None })),
            ready_path: dir.join(format!("{}.ready", digest)),
            expected_content_length: expected_content_length.filter(|length| *length > 0),
            expected_content_md5: None,
            expected_duration_ms: (expected_duration_ms > 0).then_some(expected_duration_ms),
            max_cache_bytes,
            published_path: Arc::new(Mutex::new(None)),
            bypass_ready: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn with_expected_md5(mut self, value: Option<&str>) -> AppResult<Self> {
        if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
            if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(AppError::Api("Playback source checksum is invalid".into()));
            }
            self.expected_content_md5 = Some(value.to_ascii_lowercase());
        }
        Ok(self)
    }

    fn validate_expected_md5(&self, path: &Path) -> io::Result<()> {
        let Some(expected) = self.expected_content_md5.as_deref() else { return Ok(()) };
        let mut file = File::open(path)?;
        let mut digest = md5::Md5::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 { break }
            digest.update(&buffer[..count]);
        }
        if hex::encode(digest.finalize()) != expected {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "cache MD5 does not match playback source"));
        }
        Ok(())
    }

    pub fn ready_path(&self) -> Option<PathBuf> {
        let lookup_started = Instant::now();
        let digest_prefix = self.digest.get(..8).unwrap_or(&self.digest);
        if self.bypass_ready.load(Ordering::Acquire) {
            log::info!(
                target: "remote-cache",
                "lookup bypassed digest={} elapsed_ms={}",
                digest_prefix,
                lookup_started.elapsed().as_millis(),
            );
            return None;
        }

        let marker_text = match std::fs::read_to_string(&self.ready_path) {
            Ok(value) => value,
            Err(error) => {
                log::info!(
                    target: "remote-cache",
                    "lookup miss digest={} reason=marker_read error_kind={:?} elapsed_ms={}",
                    digest_prefix,
                    error.kind(),
                    lookup_started.elapsed().as_millis(),
                );
                return None;
            }
        };
        let marker = match parse_cache_marker(marker_text.trim()) {
            Some(marker) => marker,
            None => {
                log::warn!(
                    target: "remote-cache",
                    "lookup miss digest={} reason=marker_parse elapsed_ms={}",
                    digest_prefix,
                    lookup_started.elapsed().as_millis(),
                );
                return None;
            }
        };
        let (path, marked_length, expected_sha256, file_name) = match &marker {
            CacheMarker::Legacy { .. } => {
                log::warn!(
                    target: "remote-cache",
                    "lookup miss digest={} reason=legacy_marker elapsed_ms={}",
                    digest_prefix,
                    lookup_started.elapsed().as_millis(),
                );
                return None;
            }
            CacheMarker::Validated {
                content_length,
                sha256,
                file_name,
            } => (
                self.resolve_marker_file(file_name)?,
                *content_length,
                sha256.as_str(),
                file_name.as_str(),
            ),
        };

        if let Err(err) = validate_published_cache_file(
            &path,
            marked_length,
            self.expected_content_length,
            self.expected_duration_ms,
            expected_sha256,
            file_name,
        ) {
            log::warn!(
                target: "remote-cache",
                "lookup miss digest={} reason=validation error={} elapsed_ms={}",
                digest_prefix,
                err,
                lookup_started.elapsed().as_millis(),
            );
            return None;
        }

        if self.validate_expected_md5(&path).is_err() {
            log::warn!(target: "remote-cache", "lookup miss digest={} reason=source_checksum", digest_prefix);
            return None;
        }
        if let Ok(mut published) = self.published_path.lock() {
            *published = Some(path.clone());
        }
        log::info!(
            target: "remote-cache",
            "lookup hit digest={} bytes={} elapsed_ms={}",
            digest_prefix,
            marked_length,
            lookup_started.elapsed().as_millis(),
        );
        Some(path)
    }

    pub fn bypass_ready_for_session(&self) {
        self.bypass_ready.store(true, Ordering::Release);
        if let Ok(mut published) = self.published_path.lock() {
            *published = None;
        }
    }

    pub fn fresh_staging(&self) -> AppResult<Self> {
        Ok(Self {
            cache_root: self.cache_root.clone(),
            cache_dir: self.cache_dir.clone(),
            digest: self.digest.clone(),
            legacy_data_path: self.legacy_data_path.clone(),
            staging: Arc::new(Mutex::new(CacheStaging { path: None })),
            ready_path: self.ready_path.clone(),
            expected_content_length: self.expected_content_length,
            expected_content_md5: self.expected_content_md5.clone(),
            expected_duration_ms: self.expected_duration_ms,
            max_cache_bytes: self.max_cache_bytes,
            published_path: Arc::new(Mutex::new(None)),
            bypass_ready: Arc::new(AtomicBool::new(true)),
        })
    }

    fn staging_path(&self) -> io::Result<PathBuf> {
        let mut staging = self
            .staging
            .lock()
            .map_err(|_| io::Error::other("cache staging lock poisoned"))?;
        if let Some(path) = &staging.path {
            return Ok(path.clone());
        }

        let path = create_cache_staging(&self.cache_dir, &self.digest)?;
        staging.path = Some(path.clone());
        Ok(path)
    }

    pub fn publish_complete_bytes(&self, bytes: &[u8]) -> AppResult<PathBuf> {
        self.prepare(bytes.len() as u64)
            .map_err(|err| AppError::Other(err.to_string()))?;
        self.write_range(0, bytes)
            .map_err(|err| AppError::Other(err.to_string()))?;
        self.mark_ready()
            .map_err(|err| AppError::Other(err.to_string()))?;
        self.published_path()
            .map_err(|err| AppError::Other(err.to_string()))?
            .ok_or_else(|| AppError::Other("cache publish produced no readable file".into()))
    }

    pub(crate) fn prepare_sequential_write(&self, total_len: u64) -> AppResult<PathBuf> {
        self.prepare(total_len)
            .map_err(|err| AppError::Other(err.to_string()))?;
        self.staging_path()
            .map_err(|err| AppError::Other(err.to_string()))
    }

    pub(crate) fn publish_sequential_write(&self) -> AppResult<PathBuf> {
        self.mark_ready()
            .map_err(|err| AppError::Other(err.to_string()))?;
        self.published_path()
            .map_err(|err| AppError::Other(err.to_string()))?
            .ok_or_else(|| AppError::Other("cache publish produced no readable file".into()))
    }

    fn prepare(&self, total_len: u64) -> io::Result<()> {
        if self
            .expected_content_length
            .is_some_and(|expected| expected != total_len)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "remote content length {} does not match expected {}",
                    total_len,
                    self.expected_content_length.unwrap_or_default()
                ),
            ));
        }
        if let Some(path) = self.ready_path() {
            if std::fs::metadata(&path)?.len() == total_len {
                return Ok(());
            }
        }
        if let Ok(mut published) = self.published_path.lock() {
            *published = None;
        }
        let staging_path = self.staging_path()?;
        if let Some(parent) = staging_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(staging_path)?;
        file.set_len(total_len)?;
        Ok(())
    }

    fn write_range(&self, start: u64, data: &[u8]) -> io::Result<()> {
        if data.is_empty() || self.published_path().ok().flatten().is_some() {
            return Ok(());
        }
        let end = start
            .checked_add(data.len() as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "cache range overflow"))?;
        let staging_path = self.staging_path()?;
        let staging_len = std::fs::metadata(&staging_path)?.len();
        if end > staging_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("cache range {}..{} exceeds {}", start, end, staging_len),
            ));
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(staging_path)?;
        file.seek(SeekFrom::Start(start))?;
        file.write_all(data)?;
        Ok(())
    }

    fn read_cached(&self, start: u64, out: &mut [u8]) -> io::Result<usize> {
        let published = self.published_path().ok().flatten();
        let path = match &published {
            Some(path) => path.clone(),
            None => self.staging_path()?,
        };
        let mut file = match std::fs::OpenOptions::new().read(true).open(path) {
            Ok(file) => file,
            Err(staging_error) if published.is_none() => {
                let Some(published_path) = self.published_path().ok().flatten() else {
                    return Err(staging_error);
                };
                std::fs::OpenOptions::new()
                    .read(true)
                    .open(published_path)?
            }
            Err(err) => return Err(err),
        };
        file.seek(SeekFrom::Start(start))?;
        file.read(out)
    }

    fn mark_ready(&self) -> io::Result<()> {
        if self.ready_path().is_some() {
            return Ok(());
        }
        let staging_path = self.staging_path()?;
        self.validate_expected_md5(&staging_path)?;
        let validated = validate_cache_file(
            &staging_path,
            std::fs::metadata(&staging_path)?.len(),
            self.expected_content_length,
            self.expected_duration_ms,
            None,
        )
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        // Windows 的 FlushFileBuffers 需要可写句柄
        std::fs::OpenOptions::new().read(true).write(true).open(&staging_path)?.sync_all()?;

        let staging_stem = staging_path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(&self.digest);
        let final_file_name = format!("{}.{}.audio", staging_stem, validated.sha256);
        let final_path = self.cache_dir.join(&final_file_name);
        if final_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("cache publish target already exists: {}", final_path.display()),
            ));
        }
        std::fs::rename(&staging_path, &final_path)?;
        sync_parent_directory(&final_path)?;
        let final_modified = std::fs::metadata(&final_path)
            .and_then(|metadata| metadata.modified())
            .ok();
        remember_validated_audio_cache(&final_path, final_modified, &validated);
        if let Ok(mut published) = self.published_path.lock() {
            *published = Some(final_path.clone());
        }

        let marker = format_cache_marker(
            validated.content_length,
            &validated.sha256,
            &final_file_name,
        );
        atomic_write_cache_marker(&self.ready_path, &marker)?;
        sync_parent_directory(&self.ready_path)?;
        prune_disk_cache(
            &self.cache_root,
            self.max_cache_bytes
                .saturating_sub(validated.content_length),
            &self.digest,
        );
        Ok(())
    }

    fn published_path(&self) -> io::Result<Option<PathBuf>> {
        self.published_path
            .lock()
            .map(|path| path.clone())
            .map_err(|_| io::Error::other("published cache path lock poisoned"))
    }

    fn resolve_marker_file(&self, file_name: &str) -> Option<PathBuf> {
        let path = Path::new(file_name);
        if path.components().count() != 1
            || path.extension().and_then(|value| value.to_str()) != Some("audio")
            || !file_name.starts_with(&self.digest)
        {
            return None;
        }
        Some(self.cache_dir.join(path))
    }
}

fn create_cache_staging(directory: &Path, digest: &str) -> io::Result<PathBuf> {
    std::fs::create_dir_all(directory)?;
    let staging_file = tempfile::Builder::new()
        .prefix(&format!("{}.", digest))
        .suffix(".part")
        .tempfile_in(directory)?;
    let (staging_file, staging_path) = staging_file
        .keep()
        .map_err(|err| err.error)?;
    drop(staging_file);
    Ok(staging_path)
}

fn format_cache_marker(content_length: u64, sha256: &str, file_name: &str) -> String {
    format!(
        "{}:{}:{}:{}",
        CACHE_MARKER_VERSION, content_length, sha256, file_name
    )
}

fn parse_cache_marker(marker: &str) -> Option<CacheMarker> {
    if let Some(content_length) = marker
        .strip_prefix("v1:")
        .and_then(|value| value.parse::<u64>().ok())
    {
        return (content_length > 0).then_some(CacheMarker::Legacy { content_length });
    }

    let mut parts = marker.splitn(4, ':');
    if parts.next()? != CACHE_MARKER_VERSION {
        return None;
    }
    let content_length = parts.next()?.parse::<u64>().ok()?;
    let sha256 = parts.next()?.trim().to_ascii_lowercase();
    let file_name = parts.next()?.trim().to_string();
    if content_length == 0
        || sha256.len() != 64
        || !sha256.bytes().all(|value| value.is_ascii_hexdigit())
        || file_name.is_empty()
    {
        return None;
    }
    Some(CacheMarker::Validated {
        content_length,
        sha256,
        file_name,
    })
}

fn validate_cache_file(
    path: &Path,
    marked_length: u64,
    expected_length: Option<u64>,
    expected_duration_ms: Option<u64>,
    expected_sha256: Option<&str>,
) -> Result<ValidatedCacheFile, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|err| format!("cache metadata unavailable: {}", err))?;
    let content_length = metadata.len();
    if !metadata.is_file() || content_length == 0 || content_length != marked_length {
        return Err(format!(
            "cache length mismatch: marked={}, actual={}",
            marked_length, content_length
        ));
    }
    if expected_length.is_some_and(|expected| expected != content_length) {
        return Err(format!(
            "cache length does not match source: expected={}, actual={}",
            expected_length.unwrap_or_default(),
            content_length
        ));
    }
    let modified = metadata.modified().ok();
    if let Some(validated) = reuse_validated_audio_cache(
        path,
        content_length,
        modified,
        expected_sha256,
    ) {
        return Ok(validated);
    }

    validate_audio_file(path, expected_duration_ms)?;
    let sha256 = sha256_file(path).map_err(|err| format!("cache hash failed: {}", err))?;
    if expected_sha256.is_some_and(|expected| !sha256.eq_ignore_ascii_case(expected)) {
        return Err("cache SHA-256 mismatch".into());
    }
    let validated = ValidatedCacheFile {
        content_length,
        sha256,
    };
    remember_validated_audio_cache(path, modified, &validated);
    Ok(validated)
}

fn validate_published_cache_file(
    path: &Path,
    marked_length: u64,
    expected_length: Option<u64>,
    _expected_duration_ms: Option<u64>,
    expected_sha256: &str,
    file_name: &str,
) -> Result<ValidatedCacheFile, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|err| format!("cache metadata unavailable: {}", err))?;
    let content_length = metadata.len();
    if !metadata.is_file() || content_length == 0 || content_length != marked_length {
        return Err(format!(
            "cache length mismatch: marked={}, actual={}",
            marked_length, content_length
        ));
    }
    if expected_length.is_some_and(|expected| expected != content_length) {
        return Err(format!(
            "cache length does not match source: expected={}, actual={}",
            expected_length.unwrap_or_default(),
            content_length
        ));
    }
    let expected_suffix = format!(".{}.audio", expected_sha256);
    if !file_name.ends_with(&expected_suffix) {
        return Err("cache file name does not match published SHA-256".into());
    }

    if reuse_validated_audio_cache(
        path,
        content_length,
        metadata.modified().ok(),
        Some(expected_sha256),
    )
    .is_none()
    {
        // 冷启动或文件已变化时重新检查内容，文件名不能证明文件仍然完整
        // 校验摘要即可，无需再次解析整个音频容器
        let actual = sha256_file(path).map_err(|err| format!("cache hash failed: {err}"))?;
        if !actual.eq_ignore_ascii_case(expected_sha256) {
            return Err("cache SHA-256 mismatch".into());
        }
    }
    let validated = ValidatedCacheFile {
        content_length,
        sha256: expected_sha256.to_string(),
    };
    remember_validated_audio_cache(path, metadata.modified().ok(), &validated);
    Ok(validated)
}

fn reuse_validated_audio_cache(
    path: &Path,
    content_length: u64,
    modified: Option<SystemTime>,
    expected_sha256: Option<&str>,
) -> Option<ValidatedCacheFile> {
    let modified = modified?;
    let expected_sha256 = expected_sha256?;
    let cache = VALIDATED_AUDIO_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let cache = cache.lock().ok()?;
    let stamp = cache.get(path)?;
    if stamp.content_length != content_length
        || stamp.modified != modified
        || !stamp.sha256.eq_ignore_ascii_case(expected_sha256)
    {
        return None;
    }
    Some(ValidatedCacheFile {
        content_length,
        sha256: stamp.sha256.clone(),
    })
}

fn remember_validated_audio_cache(
    path: &Path,
    modified: Option<SystemTime>,
    validated: &ValidatedCacheFile,
) {
    let Some(modified) = modified else {
        return;
    };
    let cache = VALIDATED_AUDIO_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut cache) = cache.lock() else {
        return;
    };
    if cache.len() >= MAX_VALIDATED_CACHE_STAMPS && !cache.contains_key(path) {
        cache.clear();
    }
    cache.insert(
        path.to_path_buf(),
        ValidatedCacheStamp {
            content_length: validated.content_length,
            modified,
            sha256: validated.sha256.clone(),
        },
    );
}

fn validate_audio_file(path: &Path, expected_duration_ms: Option<u64>) -> Result<(), String> {
    let decoder = SymphoniaAudioDecoder::new_file(path)?;
    let actual_duration_ms = decoder
        .total_duration()
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    if cache_duration_is_suspicious(expected_duration_ms, actual_duration_ms) {
        return Err(format!(
            "cache duration is suspiciously short: expected={}ms, actual={}ms",
            expected_duration_ms.unwrap_or_default(),
            actual_duration_ms
        ));
    }
    Ok(())
}

fn cache_duration_is_suspicious(expected_duration_ms: Option<u64>, actual_duration_ms: u64) -> bool {
    let Some(expected_duration_ms) = expected_duration_ms.filter(|duration| *duration > 0) else {
        return false;
    };
    actual_duration_ms > 0
        && actual_duration_ms.saturating_add(CACHE_MIN_DURATION_GAP_MS) < expected_duration_ms
        && u128::from(actual_duration_ms) * 100
            < u128::from(expected_duration_ms) * CACHE_MIN_DURATION_RATIO_PERCENT as u128
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn atomic_write_cache_marker(path: &Path, marker: &str) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("cache marker has no parent directory"))?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".ready-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    temporary.write_all(marker.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|err| err.error)
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("cache file has no parent directory"))?;
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn prune_disk_cache(root: &PathBuf, max_cache_bytes: u64, keep_digest: &str) {
    if !root.exists() {
        return;
    }

    let mut groups: HashMap<String, Vec<(PathBuf, u64, std::time::SystemTime)>> = HashMap::new();
    collect_disk_cache_files(root, &mut groups, keep_digest);
    let mut total = groups
        .values()
        .flat_map(|files| files.iter())
        .map(|(_, size, _)| *size)
        .sum::<u64>();
    if total <= max_cache_bytes {
        return;
    }

    let mut ordered = groups.into_iter().collect::<Vec<_>>();
    ordered.sort_by_key(|(_, files)| {
        files
            .iter()
            .map(|(_, _, modified)| *modified)
            .min()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    for (_, files) in ordered {
        if total <= max_cache_bytes {
            break;
        }
        for (path, size, _) in files {
            if std::fs::remove_file(path).is_ok() {
                total = total.saturating_sub(size);
            }
        }
    }
}

fn collect_disk_cache_files(
    root: &PathBuf,
    groups: &mut HashMap<String, Vec<(PathBuf, u64, std::time::SystemTime)>>,
    keep_digest: &str,
) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_disk_cache_files(&path, groups, keep_digest);
            continue;
        }
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        if !matches!(extension, "audio" | "part" | "ready") {
            continue;
        }
        let metadata = entry.metadata().ok();
        if extension == "part"
            && metadata
                .as_ref()
                .and_then(|value| value.modified().ok())
                .and_then(|modified| modified.elapsed().ok())
                .is_none_or(|age| age < STALE_CACHE_PART_AGE)
        {
            continue;
        }
        let Some(digest) = cache_file_digest(&path) else {
            continue;
        };
        if digest == keep_digest {
            continue;
        }
        groups.entry(digest.to_string()).or_default().push((
            path,
            metadata.as_ref().map(|value| value.len()).unwrap_or(0),
            metadata
                .and_then(|value| value.modified().ok())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
        ));
    }
}

fn cache_file_digest(path: &Path) -> Option<&str> {
    let file_name = path.file_name()?.to_str()?;
    let digest = file_name.get(0..64)?;
    (digest.bytes().all(|value| value.is_ascii_hexdigit())).then_some(digest)
}

impl MediaSource for RemoteAudioSource {
    fn is_seekable(&self) -> bool {
        // 仅 try_new 扫描顶层 atom 时隐藏 seekable，避免扫完整 mdat
        if self.is_demuxer_open_sequential() {
            return false;
        }
        // virtual-body：打开后必须可 seek，样本要从 mdat 回跳
        if self.inner.virtual_body_origin.load(Ordering::Acquire) > 0 {
            return true;
        }
        self.access_mode.is_seekable()
    }

    fn byte_len(&self) -> Option<u64> {
        if self.is_demuxer_open_sequential() {
            return None;
        }
        if self.inner.virtual_body_origin.load(Ordering::Acquire) > 0 {
            return Some(self.logical_len());
        }
        self.access_mode
            .is_seekable()
            .then_some(self.inner.total_len)
    }
}

fn is_fragmented_mp4_url(value: &str) -> bool {
    url::Url::parse(value)
        .ok()
        .map(|url| url.path().to_ascii_lowercase().ends_with(".m4s"))
        .unwrap_or_else(|| {
            value
                .split(['?', '#'])
                .next()
                .is_some_and(|path| path.to_ascii_lowercase().ends_with(".m4s"))
        })
}

/// 从首包字节嗅探分片 MP4
///
/// 仅靠 URL 后缀不够：Bilibili DASH 用 `.m4s`，但 YouTube 的 googlevideo 直链
/// 形如 `/videoplayback?itag=140&...`，路径没有任何扩展名，却同样是分片 MP4
/// （`moov` 里 `stbl` 为空，时间轴全在 `moof` 中）。误判成普通 MP4 会让
/// demuxer 拿着空样本表做 seek，长内容上表现为 seek 无响应。
fn detect_fragmented_mp4(data: &[u8]) -> bool {
    let mut offset = 0usize;
    while offset + 8 <= data.len() {
        let Ok(size_bytes) = data[offset..offset + 4].try_into() else {
            return false;
        };
        let size = u32::from_be_bytes(size_bytes) as usize;
        let kind = &data[offset + 4..offset + 8];

        // styp/sidx/moof 是分片 MP4 的直接标志
        if kind == b"styp" || kind == b"sidx" || kind == b"moof" {
            return true;
        }
        // moov 内含 mvex 表示后续为 movie fragment
        if kind == b"moov" {
            let header_len = if size == 1 { 16 } else { 8 };
            let end = if size == 0 {
                data.len()
            } else {
                (offset + size).min(data.len())
            };
            let body_start = (offset + header_len).min(end);
            if data[body_start..end]
                .windows(4)
                .any(|window| window == b"mvex")
            {
                return true;
            }
        }

        let header_len = if size == 1 { 16 } else { 8 };
        let atom_size = if size == 1 {
            let Ok(size_bytes) = data[offset + 8..offset + 16].try_into() else {
                return false;
            };
            u64::from_be_bytes(size_bytes) as usize
        } else if size == 0 {
            return false;
        } else {
            size
        };
        if atom_size < header_len {
            return false;
        }
        // mdat 之后不会再有头部信息，且首包通常放不下整个 mdat
        if kind == b"mdat" {
            return false;
        }
        offset += atom_size;
    }
    false
}

pub struct SymphoniaAudioDecoder {
    decoder: Box<dyn Decoder>,
    current_frame_offset: usize,
    format: Box<dyn FormatReader>,
    track_id: u32,
    total_duration: Option<Time>,
    buffer: SampleBuffer<f32>,
    spec: SignalSpec,
    retry_virtual_body_edges: bool,
}

impl SymphoniaAudioDecoder {
    pub fn new(source: Box<dyn MediaSource>, extension: Option<&str>) -> Result<Self, String> {
        let options = MediaSourceStreamOptions {
            buffer_len: REMOTE_DECODER_BUFFER_BYTES,
        };
        let mss = MediaSourceStream::new(source, options);
        Self::init(mss, extension).map_err(|err| format!("Decode error: {}", err))
    }

    /// 远程源：若处于 demuxer_open_sequential，打开时对 isomp4 隐藏 seekable
    pub fn new_remote(source: RemoteAudioSource) -> Result<Self, String> {
        Self::new_remote_virtual(source, false)
    }

    /// 远程源：virtual_body 模式用于长内容 seek（header + 目标 moof）
    pub fn new_remote_virtual(
        mut source: RemoteAudioSource,
        keep_virtual_body: bool,
    ) -> Result<Self, String> {
        let finish_handle = RemoteAudioSource {
            inner: Arc::clone(&source.inner),
            pos: 0,
            access_mode: source.access_mode,
            read_cancellation: source.read_cancellation.clone(),
        };
        source.pos = 0;
        if keep_virtual_body {
            source
                .inner
                .demuxer_open_sequential
                .store(true, Ordering::Release);

            // 关键修复：把 header + 目标 moof/mdat 拼成连续内存 demux。
            // 旧路径让 isomp4 在 virtual-body 上自己 next_packet：
            // - MoovSegment stco 样本会被映射到中段字节 → 假 AAC
            // - Unsupported("coupling channel element") 会直接杀 worker
            // - 段边界假 EOF 会让 worker frames≈1k 就 exhausted
            // 拼接后 demuxer 看到的是真实连续字节，坐标无歧义。
            return Self::new_remote_virtual_spliced(source, finish_handle);
        }
        let buffer_len = REMOTE_DECODER_BUFFER_BYTES;
        let options = MediaSourceStreamOptions { buffer_len };
        let mss = MediaSourceStream::new(Box::new(source), options);
        let decoder = Self::init_remote(mss, None)
            .map_err(|err| format!("Decode error: {}", err))?;
        finish_handle.finish_demuxer_open();
        finish_handle.clear_virtual_body();
        log::info!(
            target: "remote-audio",
            "demuxer open finished sequential_cleared=true virtual_body=false",
        );
        Ok(decoder)
    }

    /// virtual-body：header ∥ 目标 moof 的**惰性**拼接视图
    ///
    /// 不预先把 body 拉进内存。`RemoteAudioSource` 自己做逻辑↔物理地址翻译
    /// （`map_logical_to_physical` / `logical_len` / `normalize_seek_offset`），
    /// demuxer 读到哪就按需拉哪一块。
    ///
    /// 之前那版先阻塞拉满 2MB 再交给 `Cursor`，两个后果都致命：
    /// - 慢：4 次串行 Range 请求，实测 seek 要等 8 秒
    /// - 错：2MB 处凭空多出一个 EOF，probe 扫 atom 直接撞上 end of stream
    fn new_remote_virtual_spliced(
        source: RemoteAudioSource,
        finish_handle: RemoteAudioSource,
    ) -> Result<Self, String> {
        let header_end = source.inner.header_end.load(Ordering::Acquire);
        let origin = source.inner.virtual_body_origin.load(Ordering::Acquire);
        if header_end == 0 || origin == 0 {
            return Err("virtual-body splice requires header_end and body_origin".into());
        }
        log::info!(
            target: "remote-audio",
            "virtual-body lazy view header_end={}, body_origin={}, logical_len={}",
            header_end,
            origin,
            source.logical_len(),
        );

        let options = MediaSourceStreamOptions {
            buffer_len: REMOTE_VIRTUAL_DECODER_BUFFER_BYTES,
        };
        let mss = MediaSourceStream::new(Box::new(source), options);
        // 探测期间必须维持 demuxer_open_sequential：
        // is_seekable 一旦为真，try_new 会把整条逻辑流的顶层 atom 扫一遍去找
        // 全部 sidx，等于把刚省下的顺序读又加回来。顺序模式下它遇到第一个
        // moof 就 break，正好停在目标分片上。探测结束后由 init 清标志，
        // 之后的样本级 Seek 与 ignore_bytes 跳段才能生效。
        let decoder = Self::init_remote_spliced(mss, Some(finish_handle))
            .map_err(|err| format!("Decode error: {}", err))?;
        log::info!(
            target: "remote-audio",
            "demuxer open finished sequential_cleared=true virtual_body=true lazy=true",
        );
        Ok(decoder)
    }

    pub fn new_file(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|err| format!("Cannot open file: {}", err))?;
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| !matches!(*value, "audio" | "part"));
        Self::new(Box::new(file), extension)
    }

    fn init(
        mss: MediaSourceStream,
        extension: Option<&str>,
    ) -> symphonia::core::errors::Result<Self> {
        let mut hint = Hint::new();
        if let Some(ext) = extension {
            hint.with_extension(ext);
        }

        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let metadata_opts: MetadataOptions = Default::default();
        let mut probed = get_probe().format(&hint, mss, &format_opts, &metadata_opts)?;

        let track_id = probed
            .format
            .tracks()
            .iter()
            .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or(SymphoniaError::Unsupported("No track with supported codec"))?
            .id;

        let track = probed
            .format
            .tracks()
            .iter()
            .find(|track| track.id == track_id)
            .ok_or(SymphoniaError::Unsupported("Selected track disappeared"))?;

        let mut decoder = get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
        let total_duration = track_duration(
            track.codec_params.time_base,
            track.codec_params.n_frames,
        );

        let mut decode_errors = 0usize;
        let decoded = loop {
            let packet = probed.format.next_packet()?;
            if packet.track_id() != track_id {
                continue;
            }

            match decoder.decode(&packet) {
                Ok(decoded) => break decoded,
                Err(SymphoniaError::DecodeError(_)) if decode_errors < MAX_DECODE_RETRIES => {
                    decode_errors += 1;
                }
                Err(err) => return Err(err),
            }
        };
        let spec = decoded.spec().to_owned();
        let buffer = Self::copy_buffer(decoded, &spec);

        Ok(Self {
            decoder,
            current_frame_offset: 0,
            format: probed.format,
            track_id,
            total_duration,
            buffer,
            spec,
            retry_virtual_body_edges: false,
        })
    }

    /// 远程打开：try_new 期间可保持 sequential；探测完成后立刻恢复 seekable 再读首包
    fn init_remote(
        mss: MediaSourceStream,
        enable_seek_after_probe: Option<RemoteAudioSource>,
    ) -> symphonia::core::errors::Result<Self> {
        let hint = Hint::new();
        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let metadata_opts: MetadataOptions = Default::default();
        let mut probed = get_probe().format(&hint, mss, &format_opts, &metadata_opts)?;

        // try_new 已结束：允许样本级 Seek（virtual-body 必需）
        if let Some(handle) = enable_seek_after_probe.as_ref() {
            handle.finish_demuxer_open();
        }

        let track_id = probed
            .format
            .tracks()
            .iter()
            .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or(SymphoniaError::Unsupported("No track with supported codec"))?
            .id;

        let track = probed
            .format
            .tracks()
            .iter()
            .find(|track| track.id == track_id)
            .ok_or(SymphoniaError::Unsupported("Selected track disappeared"))?;

        let mut decoder = get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
        let total_duration = track_duration(
            track.codec_params.time_base,
            track.codec_params.n_frames,
        );

        // 中段 moof 首帧可能非 RAP，多跳过一些 DecodeError。
        // 注意：fragmented 流里 moov 常无样本，首个 moof 的 first_ts 也可能是 0，
        // 绝不能按 ts==0 丢弃，否则会把目标段整段扔掉。
        let max_errors = if enable_seek_after_probe.is_some() {
            MAX_VIRTUAL_BODY_SKIP_PACKETS
        } else {
            MAX_DECODE_RETRIES
        };
        let mut decode_errors = 0usize;
        let mut packet_idx = 0usize;
        let decoded = loop {
            let packet = probed.format.next_packet()?;
            if packet.track_id() != track_id {
                continue;
            }
            packet_idx += 1;
            match decoder.decode(&packet) {
                Ok(decoded) => {
                    if enable_seek_after_probe.is_some() {
                        log::info!(
                            target: "remote-audio",
                            "virtual-body first good packet idx={} ts={} dur={} bytes={} after_errors={}",
                            packet_idx,
                            packet.ts(),
                            packet.dur(),
                            packet.buf().len(),
                            decode_errors,
                        );
                    }
                    break decoded;
                }
                Err(SymphoniaError::DecodeError(msg)) if decode_errors < max_errors => {
                    decode_errors += 1;
                    if enable_seek_after_probe.is_some() && decode_errors <= 4 {
                        log::warn!(
                            target: "remote-audio",
                            "virtual-body decode error idx={} ts={} bytes={} err={}",
                            packet_idx,
                            packet.ts(),
                            packet.buf().len(),
                            msg,
                        );
                    }
                    decoder.reset();
                }
                Err(err) => return Err(err),
            }
        };
        let spec = decoded.spec().to_owned();
        let buffer = Self::copy_buffer(decoded, &spec);

        Ok(Self {
            decoder,
            current_frame_offset: 0,
            format: probed.format,
            track_id,
            total_duration,
            buffer,
            spec,
            retry_virtual_body_edges: enable_seek_after_probe.is_some(),
        })
    }

    /// 拼接 demux：本地 Cursor 全程 seekable；首包必须来自 moof（跳过 moov stco 假样本）
    fn init_remote_spliced(
        mss: MediaSourceStream,
        enable_seek_after_probe: Option<RemoteAudioSource>,
    ) -> symphonia::core::errors::Result<Self> {
        let hint = Hint::new();
        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let metadata_opts: MetadataOptions = Default::default();
        let mut probed = get_probe().format(&hint, mss, &format_opts, &metadata_opts)?;

        // try_new 已结束：放开可 seek，样本级跳转与 ignore_bytes 跳段都要靠它
        if let Some(handle) = enable_seek_after_probe.as_ref() {
            handle.finish_demuxer_open();
        }

        let track_id = probed
            .format
            .tracks()
            .iter()
            .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or(SymphoniaError::Unsupported("No track with supported codec"))?
            .id;

        let track = probed
            .format
            .tracks()
            .iter()
            .find(|track| track.id == track_id)
            .ok_or(SymphoniaError::Unsupported("Selected track disappeared"))?;

        let mut decoder = get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
        let total_duration = track_duration(
            track.codec_params.time_base,
            track.codec_params.n_frames,
        );

        // 先吞掉 moov 段样本（stco 指向文件头 mdat，在拼接视图里是错误数据）
        // 策略：连续 DecodeError / Unsupported 都跳；一旦解出好包就停
        let mut skipped = 0usize;
        let mut decode_errors = 0usize;
        let decoded = loop {
            let packet = match probed.format.next_packet() {
                Ok(p) => p,
                Err(err)
                    if skipped < MAX_VIRTUAL_BODY_SKIP_MOOV_SAMPLES
                        && splice_skip_retryable(&err) =>
                {
                    // 可能还在假样本区；再试
                    skipped += 1;
                    if skipped <= 4 {
                        log::warn!(
                            target: "remote-audio",
                            "splice next_packet err while skipping moov: {}",
                            err,
                        );
                    }
                    continue;
                }
                Err(err) => return Err(err),
            };
            if packet.track_id() != track_id {
                continue;
            }
            skipped += 1;
            match decoder.decode(&packet) {
                Ok(decoded) => {
                    log::info!(
                        target: "remote-audio",
                        "splice first good packet after_skip={} ts={} dur={} bytes={}",
                        skipped,
                        packet.ts(),
                        packet.dur(),
                        packet.buf().len(),
                    );
                    break decoded;
                }
                Err(SymphoniaError::DecodeError(_))
                | Err(SymphoniaError::Unsupported(_))
                    if decode_errors < MAX_VIRTUAL_BODY_SKIP_PACKETS =>
                {
                    decode_errors += 1;
                    decoder.reset();
                }
                Err(SymphoniaError::ResetRequired) => {
                    decoder.reset();
                }
                Err(err)
                    if skipped < MAX_VIRTUAL_BODY_SKIP_MOOV_SAMPLES
                        && splice_skip_retryable(&err) =>
                {
                    // 其它瞬时错误也跳
                    if skipped <= 4 {
                        log::warn!(
                            target: "remote-audio",
                            "splice skip decode err: {}",
                            err,
                        );
                    }
                    decoder.reset();
                }
                Err(err) => return Err(err),
            }
            if skipped >= MAX_VIRTUAL_BODY_SKIP_MOOV_SAMPLES {
                return Err(SymphoniaError::DecodeError(
                    "virtual-body splice: could not find decodable moof sample",
                ));
            }
        };

        let spec = decoded.spec().to_owned();
        let buffer = Self::copy_buffer(decoded, &spec);
        Ok(Self {
            decoder,
            current_frame_offset: 0,
            format: probed.format,
            track_id,
            total_duration,
            buffer,
            spec,
            retry_virtual_body_edges: true,
        })
    }

    /// 已有好包后继续跳过坏包（当前未使用；拼接路径在 init 内已处理）
    #[allow(dead_code)]
    fn skip_to_fragment_samples(&mut self, max_packets: usize) -> usize {
        let mut skipped = 0usize;
        while skipped < max_packets {
            // 已有 buffer 里是好包就直接返回
            if !self.buffer.is_empty() && self.current_frame_offset < self.buffer.len() {
                break;
            }
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(_) => break,
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    decoded.spec().clone_into(&mut self.spec);
                    self.buffer = Self::copy_buffer(decoded, &self.spec);
                    self.current_frame_offset = 0;
                    break;
                }
                Err(SymphoniaError::DecodeError(_)) | Err(SymphoniaError::Unsupported(_)) => {
                    skipped += 1;
                    self.decoder.reset();
                }
                Err(SymphoniaError::ResetRequired) => {
                    self.decoder.reset();
                }
                Err(_) => break,
            }
        }
        skipped
    }

    fn copy_buffer(decoded: AudioBufferRef<'_>, spec: &SignalSpec) -> SampleBuffer<f32> {
        let duration = units::Duration::from(decoded.capacity() as u64);
        let mut buffer = SampleBuffer::<f32>::new(duration, *spec);
        buffer.copy_interleaved_ref(decoded);
        buffer
    }

    fn refine_position(&mut self, seek_res: SeekedTo) -> Result<(), PcmSeekError> {
        let mut samples_to_pass = seek_res.required_ts.saturating_sub(seek_res.actual_ts);
        let packet = loop {
            let candidate = self
                .format
                .next_packet()
                .map_err(|err| seek_error(format!("Could not refine seek: {}", err)))?;
            if candidate.dur() > samples_to_pass {
                break candidate;
            }
            samples_to_pass = samples_to_pass.saturating_sub(candidate.dur());
        };

        let mut decoded = self.decoder.decode(&packet);
        for _ in 0..MAX_DECODE_RETRIES {
            if decoded.is_ok() {
                break;
            }
            let packet = self
                .format
                .next_packet()
                .map_err(|err| seek_error(format!("Could not retry after seek: {}", err)))?;
            decoded = self.decoder.decode(&packet);
        }
        let decoded =
            decoded.map_err(|err| seek_error(format!("Could not decode after seek: {}", err)))?;
        decoded.spec().clone_into(&mut self.spec);
        self.buffer = Self::copy_buffer(decoded, &self.spec);
        self.current_frame_offset = samples_to_pass as usize * self.channels() as usize;
        Ok(())
    }
}

// skip_decode_errors 已移除：init_remote 内已处理坏包；外部再跳会让 buffer 与 format 失步

impl Iterator for SymphoniaAudioDecoder {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.current_frame_offset >= self.buffer.len() {
            // 中段流允许跳过更多坏包；真 EOF 才返回 None
            let mut decode_errors = 0usize;
            let mut consecutive_io = 0usize;
            let decoded = loop {
                let packet = match self.format.next_packet() {
                    Ok(packet) => {
                        consecutive_io = 0;
                        packet
                    }
                    Err(SymphoniaError::ResetRequired) => {
                        self.decoder.reset();
                        continue;
                    }
                    Err(SymphoniaError::IoError(err))
                        if self.retry_virtual_body_edges
                            && consecutive_io < MAX_VIRTUAL_BODY_SKIP_PACKETS
                            // Interrupted 一律是取消令牌（superseded/停会话），对本次
                            // 操作是永久态：重试 3 次只是把新 seek 的接管拖慢 60ms，
                            // 还会在日志里刷出误导性的 retry 噪音
                            && (err.kind() == std::io::ErrorKind::UnexpectedEof
                                || err.kind() == std::io::ErrorKind::WouldBlock) =>
                    {
                        // 预取窗口边缘瞬时 EOF / 被抢占：稍等再试，避免整段饿死
                        consecutive_io += 1;
                        if consecutive_io <= 3 {
                            log::warn!(
                                target: "remote-audio",
                                "virtual-body next_packet io retry={} kind={} msg={}",
                                consecutive_io,
                                err.kind(),
                                err,
                            );
                        }
                        std::thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    Err(SymphoniaError::IoError(error))
                        if error.kind() == io::ErrorKind::UnexpectedEof => return None,
                    Err(err) => {
                        log::warn!(
                            target: "remote-audio",
                            "virtual-body next_packet fatal: {}",
                            err,
                        );
                        return None;
                    }
                };
                if packet.track_id() != self.track_id {
                    continue;
                }

                match self.decoder.decode(&packet) {
                    Ok(decoded) => break decoded,
                    // DecodeError + Unsupported（如 aac coupling）都可跳，不能当 fatal 杀 worker
                    Err(SymphoniaError::DecodeError(_))
                    | Err(SymphoniaError::Unsupported(_))
                        if decode_errors < MAX_VIRTUAL_BODY_SKIP_PACKETS =>
                    {
                        decode_errors += 1;
                        if decode_errors <= 4 || decode_errors.is_multiple_of(16) {
                            log::warn!(
                                target: "remote-audio",
                                "virtual-body stream decode skip count={}",
                                decode_errors,
                            );
                        }
                        self.decoder.reset();
                    }
                    Err(SymphoniaError::ResetRequired) => {
                        self.decoder.reset();
                    }
                    Err(err) => {
                        log::warn!(
                            target: "remote-audio",
                            "virtual-body stream decode fatal: {}",
                            err,
                        );
                        return None;
                    }
                }
            };
            decoded.spec().clone_into(&mut self.spec);
            self.buffer = Self::copy_buffer(decoded, &self.spec);
            self.current_frame_offset = 0;
        }

        let sample = *self.buffer.samples().get(self.current_frame_offset)?;
        self.current_frame_offset += 1;
        Some(sample)
    }
}

impl PcmSource for SymphoniaAudioDecoder {
    fn channels(&self) -> u16 {
        self.spec.channels.count() as u16
    }

    fn sample_rate(&self) -> u32 {
        self.spec.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total_duration.map(time_to_duration)
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), PcmSeekError> {
        let time = seek_target_time(self.total_duration, pos);

        let channel_offset = self.current_frame_offset % self.channels() as usize;
        let seek_res = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: None,
                },
            )
            .map_err(|err| seek_error(format!("Format seek failed: {}", err)))?;
        self.decoder.reset();
        self.refine_position(seek_res)?;
        self.current_frame_offset += channel_offset;
        Ok(())
    }
}

async fn probe_range_len(
    client: &reqwest::Client,
    url: &str,
    referer: &str,
    initial_block_bytes: u64,
) -> AppResult<(u64, Vec<u8>)> {
    let range_end = initial_block_bytes.max(REMOTE_INITIAL_BLOCK_BYTES).saturating_sub(1);
    let response = client
        .get(url)
        .header(REFERER, referer)
        .header(USER_AGENT, playback_user_agent(url))
        .header(RANGE, format!("bytes=0-{}", range_end))
        .timeout(REMOTE_PROBE_TIMEOUT)
        .send()
        .await
        .map_err(AppError::Network)?;

    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(AppError::Audio(format!(
            "Remote source does not support HTTP Range: {}",
            response.status()
        )));
    }

    let content_range = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(parse_content_range)
        .ok_or_else(|| {
            AppError::Audio("Remote Range response did not include a valid byte range".into())
        })?;
    let data = response
        .bytes()
        .await
        .map_err(AppError::Network)?
        .to_vec();
    validate_http_range_data(content_range, 0, range_end, &data)
        .map_err(|err| AppError::Audio(err.to_string()))?;

    Ok((content_range.total, data))
}

fn fetch_range_block(
    inner: &RemoteAudioInner,
    start: u64,
    end: u64,
    read_cancellation: Option<&RemoteReadCancellation>,
) -> io::Result<Vec<u8>> {
    tauri::async_runtime::block_on(fetch_range_block_async(
        inner,
        start,
        end,
        read_cancellation,
    ))
}

async fn fetch_range_block_async(
    inner: &RemoteAudioInner,
    start: u64,
    end: u64,
    read_cancellation: Option<&RemoteReadCancellation>,
) -> io::Result<Vec<u8>> {
    let request_started = Instant::now();
    inner.ensure_playback_current()?;
    if read_cancellation.is_some_and(RemoteReadCancellation::is_cancelled) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "remote read superseded",
        ));
    }
    let requested_len = end.saturating_sub(start).saturating_add(1);
    // 超时预算随块大小/近期吞吐缩放：块大或网慢时给足传输时间，
    // 防止固定 15s 在慢链路上"块越大越必超时"
    let request_timeout = request_timeout_for(requested_len, inner.throughput_bps());
    let request = async {
        let response = inner
            .client
            .get(&inner.url)
            .header(REFERER, inner.referer.as_str())
            .header(USER_AGENT, playback_user_agent(&inner.url))
            .header(RANGE, format!("bytes={}-{}", start, end))
            .timeout(request_timeout)
            .send()
            .await?;
        let status = response.status();
        let content_range = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(parse_content_range);
        // 只有 206 的响应体才会进入缓存。对 200/4xx 直接丢弃 response，
        // 避免 Range 被网关忽略时把整首音频读进内存后再报错（SR-04）
        let bytes = if status == StatusCode::PARTIAL_CONTENT {
            response.bytes().await?.to_vec()
        } else {
            Vec::new()
        };
        Ok::<_, reqwest::Error>((status, content_range, bytes))
    };
    tokio::pin!(request);
    let result = tokio::select! {
        result = &mut request => {
            match result {
                Ok(result) => result,
                Err(error) => {
                    let elapsed_ms = request_started.elapsed().as_millis() as u64;
                    let kind = classify_reqwest_error(&error);
                    // 超时/中断说明链路吞吐撑不起当前块：向下修正估计，
                    // 让下一个块（含折半重试）自动缩小
                    if matches!(
                        kind,
                        io::ErrorKind::TimedOut | io::ErrorKind::ConnectionAborted
                    ) {
                        inner.record_fetch_failure(requested_len, elapsed_ms);
                    }
                    log::warn!(
                        target: "remote-range",
                        "request failed generation={}, range={}..{}, elapsed_ms={}, timeout_ms={}, kind={:?}, error={}",
                        inner.expected_generation,
                        start,
                        end,
                        elapsed_ms,
                        request_timeout.as_millis(),
                        kind,
                        summarize_remote_error(&error),
                    );
                    return Err(io::Error::new(kind, error.to_string()));
                }
            }
        }
        () = wait_for_remote_read_cancellation(inner, read_cancellation) => {
            log::info!(
                target: "remote-range",
                "request cancelled generation={}, range={}..{}, elapsed_ms={}",
                inner.expected_generation,
                start,
                end,
                request_started.elapsed().as_millis(),
            );
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "remote read superseded",
            ));
        }
    };

    let (status, content_range, data) = result;
    let elapsed_ms = request_started.elapsed().as_millis();
    if start == 0 || elapsed_ms >= 200 {
        log::info!(
            target: "remote-range",
            "response generation={}, range={}..{}, status={}, bytes={}, elapsed_ms={}",
            inner.expected_generation,
            start,
            end,
            status,
            data.len(),
            elapsed_ms,
        );
    } else {
        log::debug!(
            target: "remote-range",
            "response generation={}, range={}..{}, status={}, bytes={}, elapsed_ms={}",
            inner.expected_generation,
            start,
            end,
            status,
            data.len(),
            elapsed_ms,
        );
    }
    if status == StatusCode::PARTIAL_CONTENT {
        let content_range = content_range.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "partial response omitted Content-Range",
            )
        })?;
        if content_range.total != inner.total_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "remote length changed from {} to {}",
                    inner.total_len, content_range.total
                ),
            ));
        }
        validate_http_range_data(content_range, start, end, &data)?;
        inner.record_fetch_success(data.len() as u64, elapsed_ms.max(1) as u64);
        write_disk_range(inner, start, &data);
        return Ok(data);
    }

    if status == StatusCode::RANGE_NOT_SATISFIABLE {
        // 远端内容长度发生变化时，空 Vec 会被 Read 当作 EOF，导致无声截断
        // 并错误推进到下一首。把永久范围错误交给上层的 URL 刷新/恢复路径
        // 处理，不能把它伪装成正常结束（SR-03）
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "remote range no longer satisfiable: 416",
        ));
    }

    if status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED {
        // 对齐 Android YouTubeGoogleVideoRangeSupport.shouldRetryChunkError：
        // 403 是 CDN 拒绝访问（直链过期/远端 range 风控），重试与缩块都无解，
        // 必须按永久错误上抛让上层回滚或换直链；同时停掉本源后续预取孵化，
        // 否则每次读都会重新发起注定 403 的后台请求
        inner.remote_forbidden.store(true, Ordering::Release);
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("remote range request forbidden: {}", status),
        ));
    }

    Err(io::Error::other(format!(
        "unexpected remote range status: {}",
        status
    )))
}

async fn wait_for_remote_read_cancellation(
    inner: &RemoteAudioInner,
    read_cancellation: Option<&RemoteReadCancellation>,
) {
    while !inner.playback_cancelled()
        && !read_cancellation.is_some_and(RemoteReadCancellation::is_cancelled)
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for_generation_change(generation: &AtomicU64, expected: u64) {
    while generation.load(Ordering::Acquire) == expected {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// 单请求实测吞吐样本（bytes/s）
fn throughput_sample_bps(bytes: u64, elapsed_ms: u64) -> u64 {
    if bytes == 0 || elapsed_ms == 0 {
        return 0;
    }
    (u128::from(bytes) * 1000 / u128::from(elapsed_ms)) as u64
}

/// 成功样本按固定权重混入 EWMA；首个样本直接采纳
fn ewma_bps_after_success(old_bps: u64, sample_bps: u64) -> u64 {
    if sample_bps == 0 {
        return old_bps;
    }
    if old_bps == 0 {
        return sample_bps;
    }
    let keep = 100 - THROUGHPUT_EWMA_NEW_WEIGHT_PERCENT;
    ((u128::from(old_bps) * u128::from(keep)
        + u128::from(sample_bps) * u128::from(THROUGHPUT_EWMA_NEW_WEIGHT_PERCENT))
        / 100) as u64
}

/// 失败样本（超时/传输中断）：requested/elapsed 只是真实吞吐的上界，
/// 只允许把估计往下拉；跑得太短的失败（连接秒断）不计入
fn ewma_bps_after_failure(old_bps: u64, requested_bytes: u64, elapsed_ms: u64) -> u64 {
    if elapsed_ms < THROUGHPUT_FAILURE_MIN_ELAPSED_MS {
        return old_bps;
    }
    let upper_bound = throughput_sample_bps(requested_bytes, elapsed_ms);
    if upper_bound == 0 {
        return old_bps;
    }
    if old_bps == 0 {
        return upper_bound;
    }
    if upper_bound >= old_bps {
        return old_bps;
    }
    ewma_bps_after_success(old_bps, upper_bound)
}

/// 按近期吞吐选块：目标 8s 内可完成，64KB 对齐，夹在 [64KB, max_block]。
/// 无样本时用调用方默认块（保持冷启动行为不变）
fn adaptive_block_bytes(ewma_bps: u64, default_block: u64, max_block: u64) -> u64 {
    if ewma_bps == 0 {
        return default_block.clamp(REMOTE_MIN_BLOCK_BYTES, max_block.max(REMOTE_MIN_BLOCK_BYTES));
    }
    let raw = ewma_bps.saturating_mul(REMOTE_BLOCK_FILL_TARGET_SECS);
    let aligned = (raw / REMOTE_MIN_BLOCK_BYTES) * REMOTE_MIN_BLOCK_BYTES;
    aligned.clamp(REMOTE_MIN_BLOCK_BYTES, max_block.max(REMOTE_MIN_BLOCK_BYTES))
}

/// per-request 超时预算：预计传输时长 ×1.5 + 5s 固定裕量，夹在 [20s, 60s]。
/// 无吞吐样本时用 20s 下限（冷启动块都不大，20s 足够 40KB/s 拉完 512KB）
fn request_timeout_for(len: u64, ewma_bps: u64) -> Duration {
    if ewma_bps == 0 {
        return REMOTE_REQUEST_TIMEOUT_FLOOR;
    }
    let expected_ms = (u128::from(len) * 1000 / u128::from(ewma_bps)).min(u128::from(u64::MAX)) as u64;
    let budget = Duration::from_millis(expected_ms.saturating_mul(3) / 2 + 5_000);
    budget.clamp(REMOTE_REQUEST_TIMEOUT_FLOOR, REMOTE_REQUEST_TIMEOUT_CEILING)
}

/// 慢网降低预取并发：EWMA 低于码率时单流都喂不饱，多开只会挤占需求读；
/// 码率或吞吐未知时维持默认并发（冷启动行为不变）
fn prefetch_lane_count(ewma_bps: u64, bitrate_bps: u64) -> usize {
    if ewma_bps == 0 || bitrate_bps == 0 {
        return REMOTE_PREFETCH_CONCURRENCY;
    }
    if ewma_bps < bitrate_bps {
        1
    } else if ewma_bps < bitrate_bps.saturating_mul(2) {
        2
    } else {
        REMOTE_PREFETCH_CONCURRENCY
    }
}

/// 需求读折半重试的下一个长度（下限 64KB）
fn halved_fetch_len(len: u64) -> u64 {
    (len / 2).clamp(REMOTE_MIN_BLOCK_BYTES.min(len), len.max(1))
}

/// 超时/传输中断类错误：可通过折半重试恢复；
/// 4xx/长度变化/取消（Interrupted）等永久失败不在此列
/// splice 打开期能否把 next_packet/decode 错误当「moov 假样本区」继续跳过：
/// 解码/格式类错误可跳；IO 错误里只有预取边缘的瞬时类可跳。
/// PermissionDenied（403 直链失效）、Interrupted（读已被更新操作取消）等
/// 永久态必须立即上抛——每次重试都会对同一 range 重发注定失败的网络请求，
/// 4096 次上限等于无限刷 403（实机日志：seek 后每 ~600ms 一次 403 卡死）
fn splice_skip_retryable(err: &SymphoniaError) -> bool {
    match err {
        SymphoniaError::IoError(io_err) => matches!(
            io_err.kind(),
            io::ErrorKind::UnexpectedEof
                | io::ErrorKind::WouldBlock
                | io::ErrorKind::TimedOut
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::BrokenPipe
        ),
        _ => true,
    }
}

fn is_transient_fetch_error(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::TimedOut
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::BrokenPipe
    )
}

fn can_return_partial_read(err: &io::Error) -> bool {
    is_transient_fetch_error(err) || err.kind() == io::ErrorKind::UnexpectedEof
}

/// reqwest 错误分类：超时（含 body 读取被 per-request 超时打断的
/// "error decoding response body"）与传输中断映射为可重试 kind
fn classify_reqwest_error(error: &reqwest::Error) -> io::ErrorKind {
    if error.is_timeout() {
        io::ErrorKind::TimedOut
    } else if error.is_connect() || error.is_body() || error.is_decode() || error.is_request() {
        io::ErrorKind::ConnectionAborted
    } else {
        io::ErrorKind::Other
    }
}

fn prefetch_window(total_len: u64, duration_hint_ms: u64) -> PrefetchWindow {
    let target_bytes = estimated_buffer_bytes(
        total_len,
        duration_hint_ms,
        REMOTE_PREFETCH_TARGET_MS,
        REMOTE_PREFETCH_FALLBACK_TARGET_BYTES,
        REMOTE_PREFETCH_MIN_TARGET_BYTES,
        REMOTE_PREFETCH_MAX_TARGET_BYTES,
    );
    let low_water_bytes = estimated_buffer_bytes(
        total_len,
        duration_hint_ms,
        REMOTE_PREFETCH_LOW_WATER_MS,
        REMOTE_PREFETCH_FALLBACK_LOW_WATER_BYTES,
        REMOTE_PREFETCH_MIN_LOW_WATER_BYTES,
        REMOTE_PREFETCH_MAX_LOW_WATER_BYTES,
    )
    .min((target_bytes / 2).max(1));

    PrefetchWindow {
        low_water_bytes,
        target_bytes,
    }
}

fn estimated_buffer_bytes(
    total_len: u64,
    duration_hint_ms: u64,
    buffer_duration_ms: u64,
    fallback_bytes: u64,
    min_bytes: u64,
    max_bytes: u64,
) -> u64 {
    if total_len == 0 {
        return 0;
    }
    if duration_hint_ms == 0 {
        return fallback_bytes.clamp(min_bytes, max_bytes).min(total_len);
    }

    let estimated = (u128::from(total_len) * u128::from(buffer_duration_ms))
        .div_ceil(u128::from(duration_hint_ms))
        .min(u128::from(u64::MAX)) as u64;
    estimated.clamp(min_bytes, max_bytes).min(total_len)
}

fn prefetch_plan(
    read_pos: u64,
    cached_end: u64,
    total_len: u64,
    window: PrefetchWindow,
) -> Option<PrefetchPlan> {
    let read_pos = read_pos.min(total_len);
    if read_pos >= total_len {
        return None;
    }

    let cached_end = cached_end.clamp(read_pos, total_len);
    let buffered_bytes = cached_end.saturating_sub(read_pos);
    if buffered_bytes > window.low_water_bytes {
        return None;
    }

    let target_end = read_pos.saturating_add(window.target_bytes).min(total_len);
    (cached_end < target_end).then_some(PrefetchPlan {
        start: cached_end,
        target_end,
    })
}

fn replenish_prefetch_window(inner: &Arc<RemoteAudioInner>, read_pos: u64) {
    if inner.playback_cancelled() {
        return;
    }
    // 远端已 403/401：直链对新 range 永久拒绝，不再孵化任何预取
    if inner.remote_forbidden.load(Ordering::Acquire) {
        return;
    }
    let cached_end = contiguous_cached_end(inner, read_pos);
    if cached_end >= inner.total_len {
        try_mark_disk_cache_ready(inner);
        return;
    }

    let Some(plan) = prefetch_plan(read_pos, cached_end, inner.total_len, inner.prefetch_window)
    else {
        return;
    };
    spawn_prefetch_sequence(inner.clone(), plan);
}

fn spawn_prefetch_sequence(inner: Arc<RemoteAudioInner>, plan: PrefetchPlan) {
    // 先占首块：既是 0 号 lane 的首个任务，也天然拒绝重复提交的整段预取
    let Some((first_start, first_end, first_entry)) = claim_prefetch_sequence_start(&inner, plan)
    else {
        return;
    };
    let epoch = inner.prefetch_epoch.load(Ordering::Acquire);
    // 共享游标：各 lane 从这里领取下一块起点，构成并发流水线，
    // 掩盖单请求往返延迟（串行 256KB 块的有效吞吐在慢链路上低于码率）
    let cursor = Arc::new(AtomicU64::new(first_end.saturating_add(1)));
    let stop = Arc::new(AtomicBool::new(false));
    // 慢网降并发：EWMA 低于码率时单流都喂不饱，多 lane 只会挤占需求读带宽
    let lanes = prefetch_lane_count(inner.throughput_bps(), inner.bitrate_bps);
    for lane in 0..lanes {
        let preclaimed =
            (lane == 0).then(|| (first_start, first_end, Arc::clone(&first_entry)));
        let inner = Arc::clone(&inner);
        let cursor = Arc::clone(&cursor);
        let stop = Arc::clone(&stop);
        tauri::async_runtime::spawn(async move {
            prefetch_lane(inner, plan, epoch, cursor, stop, preclaimed).await;
        });
    }
}

/// 当前预取块大小：按近期吞吐自适应（8s 可完成），无样本时用 512KB 默认
fn current_prefetch_block_bytes(inner: &RemoteAudioInner) -> u64 {
    adaptive_block_bytes(
        inner.throughput_bps(),
        REMOTE_PREFETCH_BLOCK_BYTES,
        REMOTE_ADAPTIVE_MAX_BLOCK_BYTES,
    )
}

/// 预取块长度：夹到窗口/文件边界
fn prefetch_block_len(block: u64, start: u64, limit: u64) -> u64 {
    block.min(limit.saturating_sub(start))
}

/// 从共享游标领取下一块 [start, end]。块大小逐次按最新吞吐估计计算，
/// CAS 推进游标保证变长块之间无空洞、无重叠
fn take_prefetch_block(cursor: &AtomicU64, limit: u64, block: u64) -> Option<(u64, u64)> {
    loop {
        let start = cursor.load(Ordering::Acquire);
        if start >= limit {
            return None;
        }
        let len = prefetch_block_len(block, start, limit);
        if len == 0 {
            return None;
        }
        if cursor
            .compare_exchange(start, start + len, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Some((start, start + len - 1));
        }
    }
}

/// 单条预取 lane：从共享游标领块 → 登记在途 → 拉取 → 入缓存 → 发布。
/// 出错/短响应/换代即停整条流水线，空洞由下一轮 replenish 从
/// contiguous_cached_end 重新规划补齐
async fn prefetch_lane(
    inner: Arc<RemoteAudioInner>,
    plan: PrefetchPlan,
    epoch: u64,
    cursor: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    preclaimed: Option<(u64, u64, Arc<InflightFetch>)>,
) {
    let limit = plan.target_end.min(inner.total_len);
    let mut pending = preclaimed;
    loop {
        let lane_cancelled = || {
            stop.load(Ordering::Acquire)
                || inner.playback_cancelled()
                || inner.prefetch_epoch.load(Ordering::Acquire) != epoch
        };
        let (start, end, entry) = match pending.take() {
            Some(next) => next,
            None => {
                // 需求读优先：解码线程正亲自拉块时暂缓开新请求，
                // 已在途的请求继续跑完，只是不再抢新带宽
                while inner.demand_fetches.load(Ordering::Acquire) > 0 && !lane_cancelled() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                if lane_cancelled() {
                    break;
                }
                // 每块领取时都按最新吞吐估计选块：慢网自动缩小、快网恢复大块
                let block = current_prefetch_block_bytes(&inner);
                let Some((start, end)) = take_prefetch_block(&cursor, limit, block) else {
                    break;
                };
                match claim_prefetch_start(&inner, start, end) {
                    Some(entry) => (start, end, entry),
                    // 已缓存或已被别人（需求读）拉着：跳过本块领下一块
                    None => continue,
                }
            }
        };
        if lane_cancelled() {
            release_prefetch_start(&inner, start, &entry);
            break;
        }
        // seek 换代时立刻中止在途预取请求，不让旧位置继续吃带宽
        let fetch = fetch_range_block_async(&inner, start, end, None);
        tokio::pin!(fetch);
        let result = tokio::select! {
            result = &mut fetch => result,
            () = wait_for_generation_change(&inner.prefetch_epoch, epoch) => Err(
                io::Error::new(io::ErrorKind::Interrupted, "prefetch epoch invalidated"),
            ),
        };

        let requested = end - start + 1;
        match result {
            Ok(data) if !data.is_empty() => {
                let fetched_len = data.len() as u64;
                // epoch 变了：丢弃本块，避免旧位置继续污染缓存中心
                if inner.prefetch_epoch.load(Ordering::Acquire) == epoch {
                    if let Ok(mut cache) = inner.cache.lock() {
                        if !cache_contains_position(&cache, start) {
                            cache.push(CachedSegment { start, data });
                            cache.sort_by_key(|segment| segment.start);
                            trim_cache(&inner, &mut cache, cache_center(&inner, start));
                        }
                    }
                } else {
                    stop.store(true, Ordering::Release);
                }
                release_prefetch_start(&inner, start, &entry);
                // 短响应在块尾留下空洞：停流水线交给 replenish 补齐
                if fetched_len < requested {
                    stop.store(true, Ordering::Release);
                    break;
                }
            }
            Ok(_) => {
                release_prefetch_start(&inner, start, &entry);
                stop.store(true, Ordering::Release);
                break;
            }
            Err(err) => {
                release_prefetch_start(&inner, start, &entry);
                if err.kind() != io::ErrorKind::Interrupted {
                    log::warn!(
                        target: "remote-audio",
                        "prefetch stopped at {}: {}",
                        start, err
                    );
                }
                stop.store(true, Ordering::Release);
                break;
            }
        }
    }
    // 走到文件尾时触发磁盘缓存收尾检查（内部校验覆盖完整性，幂等）
    if cursor.load(Ordering::Acquire) >= inner.total_len {
        try_mark_disk_cache_ready(&inner);
    }
}

fn claim_prefetch_sequence_start(
    inner: &RemoteAudioInner,
    plan: PrefetchPlan,
) -> Option<(u64, u64, Arc<InflightFetch>)> {
    let limit = plan.target_end.min(inner.total_len);
    let start = plan.start;
    if start >= limit {
        return None;
    }
    let block = current_prefetch_block_bytes(inner);
    let end = start + prefetch_block_len(block, start, limit) - 1;
    claim_prefetch_start(inner, start, end).map(|entry| (start, end, entry))
}

fn contiguous_cached_end(inner: &RemoteAudioInner, pos: u64) -> u64 {
    let mut ranges = Vec::new();
    if let Ok(cache) = inner.cache.lock() {
        ranges.extend(cache.iter().map(|segment| CachedRange {
            start: segment.start,
            end: segment.start.saturating_add(segment.data.len() as u64),
        }));
    }
    if let Ok(disk_ranges) = inner.disk_ranges.lock() {
        ranges.extend(disk_ranges.iter().copied());
    }
    ranges.sort_by_key(|range| range.start);

    let mut end = pos.min(inner.total_len);
    for range in ranges {
        if range.end <= end {
            continue;
        }
        if range.start > end {
            break;
        }
        end = range.end.min(inner.total_len);
    }
    end
}

fn write_disk_range(inner: &RemoteAudioInner, start: u64, data: &[u8]) {
    let Some(cache) = &inner.disk_cache else {
        return;
    };
    if cache.write_range(start, data).is_ok() {
        record_disk_range(inner, start, data.len() as u64);
    }
}

fn record_disk_range(inner: &RemoteAudioInner, start: u64, len: u64) {
    if len == 0 {
        return;
    }
    let mut next = CachedRange {
        start,
        end: start.saturating_add(len).min(inner.total_len),
    };
    let Ok(mut ranges) = inner.disk_ranges.lock() else {
        return;
    };
    ranges.push(next);
    ranges.sort_by_key(|range| range.start);

    let mut merged: Vec<CachedRange> = Vec::with_capacity(ranges.len());
    for range in ranges.iter().copied() {
        if let Some(last) = merged.last_mut() {
            if range.start <= last.end {
                last.end = last.end.max(range.end);
                continue;
            }
        }
        next = range;
        merged.push(next);
    }
    *ranges = merged;
}

fn disk_cached_len(inner: &RemoteAudioInner, pos: u64, wanted_len: usize) -> Option<usize> {
    let ranges = inner.disk_ranges.lock().ok()?;
    ranges.iter().find_map(|range| {
        if pos < range.start || pos >= range.end {
            return None;
        }
        Some((range.end - pos).min(wanted_len as u64) as usize)
    })
}

fn try_mark_disk_cache_ready(inner: &RemoteAudioInner) {
    let Some(cache) = &inner.disk_cache else {
        return;
    };
    let is_complete = inner
        .disk_ranges
        .lock()
        .map(|ranges| ranges_cover_source(&ranges, inner.total_len))
        .unwrap_or(false);
    if !is_complete
        || inner
            .cache_finalize_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        return;
    }

    let cache = cache.clone();
    if let Err(spawn_error) = std::thread::Builder::new()
        .name("audio-cache-finalize".into())
        .spawn(move || {
            if let Err(err) = cache.mark_ready() {
                log::warn!(target: "remote-audio", "cache finalize rejected: {}", err);
            }
        })
    {
        log::warn!(
            target: "remote-audio",
            "cache finalize worker unavailable: {}",
            spawn_error
        );
    }
}

fn ranges_cover_source(ranges: &[CachedRange], total_len: u64) -> bool {
    if total_len == 0 {
        return false;
    }

    let mut covered_end = 0u64;
    for range in ranges {
        if range.start > covered_end {
            return false;
        }
        covered_end = covered_end.max(range.end);
        if covered_end >= total_len {
            return true;
        }
    }
    false
}

fn remember_read_pos(inner: &RemoteAudioInner, pos: u64) {
    if let Ok(mut last_read_pos) = inner.last_read_pos.lock() {
        *last_read_pos = pos;
    }
}

fn cache_center(inner: &RemoteAudioInner, fallback: u64) -> u64 {
    inner.last_read_pos.lock().map(|pos| *pos).unwrap_or(fallback)
}

fn cache_contains_position(cache: &[CachedSegment], pos: u64) -> bool {
    cache.iter().any(|segment| {
        let segment_end = segment.start + segment.data.len() as u64;
        pos >= segment.start && pos < segment_end
    })
}

fn position_is_cached_range(inner: &RemoteAudioInner, start: u64, end: u64) -> bool {
    if end < start {
        return false;
    }
    // 只要 start 已缓存就认为窗口可用（按需读会补齐）
    if let Ok(cache) = inner.cache.lock() {
        if cache_contains_position(&cache, start) {
            return true;
        }
    }
    disk_cached_len(inner, start, 1).is_some()
}

/// 哪些流需要在 open 阶段用首包 sidx 建 seek 索引
///
/// symphonia 的 isomp4 对分片流的 format.seek 是顺着 moof 链爬的
/// O(文件大小) 行为，所以凡是分片内容都必须自建索引走字节落点：
/// - 嗅探出的分片单文件（YouTube DASH 直链，URL 看不出分片）
/// - LongFormProgressive（长内容渐进，sidx/moov 定位）
/// - FragmentedProgressive（Bilibili .m4s 直链）——旧代码漏掉了它，
///   2 小时 118MB 的 fMP4 每次 seek 都要顺序爬 moof ~12s
fn needs_head_seek_index(access_mode: RemoteAccessMode, sniffed_fragmented: bool) -> bool {
    sniffed_fragmented
        || matches!(
            access_mode,
            RemoteAccessMode::LongFormProgressive | RemoteAccessMode::FragmentedProgressive
        )
}

/// 只用首包的 sidx 建索引，成功返回 true；失败不写入，交给调用方兜底
fn build_seek_index_from_head(inner: &RemoteAudioInner, duration_hint_ms: u64) -> bool {
    // hint=0（时长未知）原样传入：parse 层只在 >0 时做 clamp。
    // 旧的 .max(1) 会把索引里所有 time_ms 夹到 ≤1ms，等于毁掉整张索引
    let Some(index) = parse_sidx_seek_index(inner, duration_hint_ms) else {
        return false;
    };
    if index.entries.is_empty() {
        return false;
    }
    log::info!(
        target: "remote-audio",
        "seek index from head sidx duration_ms={}, entries={}",
        index.duration_ms,
        index.entries.len()
    );
    let Ok(mut slot) = inner.seek_index.lock() else {
        return false;
    };
    *slot = Some(index);
    true
}

fn build_seek_index_for_long_form(
    inner: &RemoteAudioInner,
    duration_hint_ms: u64,
    allow_linear_fallback: bool,
) {
    // duration_hint_ms=0 表示时长未知：sidx 解析不受影响（自带 timescale，
    // clamp 在 parse 层按 >0 守卫跳过），但线性估算没有时长就无从落点，禁用
    let mut index = parse_sidx_seek_index(inner, duration_hint_ms)
        .or_else(|| parse_sidx_seek_index_from_tail(inner, duration_hint_ms));
    let allow_linear_fallback = allow_linear_fallback && duration_hint_ms > 0;
    if index.is_none() && !allow_linear_fallback {
        // 分片内容没解析到真实 sidx 时不启用虚拟 body：线性估算落点不可靠，
        // 保持原有的 format.seek 行为，避免把可用路径换成更差的路径
        log::info!(
            target: "remote-audio",
            "seek index skipped: no sidx found and linear fallback disabled"
        );
        return;
    }
    if index.is_none() {
        // 线性估算兜底：足够让字节跳转落到大致区域（仅时长已知时启用）
        let total = inner.total_len.max(1);
        let steps = 32u64;
        let mut entries = Vec::with_capacity(steps as usize + 1);
        for i in 0..=steps {
            let time_ms = duration_hint_ms.saturating_mul(i) / steps;
            let byte_offset = total.saturating_mul(i) / steps;
            entries.push(SeekIndexEntry {
                time_ms,
                byte_offset,
            });
        }
        index = Some(RemoteSeekIndex {
            duration_ms: duration_hint_ms,
            entries,
        });
        log::info!(
            target: "remote-audio",
            "seek index fallback linear duration_ms={}, entries={}",
            duration_hint_ms,
            steps + 1
        );
    } else if let Some(ref built) = index {
        log::info!(
            target: "remote-audio",
            "seek index from sidx duration_ms={}, entries={}",
            built.duration_ms,
            built.entries.len()
        );
    }
    if let Ok(mut slot) = inner.seek_index.lock() {
        *slot = index;
    }
}

fn parse_sidx_seek_index(inner: &RemoteAudioInner, duration_ms: u64) -> Option<RemoteSeekIndex> {
    // 从头缓存里找 sidx
    let head = {
        let cache = inner.cache.lock().ok()?;
        cache
            .iter()
            .find(|segment| segment.start == 0)
            .map(|segment| segment.data.clone())
    }?;
    parse_sidx_from_bytes(&head, 0, duration_ms, inner.total_len)
}

fn parse_sidx_seek_index_from_tail(
    inner: &RemoteAudioInner,
    duration_ms: u64,
) -> Option<RemoteSeekIndex> {
    let cache = inner.cache.lock().ok()?;
    let tail = cache
        .iter()
        .filter(|segment| segment.start > 0)
        .max_by_key(|segment| segment.start)?;
    parse_sidx_from_bytes(&tail.data, tail.start, duration_ms, inner.total_len)
}


fn detect_mp4_header_end(data: &[u8]) -> Option<u64> {
    let mut offset = 0usize;
    while offset + 8 <= data.len() {
        let size = u32::from_be_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
        let kind = &data[offset + 4..offset + 8];
        let header_len = if size == 1 { 16 } else { 8 };
        let atom_size = if size == 1 {
            if offset + 16 > data.len() {
                break;
            }
            u64::from_be_bytes(data[offset + 8..offset + 16].try_into().ok()?) as usize
        } else if size == 0 {
            // 到文件尾：对 header 探测无意义
            break;
        } else {
            size
        };
        if atom_size < header_len {
            break;
        }
        // 首个 mdat/moof 即媒体起点
        if kind == b"mdat" || kind == b"moof" {
            return Some(offset as u64);
        }
        if offset + atom_size > data.len() {
            // 不完整 atom：若不是 mdat/moof，停止
            break;
        }
        offset += atom_size;
    }
    None
}

/// 在 [start, start+window) 内找第一个 moof atom 相对偏移；用于 sidx 落点校正
fn find_moof_offset_near(data: &[u8], start: usize, window: u64) -> Option<u64> {
    if start >= data.len() {
        return None;
    }
    let end = (start + window as usize).min(data.len());
    let mut offset = start;
    while offset + 8 <= end {
        let size = u32::from_be_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
        let kind = &data[offset + 4..offset + 8];
        if kind == b"moof" {
            return Some((offset - start) as u64);
        }
        let atom_size = if size == 1 {
            if offset + 16 > data.len() {
                break;
            }
            u64::from_be_bytes(data[offset + 8..offset + 16].try_into().ok()?) as usize
        } else if size == 0 {
            break;
        } else {
            size
        };
        if atom_size < 8 {
            // 坏 size：逐字节扫描找 "moof" fourcc
            offset += 1;
            continue;
        }
        if offset + atom_size > data.len() {
            // 不完整：在剩余窗口里暴力找 moof fourcc
            let search = &data[offset..end];
            if let Some(pos) = search
                .windows(4)
                .position(|w| w == b"moof")
            {
                // fourcc 在 size 之后 4 字节，atom 起点 = pos - 4
                if pos >= 4 {
                    return Some((offset + pos - 4 - start) as u64);
                }
            }
            break;
        }
        offset += atom_size;
    }
    // 兜底：窗口内暴力找 moof fourcc
    let search = &data[start..end];
    if let Some(pos) = search.windows(4).position(|w| w == b"moof") {
        if pos >= 4 {
            return Some((pos - 4) as u64);
        }
    }
    None
}

fn parse_sidx_from_bytes(
    data: &[u8],
    base_offset: u64,
    duration_ms: u64,
    total_len: u64,
) -> Option<RemoteSeekIndex> {
    // 在缓冲里扫描 sidx box
    let mut offset = 0usize;
    while offset + 8 <= data.len() {
        let size = u32::from_be_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
        let kind = &data[offset + 4..offset + 8];
        let atom_size = if size == 1 {
            if offset + 16 > data.len() {
                break;
            }
            u64::from_be_bytes(data[offset + 8..offset + 16].try_into().ok()?) as usize
        } else if size == 0 {
            data.len().saturating_sub(offset)
        } else {
            size
        };
        if atom_size < 8 || offset + atom_size > data.len() {
            break;
        }
        if kind == b"sidx" {
            let header_len = if size == 1 { 16 } else { 8 };
            let body = &data[offset + header_len..offset + atom_size];
            if let Some(index) =
                parse_sidx_body(body, base_offset + offset as u64 + atom_size as u64, duration_ms, total_len)
            {
                return Some(index);
            }
        }
        if atom_size == 0 {
            break;
        }
        offset += atom_size;
    }
    None
}

fn parse_sidx_body(
    body: &[u8],
    first_offset_anchor: u64,
    duration_ms: u64,
    total_len: u64,
) -> Option<RemoteSeekIndex> {
    if body.len() < 20 {
        return None;
    }
    let version = body[0];
    let mut o = 4usize; // skip version+flags
    o += 4; // reference_id
    if o + 4 > body.len() {
        return None;
    }
    let timescale = u32::from_be_bytes(body[o..o + 4].try_into().ok()?) as u64;
    o += 4;
    if timescale == 0 {
        return None;
    }
    let (earliest_pts, first_offset) = if version == 0 {
        if o + 8 > body.len() {
            return None;
        }
        let pts = u32::from_be_bytes(body[o..o + 4].try_into().ok()?) as u64;
        o += 4;
        let off = u32::from_be_bytes(body[o..o + 4].try_into().ok()?) as u64;
        o += 4;
        (pts, first_offset_anchor + off)
    } else {
        if o + 16 > body.len() {
            return None;
        }
        let pts = u64::from_be_bytes(body[o..o + 8].try_into().ok()?);
        o += 8;
        let off = u64::from_be_bytes(body[o..o + 8].try_into().ok()?);
        o += 8;
        (pts, first_offset_anchor + off)
    };
    if o + 4 > body.len() {
        return None;
    }
    o += 2; // reserved
    let reference_count = u16::from_be_bytes(body[o..o + 2].try_into().ok()?) as usize;
    o += 2;

    let mut entries = Vec::with_capacity(reference_count.saturating_add(1));
    let mut time_ticks = earliest_pts;
    let mut byte_pos = first_offset.min(total_len);
    entries.push(SeekIndexEntry {
        time_ms: time_ticks.saturating_mul(1000) / timescale,
        byte_offset: byte_pos,
    });
    for _ in 0..reference_count {
        if o + 12 > body.len() {
            break;
        }
        let reference = u32::from_be_bytes(body[o..o + 4].try_into().ok()?);
        o += 4;
        let subsegment_duration = u32::from_be_bytes(body[o..o + 4].try_into().ok()?) as u64;
        o += 4;
        o += 4; // SAP
        let reference_type = (reference & 0x8000_0000) != 0;
        let reference_size = (reference & 0x7fff_ffff) as u64;
        if reference_type {
            // 嵌套 sidx，跳过大小但仍推进偏移
            byte_pos = byte_pos.saturating_add(reference_size).min(total_len);
            continue;
        }
        byte_pos = byte_pos.saturating_add(reference_size).min(total_len);
        time_ticks = time_ticks.saturating_add(subsegment_duration);
        entries.push(SeekIndexEntry {
            time_ms: time_ticks.saturating_mul(1000) / timescale,
            byte_offset: byte_pos,
        });
    }
    if entries.len() < 2 {
        return None;
    }
    // 夹到声明时长
    for entry in &mut entries {
        if duration_ms > 0 {
            entry.time_ms = entry.time_ms.min(duration_ms);
        }
        entry.byte_offset = entry.byte_offset.min(total_len);
    }
    Some(RemoteSeekIndex {
        duration_ms: duration_ms.max(entries.last().map(|e| e.time_ms).unwrap_or(0)),
        entries,
    })
}

/// 登记 [start, end] 的在途请求；起点已被在途请求覆盖或已被连续缓存覆盖时
/// 返回 None（调用方应转入等待/命中缓存，而不是重复发起请求）
fn claim_prefetch_start(
    inner: &RemoteAudioInner,
    start: u64,
    end: u64,
) -> Option<Arc<InflightFetch>> {
    if contiguous_cached_end(inner, start) > start {
        return None;
    }
    let Ok(mut in_flight) = inner.in_flight.lock() else {
        return None;
    };
    // 只按「起点被覆盖」判重：部分尾部重叠的少量浪费可接受，
    // 若按整段重叠判重，需求读会在等不到对应登记项时陷入空转
    if in_flight.iter().any(|(entry_start, entry)| {
        *entry_start <= start && start <= entry.end.load(Ordering::Acquire)
    }) {
        return None;
    }
    let entry = Arc::new(InflightFetch {
        end: AtomicU64::new(end),
        done: AtomicBool::new(false),
    });
    in_flight.insert(start, Arc::clone(&entry));
    Some(entry)
}

/// 发布在途请求结果：置 done 唤醒共享等待方，并摘除登记项。
/// 摘除前校验仍是同一个 Arc——代际作废会清空注册表，同一起点可能已被
/// 新请求重新占用，不能误删别人的登记
fn release_prefetch_start(inner: &RemoteAudioInner, start: u64, entry: &Arc<InflightFetch>) {
    entry.done.store(true, Ordering::Release);
    if let Ok(mut in_flight) = inner.in_flight.lock() {
        if in_flight
            .get(&start)
            .is_some_and(|current| Arc::ptr_eq(current, entry))
        {
            in_flight.remove(&start);
        }
    }
    inner.range_available.notify_all();
}

/// seek 时作废旧预取：抬 epoch + 置 done 解除所有共享等待方 + 清空注册表，
/// 让需求读立刻能 claim（seek 到未缓存位置保持低延迟）
fn invalidate_prefetch_epoch(inner: &RemoteAudioInner) {
    inner.prefetch_epoch.fetch_add(1, Ordering::AcqRel);
    if let Ok(mut in_flight) = inner.in_flight.lock() {
        for entry in in_flight.values() {
            entry.done.store(true, Ordering::Release);
        }
        in_flight.clear();
    }
    inner.range_available.notify_all();
}

fn protect_cached_range(inner: &RemoteAudioInner, start: u64, len: u64) {
    if len == 0 {
        return;
    }
    let next = CachedRange {
        start,
        end: start.saturating_add(len).min(inner.total_len),
    };
    let Ok(mut ranges) = inner.protected_ranges.lock() else {
        return;
    };
    ranges.push(next);
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<CachedRange> = Vec::with_capacity(ranges.len());
    for range in ranges.iter().copied() {
        if let Some(last) = merged.last_mut() {
            if range.start <= last.end {
                last.end = last.end.max(range.end);
                continue;
            }
        }
        merged.push(range);
    }
    *ranges = merged;
}

fn range_overlaps(a: CachedRange, b: CachedRange) -> bool {
    a.start < b.end && b.start < a.end
}

fn segment_is_protected(inner: &RemoteAudioInner, segment: &CachedSegment) -> bool {
    let segment_range = CachedRange {
        start: segment.start,
        end: segment.start.saturating_add(segment.data.len() as u64),
    };
    let Ok(ranges) = inner.protected_ranges.lock() else {
        return false;
    };
    ranges
        .iter()
        .any(|range| range_overlaps(*range, segment_range))
}

fn parse_content_range(value: &reqwest::header::HeaderValue) -> Option<HttpByteRange> {
    let text = value.to_str().ok()?;
    let value = text.strip_prefix("bytes ")?;
    let (range, total) = value.split_once('/')?;
    if total == "*" || range == "*" {
        return None;
    }
    let (start, end) = range.split_once('-')?;
    let parsed = HttpByteRange {
        start: start.parse::<u64>().ok()?,
        end: end.parse::<u64>().ok()?,
        total: total.parse::<u64>().ok()?,
    };
    (parsed.start <= parsed.end && parsed.end < parsed.total).then_some(parsed)
}

fn validate_http_range_data(
    actual: HttpByteRange,
    requested_start: u64,
    requested_end: u64,
    data: &[u8],
) -> io::Result<()> {
    if actual.start != requested_start || actual.end > requested_end {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unexpected Content-Range {}-{}/{} for request {}-{}",
                actual.start, actual.end, actual.total, requested_start, requested_end
            ),
        ));
    }
    let expected_len = actual
        .end
        .checked_sub(actual.start)
        .and_then(|length| length.checked_add(1))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid byte range"))?;
    if data.len() as u64 != expected_len {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "range body length {} does not match Content-Range length {}",
                data.len(),
                expected_len
            ),
        ));
    }
    Ok(())
}

fn trim_cache(inner: &RemoteAudioInner, cache: &mut Vec<CachedSegment>, current_pos: u64) {
    let mut total: usize = cache.iter().map(|segment| segment.data.len()).sum();
    if total <= REMOTE_MAX_CACHE_BYTES {
        return;
    }

    // 先丢离读头最远且未受保护（非头/尾 moov）的块
    cache.sort_by_key(|segment| {
        let protected = segment_is_protected(inner, segment);
        (protected, segment_distance(segment, current_pos))
    });
    while total > REMOTE_MAX_CACHE_BYTES {
        let Some(segment) = cache.pop() else {
            break;
        };
        // 只剩保护块时停止，避免把 moov 裁掉导致 seek 重开 demuxer 扫全文件
        if segment_is_protected(inner, &segment) {
            cache.push(segment);
            break;
        }
        total = total.saturating_sub(segment.data.len());
    }
    cache.sort_by_key(|segment| segment.start);
}

fn segment_distance(segment: &CachedSegment, current_pos: u64) -> u64 {
    let segment_end = segment.start + segment.data.len() as u64;
    if current_pos < segment.start {
        segment.start - current_pos
    } else {
        current_pos.saturating_sub(segment_end)
    }
}

fn time_to_duration(time: Time) -> Duration {
    Duration::from_secs(time.seconds)
        + Duration::from_nanos((time.frac * 1_000_000_000.0).max(0.0) as u64)
}

fn track_duration(time_base: Option<TimeBase>, n_frames: Option<u64>) -> Option<Time> {
    let frames = n_frames.filter(|frames| *frames > 0)?;
    time_base.map(|base| base.calc_time(frames))
}

fn seek_target_time(total_duration: Option<Time>, position: Duration) -> Time {
    match total_duration.filter(|duration| !time_to_duration(*duration).is_zero()) {
        Some(duration)
            if time_to_duration(duration)
                .saturating_sub(position)
                .as_millis()
                < 1 =>
        {
            skip_back_tiny_bit(duration)
        }
        _ => position.as_secs_f64().into(),
    }
}

fn skip_back_tiny_bit(mut time: Time) -> Time {
    time.frac -= 0.0001;
    if time.frac < 0.0 {
        time.seconds = time.seconds.saturating_sub(1);
        time.frac += 1.0;
    }
    time
}

fn seek_error(message: String) -> PcmSeekError {
    PcmSeekError::new(message)
}

#[cfg(test)]
mod tests {
    #[test]
    fn android_alignment_completed_aac_reaches_eof_without_virtual_body_waits() {
        let started = std::time::Instant::now();
        let decoder = super::SymphoniaAudioDecoder::new(
            Box::new(std::io::Cursor::new(include_bytes!("fixtures/hls-silence.aac").as_slice())),
            Some("aac"),
        ).unwrap();
        assert_eq!(decoder.count(), 9216);
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "normal EOF must not retry for 5.12 seconds");
    }

    use super::{
        adaptive_block_bytes, cache_duration_is_suspicious, ewma_bps_after_failure,
        ewma_bps_after_success, format_cache_marker, halved_fetch_len, is_fragmented_mp4_url,
        can_return_partial_read,
        claim_prefetch_sequence_start, parse_cache_marker, parse_content_range, prefetch_lane_count,
        prefetch_plan, prefetch_window,
        ranges_cover_source,
        release_prefetch_start, remember_validated_audio_cache, replenish_prefetch_window,
        request_timeout_for, reuse_validated_audio_cache, splice_skip_retryable,
        seek_target_time, throughput_sample_bps, track_duration, CacheMarker, CachedRange,
        CachedSegment, HttpByteRange,
        InflightFetch,
        PrefetchPlan, PrefetchWindow, RemoteAccessMode, RemoteAudioCache, RemoteAudioInner,
        RemoteSeekIndex, SeekIndexEntry,
        RemoteAudioSource, RemoteReadCancellation, SymphoniaAudioDecoder, ValidatedCacheFile,
        REMOTE_PREFETCH_CONCURRENCY, REMOTE_PREFETCH_FALLBACK_LOW_WATER_BYTES,
        REMOTE_PREFETCH_FALLBACK_TARGET_BYTES,
    };
    use reqwest::header::HeaderValue;
    use std::collections::HashMap;
    use std::io::{Cursor, Read};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread;
    use std::time::Duration;
    use symphonia::core::units::{Time, TimeBase};

    fn pcm_wav(sample_count: u32) -> Vec<u8> {
        let data_len = sample_count * 2;
        let mut wav = Vec::with_capacity(44 + data_len as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&8_000u32.to_le_bytes());
        wav.extend_from_slice(&16_000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.resize(44 + data_len as usize, 0);
        wav
    }

    #[test]
    fn permanent_range_error_is_not_folded_into_partial_eof() {
        let permanent = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "remote range no longer satisfiable: 416",
        );
        let transient = std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "connection reset while fetching range",
        );

        assert!(!can_return_partial_read(&permanent));
        assert!(can_return_partial_read(&transient));
    }

    fn pcm_wav_24(samples: &[i32]) -> Vec<u8> {
        let data_len = samples.len() as u32 * 3;
        let mut wav = Vec::with_capacity(44 + data_len as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&8_000u32.to_le_bytes());
        wav.extend_from_slice(&24_000u32.to_le_bytes());
        wav.extend_from_slice(&3u16.to_le_bytes());
        wav.extend_from_slice(&24u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples {
            let bytes = sample.to_le_bytes();
            wav.extend_from_slice(&bytes[..3]);
        }
        wav
    }

    #[test]
    fn decoder_preserves_pcm_below_sixteen_bit_resolution() {
        let wav = pcm_wav_24(&[1, 256]);
        let mut decoder = SymphoniaAudioDecoder::new(Box::new(Cursor::new(wav)), Some("wav"))
            .expect("24-bit wav decoder");

        let least_significant_sample = decoder.next().expect("first PCM sample");

        assert!(least_significant_sample > 0.0);
        assert!(least_significant_sample < 1.0 / 32_768.0);
    }

    #[test]
    fn prefetch_window_uses_duration_hint_for_time_based_buffering() {
        // 9MB / 180s ≈ 50KB/s → 5s=250KB、15s=750KB，均被抬到 min
        let window = prefetch_window(9_000_000, 180_000);
        assert_eq!(window.low_water_bytes, 256 * 1024);
        assert_eq!(window.target_bytes, 1024 * 1024);

        // 更高码率时按时间窗口缩放
        let hi = prefetch_window(36_000_000, 180_000);
        assert_eq!(hi.low_water_bytes, 1_000_000);
        assert_eq!(hi.target_bytes, 3_000_000);
    }

    #[test]
    fn prefetch_window_falls_back_when_duration_is_unknown() {
        let window = prefetch_window(10_000_000, 0);

        assert_eq!(
            window,
            PrefetchWindow {
                low_water_bytes: REMOTE_PREFETCH_FALLBACK_LOW_WATER_BYTES,
                target_bytes: REMOTE_PREFETCH_FALLBACK_TARGET_BYTES,
            }
        );
    }

    #[test]
    fn prefetch_plan_refills_only_after_reaching_low_watermark() {
        let window = PrefetchWindow {
            low_water_bytes: 250_000,
            target_bytes: 750_000,
        };

        assert_eq!(prefetch_plan(1_000_000, 1_250_001, 5_000_000, window), None);
        assert_eq!(
            prefetch_plan(1_000_000, 1_250_000, 5_000_000, window),
            Some(PrefetchPlan {
                start: 1_250_000,
                target_end: 1_750_000,
            })
        );
    }

    #[test]
    fn prefetch_plan_never_reads_past_source_end() {
        let window = PrefetchWindow {
            low_water_bytes: 250_000,
            target_bytes: 750_000,
        };

        assert_eq!(
            prefetch_plan(4_800_000, 4_900_000, 5_000_000, window),
            Some(PrefetchPlan {
                start: 4_900_000,
                target_end: 5_000_000,
            })
        );
    }

    #[test]
    fn duplicate_prefetch_sequence_is_rejected_before_task_submission() {
        let inner = RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "http://127.0.0.1:9/should-not-be-requested".into(),
            referer: "https://example.com".into(),
            total_len: 4,
            header_end: AtomicU64::new(0),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 1,
                target_bytes: 4,
            },
            cache: Mutex::new(Vec::new()),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::new()),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(0),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::new(AtomicU64::new(1)),
            expected_generation: 1,
        };
        let plan = PrefetchPlan {
            start: 0,
            target_end: 4,
        };

        let (start, _end, entry) =
            claim_prefetch_sequence_start(&inner, plan).expect("first sequence claim");
        assert_eq!(start, 0);
        assert!(claim_prefetch_sequence_start(&inner, plan).is_none());
        assert_eq!(inner.in_flight.lock().expect("in-flight lock").len(), 1);

        release_prefetch_start(&inner, 0, &entry);
        let (start, _end, entry) =
            claim_prefetch_sequence_start(&inner, plan).expect("reclaim after release");
        assert_eq!(start, 0);
        release_prefetch_start(&inner, 0, &entry);
    }

    #[test]
    fn demand_read_waits_for_matching_prefetch_instead_of_downloading_twice() {
        let playback_generation = Arc::new(AtomicU64::new(1));
        let inflight_entry = Arc::new(InflightFetch {
            end: AtomicU64::new(3),
            done: AtomicBool::new(false),
        });
        let inner = Arc::new(RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "http://127.0.0.1:9/should-not-be-requested".into(),
            referer: "https://example.com".into(),
            total_len: 4,
            header_end: AtomicU64::new(0),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 1,
                target_bytes: 1,
            },
            cache: Mutex::new(Vec::new()),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::from([(0, Arc::clone(&inflight_entry))])),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(0),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation,
            expected_generation: 1,
        });
        let producer_inner = Arc::clone(&inner);
        let producer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            producer_inner
                .cache
                .lock()
                .expect("cache lock")
                .push(CachedSegment {
                    start: 0,
                    data: vec![1, 2, 3, 4],
                });
            release_prefetch_start(&producer_inner, 0, &inflight_entry);
        });
        let mut source = RemoteAudioSource {
            inner,
            pos: 0,
            access_mode: RemoteAccessMode::StandardSeekable,
            read_cancellation: None,
        };
        let mut bytes = [0; 4];

        source.read_exact(&mut bytes).expect("prefetched read");

        assert_eq!(bytes, [1, 2, 3, 4]);
        assert!(producer.join().is_ok());
    }

    /// 需求读落在在途区间中段（起点不同）也必须共享结果而非重复拉取
    #[test]
    fn demand_read_shares_inflight_range_covering_its_offset() {
        let inflight_entry = Arc::new(InflightFetch {
            end: AtomicU64::new(1023),
            done: AtomicBool::new(false),
        });
        let inner = Arc::new(RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "http://127.0.0.1:9/should-not-be-requested".into(),
            referer: "https://example.com".into(),
            total_len: 1024,
            header_end: AtomicU64::new(0),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 1,
                target_bytes: 1,
            },
            cache: Mutex::new(Vec::new()),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::from([(0, Arc::clone(&inflight_entry))])),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(0),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::new(AtomicU64::new(1)),
            expected_generation: 1,
        });
        let producer_inner = Arc::clone(&inner);
        let producer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            producer_inner
                .cache
                .lock()
                .expect("cache lock")
                .push(CachedSegment {
                    start: 0,
                    data: vec![7; 1024],
                });
            release_prefetch_start(&producer_inner, 0, &inflight_entry);
        });
        let mut source = RemoteAudioSource {
            inner,
            pos: 512,
            access_mode: RemoteAccessMode::StandardSeekable,
            read_cancellation: None,
        };
        let mut bytes = [0; 4];

        source.read_exact(&mut bytes).expect("mid-range read");

        assert_eq!(bytes, [7, 7, 7, 7]);
        assert!(producer.join().is_ok());
    }

    /// 代际作废必须立刻解除共享等待方，不允许陪旧预取等满超时预算
    #[test]
    fn epoch_invalidation_releases_inflight_waiters_immediately() {
        let inner = Arc::new(RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "http://127.0.0.1:9/should-not-be-requested".into(),
            referer: "https://example.com".into(),
            total_len: 4,
            header_end: AtomicU64::new(0),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 1,
                target_bytes: 1,
            },
            cache: Mutex::new(Vec::new()),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::from([(
                0,
                Arc::new(InflightFetch {
                    end: AtomicU64::new(3),
                    done: AtomicBool::new(false),
                }),
            )])),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(0),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::new(AtomicU64::new(1)),
            expected_generation: 1,
        });
        let invalidator_inner = Arc::clone(&inner);
        let invalidator = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            // seek 语义：数据入缓存后作废旧代，等待方应立即醒来并命中缓存
            invalidator_inner
                .cache
                .lock()
                .expect("cache lock")
                .push(CachedSegment {
                    start: 0,
                    data: vec![5, 6, 7, 8],
                });
            super::invalidate_prefetch_epoch(&invalidator_inner);
        });
        let mut source = RemoteAudioSource {
            inner,
            pos: 0,
            access_mode: RemoteAccessMode::StandardSeekable,
            read_cancellation: None,
        };
        let mut bytes = [0; 4];
        let started = std::time::Instant::now();

        source.read_exact(&mut bytes).expect("read after invalidate");

        assert_eq!(bytes, [5, 6, 7, 8]);
        // 远小于 60s+5s 的防御预算，证明等待方是被 invalidate 主动唤醒的
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(invalidator.join().is_ok());
    }

    /// 起点被在途区间覆盖时不得再登记新请求（转入共享等待）
    #[test]
    fn claim_rejects_offset_covered_by_inflight_range() {
        let inner = RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "http://127.0.0.1:9/should-not-be-requested".into(),
            referer: "https://example.com".into(),
            total_len: 1024,
            header_end: AtomicU64::new(0),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 1,
                target_bytes: 1,
            },
            cache: Mutex::new(Vec::new()),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::from([(
                0,
                Arc::new(InflightFetch {
                    end: AtomicU64::new(255),
                    done: AtomicBool::new(false),
                }),
            )])),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(0),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::new(AtomicU64::new(1)),
            expected_generation: 1,
        };

        // 中段偏移已被 [0, 255] 在途请求覆盖 → 拒绝重复登记
        assert!(super::claim_prefetch_start(&inner, 128, 383).is_none());
        // 在途区间之外可以正常登记
        let entry = super::claim_prefetch_start(&inner, 256, 511).expect("claim beyond range");
        super::release_prefetch_start(&inner, 256, &entry);
    }

    #[test]
    fn superseded_remote_source_stops_before_network_or_cache_io() {
        let playback_generation = Arc::new(AtomicU64::new(1));
        let inner = Arc::new(RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "http://127.0.0.1:9/should-not-be-requested".into(),
            referer: "https://example.com".into(),
            total_len: 4,
            header_end: AtomicU64::new(0),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 1,
                target_bytes: 1,
            },
            cache: Mutex::new(Vec::new()),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::new()),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(0),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::clone(&playback_generation),
            expected_generation: 1,
        });
        playback_generation.store(2, std::sync::atomic::Ordering::Release);
        let mut source = RemoteAudioSource {
            inner,
            pos: 0,
            access_mode: RemoteAccessMode::StandardSeekable,
            read_cancellation: None,
        };
        let mut byte = [0];

        let error = source.read(&mut byte).expect_err("superseded read");

        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    }

    #[test]
    fn stopped_session_cancels_remote_read_without_changing_track_generation() {
        let playback_generation = Arc::new(AtomicU64::new(1));
        let session_cancelled = Arc::new(AtomicBool::new(false));
        let inner = Arc::new(RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "http://127.0.0.1:9/should-not-be-requested".into(),
            referer: "https://example.com".into(),
            total_len: 4,
            header_end: AtomicU64::new(0),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 1,
                target_bytes: 1,
            },
            cache: Mutex::new(Vec::new()),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::new()),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(0),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation,
            expected_generation: 1,
        });
        let mut source = RemoteAudioSource {
            inner,
            pos: 0,
            access_mode: RemoteAccessMode::StandardSeekable,
            read_cancellation: Some(RemoteReadCancellation::new(
                Arc::clone(&session_cancelled),
                None,
            )),
        };
        session_cancelled.store(true, Ordering::Release);

        let error = source
            .read(&mut [0])
            .expect_err("stopped session read");

        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    }

    #[test]
    fn fragmented_mp4_starts_progressively_and_promotes_for_seek() {
        assert!(is_fragmented_mp4_url(
            "https://cdn.example/audio/track.M4S?deadline=123"
        ));
        assert!(!is_fragmented_mp4_url(
            "https://cdn.example/audio/track.flac?deadline=123"
        ));

        let source = RemoteAudioSource {
            inner: Arc::new(RemoteAudioInner {
                client: reqwest::Client::new(),
                url: "https://cdn.example/audio/track.m4s".into(),
                referer: "https://example.com".into(),
                total_len: 4,
                header_end: AtomicU64::new(0),
                prefetch_window: PrefetchWindow {
                    low_water_bytes: 1,
                    target_bytes: 1,
                },
                cache: Mutex::new(Vec::new()),
                disk_ranges: Mutex::new(Vec::new()),
                in_flight: Mutex::new(HashMap::new()),
                range_available: Condvar::new(),
                demand_fetches: AtomicUsize::new(0),
                last_read_pos: Mutex::new(0),
                protected_ranges: Mutex::new(Vec::new()),
                disk_cache: None,
                cache_finalize_started: AtomicBool::new(false),
                prefetch_epoch: AtomicU64::new(0),
                remote_forbidden: AtomicBool::new(false),
                demuxer_open_sequential: AtomicBool::new(false),
                virtual_body_origin: AtomicU64::new(0),
                seek_index: Mutex::new(None),
                throughput_ewma_bps: AtomicU64::new(0),
                bitrate_bps: 0,
                playback_generation: Arc::new(AtomicU64::new(1)),
                expected_generation: 1,
            }),
            pos: 0,
            access_mode: RemoteAccessMode::FragmentedProgressive,
            read_cancellation: None,
        };

        assert!(!symphonia::core::io::MediaSource::is_seekable(&source));
        assert!(symphonia::core::io::MediaSource::byte_len(&source).is_none());
        let seekable = source.seekable_clone();
        // demuxer open 阶段隐藏 seekable，避免扫 mdat
        assert!(!symphonia::core::io::MediaSource::is_seekable(&seekable));
        assert!(source.inner.demuxer_open_sequential.load(std::sync::atomic::Ordering::Acquire));
        seekable.finish_demuxer_open();
        assert!(symphonia::core::io::MediaSource::is_seekable(&seekable));
        assert_eq!(
            seekable.access_mode.demand_fetch_block_bytes(),
            super::REMOTE_FRAGMENTED_SEEK_BLOCK_BYTES
        );
    }

    #[test]
    fn long_form_progressive_promotes_to_standard_seekable() {
        let source = RemoteAudioSource {
            inner: Arc::new(RemoteAudioInner {
                client: reqwest::Client::new(),
                url: "https://cdn.example/audio/long.m4a".into(),
                referer: "https://example.com".into(),
                total_len: 135_000_000,
                header_end: AtomicU64::new(0),
                prefetch_window: PrefetchWindow {
                    low_water_bytes: 256 * 1024,
                    target_bytes: 1024 * 1024,
                },
                cache: Mutex::new(Vec::new()),
                disk_ranges: Mutex::new(Vec::new()),
                in_flight: Mutex::new(HashMap::new()),
                range_available: Condvar::new(),
                demand_fetches: AtomicUsize::new(0),
                last_read_pos: Mutex::new(0),
                protected_ranges: Mutex::new(Vec::new()),
                disk_cache: None,
                cache_finalize_started: AtomicBool::new(false),
                prefetch_epoch: AtomicU64::new(0),
                remote_forbidden: AtomicBool::new(false),
                demuxer_open_sequential: AtomicBool::new(false),
                virtual_body_origin: AtomicU64::new(0),
                seek_index: Mutex::new(None),
                throughput_ewma_bps: AtomicU64::new(0),
                bitrate_bps: 0,
                playback_generation: Arc::new(AtomicU64::new(1)),
                expected_generation: 1,
            }),
            pos: 0,
            access_mode: RemoteAccessMode::LongFormProgressive,
            read_cancellation: None,
        };

        assert!(!symphonia::core::io::MediaSource::is_seekable(&source));
        assert!(symphonia::core::io::MediaSource::byte_len(&source).is_none());

        let seekable = source.seekable_clone();
        assert_eq!(seekable.access_mode, RemoteAccessMode::StandardSeekable);
        // demuxer open 阶段隐藏 seekable，finish 后才对 demuxer 暴露
        assert!(!symphonia::core::io::MediaSource::is_seekable(&seekable));
        assert!(source.inner.demuxer_open_sequential.load(std::sync::atomic::Ordering::Acquire));
        seekable.finish_demuxer_open();
        assert!(symphonia::core::io::MediaSource::is_seekable(&seekable));
        // 绝不能走分片 8MB 按需块，否则 seek 会卡死
        assert_eq!(
            seekable.access_mode.demand_fetch_block_bytes(),
            super::REMOTE_FETCH_BLOCK_BYTES
        );
        // seekable_clone 必须作废旧预取 epoch，否则 seek 会撞 in_flight
        assert_eq!(
            source.inner.prefetch_epoch.load(std::sync::atomic::Ordering::Acquire),
            1
        );
        assert!(source.inner.in_flight.lock().expect("lock").is_empty());
    }

    #[test]
    fn long_form_prefers_virtual_body_before_and_after_promote() {
        let index = super::RemoteSeekIndex {
            duration_ms: 100_000,
            entries: vec![
                super::SeekIndexEntry {
                    time_ms: 0,
                    byte_offset: 1_000,
                },
                super::SeekIndexEntry {
                    time_ms: 50_000,
                    byte_offset: 50_000_000,
                },
                super::SeekIndexEntry {
                    time_ms: 100_000,
                    byte_offset: 100_000_000,
                },
            ],
        };
        let source = RemoteAudioSource {
            inner: Arc::new(RemoteAudioInner {
                client: reqwest::Client::new(),
                url: "https://cdn.example/audio/long.m4a".into(),
                referer: "https://example.com".into(),
                total_len: 135_000_000,
                header_end: AtomicU64::new(10_799),
                prefetch_window: PrefetchWindow {
                    low_water_bytes: 256 * 1024,
                    target_bytes: 1024 * 1024,
                },
                cache: Mutex::new(Vec::new()),
                disk_ranges: Mutex::new(Vec::new()),
                in_flight: Mutex::new(HashMap::new()),
                range_available: Condvar::new(),
                demand_fetches: AtomicUsize::new(0),
                last_read_pos: Mutex::new(0),
                protected_ranges: Mutex::new(Vec::new()),
                disk_cache: None,
                cache_finalize_started: AtomicBool::new(false),
                prefetch_epoch: AtomicU64::new(0),
                remote_forbidden: AtomicBool::new(false),
                demuxer_open_sequential: AtomicBool::new(false),
                virtual_body_origin: AtomicU64::new(0),
                seek_index: Mutex::new(Some(index)),
                throughput_ewma_bps: AtomicU64::new(0),
                bitrate_bps: 0,
                playback_generation: Arc::new(AtomicU64::new(1)),
                expected_generation: 1,
            }),
            pos: 0,
            access_mode: RemoteAccessMode::LongFormProgressive,
            read_cancellation: None,
        };

        // 关键：clone 前 LongForm 也必须 prefers，否则 seek 会先选 format.seek
        assert!(source.prefers_virtual_body_seek());
        let seekable = source.seekable_clone();
        assert!(seekable.prefers_virtual_body_seek());
        assert_eq!(seekable.access_mode, RemoteAccessMode::StandardSeekable);
    }

    #[test]
    fn virtual_body_media_source_seekable_after_open_only() {
        let source = RemoteAudioSource {
            inner: Arc::new(RemoteAudioInner {
                client: reqwest::Client::new(),
                url: "https://cdn.example/audio/long.m4a".into(),
                referer: "https://example.com".into(),
                total_len: 10_000,
                header_end: AtomicU64::new(100),
                prefetch_window: PrefetchWindow {
                    low_water_bytes: 1,
                    target_bytes: 1,
                },
                cache: Mutex::new(Vec::new()),
                disk_ranges: Mutex::new(Vec::new()),
                in_flight: Mutex::new(HashMap::new()),
                range_available: Condvar::new(),
                demand_fetches: AtomicUsize::new(0),
                last_read_pos: Mutex::new(0),
                protected_ranges: Mutex::new(Vec::new()),
                disk_cache: None,
                cache_finalize_started: AtomicBool::new(false),
                prefetch_epoch: AtomicU64::new(0),
                remote_forbidden: AtomicBool::new(false),
                demuxer_open_sequential: AtomicBool::new(true),
                virtual_body_origin: AtomicU64::new(5_000),
                seek_index: Mutex::new(None),
                throughput_ewma_bps: AtomicU64::new(0),
                bitrate_bps: 0,
                playback_generation: Arc::new(AtomicU64::new(1)),
                expected_generation: 1,
            }),
            pos: 0,
            access_mode: RemoteAccessMode::StandardSeekable,
            read_cancellation: None,
        };

        assert!(!symphonia::core::io::MediaSource::is_seekable(&source));
        assert!(symphonia::core::io::MediaSource::byte_len(&source).is_none());

        source.finish_demuxer_open();
        assert!(symphonia::core::io::MediaSource::is_seekable(&source));
        assert_eq!(
            symphonia::core::io::MediaSource::byte_len(&source),
            Some(source.logical_len())
        );
        // 纯逻辑连续：header + (total - origin)
        assert_eq!(source.logical_len(), 100 + (10_000 - 5_000));
    }



    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn fragmented_mp4_is_sniffed_from_sidx_moof_and_mvex() {
        // ftyp + sidx（YouTube itag140 的典型头部布局）
        let mut sidx_head = mp4_box(b"ftyp", &[0u8; 8]);
        sidx_head.extend_from_slice(&mp4_box(b"sidx", &[0u8; 16]));
        assert!(super::detect_fragmented_mp4(&sidx_head));

        // ftyp + moov(含 mvex)
        let moov_body = mp4_box(b"mvex", &[0u8; 8]);
        let mut mvex_head = mp4_box(b"ftyp", &[0u8; 8]);
        mvex_head.extend_from_slice(&mp4_box(b"moov", &moov_body));
        assert!(super::detect_fragmented_mp4(&mvex_head));

        // styp 开头的独立分片
        assert!(super::detect_fragmented_mp4(&mp4_box(b"styp", &[0u8; 8])));
    }

    /// 回归：嗅探到分片只能影响「打开阶段顺序读」，绝不能启用虚拟 body
    ///
    /// 曾经因为给这类流建了 seek 索引而走上 virtual-body 拼接，
    /// 样本偏移对不上导致 seek 与回滚双双 end of stream，播放直接死掉。
    #[test]
    fn sniffed_fragments_must_not_enable_virtual_body_seek() {
        let source = RemoteAudioSource {
            inner: Arc::new(RemoteAudioInner {
                client: reqwest::Client::new(),
                url: "https://rr1---sn-x.googlevideo.com/videoplayback?itag=140".into(),
                referer: "https://www.youtube.com".into(),
                total_len: 4_842_215,
                header_end: AtomicU64::new(1151),
                prefetch_window: PrefetchWindow {
                    low_water_bytes: 262_144,
                    target_bytes: 1_048_576,
                },
                cache: Mutex::new(Vec::new()),
                disk_ranges: Mutex::new(Vec::new()),
                in_flight: Mutex::new(HashMap::new()),
                range_available: Condvar::new(),
                demand_fetches: AtomicUsize::new(0),
                last_read_pos: Mutex::new(0),
                protected_ranges: Mutex::new(Vec::new()),
                disk_cache: None,
                cache_finalize_started: AtomicBool::new(false),
                prefetch_epoch: AtomicU64::new(0),
                remote_forbidden: AtomicBool::new(false),
                // open 阶段顺序读，避免 symphonia 顺着 moof 链扫完整个文件
                demuxer_open_sequential: AtomicBool::new(true),
                virtual_body_origin: AtomicU64::new(0),
                // 嗅探路径不建索引，交给 symphonia 原生 sidx 定位
                seek_index: Mutex::new(None),
                throughput_ewma_bps: AtomicU64::new(0),
                bitrate_bps: 0,
                playback_generation: Arc::new(AtomicU64::new(1)),
                expected_generation: 1,
            }),
            pos: 0,
            access_mode: RemoteAccessMode::StandardSeekable,
            read_cancellation: None,
        };

        // 打开阶段对 demuxer 隐藏 seekable / byte_len，否则会扫全文件
        assert!(!symphonia::core::io::MediaSource::is_seekable(&source));
        assert!(symphonia::core::io::MediaSource::byte_len(&source).is_none());

        source.finish_demuxer_open();
        // 打开完成后恢复可 seek，让 symphonia 自己按 sidx 跳转
        assert!(symphonia::core::io::MediaSource::is_seekable(&source));
        assert!(!source.prefers_virtual_body_seek(), "虚拟 body 不适用于自带 sidx 的流");

        let seekable = source.seekable_clone();
        assert!(!seekable.prefers_virtual_body_seek());
        // 按需块必须保持 256KB：8MB 是给 Bilibili 独立 .m4s 分片调的
        assert_eq!(
            seekable.access_mode.demand_fetch_block_bytes(),
            super::REMOTE_FETCH_BLOCK_BYTES
        );
    }

    /// 回归：分片流必须走虚拟 body，且两种模式都不许退化成 8MB 块
    ///
    /// symphonia 0.5.5 解析 sidx 后直接丢弃，format.seek 只能逐个 moof 爬，
    /// 代价 O(文件大小)——实测 68MB 音频跳到中段要顺序读满 30MB。
    /// 所以只要建得出 sidx 索引就必须改走虚拟 body 的字节跳转；
    /// 同时按需块要维持 256KB，8MB 会让一次 seek 卡在整块下完之前。
    #[test]
    fn fragmented_streams_seek_through_the_virtual_body() {
        for mode in [
            RemoteAccessMode::StandardSeekable,
            RemoteAccessMode::LongFormProgressive,
        ] {
            let source = RemoteAudioSource {
                inner: Arc::new(RemoteAudioInner {
                    client: reqwest::Client::new(),
                    url: "https://rr5---sn-x.googlevideo.com/videoplayback?itag=140".into(),
                    referer: "https://www.youtube.com".into(),
                    total_len: 135_169_876,
                    header_end: AtomicU64::new(10_799),
                    prefetch_window: PrefetchWindow {
                        low_water_bytes: 262_144,
                        target_bytes: 1_048_576,
                    },
                    cache: Mutex::new(Vec::new()),
                    disk_ranges: Mutex::new(Vec::new()),
                    in_flight: Mutex::new(HashMap::new()),
                    range_available: Condvar::new(),
                    demand_fetches: AtomicUsize::new(0),
                    last_read_pos: Mutex::new(0),
                    protected_ranges: Mutex::new(Vec::new()),
                    disk_cache: None,
                    cache_finalize_started: AtomicBool::new(false),
                    prefetch_epoch: AtomicU64::new(0),
                    remote_forbidden: AtomicBool::new(false),
                    demuxer_open_sequential: AtomicBool::new(true),
                    virtual_body_origin: AtomicU64::new(0),
                    seek_index: Mutex::new(Some(RemoteSeekIndex {
                        duration_ms: 8_352_000,
                        entries: vec![
                            SeekIndexEntry { time_ms: 0, byte_offset: 10_799 },
                            SeekIndexEntry { time_ms: 10_000, byte_offset: 172_000 },
                        ],
                    })),
                    throughput_ewma_bps: AtomicU64::new(0),
                    bitrate_bps: 0,
                    playback_generation: Arc::new(AtomicU64::new(1)),
                    expected_generation: 1,
                }),
                pos: 0,
                access_mode: mode,
                read_cancellation: None,
            };

            let seekable = source.seekable_clone();
            assert!(
                seekable.prefers_virtual_body_seek(),
                "{mode:?} 有 sidx 索引就必须走虚拟 body，绝不能退回 format.seek",
            );
            assert_eq!(
                seekable.access_mode.seek_fetch_block_bytes(),
                super::REMOTE_FETCH_BLOCK_BYTES,
                "{mode:?} 的 seek 块不得退化成 8MB",
            );
            seekable.finish_demuxer_open();
            // 打开后必须对 demuxer 可 seek，样本才能在拼接视图里回跳
            assert!(symphonia::core::io::MediaSource::is_seekable(&seekable));
        }
    }

    /// 回归：代际守卫只许在 prepare 阶段杀读，提交后的会话必须免疫
    ///
    /// seek 代际在发送命令的瞬间递增。旧实现的令牌终生生效——用户下一次
    /// seek 会先把还在正常播放的会话的按需读掐死（实机：seek 后 6 秒
    /// next_packet fatal: remote read superseded，随后一次可听见的重建）。
    #[test]
    fn operation_guard_stops_killing_reads_after_disarm() {
        let generation = Arc::new(AtomicU64::new(7));
        let session_cancelled = Arc::new(AtomicBool::new(false));
        let cancellation = RemoteReadCancellation::new(
            Arc::clone(&session_cancelled),
            Some((Arc::clone(&generation), 7)),
        );
        // 克隆共享同一守卫：prepare 把令牌分发给解码/预取各处后，
        // 在任何一份上解除都要对全体生效
        let reader_copy = cancellation.clone();

        // prepare 阶段：代际一致不取消，被更新的 seek 抢先则必须取消
        assert!(!reader_copy.is_cancelled());
        generation.store(8, Ordering::Release);
        assert!(reader_copy.is_cancelled(), "prepare 输给新 seek 时必须可被打断");

        // 会话提交：解除代际守卫，之后代际再怎么涨都不关本会话的事
        generation.store(7, Ordering::Release);
        cancellation.disarm_operation_guard();
        generation.store(99, Ordering::Release);
        assert!(
            !reader_copy.is_cancelled(),
            "已提交会话不得被后续 seek 的代际递增杀死",
        );

        // 会话自身的收尾通道必须仍然有效
        session_cancelled.store(true, Ordering::Release);
        assert!(reader_copy.is_cancelled(), "previous.stop() 仍要能停掉读");
    }

    /// 回归：迟到的 prepare 超时旗标不得杀死已提交的会话
    ///
    /// 回包超时与提交存在竞态窗口：命令侧 recv 超时置位旗标的一瞬，
    /// 音频线程可能刚提交完会话。旧实现里 external_cancel 终生生效，
    /// 这一下就把好端端的会话无声杀死——偶发「突然不播了」。
    #[test]
    fn late_prepare_timeout_cannot_kill_a_committed_session() {
        let external = Arc::new(AtomicBool::new(false));
        let cancellation = RemoteReadCancellation::new(
            Arc::new(AtomicBool::new(false)),
            None,
        )
        .with_external_cancel(Arc::clone(&external));

        // prepare 期内超时旗标必须能打断
        external.store(true, Ordering::Release);
        assert!(cancellation.is_cancelled(), "prepare 卡死时必须可被超时解卡");
        external.store(false, Ordering::Release);

        // 提交后迟到的置位不得再影响
        cancellation.disarm_operation_guard();
        external.store(true, Ordering::Release);
        assert!(
            !cancellation.is_cancelled(),
            "迟到的 prepare 超时不得杀死已提交的会话",
        );
    }

    /// 端到端复现：真实 YouTube fMP4 字节走完整虚拟 body 打开 + 解码
    ///
    /// 需要实机字节（含版权内容，不入库）。运行方式：
    /// NERI_YT_REPRO_DIR=<dir> cargo test -- --ignored yt_virtual_body
    /// dir 下需有 yt-head.bin（物理 0..512KB）与 yt-mid.bin（物理 25212301 起 ~1MB）。
    /// 修复前该流在 ~12k 帧处死于 "unexpected end of bitstream"；
    /// 修复后应能稳定解出 5 秒以上的 PCM。
    #[test]
    #[ignore]
    fn yt_virtual_body_end_to_end_decodes_real_bytes() {
        let Ok(dir) = std::env::var("NERI_YT_REPRO_DIR") else {
            eprintln!("NERI_YT_REPRO_DIR 未设置，跳过");
            return;
        };
        let head = std::fs::read(format!("{dir}/yt-head.bin")).expect("yt-head.bin");
        let mid = std::fs::read(format!("{dir}/yt-mid.bin")).expect("yt-mid.bin");

        let source = RemoteAudioSource {
            inner: Arc::new(RemoteAudioInner {
                client: reqwest::Client::new(),
                url: "https://example.invalid/videoplayback".into(),
                referer: String::new(),
                total_len: 68_201_223,
                header_end: AtomicU64::new(5_831),
                prefetch_window: PrefetchWindow {
                    low_water_bytes: 262_144,
                    target_bytes: 1_048_576,
                },
                cache: Mutex::new(vec![
                    CachedSegment { start: 0, data: head },
                    CachedSegment { start: 25_212_301, data: mid },
                ]),
                disk_ranges: Mutex::new(Vec::new()),
                in_flight: Mutex::new(HashMap::new()),
                range_available: Condvar::new(),
                demand_fetches: AtomicUsize::new(0),
                last_read_pos: Mutex::new(0),
                protected_ranges: Mutex::new(Vec::new()),
                disk_cache: None,
                cache_finalize_started: AtomicBool::new(false),
                prefetch_epoch: AtomicU64::new(0),
                remote_forbidden: AtomicBool::new(false),
                demuxer_open_sequential: AtomicBool::new(true),
                virtual_body_origin: AtomicU64::new(25_212_301),
                seek_index: Mutex::new(None),
                throughput_ewma_bps: AtomicU64::new(0),
                bitrate_bps: 0,
                playback_generation: Arc::new(AtomicU64::new(1)),
                expected_generation: 1,
            }),
            pos: 0,
            access_mode: RemoteAccessMode::StandardSeekable,
            read_cancellation: None,
        };

        let decoder = SymphoniaAudioDecoder::new_remote_virtual(source, true)
            .expect("virtual body open should succeed");
        // 5 秒 PCM（44.1kHz 立体声）；坏字节流在 0.25 秒左右就会枯竭
        let want = 44_100 * 2 * 5;
        let got = decoder.take(want).count();
        assert_eq!(got, want, "解码在 {got} 采样处枯竭（应 >= {want}）");
    }

    /// 回归：单次 read 绝不许跨过 header_end 的拼接边界
    ///
    /// 实机症状：seek 后 AAC 全是真假混杂的垃圾帧，0.25 秒后
    /// "unexpected end of bitstream" 杀死解码器。根因是头部缓存段在物理
    /// 空间连续（0..512KB），从 logical<header_end 起读时一路读进
    /// 「文件开头的第一个 moof」——它也是合法 moof，demuxer 便按它的
    /// trun 取样本。ts=5120 正是文件第一段的时间戳，铁证。
    #[test]
    fn reads_never_cross_the_virtual_body_boundary() {
        const HEADER_END: u64 = 5_831;
        const ORIGIN: u64 = 25_212_301;
        // 物理头部缓存（模拟启动时拉的 512KB 首包）：全部填 'H'
        let head_segment = CachedSegment {
            start: 0,
            data: vec![b'H'; 524_288],
        };
        // 目标 moof 处的缓存：全部填 'B'
        let body_segment = CachedSegment {
            start: ORIGIN,
            data: vec![b'B'; 262_144],
        };
        let mut source = RemoteAudioSource {
            inner: Arc::new(RemoteAudioInner {
                client: reqwest::Client::new(),
                url: "https://example.invalid/audio.m4a".into(),
                referer: String::new(),
                total_len: 68_201_223,
                header_end: AtomicU64::new(HEADER_END),
                prefetch_window: PrefetchWindow {
                    low_water_bytes: 262_144,
                    target_bytes: 1_048_576,
                },
                cache: Mutex::new(vec![head_segment, body_segment]),
                disk_ranges: Mutex::new(Vec::new()),
                in_flight: Mutex::new(HashMap::new()),
                range_available: Condvar::new(),
                demand_fetches: AtomicUsize::new(0),
                last_read_pos: Mutex::new(0),
                protected_ranges: Mutex::new(Vec::new()),
                disk_cache: None,
                cache_finalize_started: AtomicBool::new(false),
                prefetch_epoch: AtomicU64::new(0),
                remote_forbidden: AtomicBool::new(false),
                demuxer_open_sequential: AtomicBool::new(true),
                virtual_body_origin: AtomicU64::new(ORIGIN),
                seek_index: Mutex::new(None),
                throughput_ewma_bps: AtomicU64::new(0),
                bitrate_bps: 0,
                playback_generation: Arc::new(AtomicU64::new(1)),
                expected_generation: 1,
            }),
            pos: 0,
            access_mode: RemoteAccessMode::StandardSeekable,
            read_cancellation: None,
        };

        // 一次性请求 8KB，跨过 header_end：前 5831 字节必须是 'H'，之后必须是 'B'
        let mut out = vec![0_u8; 8_192];
        let mut filled = 0;
        while filled < out.len() {
            let read = std::io::Read::read(&mut source, &mut out[filled..]).unwrap();
            assert!(read > 0, "boundary read stalled at {filled}");
            filled += read;
        }
        assert!(out[..HEADER_END as usize].iter().all(|byte| *byte == b'H'));
        assert!(
            out[HEADER_END as usize..].iter().all(|byte| *byte == b'B'),
            "读取跨过了拼接边界：header 之后必须切到 body 物理段",
        );
    }

    /// 虚拟 body 的逻辑坐标必须连续，且长度不得被截断成已拉取的那一段
    ///
    /// 旧实现先把 2MB body 拉进内存再交给 Cursor，demuxer 于是在 2MB 处
    /// 撞上一个凭空多出来的 EOF，probe 直接 end of stream。
    #[test]
    fn virtual_body_reports_the_full_remaining_length() {
        let source = RemoteAudioSource {
            inner: Arc::new(RemoteAudioInner {
                client: reqwest::Client::new(),
                url: "https://rr5---sn-x.googlevideo.com/videoplayback".into(),
                referer: String::new(),
                total_len: 135_169_876,
                header_end: AtomicU64::new(10_799),
                prefetch_window: PrefetchWindow {
                    low_water_bytes: 262_144,
                    target_bytes: 1_048_576,
                },
                cache: Mutex::new(Vec::new()),
                disk_ranges: Mutex::new(Vec::new()),
                in_flight: Mutex::new(HashMap::new()),
                range_available: Condvar::new(),
                demand_fetches: AtomicUsize::new(0),
                last_read_pos: Mutex::new(0),
                protected_ranges: Mutex::new(Vec::new()),
                disk_cache: None,
                cache_finalize_started: AtomicBool::new(false),
                prefetch_epoch: AtomicU64::new(0),
                remote_forbidden: AtomicBool::new(false),
                demuxer_open_sequential: AtomicBool::new(false),
                virtual_body_origin: AtomicU64::new(57_047_830),
                seek_index: Mutex::new(None),
                throughput_ewma_bps: AtomicU64::new(0),
                bitrate_bps: 0,
                playback_generation: Arc::new(AtomicU64::new(1)),
                expected_generation: 1,
            }),
            pos: 0,
            access_mode: RemoteAccessMode::StandardSeekable,
            read_cancellation: None,
        };

        // 逻辑长度 = 头部 + 从落点到文件尾，而不是「已经拉下来的那一段」
        let expected = 10_799 + (135_169_876 - 57_047_830);
        assert_eq!(
            symphonia::core::io::MediaSource::byte_len(&source),
            Some(expected),
        );

        // [0, header_end) 映射到物理头部，之后按落点平移，坐标必须连续
        assert_eq!(source.map_logical_to_physical(0), 0);
        assert_eq!(source.map_logical_to_physical(10_798), 10_798);
        assert_eq!(source.map_logical_to_physical(10_799), 57_047_830);
        assert_eq!(source.map_logical_to_physical(10_799 + 4_096), 57_051_926);

        // demuxer 给的绝对 base_data_offset 要能折回逻辑坐标
        assert_eq!(source.normalize_seek_offset(57_047_830), 10_799);
        assert_eq!(source.normalize_seek_offset(57_051_926), 10_799 + 4_096);
    }

    #[test]
    fn progressive_mp4_is_not_sniffed_as_fragmented() {
        // ftyp + moov(仅 mvhd) + mdat：普通渐进式 MP4
        let moov_body = mp4_box(b"mvhd", &[0u8; 16]);
        let mut head = mp4_box(b"ftyp", &[0u8; 8]);
        head.extend_from_slice(&mp4_box(b"moov", &moov_body));
        head.extend_from_slice(&mp4_box(b"mdat", &[0u8; 32]));

        assert!(!super::detect_fragmented_mp4(&head));
        assert!(!super::detect_fragmented_mp4(&[]));
        assert!(!super::detect_fragmented_mp4(b"OggS\x00\x02"));
    }

    #[test]
    fn youtube_style_url_needs_payload_sniffing() {
        // googlevideo 直链没有扩展名，仅靠 URL 判不出分片
        let url = "https://rr5---sn-abc.googlevideo.com/videoplayback?expire=1&itag=140";
        assert!(!super::is_fragmented_mp4_url(url));
        assert!(super::is_fragmented_mp4_url("https://cdn.example/audio/seg-1.m4s"));
    }

    #[test]
    fn detect_mp4_header_end_finds_mdat() {
        // ftyp(16) + free(8) + mdat(size=100)
        let mut data = Vec::new();
        data.extend_from_slice(&16u32.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(&[0u8; 8]);
        data.extend_from_slice(&8u32.to_be_bytes());
        data.extend_from_slice(b"free");
        data.extend_from_slice(&100u32.to_be_bytes());
        data.extend_from_slice(b"mdat");
        data.extend_from_slice(&[0u8; 20]);
        assert_eq!(super::detect_mp4_header_end(&data), Some(24));
    }

    #[test]
    fn find_moof_offset_near_snaps_past_garbage_prefix() {
        // 12 字节垃圾 + moof(size=32)
        let mut data = vec![0u8; 12];
        data.extend_from_slice(&32u32.to_be_bytes());
        data.extend_from_slice(b"moof");
        data.extend_from_slice(&[0u8; 24]);
        assert_eq!(super::find_moof_offset_near(&data, 0, 64), Some(12));
        // 已对齐
        assert_eq!(super::find_moof_offset_near(&data[12..], 0, 64), Some(0));
    }

    #[test]
    fn virtual_body_maps_logical_header_and_body() {
        let source = RemoteAudioSource {
            inner: Arc::new(RemoteAudioInner {
                client: reqwest::Client::new(),
                url: "https://cdn.example/audio/long.m4a".into(),
                referer: "https://example.com".into(),
                total_len: 10_000,
                header_end: AtomicU64::new(100),
                prefetch_window: PrefetchWindow {
                    low_water_bytes: 1,
                    target_bytes: 1,
                },
                cache: Mutex::new(Vec::new()),
                disk_ranges: Mutex::new(Vec::new()),
                in_flight: Mutex::new(HashMap::new()),
                range_available: Condvar::new(),
                demand_fetches: AtomicUsize::new(0),
                last_read_pos: Mutex::new(0),
                protected_ranges: Mutex::new(Vec::new()),
                disk_cache: None,
                cache_finalize_started: AtomicBool::new(false),
                prefetch_epoch: AtomicU64::new(0),
                remote_forbidden: AtomicBool::new(false),
                demuxer_open_sequential: AtomicBool::new(false),
                virtual_body_origin: AtomicU64::new(5_000),
                seek_index: Mutex::new(None),
                throughput_ewma_bps: AtomicU64::new(0),
                bitrate_bps: 0,
                playback_generation: Arc::new(AtomicU64::new(1)),
                expected_generation: 1,
            }),
            pos: 0,
            access_mode: RemoteAccessMode::StandardSeekable,
            read_cancellation: None,
        };

        assert_eq!(source.map_logical_to_physical(0), 0);
        assert_eq!(source.map_logical_to_physical(99), 99);
        // 相对 moof：逻辑 body 映射到 physical origin+
        assert_eq!(source.map_logical_to_physical(100), 5_000);
        assert_eq!(source.map_logical_to_physical(150), 5_050);
        // 纯逻辑：pos 绝不能用物理恒等，否则 MSS/AtomIterator 失步
        // 5_000 在逻辑里是 body 中点：physical = 5000 + (5000-100) = 9900
        assert_eq!(source.map_logical_to_physical(5_000), 9_900);
        assert_eq!(source.logical_len(), 100 + (10_000 - 5_000));
    }

    #[test]
    fn virtual_body_normalizes_absolute_sample_seeks() {
        use std::io::{Seek, SeekFrom};
        // 对齐真实长视频：header=10799, body_origin≈55MB, total≈135MB
        let mut source = RemoteAudioSource {
            inner: Arc::new(RemoteAudioInner {
                client: reqwest::Client::new(),
                url: "https://cdn.example/audio/long.m4a".into(),
                referer: "https://example.com".into(),
                total_len: 135_169_876,
                header_end: AtomicU64::new(10_799),
                prefetch_window: PrefetchWindow {
                    low_water_bytes: 1,
                    target_bytes: 1,
                },
                cache: Mutex::new(Vec::new()),
                disk_ranges: Mutex::new(Vec::new()),
                in_flight: Mutex::new(HashMap::new()),
                range_available: Condvar::new(),
                demand_fetches: AtomicUsize::new(0),
                last_read_pos: Mutex::new(0),
                protected_ranges: Mutex::new(Vec::new()),
                disk_cache: None,
                cache_finalize_started: AtomicBool::new(false),
                prefetch_epoch: AtomicU64::new(0),
                remote_forbidden: AtomicBool::new(false),
                demuxer_open_sequential: AtomicBool::new(false),
                virtual_body_origin: AtomicU64::new(54_947_352),
                seek_index: Mutex::new(None),
                throughput_ewma_bps: AtomicU64::new(0),
                bitrate_bps: 0,
                playback_generation: Arc::new(AtomicU64::new(1)),
                expected_generation: 1,
            }),
            pos: 0,
            access_mode: RemoteAccessMode::StandardSeekable,
            read_cancellation: None,
        };

        // 相对样本：逻辑 header_end → 物理 origin
        assert_eq!(source.map_logical_to_physical(10_799), 54_947_352);
        assert_eq!(source.map_logical_to_physical(11_299), 54_947_852);
        // 绝对 base_data_offset：Seek(物理) 必须折成逻辑并返回逻辑 pos
        let logical = source.seek(SeekFrom::Start(54_947_852)).unwrap();
        assert_eq!(logical, 10_799 + 500);
        assert_eq!(source.pos, 10_799 + 500);
        assert_eq!(source.map_logical_to_physical(source.pos), 54_947_852);
        // logical_len = header + (total - origin)，不能被拉成 total
        assert_eq!(
            source.logical_len(),
            10_799 + (135_169_876 - 54_947_352)
        );
    }

    #[test]
    fn estimate_segment_start_snaps_to_entry_boundary() {
        let index = super::RemoteSeekIndex {
            duration_ms: 100_000,
            entries: vec![
                super::SeekIndexEntry {
                    time_ms: 0,
                    byte_offset: 1_000,
                },
                super::SeekIndexEntry {
                    time_ms: 50_000,
                    byte_offset: 50_000,
                },
                super::SeekIndexEntry {
                    time_ms: 100_000,
                    byte_offset: 100_000,
                },
            ],
        };
        assert_eq!(index.estimate_segment_start(0), 1_000);
        assert_eq!(index.estimate_segment_start(49_999), 1_000);
        assert_eq!(index.estimate_segment_start(50_000), 50_000);
        assert_eq!(index.estimate_segment_start(90_000), 50_000);
        assert_eq!(index.estimate_segment_start(100_000), 100_000);
    }

    #[test]
    fn virtual_body_seek_uses_logical_sample_offsets() {
        use std::io::{Seek, SeekFrom};
        let mut source = RemoteAudioSource {
            inner: Arc::new(RemoteAudioInner {
                client: reqwest::Client::new(),
                url: "https://cdn.example/audio/long.m4a".into(),
                referer: "https://example.com".into(),
                total_len: 10_000,
                header_end: AtomicU64::new(100),
                prefetch_window: PrefetchWindow {
                    low_water_bytes: 1,
                    target_bytes: 1,
                },
                cache: Mutex::new(Vec::new()),
                disk_ranges: Mutex::new(Vec::new()),
                in_flight: Mutex::new(HashMap::new()),
                range_available: Condvar::new(),
                demand_fetches: AtomicUsize::new(0),
                last_read_pos: Mutex::new(0),
                protected_ranges: Mutex::new(Vec::new()),
                disk_cache: None,
                cache_finalize_started: AtomicBool::new(false),
                prefetch_epoch: AtomicU64::new(0),
                remote_forbidden: AtomicBool::new(false),
                demuxer_open_sequential: AtomicBool::new(false),
                virtual_body_origin: AtomicU64::new(5_000),
                seek_index: Mutex::new(None),
                throughput_ewma_bps: AtomicU64::new(0),
                bitrate_bps: 0,
                playback_generation: Arc::new(AtomicU64::new(1)),
                expected_generation: 1,
            }),
            pos: 0,
            access_mode: RemoteAccessMode::StandardSeekable,
            read_cancellation: None,
        };

        // sample Seek 用逻辑坐标；映射到物理 body
        assert_eq!(source.seek(SeekFrom::Start(100)).unwrap(), 100);
        assert_eq!(source.map_logical_to_physical(source.pos), 5_000);
        assert_eq!(source.seek(SeekFrom::Start(150)).unwrap(), 150);
        assert_eq!(source.map_logical_to_physical(source.pos), 5_050);
        assert_eq!(source.seek(SeekFrom::Start(50)).unwrap(), 50);
        assert_eq!(source.map_logical_to_physical(source.pos), 50);
    }

    #[test]
    fn linear_seek_index_estimates_midpoint_bytes() {
        let index = super::RemoteSeekIndex {
            duration_ms: 100_000,
            entries: vec![
                super::SeekIndexEntry {
                    time_ms: 0,
                    byte_offset: 0,
                },
                super::SeekIndexEntry {
                    time_ms: 50_000,
                    byte_offset: 500,
                },
                super::SeekIndexEntry {
                    time_ms: 100_000,
                    byte_offset: 1_000,
                },
            ],
        };
        assert_eq!(index.estimate_byte(0), 0);
        assert_eq!(index.estimate_byte(25_000), 250);
        assert_eq!(index.estimate_byte(75_000), 750);
        assert_eq!(index.estimate_byte(100_000), 1_000);
    }

    #[test]
    fn parse_sidx_body_builds_time_byte_entries() {
        // version0 sidx: timescale=1000, earliest=0, first_offset=0, 2 refs size=100 dur=1000
        let mut body = Vec::new();
        body.push(0); // version
        body.extend_from_slice(&[0, 0, 0]); // flags
        body.extend_from_slice(&1u32.to_be_bytes()); // reference_id
        body.extend_from_slice(&1000u32.to_be_bytes()); // timescale
        body.extend_from_slice(&0u32.to_be_bytes()); // earliest_pts
        body.extend_from_slice(&0u32.to_be_bytes()); // first_offset
        body.extend_from_slice(&0u16.to_be_bytes()); // reserved
        body.extend_from_slice(&2u16.to_be_bytes()); // reference_count
        for _ in 0..2 {
            body.extend_from_slice(&100u32.to_be_bytes()); // size, type=media
            body.extend_from_slice(&1000u32.to_be_bytes()); // duration ticks
            body.extend_from_slice(&0u32.to_be_bytes()); // sap
        }
        let index = super::parse_sidx_body(&body, 0, 10_000, 10_000).expect("sidx");
        assert!(index.entries.len() >= 3);
        assert_eq!(index.entries[0].byte_offset, 0);
        assert_eq!(index.entries[1].byte_offset, 100);
        assert_eq!(index.entries[1].time_ms, 1000);
    }

    #[test]
    fn invalidate_prefetch_epoch_clears_in_flight_and_allows_reclaim() {
        let inner = Arc::new(RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "https://cdn.example/audio/long.m4a".into(),
            referer: "https://example.com".into(),
            total_len: 1024,
            header_end: AtomicU64::new(0),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 1,
                target_bytes: 1,
            },
            cache: Mutex::new(Vec::new()),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::from([
                (
                    0,
                    Arc::new(InflightFetch {
                        end: AtomicU64::new(255),
                        done: AtomicBool::new(false),
                    }),
                ),
                (
                    256,
                    Arc::new(InflightFetch {
                        end: AtomicU64::new(511),
                        done: AtomicBool::new(false),
                    }),
                ),
            ])),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(0),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::new(AtomicU64::new(1)),
            expected_generation: 1,
        });
        assert!(super::claim_prefetch_start(&inner, 0, 255).is_none());
        super::invalidate_prefetch_epoch(&inner);
        assert_eq!(
            inner.prefetch_epoch.load(std::sync::atomic::Ordering::Acquire),
            1
        );
        assert!(inner.in_flight.lock().expect("lock").is_empty());
        let entry = super::claim_prefetch_start(&inner, 0, 255).expect("reclaim after invalidate");
        super::release_prefetch_start(&inner, 0, &entry);
    }

    #[test]
    fn trim_cache_keeps_protected_tail_segments() {
        let protected = CachedSegment {
            start: 900,
            data: vec![9; 100],
        };
        let near = CachedSegment {
            start: 0,
            data: vec![1; super::REMOTE_MAX_CACHE_BYTES],
        };
        let inner = RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "https://cdn.example/audio/long.m4a".into(),
            referer: "https://example.com".into(),
            total_len: 1000,
            header_end: AtomicU64::new(0),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 1,
                target_bytes: 1,
            },
            cache: Mutex::new(vec![near, protected]),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::new()),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(vec![CachedRange {
                start: 900,
                end: 1000,
            }]),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(0),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::new(AtomicU64::new(1)),
            expected_generation: 1,
        };
        let mut cache = {
            let guard = inner.cache.lock().expect("cache");
            guard
                .iter()
                .map(|segment| CachedSegment {
                    start: segment.start,
                    data: segment.data.clone(),
                })
                .collect::<Vec<_>>()
        };
        super::trim_cache(&inner, &mut cache, 0);
        assert!(
            cache.iter().any(|segment| segment.start == 900),
            "protected tail must survive trim"
        );
    }

    #[test]
    fn disk_cache_is_ready_only_when_ranges_cover_entire_source() {
        assert!(!ranges_cover_source(
            &[
                CachedRange { start: 0, end: 512 },
                CachedRange {
                    start: 768,
                    end: 1_024,
                },
            ],
            1_024,
        ));
        assert!(ranges_cover_source(
            &[
                CachedRange { start: 0, end: 512 },
                CachedRange {
                    start: 512,
                    end: 1_024,
                },
            ],
            1_024,
        ));
    }

    #[test]
    fn cache_marker_requires_hash_length_and_file_name() {
        let hash = "ab".repeat(32);
        let marker = format_cache_marker(2_048, &hash, "digest.audio");

        assert_eq!(
            parse_cache_marker(&marker),
            Some(CacheMarker::Validated {
                content_length: 2_048,
                sha256: hash,
                file_name: "digest.audio".into(),
            })
        );
        assert_eq!(
            parse_cache_marker("v1:1024"),
            Some(CacheMarker::Legacy {
                content_length: 1_024,
            })
        );
        assert_eq!(parse_cache_marker("v1:0"), None);
        assert_eq!(parse_cache_marker("v2:1024:bad:digest.audio"), None);
    }

    #[test]
    fn content_range_parser_rejects_wildcards_and_invalid_bounds() {
        assert_eq!(
            parse_content_range(&HeaderValue::from_static("bytes 0-511/1024")),
            Some(HttpByteRange {
                start: 0,
                end: 511,
                total: 1_024,
            })
        );
        assert_eq!(
            parse_content_range(&HeaderValue::from_static("bytes */1024")),
            None
        );
        assert_eq!(
            parse_content_range(&HeaderValue::from_static("bytes 512-511/1024")),
            None
        );
    }

    #[test]
    fn duration_guard_only_rejects_materially_short_media() {
        assert!(!cache_duration_is_suspicious(Some(272_000), 271_530));
        assert!(!cache_duration_is_suspicious(Some(272_000), 0));
        assert!(cache_duration_is_suspicious(Some(272_000), 30_000));
    }

    #[test]
    fn zero_frame_track_duration_is_treated_as_unknown() {
        assert_eq!(
            track_duration(Some(TimeBase::new(1, 96_000)), Some(0)),
            None
        );
    }

    #[test]
    fn unknown_zero_duration_does_not_clamp_seek_to_track_start() {
        let resolved = seek_target_time(
            Some(Time::new(0, 0.0)),
            Duration::from_millis(22_720),
        );

        assert_eq!(resolved.seconds, 22);
        assert!((resolved.frac - 0.72).abs() < 0.000_001);
    }

    #[test]
    fn known_duration_still_clamps_seek_just_before_track_end() {
        let resolved = seek_target_time(
            Some(Time::new(272, 0.0)),
            Duration::from_millis(300_000),
        );

        assert_eq!(resolved.seconds, 271);
        assert!((resolved.frac - 0.9999).abs() < 0.000_001);
    }

    #[test]
    fn cache_publish_requires_decodable_complete_staging_file() {
        let root = tempfile::tempdir().expect("temp cache root");
        let wav = pcm_wav(800);
        let cache = RemoteAudioCache::new(
            root.path().to_path_buf(),
            "valid-audio",
            1024 * 1024,
            Some(wav.len() as u64),
            100,
        )
        .expect("cache");

        cache.prepare(wav.len() as u64).expect("prepare");
        cache.write_range(0, &wav).expect("write staging");
        let staging_path = cache.staging_path().expect("staging path");
        assert!(!cache.ready_path.exists());

        cache.mark_ready().expect("publish validated cache");
        let published = cache.ready_path().expect("ready cache");
        assert_eq!(std::fs::read(published).expect("published bytes"), wav);
        assert!(!staging_path.exists());
    }

    #[test]
    fn invalid_media_never_creates_ready_marker() {
        let root = tempfile::tempdir().expect("temp cache root");
        let invalid = b"<html>not audio</html>";
        let cache = RemoteAudioCache::new(
            root.path().to_path_buf(),
            "invalid-audio",
            1024 * 1024,
            Some(invalid.len() as u64),
            0,
        )
        .expect("cache");

        cache.prepare(invalid.len() as u64).expect("prepare");
        cache.write_range(0, invalid).expect("write staging");

        assert!(cache.mark_ready().is_err());
        assert!(!cache.ready_path.exists());
        assert!(cache.ready_path().is_none());
    }

    #[test]
    fn concurrent_cache_instances_never_share_staging_files() {
        let root = tempfile::tempdir().expect("temp cache root");
        let first = RemoteAudioCache::new(
            root.path().to_path_buf(),
            "same-key",
            1024 * 1024,
            None,
            0,
        )
        .expect("first cache");
        let second = RemoteAudioCache::new(
            root.path().to_path_buf(),
            "same-key",
            1024 * 1024,
            None,
            0,
        )
        .expect("second cache");

        assert_ne!(
            first.staging_path().expect("first staging"),
            second.staging_path().expect("second staging")
        );
    }

    #[test]
    fn complete_fallback_download_publishes_through_fresh_staging() {
        let root = tempfile::tempdir().expect("temp cache root");
        let wav = pcm_wav(800);
        let initial = RemoteAudioCache::new(
            root.path().to_path_buf(),
            "fallback-audio",
            1024 * 1024,
            Some(wav.len() as u64),
            100,
        )
        .expect("initial cache");
        let fallback = initial.fresh_staging().expect("fresh staging");
        let initial_staging = initial.staging_path().expect("initial staging");

        let published = fallback
            .publish_complete_bytes(&wav)
            .expect("publish fallback bytes");
        let fallback_staging = fallback.staging_path().expect("fallback staging");

        assert_ne!(initial_staging, fallback_staging);
        assert_eq!(std::fs::read(published).expect("published bytes"), wav);
        assert!(fallback.ready_path().is_none());
        fallback
            .bypass_ready
            .store(false, std::sync::atomic::Ordering::Release);
        assert!(fallback.ready_path().is_some());
    }

    #[test]
    fn sequential_stream_publish_keeps_cache_unready_until_download_finishes() {
        let root = tempfile::tempdir().expect("temp cache root");
        let wav = pcm_wav(800);
        let cache = RemoteAudioCache::new(
            root.path().to_path_buf(),
            "sequential-audio",
            1024 * 1024,
            Some(wav.len() as u64),
            100,
        )
        .expect("cache");

        let staging = cache
            .prepare_sequential_write(wav.len() as u64)
            .expect("prepare sequential cache");
        std::fs::write(&staging, &wav).expect("write sequential cache");
        assert!(cache.ready_path().is_none());

        let published = cache
            .publish_sequential_write()
            .expect("publish sequential cache");
        assert_eq!(std::fs::read(published).expect("published bytes"), wav);
    }

    #[test]
    fn cache_hit_does_not_create_unused_staging_file() {
        let root = tempfile::tempdir().expect("temp cache root");
        let wav = pcm_wav(800);
        let writer = RemoteAudioCache::new(
            root.path().to_path_buf(),
            "cache-hit",
            1024 * 1024,
            Some(wav.len() as u64),
            100,
        )
        .expect("writer cache");
        writer
            .publish_complete_bytes(&wav)
            .expect("publish cached audio");
        drop(writer);

        let hit = RemoteAudioCache::new(
            root.path().to_path_buf(),
            "cache-hit",
            1024 * 1024,
            Some(wav.len() as u64),
            100,
        )
        .expect("cache hit");

        assert!(hit.ready_path().is_some());
        assert!(hit.staging.lock().expect("staging lock").path.is_none());
    }

    #[test]
    fn cache_lookup_miss_does_not_create_a_shard_directory() {
        let root = tempfile::tempdir().expect("cache root");
        let cache = RemoteAudioCache::new(
            root.path().join("audio"),
            "lookup-only",
            512 * 1024 * 1024,
            None,
            1_000,
        )
        .expect("cache");

        assert!(!cache.cache_dir.exists());
        assert!(cache.ready_path().is_none());
        assert!(!cache.cache_dir.exists());
    }

    #[test]
    fn cache_capacity_scan_runs_only_after_a_complete_file_is_published() {
        let root = tempfile::tempdir().expect("temp cache root");
        let wav = pcm_wav(800);
        let existing = RemoteAudioCache::new(
            root.path().to_path_buf(),
            "existing-audio",
            1024 * 1024,
            Some(wav.len() as u64),
            100,
        )
        .expect("existing cache");
        let existing_path = existing
            .publish_complete_bytes(&wav)
            .expect("publish existing audio");

        let lookup = RemoteAudioCache::new(
            root.path().to_path_buf(),
            "new-audio",
            wav.len() as u64,
            Some(wav.len() as u64),
            100,
        )
        .expect("new cache lookup");
        assert!(existing_path.exists());

        lookup
            .prepare(wav.len() as u64)
            .expect("prepare cache miss");
        assert!(existing_path.exists());
        lookup.write_range(0, &wav).expect("write complete cache");
        lookup.mark_ready().expect("publish complete cache");
        assert!(!existing_path.exists());
    }

    #[test]
    fn validated_cache_stamp_is_reused_only_for_unchanged_file_metadata() {
        let path = PathBuf::from("validated-cache-stamp-test.audio");
        let modified = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(42);
        let sha256 = "ab".repeat(32);
        let validated = ValidatedCacheFile {
            content_length: 1_024,
            sha256: sha256.clone(),
        };
        remember_validated_audio_cache(&path, Some(modified), &validated);

        assert!(reuse_validated_audio_cache(
            &path,
            1_024,
            Some(modified),
            Some(&sha256),
        )
        .is_some());
        assert!(reuse_validated_audio_cache(
            &path,
            1_025,
            Some(modified),
            Some(&sha256),
        )
        .is_none());
        assert!(reuse_validated_audio_cache(
            &path,
            1_024,
            Some(modified + Duration::from_secs(1)),
            Some(&sha256),
        )
        .is_none());
    }

    #[test]
    fn android_alignment_published_cache_rejects_same_length_corruption_without_a_stamp() {
        use sha2::{Digest, Sha256};
        let root = tempfile::tempdir().expect("temporary cache");
        let original = b"abcdefgh";
        let hash = hex::encode(Sha256::digest(original));
        let name = format!("cached.{hash}.audio");
        let path = root.path().join(&name);
        std::fs::write(&path, b"ABCDEFGH").expect("corrupted file");
        assert!(super::validate_published_cache_file(&path, 8, Some(8), None, &hash, &name).is_err());
    }

    #[test]
    fn android_alignment_published_cache_accepts_a_verified_cold_file_without_reprobing_audio() {
        use sha2::{Digest, Sha256};
        let root = tempfile::tempdir().expect("temporary cache");
        let bytes = b"abcdefgh";
        let hash = hex::encode(Sha256::digest(bytes));
        let name = format!("cached.{hash}.audio");
        let path = root.path().join(&name);
        std::fs::write(&path, bytes).expect("published file");
        assert!(super::validate_published_cache_file(&path, 8, Some(8), None, &hash, &name).is_ok());
    }

    #[test]
    fn android_alignment_source_md5_is_checked_before_publication_and_reuse() {
        use sha2::Digest;
        let root = tempfile::tempdir().expect("temporary cache");
        let wav = pcm_wav(800);
        let checksum = hex::encode(md5::Md5::digest(&wav));
        let cache = RemoteAudioCache::new(root.path().to_path_buf(), "md5-source", 1024 * 1024, Some(wav.len() as u64), 100)
            .unwrap().with_expected_md5(Some(&checksum)).unwrap();
        cache.publish_complete_bytes(&wav).unwrap();
        assert!(cache.ready_path().is_some());
        let wrong = "00".repeat(16);
        assert!(cache.clone().with_expected_md5(Some(&wrong)).unwrap().ready_path().is_none());
        let bad = RemoteAudioCache::new(root.path().to_path_buf(), "md5-mismatch", 1024 * 1024, Some(wav.len() as u64), 100)
            .unwrap().with_expected_md5(Some(&wrong)).unwrap();
        assert!(bad.publish_complete_bytes(&wav).is_err());
        assert!(!bad.ready_path.exists());
        assert!(cache.with_expected_md5(Some("invalid")).is_err());
    }

    #[test]
    fn throughput_ewma_blends_successes_and_only_sinks_on_failures() {
        // 实机日志样本：512KB / 15s ≈ 34.9KB/s
        assert_eq!(throughput_sample_bps(524_288, 15_000), 34_952);
        // 首个样本直接采纳；此后按 30% 权重混入
        assert_eq!(ewma_bps_after_success(0, 100_000), 100_000);
        assert_eq!(ewma_bps_after_success(100_000, 50_000), 85_000);
        // 失败样本是吞吐上界：跑得太短不计入；高于当前估计不上调
        assert_eq!(ewma_bps_after_failure(100_000, 524_288, 1_000), 100_000);
        assert_eq!(ewma_bps_after_failure(20_000, 524_288, 15_000), 20_000);
        // 低于当前估计时按同一权重向下混合
        assert_eq!(ewma_bps_after_failure(100_000, 524_288, 15_000), 80_485);
    }

    #[test]
    fn adaptive_block_shrinks_on_slow_and_recovers_on_fast_networks() {
        // 无样本：保持调用方默认块（冷启动行为不变）
        assert_eq!(adaptive_block_bytes(0, 256 * 1024, 256 * 1024), 256 * 1024);
        assert_eq!(adaptive_block_bytes(0, 512 * 1024, 1024 * 1024), 512 * 1024);
        // 慢网 50KB/s：8s 目标 → 400KB，向下对齐 64KB → 384KB
        assert_eq!(
            adaptive_block_bytes(51_200, 512 * 1024, 1024 * 1024),
            393_216
        );
        // 需求读上限受 mode 块钳制
        assert_eq!(
            adaptive_block_bytes(2 * 1024 * 1024, 256 * 1024, 256 * 1024),
            256 * 1024
        );
        // 快网 2MB/s：预取恢复到 1MB 上限
        assert_eq!(
            adaptive_block_bytes(2 * 1024 * 1024, 512 * 1024, 1024 * 1024),
            1024 * 1024
        );
        // 极慢网也不低于 64KB 下限
        assert_eq!(adaptive_block_bytes(1_000, 512 * 1024, 1024 * 1024), 65_536);
    }

    #[test]
    fn request_timeout_scales_with_block_size_and_throughput() {
        // 无吞吐样本：20s 下限（原 15s 已被实机 12.4s 的成功请求贴脸打穿）
        assert_eq!(request_timeout_for(524_288, 0), Duration::from_secs(20));
        // 自适应块本身就按 8s 目标选：预算落在下限
        assert_eq!(
            request_timeout_for(262_144, 51_200),
            Duration::from_secs(20)
        );
        // 分片 8MB 大块 @ 400KB/s：预计 20.5s ×1.5 + 5s ≈ 35.7s
        assert_eq!(
            request_timeout_for(8 * 1024 * 1024, 409_600),
            Duration::from_millis(35_720)
        );
        // 上限 60s：不允许无限拉长
        assert_eq!(
            request_timeout_for(8 * 1024 * 1024, 102_400),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn prefetch_concurrency_yields_to_demand_reads_on_slow_networks() {
        // 吞吐或码率未知：维持默认并发
        assert_eq!(prefetch_lane_count(0, 45_000), REMOTE_PREFETCH_CONCURRENCY);
        assert_eq!(
            prefetch_lane_count(200_000, 0),
            REMOTE_PREFETCH_CONCURRENCY
        );
        // EWMA 低于码率：单 lane，带宽让给需求读
        assert_eq!(prefetch_lane_count(40_000, 45_000), 1);
        // 1-2 倍码率之间：2 lane
        assert_eq!(prefetch_lane_count(60_000, 45_000), 2);
        // 富余吞吐：恢复满并发
        assert_eq!(
            prefetch_lane_count(200_000, 45_000),
            REMOTE_PREFETCH_CONCURRENCY
        );
    }

    #[test]
    fn halved_fetch_len_bottoms_out_at_min_block() {
        assert_eq!(halved_fetch_len(524_288), 262_144);
        assert_eq!(halved_fetch_len(131_072), 65_536);
        assert_eq!(halved_fetch_len(65_536), 65_536);
    }

    /// 记录到的 range 请求列表 (start, end)
    type SeenRanges = Arc<Mutex<Vec<(u64, u64)>>>;

    /// 本地 range 服务器：前 fail_first 个请求发送响应头后只回半个 body
    /// 即断开（模拟慢网下 per-request 超时打断 body 读取的传输中断），
    /// 之后的请求正常完整响应；记录每个请求的 range 供断言
    fn spawn_flaky_range_server(
        body: Arc<Vec<u8>>,
        fail_first: usize,
    ) -> (String, SeenRanges) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind range server");
        let addr = listener.local_addr().expect("range server addr");
        let requests: Arc<Mutex<Vec<(u64, u64)>>> = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        thread::spawn(move || {
            let mut served = 0usize;
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut request = Vec::new();
                let mut chunk = [0u8; 2048];
                loop {
                    match std::io::Read::read(&mut stream, &mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => {
                            request.extend_from_slice(&chunk[..count]);
                            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
                let text = String::from_utf8_lossy(&request);
                let Some(spec) = text
                    .lines()
                    .find(|line| line.to_ascii_lowercase().starts_with("range:"))
                    .and_then(|line| line.split('=').nth(1))
                else {
                    continue;
                };
                let mut bounds = spec.trim().split('-');
                let start: u64 = bounds.next().unwrap_or("0").parse().unwrap_or(0);
                let end: u64 = bounds.next().unwrap_or("0").parse().unwrap_or(0);
                seen.lock().expect("requests lock").push((start, end));
                let total = body.len() as u64;
                let end = end.min(total.saturating_sub(1));
                let slice = &body[start as usize..=end as usize];
                let header = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    start,
                    end,
                    total,
                    slice.len(),
                );
                if std::io::Write::write_all(&mut stream, header.as_bytes()).is_err() {
                    continue;
                }
                served += 1;
                let payload = if served <= fail_first {
                    &slice[..slice.len() / 2]
                } else {
                    slice
                };
                let _ = std::io::Write::write_all(&mut stream, payload);
                let _ = std::io::Write::flush(&mut stream);
                // fail_first 内的连接在此 drop：Content-Length 未喂满即断开
            }
        });
        (format!("http://{}/audio.bin", addr), requests)
    }

    /// 回归：慢网超时/传输中断的需求读绝不 fatal——同一起点折半块大小重试，
    /// 直到成功；解码层只看到一次变慢的 read，而不是会话死亡
    #[test]
    fn demand_fetch_retries_transient_failures_with_halved_blocks() {
        let total_len: u64 = 1024 * 1024;
        let body: Arc<Vec<u8>> = Arc::new(
            (0..total_len).map(|index| (index % 251) as u8).collect(),
        );
        // 前两个请求（256KB、128KB）中途断开，第三个（64KB）放行
        let (url, requests) = spawn_flaky_range_server(Arc::clone(&body), 2);
        let inner = Arc::new(RemoteAudioInner {
            // 必须绕过系统代理：本地回环服务器走代理会拿到 502
            client: reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("no-proxy client"),
            url,
            referer: "https://example.com".into(),
            total_len,
            header_end: AtomicU64::new(0),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 1,
                target_bytes: 1,
            },
            cache: Mutex::new(Vec::new()),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::new()),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(0),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::new(AtomicU64::new(1)),
            expected_generation: 1,
        });
        let mut source = RemoteAudioSource {
            inner: Arc::clone(&inner),
            pos: 0,
            access_mode: RemoteAccessMode::StandardSeekable,
            read_cancellation: None,
        };
        let mut bytes = [0u8; 4];

        source
            .read_exact(&mut bytes)
            .expect("demand read must survive transient failures");

        assert_eq!(bytes, [0, 1, 2, 3]);
        // 断言折半序列：256KB → 128KB → 64KB
        assert_eq!(
            requests.lock().expect("requests lock").clone(),
            vec![(0, 262_143), (0, 131_071), (0, 65_535)],
        );
        // 成功后在途登记必须清空（等待方不会悬挂在残留登记上）
        assert!(inner.in_flight.lock().expect("in-flight lock").is_empty());
        // 成功样本已混入吞吐估计，供后续自适应选块
        assert!(inner.throughput_bps() > 0);
    }

    /// 起始偏移 >= forbid_from 的 range 一律回 403（模拟 googlevideo
    /// 直链过期/远端 range 风控），其余按 206 正常切片
    fn spawn_forbidden_range_server(
        body: Arc<Vec<u8>>,
        forbid_from: u64,
    ) -> (String, SeenRanges) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind range server");
        let addr = listener.local_addr().expect("range server addr");
        let requests: Arc<Mutex<Vec<(u64, u64)>>> = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut request = Vec::new();
                let mut chunk = [0u8; 2048];
                loop {
                    match std::io::Read::read(&mut stream, &mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => {
                            request.extend_from_slice(&chunk[..count]);
                            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
                let text = String::from_utf8_lossy(&request);
                let Some(spec) = text
                    .lines()
                    .find(|line| line.to_ascii_lowercase().starts_with("range:"))
                    .and_then(|line| line.split('=').nth(1))
                else {
                    continue;
                };
                let mut bounds = spec.trim().split('-');
                let start: u64 = bounds.next().unwrap_or("0").parse().unwrap_or(0);
                let end: u64 = bounds.next().unwrap_or("0").parse().unwrap_or(0);
                seen.lock().expect("requests lock").push((start, end));
                if start >= forbid_from {
                    let _ = std::io::Write::write_all(
                        &mut stream,
                        b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    let _ = std::io::Write::flush(&mut stream);
                    continue;
                }
                let total = body.len() as u64;
                let end = end.min(total.saturating_sub(1));
                let slice = &body[start as usize..=end as usize];
                let header = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    start,
                    end,
                    total,
                    slice.len(),
                );
                if std::io::Write::write_all(&mut stream, header.as_bytes()).is_err() {
                    continue;
                }
                let _ = std::io::Write::write_all(&mut stream, slice);
                let _ = std::io::Write::flush(&mut stream);
            }
        });
        (format!("http://{}/audio.bin", addr), requests)
    }

    fn forbidden_test_inner(url: String, total_len: u64) -> Arc<RemoteAudioInner> {
        Arc::new(RemoteAudioInner {
            client: reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("no-proxy client"),
            url,
            referer: "https://example.com".into(),
            total_len,
            header_end: AtomicU64::new(0),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 1,
                target_bytes: 1,
            },
            cache: Mutex::new(Vec::new()),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::new()),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(0),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::new(AtomicU64::new(1)),
            expected_generation: 1,
        })
    }

    /// 回归（YouTube seek 后 403 无限重试）：403 是永久错误，需求读必须
    /// 一次失败立即上抛 PermissionDenied——绝不折半重试、绝不重复打请求，
    /// 并且置位 remote_forbidden 停掉后续预取孵化
    #[test]
    fn range_403_fails_fast_without_retry_and_disables_prefetch() {
        let total_len: u64 = 1024 * 1024;
        let body: Arc<Vec<u8>> = Arc::new(vec![0u8; total_len as usize]);
        let (url, requests) = spawn_forbidden_range_server(Arc::clone(&body), 0);
        let inner = forbidden_test_inner(url, total_len);
        let mut source = RemoteAudioSource {
            inner: Arc::clone(&inner),
            pos: 0,
            access_mode: RemoteAccessMode::StandardSeekable,
            read_cancellation: None,
        };
        let mut bytes = [0u8; 4];

        let error = source
            .read_exact(&mut bytes)
            .expect_err("403 must fail the read");

        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        // 只允许打一发请求：403 不进折半重试循环
        assert_eq!(requests.lock().expect("requests lock").len(), 1);
        assert!(inner.remote_forbidden.load(Ordering::Acquire));
        // 置位后 replenish 是空操作：不再孵化注定 403 的后台预取
        replenish_prefetch_window(&inner, 0);
        thread::sleep(Duration::from_millis(150));
        assert_eq!(requests.lock().expect("requests lock").len(), 1);
    }

    /// 回归（YouTube seek 卡死入口）：虚拟 body 探测遇 403 必须让整个
    /// seek 立即失败（上层回滚/换直链），而不是吞掉错误继续配置、
    /// 让 demuxer 打开阶段对同一 range 每 ~600ms 无限刷 403
    #[test]
    fn virtual_body_probe_403_aborts_seek_configuration() {
        let total_len: u64 = 1024 * 1024;
        let body: Arc<Vec<u8>> = Arc::new(vec![0u8; total_len as usize]);
        // 只有文件头可读，body 一律 403（模拟只允许顺序窗口的直链）
        let (url, requests) = spawn_forbidden_range_server(Arc::clone(&body), 1);
        let inner = forbidden_test_inner(url, total_len);
        inner.header_end.store(64, Ordering::Release);
        *inner.seek_index.lock().expect("seek index lock") = Some(RemoteSeekIndex {
            duration_ms: 200_000,
            entries: vec![
                SeekIndexEntry {
                    time_ms: 0,
                    byte_offset: 64,
                },
                SeekIndexEntry {
                    time_ms: 100_000,
                    byte_offset: 500_000,
                },
            ],
        });
        let source = RemoteAudioSource {
            inner: Arc::clone(&inner),
            pos: 0,
            access_mode: RemoteAccessMode::StandardSeekable,
            read_cancellation: None,
        };

        let error = source
            .configure_virtual_body_for_time(120_000)
            .expect_err("probe 403 must abort the seek");

        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        // 探测只打一发请求，配置未生效（不会留下半配置的虚拟 body）
        assert_eq!(requests.lock().expect("requests lock").len(), 1);
        assert_eq!(inner.virtual_body_origin.load(Ordering::Acquire), 0);
        assert!(inner.remote_forbidden.load(Ordering::Acquire));
    }

    /// 手工构造 version=0 的 sidx box：timescale=48000，两段各 10s / 1MB
    fn sidx_atom_two_segments() -> Vec<u8> {
        let mut body: Vec<u8> = Vec::new();
        body.extend_from_slice(&[0, 0, 0, 0]); // version + flags
        body.extend_from_slice(&1u32.to_be_bytes()); // reference_ID
        body.extend_from_slice(&48_000u32.to_be_bytes()); // timescale
        body.extend_from_slice(&0u32.to_be_bytes()); // earliest_pts
        body.extend_from_slice(&0u32.to_be_bytes()); // first_offset
        body.extend_from_slice(&0u16.to_be_bytes()); // reserved
        body.extend_from_slice(&2u16.to_be_bytes()); // reference_count
        for _ in 0..2 {
            body.extend_from_slice(&0x0010_0000u32.to_be_bytes()); // 1MB, media type
            body.extend_from_slice(&480_000u32.to_be_bytes()); // 10s @48k
            body.extend_from_slice(&0u32.to_be_bytes()); // SAP
        }
        let mut atom = Vec::with_capacity(8 + body.len());
        atom.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
        atom.extend_from_slice(b"sidx");
        atom.extend_from_slice(&body);
        atom
    }

    /// 回归（YouTube seek 慢 16s）：前端对 YouTube 常传 duration_hint=0，
    /// 旧代码既要求 hint>0 才建索引、又用 .max(1) 把 time_ms 全 clamp 到 1ms，
    /// 导致分片流退化到 format.seek 顺序爬完全文件。
    /// sidx 自带 timescale，时长未知也必须能建出精确索引
    #[test]
    fn fragmented_seek_index_builds_without_duration_hint() {
        let total_len: u64 = 4 * 1024 * 1024;
        let inner = forbidden_test_inner("http://127.0.0.1:1/audio.m4a".into(), total_len);
        inner
            .cache
            .lock()
            .expect("cache lock")
            .push(CachedSegment {
                start: 0,
                data: sidx_atom_two_segments(),
            });

        assert!(
            super::build_seek_index_from_head(&inner, 0),
            "sidx index must build with duration_hint_ms=0",
        );

        let index = inner
            .seek_index
            .lock()
            .expect("seek index lock")
            .clone()
            .expect("index stored");
        // 三个锚点：0s / 10s / 20s，时间来自 sidx 自身 timescale，绝不能被 clamp
        assert_eq!(index.entries.len(), 3);
        assert_eq!(index.entries[1].time_ms, 10_000);
        assert_eq!(index.entries[2].time_ms, 20_000);
        assert_eq!(index.duration_ms, 20_000);
        // seek 到 15s 应落在 10s 段起点（首个 moof 后 1MB 处）
        let atom_len = sidx_atom_two_segments().len() as u64;
        assert_eq!(
            index.estimate_segment(15_000).0,
            atom_len + 1024 * 1024,
        );
    }

    /// 回归（Bilibili 2 小时 fMP4 seek 爬 12s）：FragmentedProgressive
    /// （.m4s 直链）此前不在建索引名单里，seek 退化为顺序爬 moof。
    /// 它的 sidx 同样在首包，必须与嗅探分片/LongForm 一致地建索引，
    /// 且升级前（Progressive）与升级后（Seekable）都要走虚拟 body 落点
    #[test]
    fn fragmented_progressive_builds_index_and_prefers_virtual_body() {
        // 建索引名单矩阵：分片直链与长内容必建；短内容仅嗅探出分片才建
        use super::{needs_head_seek_index, RemoteAccessMode as Mode};
        assert!(needs_head_seek_index(Mode::FragmentedProgressive, false));
        assert!(needs_head_seek_index(Mode::LongFormProgressive, false));
        assert!(needs_head_seek_index(Mode::StandardSeekable, true));
        assert!(!needs_head_seek_index(Mode::StandardSeekable, false));

        let total_len: u64 = 4 * 1024 * 1024;
        let inner = forbidden_test_inner("http://127.0.0.1:1/audio.m4s".into(), total_len);
        inner
            .cache
            .lock()
            .expect("cache lock")
            .push(CachedSegment {
                start: 0,
                data: sidx_atom_two_segments(),
            });
        assert!(super::build_seek_index_from_head(&inner, 0));
        inner.header_end.store(64, Ordering::Release);

        let source = RemoteAudioSource {
            inner: Arc::clone(&inner),
            pos: 0,
            access_mode: RemoteAccessMode::FragmentedProgressive,
            read_cancellation: None,
        };
        assert!(
            source.prefers_virtual_body_seek(),
            "升级前的 .m4s 直链就必须判定为虚拟 body seek，否则误走 format.seek"
        );
        let seekable = source.seekable_clone();
        assert!(seekable.prefers_virtual_body_seek());
    }

    /// splice 打开期的错误分类：解码类可跳；IO 里只有瞬时类可跳，
    /// 403（PermissionDenied）与取消（Interrupted）必须立即上抛
    #[test]
    fn splice_skip_bails_on_permanent_io_errors() {
        use symphonia::core::errors::Error as SymphoniaError;

        let permanent = SymphoniaError::IoError(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "remote range request forbidden: 403 Forbidden",
        ));
        let cancelled = SymphoniaError::IoError(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "remote read superseded",
        ));
        let transient = SymphoniaError::IoError(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "prefetch edge",
        ));
        let decode = SymphoniaError::DecodeError("moov stco fake sample");

        assert!(!splice_skip_retryable(&permanent));
        assert!(!splice_skip_retryable(&cancelled));
        assert!(splice_skip_retryable(&transient));
        assert!(splice_skip_retryable(&decode));
    }

}
