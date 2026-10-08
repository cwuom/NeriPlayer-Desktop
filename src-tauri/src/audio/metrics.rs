//! 输出回调与起播耗时的运行时指标
//!
//! 设备回调只做原子操作（计数、`fetch_max`、取单调时钟），汇总、日志和对外查询
//! 都在控制线程或命令线程完成：回调里不加锁、不分配、不写日志。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

const RECENT_START_CAP: usize = 32;
/// 首帧迟迟不来（例如起播后立刻被暂停）时放弃等待，探针不能无限堆积
const PROBE_TIMEOUT_NS: u64 = 60_000_000_000;
/// 播放中按这个周期汇总一次回调指标；1 小时长稳测试靠它统计欠载
const STATS_LOG_INTERVAL: Duration = Duration::from_secs(60);
/// 有探针在等首帧时控制线程的最长休眠，让耗时结果及时可查
pub const PROBE_POLL_INTERVAL: Duration = Duration::from_millis(50);

static MONOTONIC_EPOCH: OnceLock<Instant> = OnceLock::new();

/// 进程内单调时钟（纳秒）
///
/// 原点在第一次调用时确定。引擎构造时先调用一次，之后回调里只剩一次原子读和 `Instant::now`。
pub fn monotonic_ns() -> u64 {
    let epoch = MONOTONIC_EPOCH.get_or_init(Instant::now);
    u64::try_from(epoch.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn ns_to_ms(nanos: u64) -> f64 {
    nanos as f64 / 1_000_000.0
}

fn unix_now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs_f64() * 1_000.0)
        .unwrap_or(0.0)
}

/// 输出回调的累计指标，进程级、跨会话累计
pub struct OutputMetrics {
    callbacks: AtomicU64,
    rendered_frames: AtomicU64,
    underruns: AtomicU64,
    underrun_frames: AtomicU64,
    max_callback_ns: AtomicU64,
    normalization_gain_mb: AtomicI64,
    limited_frames: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputMetricsSnapshot {
    pub callbacks: u64,
    pub rendered_frames: u64,
    /// 播放中 ring 被取空的次数：每次都是一处听得见的断音
    pub underruns: u64,
    pub underrun_frames: u64,
    /// 自上次周期汇总以来单次回调的最长耗时
    pub max_callback_us: u64,
    /// 最近一次回调实际施加的音量均衡增益（毫贝，关着时为 0）
    pub normalization_gain_mb: i64,
    /// 被限幅器压过的累计帧数
    pub limited_frames: u64,
}

impl Default for OutputMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputMetrics {
    pub const fn new() -> Self {
        Self {
            callbacks: AtomicU64::new(0),
            rendered_frames: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            underrun_frames: AtomicU64::new(0),
            max_callback_ns: AtomicU64::new(0),
            normalization_gain_mb: AtomicI64::new(0),
            limited_frames: AtomicU64::new(0),
        }
    }

    /// 回调处理完音效后调用：累加限幅帧数；给了增益就更新当前均衡增益的读数
    pub fn record_effects(&self, normalization_gain_mb: Option<i64>, limited_frames: u64) {
        if let Some(gain) = normalization_gain_mb {
            self.normalization_gain_mb.store(gain, Ordering::Relaxed);
        }
        if limited_frames > 0 {
            self.limited_frames.fetch_add(limited_frames, Ordering::Relaxed);
        }
    }

