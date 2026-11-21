use reqwest::header::{CONTENT_RANGE, RANGE, REFERER, USER_AGENT};
use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use symphonia::core::audio::{AudioBufferRef, SampleBuffer, SignalSpec};
use symphonia::core::codecs::{self, Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, SeekedTo};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::{self, Time, TimeBase};
use symphonia::default::{get_codecs, get_probe};

use crate::pcm::{PcmSeekError, PcmSource};
use neri_common::error::{AppError, AppResult};

// 按职责拆分的子模块，通过 `use super::*` 共享本模块的类型与常量
mod cache;
mod decoder;
mod fetch;
mod prefetch;
mod probe;
mod seek_index;
mod source;
pub use cache::prune_remote_audio_cache;
use cache::*;
pub use decoder::{SourceAudioInfo, SymphoniaAudioDecoder};
use fetch::*;
use prefetch::*;
pub use probe::prewarm;
use probe::*;
use seek_index::*;

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
// 首包探测含建连、TLS 与首段下载；经代理时单是 TLS 握手就要 2-3 秒，6 秒常常假超时
const REMOTE_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
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
        neri_platform::api::youtube::playback::stream_user_agent_for_url(url)
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
    /// 条目时间是 sidx 给的分片真实起点；线性估算只是大致落点
    exact_times: bool,
}

#[derive(Clone, Copy, Debug)]
struct SeekIndexEntry {
    time_ms: u64,
    byte_offset: u64,
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
            Err(io::Error::other("playback request superseded"))
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

#[cfg(test)]
mod tests;