    /// 每次回调末尾调用一次；`underrun` 只在播放中途取空 ring 时为真
    pub fn record_callback(
        &self,
        rendered_frames: usize,
        underrun_frames: usize,
        underrun: bool,
        elapsed_ns: u64,
    ) {
        self.callbacks.fetch_add(1, Ordering::Relaxed);
        self.rendered_frames
            .fetch_add(rendered_frames as u64, Ordering::Relaxed);
        if underrun {
            self.underruns.fetch_add(1, Ordering::Relaxed);
            self.underrun_frames
                .fetch_add(underrun_frames as u64, Ordering::Relaxed);
        }
        self.max_callback_ns.fetch_max(elapsed_ns, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> OutputMetricsSnapshot {
        OutputMetricsSnapshot {
            callbacks: self.callbacks.load(Ordering::Relaxed),
            rendered_frames: self.rendered_frames.load(Ordering::Relaxed),
            underruns: self.underruns.load(Ordering::Relaxed),
            underrun_frames: self.underrun_frames.load(Ordering::Relaxed),
            max_callback_us: self.max_callback_ns.load(Ordering::Relaxed) / 1_000,
            normalization_gain_mb: self.normalization_gain_mb.load(Ordering::Relaxed),
            limited_frames: self.limited_frames.load(Ordering::Relaxed),
        }
    }

    /// 取出并清零最长回调耗时，按周期统计峰值
    fn take_max_callback_us(&self) -> u64 {
        self.max_callback_ns.swap(0, Ordering::Relaxed) / 1_000
    }
}

pub static OUTPUT_METRICS: OutputMetrics = OutputMetrics::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StartKind {
    Play,
    Seek,
}

impl StartKind {
    fn label(self) -> &'static str {
        match self {
            Self::Play => "play",
            Self::Seek => "seek",
        }
    }
}

/// 一次起播或 seek 从命令发出到首帧交给设备的耗时
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartTiming {
    pub kind: StartKind,
    pub source: &'static str,
    pub generation: u64,
    pub position_ms: u64,
    /// `request_*` 把命令交给音频线程 → 首帧交给设备
    pub since_command_ms: f64,
    /// 其中命令在控制线程队列里等待的时长
    pub queued_ms: f64,
    /// 首帧交给设备的墙钟时刻（Unix 毫秒），可与前端日志或测量脚本的 `Date.now()` 对照
    pub first_frame_unix_ms: f64,
}

static RECENT_STARTS: Mutex<VecDeque<StartTiming>> = Mutex::new(VecDeque::new());

fn remember_start(timing: StartTiming) {
    let mut recent = RECENT_STARTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if recent.len() >= RECENT_START_CAP {
        recent.pop_front();
    }
    recent.push_back(timing);
}

/// 最近的起播/seek 耗时，新的在前
pub fn recent_starts() -> Vec<StartTiming> {
    let recent = RECENT_STARTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    recent.iter().rev().cloned().collect()
}

/// 命令发出与控制线程开始处理它时的单调时钟读数
pub struct CommandStamps {
    pub issued_ns: u64,
    pub received_ns: u64,
}

/// 等首帧的探针：控制线程持有，回调只往 `first_frame_ns` 写一次
pub struct FirstFrameProbe {
    kind: StartKind,
    source: &'static str,
    generation: u64,
    position_ms: u64,
    stamps: CommandStamps,
    first_frame_ns: Arc<AtomicU64>,
}

enum ProbeState {
    Pending,
    Done(StartTiming),
    Expired,
}

impl FirstFrameProbe {
    pub fn new(
        kind: StartKind,
        source: &'static str,
        generation: u64,
        position_ms: u64,
        stamps: CommandStamps,
        first_frame_ns: Arc<AtomicU64>,
    ) -> Self {
        Self {
            kind,
            source,
            generation,
            position_ms,
            stamps,
            first_frame_ns,
        }
    }

    fn poll(&self, now_ns: u64, now_unix_ms: f64) -> ProbeState {
        let first_frame_ns = self.first_frame_ns.load(Ordering::Acquire);
        if first_frame_ns != 0 {
            return ProbeState::Done(StartTiming {
                kind: self.kind,
                source: self.source,
                generation: self.generation,
                position_ms: self.position_ms,
                since_command_ms: ns_to_ms(first_frame_ns.saturating_sub(self.stamps.issued_ns)),
                queued_ms: ns_to_ms(self.stamps.received_ns.saturating_sub(self.stamps.issued_ns)),
                first_frame_unix_ms: now_unix_ms - ns_to_ms(now_ns.saturating_sub(first_frame_ns)),
            });
        }
        if now_ns.saturating_sub(self.stamps.issued_ns) > PROBE_TIMEOUT_NS {
            ProbeState::Expired
        } else {
            ProbeState::Pending
        }
    }
}

fn log_start_timing(timing: &StartTiming) {
    log::info!(
        target: "audio-timing",
        "first-frame kind={} source={} generation={} position_ms={} since_command_ms={:.1} queued_ms={:.1} first_frame_unix_ms={:.0}",
        timing.kind.label(),
        timing.source,
        timing.generation,
        timing.position_ms,
        timing.since_command_ms,
        timing.queued_ms,
        timing.first_frame_unix_ms,
    );
}

/// 控制线程上的指标汇报：结算首帧探针、发现欠载、播放中按周期写汇总
pub struct MetricsReporter {
    probes: Vec<FirstFrameProbe>,
    last_underruns: u64,
    window: OutputMetricsSnapshot,
    window_started: Instant,
}

impl Default for MetricsReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsReporter {
    pub fn new() -> Self {
        let snapshot = OUTPUT_METRICS.snapshot();
        Self {
            probes: Vec::new(),
            last_underruns: snapshot.underruns,
            window: snapshot,
            window_started: Instant::now(),
        }
    }

    pub fn track(&mut self, probe: FirstFrameProbe) {
        self.probes.push(probe);
    }

    pub fn has_pending_probes(&self) -> bool {
        !self.probes.is_empty()
    }

    /// `active_generation` 是当前会话的代际；只有在播（未暂停）时才累计周期窗口
    pub fn poll(&mut self, active_generation: Option<u64>, playing: bool) {
        self.settle_probes(monotonic_ns(), active_generation);
        let snapshot = OUTPUT_METRICS.snapshot();
        if snapshot.underruns > self.last_underruns {
            log::warn!(
                target: "audio-timing",
                "output underrun count=+{} total={} silent_frames_total={}",
                snapshot.underruns - self.last_underruns,
                snapshot.underruns,
                snapshot.underrun_frames,
            );
            self.last_underruns = snapshot.underruns;
        }
        if !playing {
            self.window = snapshot;
            self.window_started = Instant::now();
            return;
        }
        let elapsed = self.window_started.elapsed();
        if elapsed < STATS_LOG_INTERVAL {
            return;
        }
        log::info!(
            target: "audio-timing",
            "output stats window_s={} callbacks=+{} frames=+{} underruns=+{} underruns_total={} max_callback_us={} normalization_gain_mb={} limited_frames=+{}",
            elapsed.as_secs(),
            snapshot.callbacks - self.window.callbacks,
            snapshot.rendered_frames - self.window.rendered_frames,
            snapshot.underruns - self.window.underruns,
            snapshot.underruns,
            OUTPUT_METRICS.take_max_callback_us(),
            snapshot.normalization_gain_mb,
            snapshot.limited_frames - self.window.limited_frames,
        );
        self.window = snapshot;
        self.window_started = Instant::now();
    }

    fn settle_probes(&mut self, now_ns: u64, active_generation: Option<u64>) {
        let now_unix_ms = unix_now_ms();
        self.probes.retain(|probe| match probe.poll(now_ns, now_unix_ms) {
            ProbeState::Done(timing) => {
                log_start_timing(&timing);
                remember_start(timing);
                false
            }
            ProbeState::Expired => {
                log::info!(
                    target: "audio-timing",
                    "first-frame probe expired kind={} generation={}",
                    probe.kind.label(),
                    probe.generation,
                );
                false
            }
            ProbeState::Pending if active_generation != Some(probe.generation) => {
                log::info!(
                    target: "audio-timing",
                    "first-frame probe dropped kind={} generation={} reason=superseded",
                    probe.kind.label(),
                    probe.generation,
                );
                false
            }
            ProbeState::Pending => true,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CommandStamps, FirstFrameProbe, MetricsReporter, OutputMetrics, ProbeState, StartKind,
        PROBE_TIMEOUT_NS,
    };
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    #[test]
    fn callback_metrics_count_only_reported_underruns() {
        let metrics = OutputMetrics::new();
        metrics.record_callback(480, 0, false, 120_000);
        metrics.record_callback(200, 280, true, 90_000);
        metrics.record_callback(0, 480, false, 30_000);

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.callbacks, 3);
        assert_eq!(snapshot.rendered_frames, 680);
        assert_eq!(snapshot.underruns, 1);
        assert_eq!(snapshot.underrun_frames, 280, "未报告为欠载的静音帧不计入");
        assert_eq!(snapshot.max_callback_us, 120);
        assert_eq!(metrics.take_max_callback_us(), 120);
        assert_eq!(metrics.snapshot().max_callback_us, 0, "周期峰值取出后清零");
    }

    #[test]
    fn effects_metrics_keep_the_latest_gain_and_accumulate_limited_frames() {
        let metrics = OutputMetrics::new();
        metrics.record_effects(Some(-602), 0);
        metrics.record_effects(Some(300), 128);
        metrics.record_effects(Some(250), 0);
        // 交叉淡化里淡出的那个会话只累加限幅帧数，不改读数
        metrics.record_effects(None, 64);

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.normalization_gain_mb, 250);
        assert_eq!(snapshot.limited_frames, 192);
    }

    fn stamps(issued_ns: u64, received_ns: u64) -> CommandStamps {
        CommandStamps { issued_ns, received_ns }
    }

    #[test]
    fn probe_splits_queueing_from_the_total_once_the_first_frame_lands() {
        let first_frame = Arc::new(AtomicU64::new(0));
        let probe = FirstFrameProbe::new(
            StartKind::Play, "file", 4, 0, stamps(10_000_000, 12_500_000), Arc::clone(&first_frame),
        );
        assert!(matches!(probe.poll(20_000_000, 1_000.0), ProbeState::Pending));

        first_frame.store(73_500_000, Ordering::Release);
        let ProbeState::Done(timing) = probe.poll(83_500_000, 5_000.0) else {
            panic!("probe should settle once the first frame is recorded");
        };
        assert!((timing.since_command_ms - 63.5).abs() < 1e-9);
        assert!((timing.queued_ms - 2.5).abs() < 1e-9);
        assert!((timing.first_frame_unix_ms - 4_990.0).abs() < 1e-9, "墙钟按结算时的偏移回推");
    }

    #[test]
    fn probe_without_a_first_frame_expires_instead_of_waiting_forever() {
        let probe = FirstFrameProbe::new(
            StartKind::Seek, "remote", 9, 42_000, stamps(1, 1), Arc::new(AtomicU64::new(0)),
        );
        assert!(matches!(probe.poll(PROBE_TIMEOUT_NS, 0.0), ProbeState::Pending));
        assert!(matches!(probe.poll(PROBE_TIMEOUT_NS + 2, 0.0), ProbeState::Expired));
    }

    #[test]
    fn reporter_drops_pending_probes_of_a_replaced_session_but_keeps_settled_ones() {
        let mut reporter = MetricsReporter::new();
        let now = super::monotonic_ns();
        let settled = Arc::new(AtomicU64::new(0));
        reporter.track(FirstFrameProbe::new(
            StartKind::Play, "bytes", 5, 0, stamps(now, now), Arc::clone(&settled),
        ));
        reporter.track(FirstFrameProbe::new(
            StartKind::Play, "bytes", 6, 0, stamps(now, now), Arc::new(AtomicU64::new(0)),
        ));
        settled.store(super::monotonic_ns().max(1), Ordering::Release);

        reporter.settle_probes(super::monotonic_ns(), Some(7));
        assert!(!reporter.has_pending_probes(), "已出首帧的结算，被替换会话的丢弃");
    }
}
