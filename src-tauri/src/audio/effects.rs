//! 音效链：音量均衡、均衡器、响度增益与限幅，都在输出回调里按块处理
//!
//! 参数经原子量下发，回调每块比对一次版本号，改动从下一块（约 10 ms）开始平滑
//! 生效，不用停下会话重新解码。音量均衡的响度统计放在解码线程里做，比播放领先
//! ring 里缓冲的那一段；统计按曲目保存在 [`TrackLoudness`]，同一首歌 seek、换输出
//! 设备重建会话时沿用，切歌才从头开始。
//!
//! 音量均衡的代数与常量对齐 Android `VolumeNormalizer.kt`：整轨积分 RMS 定目标
//! 增益（-18 dBFS，-12～+6 dB，且不把已观测到的峰值推过 -2 dBFS），按块平滑
//! （下调 0.25 s、上调 4 s），再由 5 ms 起控、100 ms 释放的限幅包络保证输出峰值
//! 不过线。均衡器沿用 Android 的余量处理：整条曲线下移到最高频段为 0 dB，
//! 拉高的频段不会削波。声道平衡同 Android `StereoBalanceGains`：只衰减偏离的那一侧。

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

pub const EQ_BANDS: usize = 5;
/// 各频段中心频率，与 Android 平台均衡器的默认 5 段一致
const EQ_FREQUENCIES_HZ: [f64; EQ_BANDS] = [60.0, 230.0, 910.0, 3_600.0, 14_000.0];
const EQ_Q: f64 = 1.0;
const MAX_BAND_LEVEL_MB: i32 = 1_500;
const MAX_LOUDNESS_GAIN_MB: i32 = 1_500;
const MAX_BALANCE_CENTI: i32 = 100;

/// 目标 RMS：-18 dBFS，与 ReplayGain 2.0 的目标响度一致
const TARGET_RMS: f64 = 0.125_892_54;
/// 低于 -55 dBFS 的块不计入统计：前奏留白、淡出不拉偏整首的响度
const SILENCE_GATE_RMS: f64 = 0.001_778_28;
/// 增益范围 -12 dB ～ +6 dB
const MIN_GAIN: f64 = 0.251_188_64;
const MAX_GAIN: f64 = 1.995_262_3;
/// -2 dBFS：目标增益不把已观测的峰值推过这条线，限幅器也以它为上限
const PEAK_CEILING: f64 = 0.794_328_2;
const GAIN_REDUCTION_SECONDS: f64 = 0.25;
const GAIN_INCREASE_SECONDS: f64 = 4.0;
/// 目标来自整首（抽样估计或完整扫描）后不再随播放变化，升降都按这个时间常数走到位
///
/// 慢升是为了让随统计变化的目标不至于来回「呼吸」；目标固定之后没有这个问题，
/// 还按 4 s 慢慢抬的话，整首比开头估计更安静的曲子前十几秒都会偏小声。
const FINAL_GAIN_SECONDS: f64 = 0.5;
const LIMITER_ATTACK_SECONDS: f64 = 0.005;
const LIMITER_RELEASE_SECONDS: f64 = 0.1;
/// 解码线程每攒够这么多个样本并入一次统计（48 kHz 立体声约 43 ms）
const ANALYSIS_BLOCK_SAMPLES: usize = 4_096;
/// 新曲目开着音量均衡时至少先分析这么长再出声，第一个可闻样本就是正确响度
pub const NORMALIZE_WARMUP: Duration = Duration::from_millis(200);
/// 开关音量均衡、调响度增益时的过渡时长，增益不会台阶式跳变
const PARAMETER_RAMP_SECONDS: f64 = 0.03;
/// 均衡器换参数时新旧两组滤波器的交叉淡化时长
const EQ_CROSSFADE_SECONDS: f64 = 0.02;
/// 递归滤波器在长段静音里会衰减进次正规数，运算慢上百倍；低于此值直接归零
const DENORMAL_FLOOR: f64 = 1e-30;

/// 用户的音效设置，单位与前端一致（毫贝）
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EffectsSettings {
    /// 响度增益，0～1500 mB（0～+15 dB）
    pub loudness_gain_mb: i32,
    pub eq_enabled: bool,
    /// 各频段增益，-1500～1500 mB
    pub eq_band_levels_mb: [i32; EQ_BANDS],
    /// 音量均衡：把不同曲目的响度拉到统一目标，避免切歌忽大忽小
    pub normalize_volume: bool,
    /// 声道平衡，-100（只剩左声道）～100（只剩右声道），步进与 Android 一样是 0.01
    pub balance_centi: i32,
}

impl EffectsSettings {
    /// 「重置音效」之后的设置：清掉响度增益和均衡器
    ///
    /// 音量均衡和声道平衡是设置页里的独立设置，前端重置音效时不会重新下发它们，
    /// 这里必须保持原样，否则设置页显示的值和实际听到的对不上。
    pub fn reset_panel(self) -> Self {
        Self {
            normalize_volume: self.normalize_volume,
            balance_centi: self.balance_centi,
            ..Self::default()
        }
    }

    fn clamped(mut self) -> Self {
        self.loudness_gain_mb = self.loudness_gain_mb.clamp(0, MAX_LOUDNESS_GAIN_MB);
        self.balance_centi = self.balance_centi.clamp(-MAX_BALANCE_CENTI, MAX_BALANCE_CENTI);
        for level in &mut self.eq_band_levels_mb {
            *level = (*level).clamp(-MAX_BAND_LEVEL_MB, MAX_BAND_LEVEL_MB);
        }
        self
    }

    /// 实际交给滤波器的各频段增益（dB）
    ///
    /// 和 Android 一样整体下移到最高频段为 0 dB：滤波器只衰减不提升，
    /// 拉高的频段不会把峰值推过满幅。
    fn equalizer_levels_db(&self) -> [f64; EQ_BANDS] {
        if !self.eq_enabled {
            return [0.0; EQ_BANDS];
        }
        let levels = self.eq_band_levels_mb.map(|level| f64::from(level) / 100.0);
        let headroom = levels.iter().copied().fold(0.0, f64::max);
        levels.map(|level| level - headroom)
    }
}

/// 控制端写、各个输出回调读的音效参数
///
/// 字段都是原子量，写完全部字段后递增版本号；回调看到版本号变了才重新读取。
/// 读到一半赶上新的写入也无妨：那次写入随后会再递增版本号，下一块就会重读到最终值。
pub struct EffectsControl {
    writer: Mutex<()>,
    version: AtomicU64,
    loudness_gain_mb: AtomicI32,
    eq_enabled: AtomicBool,
    eq_band_levels_mb: [AtomicI32; EQ_BANDS],
    normalize_volume: AtomicBool,
    balance_centi: AtomicI32,
}

impl EffectsControl {
    pub fn new_shared() -> Arc<Self> {
        Arc::new(Self {
            writer: Mutex::new(()),
            version: AtomicU64::new(0),
            loudness_gain_mb: AtomicI32::new(0),
            eq_enabled: AtomicBool::new(false),
            eq_band_levels_mb: std::array::from_fn(|_| AtomicI32::new(0)),
            normalize_volume: AtomicBool::new(false),
            balance_centi: AtomicI32::new(0),
        })
    }

    pub fn snapshot(&self) -> EffectsSettings {
        EffectsSettings {
            loudness_gain_mb: self.loudness_gain_mb.load(Ordering::Relaxed),
            eq_enabled: self.eq_enabled.load(Ordering::Relaxed),
            eq_band_levels_mb: std::array::from_fn(|band| {
                self.eq_band_levels_mb[band].load(Ordering::Relaxed)
            }),
            normalize_volume: self.normalize_volume.load(Ordering::Relaxed),
            balance_centi: self.balance_centi.load(Ordering::Relaxed),
        }
    }

    /// 改设置并通知所有回调；数值越界时夹到允许范围
    pub fn update(&self, change: impl FnOnce(&mut EffectsSettings)) {
        // 只串行化写端，回调从不碰这把锁。上一个写端 panic 在改值之前，
        // 原子量里仍是完整的旧设置，接着写就是对的
        let _writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let mut settings = self.snapshot();
        change(&mut settings);
        let settings = settings.clamped();
        self.loudness_gain_mb
            .store(settings.loudness_gain_mb, Ordering::Relaxed);
        self.eq_enabled.store(settings.eq_enabled, Ordering::Relaxed);
        for (slot, level) in self.eq_band_levels_mb.iter().zip(settings.eq_band_levels_mb) {
            slot.store(level, Ordering::Relaxed);
        }
        self.normalize_volume
            .store(settings.normalize_volume, Ordering::Relaxed);
        self.balance_centi
            .store(settings.balance_centi, Ordering::Relaxed);
        self.version.fetch_add(1, Ordering::Release);
    }

    fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }
}

/// 正在攒的一块样本
#[derive(Clone, Copy, Debug, Default)]
struct Block {
    sum_squares: f64,
    samples: usize,
    peak: f64,
}

impl Block {
    /// 累加一个样本；攒满一块时返回 true
    ///
    /// 非有限值不计入：统计是整首共享的，一个 NaN 会让这首歌之后的目标增益全变成 NaN
    fn push(&mut self, sample: f32) -> bool {
        if !sample.is_finite() {
            return false;
        }
        let value = f64::from(sample);
        self.sum_squares += value * value;
        self.peak = self.peak.max(value.abs());
        self.samples += 1;
        self.samples >= ANALYSIS_BLOCK_SAMPLES
    }

    fn take(&mut self) -> Self {
        std::mem::take(self)
    }
}

/// 按块累计的整轨响度统计
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LoudnessStats {
    sum_squares: f64,
    samples: u64,
    peak: f64,
}

impl LoudnessStats {
    /// 并入一块；低于静音门的块不计入（前奏留白、淡出不拉偏整首的响度）
    fn add(&mut self, block: &Block) -> bool {
        if block.samples == 0
            || (block.sum_squares / block.samples as f64).sqrt() < SILENCE_GATE_RMS
        {
            return false;
        }
        self.sum_squares += block.sum_squares;
        self.samples += block.samples as u64;
        self.peak = self.peak.max(block.peak);
        true
    }

    /// 已计入统计的样本数（各声道合计）
    pub fn samples(&self) -> u64 {
        self.samples
    }

    pub fn rms_dbfs(&self) -> f64 {
        20.0 * (self.sum_squares / self.samples.max(1) as f64).sqrt().max(1e-9).log10()
    }

    pub fn peak_dbfs(&self) -> f64 {
        20.0 * self.peak.max(1e-9).log10()
    }

    /// 目标增益：积分 RMS 拉向 -18 dBFS，且不把峰值推过 -2 dBFS；没有非静音样本时为 None
    pub fn target_gain(&self) -> Option<f64> {
        if self.samples == 0 {
            return None;
        }
        let integrated_rms = (self.sum_squares / self.samples as f64).sqrt();
        let rms_gain = (TARGET_RMS / integrated_rms).clamp(MIN_GAIN, MAX_GAIN);
        let peak_gain = if self.peak > 0.0 {
            (PEAK_CEILING / self.peak).min(MAX_GAIN)
        } else {
            MAX_GAIN
        };
        Some(rms_gain.min(peak_gain).clamp(MIN_GAIN, MAX_GAIN))
    }
}

/// 统计来源：播放时的增量统计 -> 整首抽样估计 -> 整首扫描
const SOURCE_LIVE: u8 = 0;
const SOURCE_ESTIMATE: u8 = 1;
const SOURCE_COMPLETE: u8 = 2;

/// 一首歌的响度统计，以及回调最近施加的均衡增益
///
/// 同一首歌的各个会话（seek、换设备重建）共用一份：统计接着累积，回调从上次的
/// 增益接着走，音量不会因为重建跳一下。切歌时换新的一份。
///
/// 播放时的统计只领先 ring 里的几秒，开头几十秒里目标会随统计收敛而漂移；扫描线程
/// 先用 [`TrackLoudness::estimate`] 给出整首的抽样估计，扫完后由
/// [`TrackLoudness::complete`] 换成整首的统计，目标从此固定。
pub struct TrackLoudness {
    integrated: Mutex<LoudnessStats>,
    /// 当前统计的来源；有了整首的估计或扫描结果后，播放时的统计不再并入
    source: AtomicU8,
    /// 当前统计对应的目标增益（f64 位）；0 表示还没有非静音的统计
    target_gain: AtomicU64,
    analyzed_samples: AtomicU64,
    /// 回调最近施加的均衡增益（f64 位），新会话从这里接着走
    applied_gain: AtomicU64,
    /// 回调是否已经按统计落定过增益；落定之后的新会话不必再预热
    seeded: AtomicBool,
    scan_started: AtomicBool,
    /// 扫描线程还在算整首的抽样估计，起播可以稍等它
    estimate_pending: AtomicBool,
    /// 整首扫描的取消标志：曲目被放弃（最后一个会话释放）时置位
    scan_cancel: Arc<AtomicBool>,
}

impl TrackLoudness {
    pub fn new_shared() -> Arc<Self> {
        Arc::new(Self {
            integrated: Mutex::new(LoudnessStats::default()),
            source: AtomicU8::new(SOURCE_LIVE),
            target_gain: AtomicU64::new(0),
            analyzed_samples: AtomicU64::new(0),
            applied_gain: AtomicU64::new(1.0f64.to_bits()),
            seeded: AtomicBool::new(false),
            scan_started: AtomicBool::new(false),
            estimate_pending: AtomicBool::new(false),
            scan_cancel: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn is_seeded(&self) -> bool {
        self.seeded.load(Ordering::Acquire)
    }

    pub fn is_complete(&self) -> bool {
        self.source.load(Ordering::Acquire) == SOURCE_COMPLETE
    }

    /// 目标来自整首（抽样估计或完整扫描），不再随播放进度变化
    pub fn covers_whole_track(&self) -> bool {
        self.source.load(Ordering::Acquire) != SOURCE_LIVE
    }

    /// 已计入统计的样本数（各声道合计）
    pub fn analyzed_samples(&self) -> u64 {
        self.analyzed_samples.load(Ordering::Acquire)
    }

    /// 每首歌只扫描一次：第一次调用返回 true
    pub fn begin_scan(&self) -> bool {
        !self.scan_started.swap(true, Ordering::AcqRel)
    }

    pub fn set_estimate_pending(&self, pending: bool) {
        self.estimate_pending.store(pending, Ordering::Release);
    }

    pub fn estimate_pending(&self) -> bool {
        self.estimate_pending.load(Ordering::Acquire)
    }

    /// 换成整首的抽样估计；完整扫描的结果已经到了就不再覆盖
    ///
    /// 抽样全落在静音里时估计是空的：换上它会停掉播放时的统计，整首都没有目标增益，
    /// 这时保留播放时的统计等完整扫描。返回是否采用了估计。
    pub fn estimate(&self, stats: LoudnessStats) -> bool {
        if stats.samples == 0 {
            return false;
        }
        let mut integrated = self.lock_integrated();
        if self.is_complete() {
            return false;
        }
        *integrated = stats;
        self.source.store(SOURCE_ESTIMATE, Ordering::Release);
        self.publish(&integrated);
        true
    }

    /// 换成整首扫描的统计；之后目标增益固定
    pub fn complete(&self, stats: LoudnessStats) {
        let mut integrated = self.lock_integrated();
        *integrated = stats;
        self.source.store(SOURCE_COMPLETE, Ordering::Release);
        self.publish(&integrated);
    }

    pub fn scan_cancel(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.scan_cancel)
    }

    fn fold(&self, block: &Block) {
        if self.covers_whole_track() {
            return;
        }
        let mut integrated = self.lock_integrated();
        if !self.covers_whole_track() && integrated.add(block) {
            self.publish(&integrated);
        }
    }

    fn publish(&self, stats: &LoudnessStats) {
        if let Some(target) = stats.target_gain() {
            self.analyzed_samples.store(stats.samples, Ordering::Release);
            self.target_gain.store(target.to_bits(), Ordering::Release);
        }
    }

    /// 只有解码线程和扫描线程会拿这把锁（seek 时新旧解码线程可能短暂重叠），回调不碰。
    /// 里面都是标量，持锁方 panic 也不会留下半截状态
    fn lock_integrated(&self) -> std::sync::MutexGuard<'_, LoudnessStats> {
        self.integrated.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn target_gain(&self) -> Option<f64> {
        let bits = self.target_gain.load(Ordering::Acquire);
        (bits != 0).then(|| f64::from_bits(bits))
    }

    fn applied_gain(&self) -> f64 {
        f64::from_bits(self.applied_gain.load(Ordering::Acquire))
    }
}

impl Drop for TrackLoudness {
    fn drop(&mut self) {
        // 曲目已被放弃：扫描线程不必再读下去（边下边播时它还占着下载）
        self.scan_cancel.store(true, Ordering::Release);
    }
}

/// 解码线程逐帧喂入，攒够一块就并入整首歌的统计
pub struct LoudnessMeter {
    track: Arc<TrackLoudness>,
    block: Block,
}

impl LoudnessMeter {
    pub fn new(track: Arc<TrackLoudness>) -> Self {
        Self { track, block: Block::default() }
    }

    pub fn observe(&mut self, samples: &[f32]) {
        for &sample in samples {
            if self.block.push(sample) {
                self.track.fold(&self.block.take());
            }
        }
    }

    /// 把不足一块的残余也并入统计（曲尾）
    pub fn flush(&mut self) {
        self.track.fold(&self.block.take());
    }
}

/// 整首扫描的累计器，分块与静音门和播放时的统计完全一致
#[derive(Default)]
pub struct LoudnessScanner {
    stats: LoudnessStats,
    block: Block,
}

impl LoudnessScanner {
    pub fn observe(&mut self, samples: &[f32]) {
        for &sample in samples {
            if self.block.push(sample) {
                self.stats.add(&self.block.take());
            }
        }
    }

    pub fn finish(mut self) -> LoudnessStats {
        let tail = self.block.take();
        self.stats.add(&tail);
        self.stats
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Biquad {
    const IDENTITY: Self = Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };

    /// RBJ Audio EQ Cookbook 的峰值滤波器
    fn peaking(frequency: f64, gain_db: f64, sample_rate: f64) -> Self {
        // 中心频率贴近奈奎斯特频率时滤波器没有意义，按直通处理
        if gain_db.abs() < 0.01 || frequency >= sample_rate * 0.45 {
            return Self::IDENTITY;
        }
        let amplitude = 10.0_f64.powf(gain_db / 40.0);
        let omega = std::f64::consts::TAU * frequency / sample_rate;
        let alpha = omega.sin() / (2.0 * EQ_Q);
        let cos = omega.cos();
        let a0 = 1.0 + alpha / amplitude;
        Self {
            b0: (1.0 + alpha * amplitude) / a0,
            b1: -2.0 * cos / a0,
            b2: (1.0 - alpha * amplitude) / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha / amplitude) / a0,
        }
    }
}

/// 串联的峰值滤波器，每个声道一套延迟线（直接 I 型，换系数时比 II 型稳）
struct FilterBank {
    filters: [Biquad; EQ_BANDS],
    /// 每声道每频段的 [x1, x2, y1, y2]
    history: Vec<[f64; 4]>,
}

impl FilterBank {
    fn new(channels: usize, filters: [Biquad; EQ_BANDS]) -> Self {
        Self {
            filters,
            history: vec![[0.0; 4]; channels * EQ_BANDS],
        }
    }

    fn is_identity(&self) -> bool {
        self.filters.iter().all(|filter| *filter == Biquad::IDENTITY)
    }

    fn copy_from(&mut self, other: &Self) {
        self.filters = other.filters;
        self.history.copy_from_slice(&other.history);
    }

    fn process(&mut self, sample: f64, channel: usize) -> f64 {
        let history = &mut self.history[channel * EQ_BANDS..(channel + 1) * EQ_BANDS];
        let mut value = sample;
        for (filter, state) in self.filters.iter().zip(history) {
            let [x1, x2, y1, y2] = *state;
            let mut output = filter.b0 * value + filter.b1 * x1 + filter.b2 * x2
                - filter.a1 * y1
                - filter.a2 * y2;
            if output.abs() < DENORMAL_FLOOR {
                output = 0.0;
            }
            *state = [value, x1, output, y1];
            value = output;
        }
        value
    }
}

fn filters_for(levels_db: [f64; EQ_BANDS], sample_rate: f64) -> [Biquad; EQ_BANDS] {
    std::array::from_fn(|band| {
        Biquad::peaking(EQ_FREQUENCIES_HZ[band], levels_db[band], sample_rate)
    })
}

/// 5 段均衡器：换参数时新旧两组滤波器交叉淡化，换预设、拖滑条都不咔哒
struct Equalizer {
    active: FilterBank,
    previous: FilterBank,
    levels_db: [f64; EQ_BANDS],
    sample_rate: f64,
    fade_frames: usize,
    fade_remaining: usize,
}

impl Equalizer {
    fn new(channels: usize, sample_rate: f64, levels_db: [f64; EQ_BANDS]) -> Self {
        let filters = filters_for(levels_db, sample_rate);
        Self {
            active: FilterBank::new(channels, filters),
            previous: FilterBank::new(channels, [Biquad::IDENTITY; EQ_BANDS]),
            levels_db,
            sample_rate,
            fade_frames: ((sample_rate * EQ_CROSSFADE_SECONDS) as usize).max(1),
            fade_remaining: 0,
        }
    }

    /// 直通且不在淡化中：整段跳过，输出逐位等于输入
    fn is_idle(&self) -> bool {
        self.fade_remaining == 0 && self.active.is_identity()
    }

    /// 切到新参数；上一次淡化还没走完就先不切，下一块再试
    fn retune(&mut self, levels_db: [f64; EQ_BANDS]) {
        if self.fade_remaining > 0 || levels_db == self.levels_db {
            return;
        }
        self.levels_db = levels_db;
        let filters = filters_for(levels_db, self.sample_rate);
        if filters == self.active.filters {
            return;
        }
        let was_idle = self.active.is_identity();
        self.previous.copy_from(&self.active);
        if was_idle {
            // 直通期间延迟线没有更新，里面是陈旧数据；从零起步，
            // 淡化开头旧输出（原声）占满权重，起步瞬态听不见
            self.active.history.fill([0.0; 4]);
        }
        self.active.filters = filters;
        self.fade_remaining = self.fade_frames;
    }

    /// 推进一帧淡化，返回新滤波器的权重；不在淡化中返回 None
    fn advance_fade(&mut self) -> Option<f64> {
        if self.fade_remaining == 0 {
            return None;
        }
        self.fade_remaining -= 1;
        Some(1.0 - self.fade_remaining as f64 / self.fade_frames as f64)
    }

    fn process(&mut self, sample: f64, channel: usize, fade: Option<f64>) -> f64 {
        let wet = self.active.process(sample, channel);
        match fade {
            Some(weight) => {
                let dry = self.previous.process(sample, channel);
                dry + (wet - dry) * weight
            }
            None => wet,
        }
    }
}

/// 线性过渡到目标值
#[derive(Clone, Copy, Debug)]
struct Ramp {
    value: f64,
    target: f64,
    remaining: usize,
    length: usize,
}

impl Ramp {
    fn settled(value: f64, length: usize) -> Self {
        Self {
            value,
            target: value,
            remaining: 0,
            length: length.max(1),
        }
    }

    fn set_target(&mut self, target: f64) {
        if target != self.target {
            self.target = target;
            self.remaining = self.length;
        }
    }

    fn next(&mut self) -> f64 {
        if self.remaining > 0 {
            self.value += (self.target - self.value) / self.remaining as f64;
            self.remaining -= 1;
            if self.remaining == 0 {
                self.value = self.target;
            }
        }
        self.value
    }

    fn rests_at(&self, value: f64) -> bool {
        self.remaining == 0 && self.value == value
    }

    fn may_exceed(&self, value: f64) -> bool {
        self.value > value || self.target > value
    }
}

/// 输出回调里的音效处理器：建流时一次分配好，回调里只复用，不加锁、不分配
pub struct EffectsProcessor {
    channels: usize,
    sample_rate: f64,
    control: Arc<EffectsControl>,
    track: Arc<TrackLoudness>,
    applied_version: u64,
    settings: EffectsSettings,
    normalizer_gain: f64,
    normalizer_mix: Ramp,
    loudness_gain: Ramp,
    equalizer: Equalizer,
    balance: Ramp,
    sides: Vec<Side>,
    limiter_gain: f64,
    attack_step: f64,
    release_step: f64,
    envelope: Vec<f64>,
    limited_frames: u64,
}

impl EffectsProcessor {
    pub fn new(
        channels: usize,
        sample_rate: u32,
        control: Arc<EffectsControl>,
        track: Arc<TrackLoudness>,
        max_block_frames: usize,
    ) -> Self {
        let channels = channels.max(1);
        let sample_rate = f64::from(sample_rate.max(1));
        let applied_version = control.version();
        let settings = control.snapshot();
        let ramp_frames = (sample_rate * PARAMETER_RAMP_SECONDS) as usize;
        let normalizer_gain = if track.is_seeded() {
            track.applied_gain()
        } else {
            1.0
        };
        Self {
            channels,
            sample_rate,
            applied_version,
            normalizer_gain,
            normalizer_mix: Ramp::settled(
                if settings.normalize_volume { 1.0 } else { 0.0 },
                ramp_frames,
            ),
            loudness_gain: Ramp::settled(
                millibels_to_gain(settings.loudness_gain_mb),
                ramp_frames,
            ),
            equalizer: Equalizer::new(channels, sample_rate, settings.equalizer_levels_db()),
            balance: Ramp::settled(balance_value(settings.balance_centi), ramp_frames),
            sides: (0..channels).map(|channel| channel_side(channel, channels)).collect(),
            limiter_gain: 1.0,
            attack_step: limiter_step(sample_rate, LIMITER_ATTACK_SECONDS),
            release_step: limiter_step(sample_rate, LIMITER_RELEASE_SECONDS),
            envelope: vec![1.0; max_block_frames.max(1)],
            limited_frames: 0,
            settings,
            control,
            track,
        }
    }

    /// 原地处理一段交错样本；音效全关且已过渡完时一个样本都不改
    pub fn process(&mut self, samples: &mut [f32]) {
        self.sync_settings();
        let block_samples = self.envelope.len() * self.channels;
        for block in samples.chunks_mut(block_samples) {
            self.process_block(block);
        }
        self.track
            .applied_gain
            .store(self.normalizer_gain.to_bits(), Ordering::Release);
    }

    /// 当前实际施加的音量均衡增益；关着时为 1
    pub fn normalization_gain(&self) -> f64 {
        1.0 + (self.normalizer_gain - 1.0) * self.normalizer_mix.value
    }

    /// 取出并清零上次以来被限幅器压过的帧数
    pub fn take_limited_frames(&mut self) -> u64 {
        std::mem::take(&mut self.limited_frames)
    }

    fn sync_settings(&mut self) {
        let version = self.control.version();
        if version == self.applied_version {
            return;
        }
        self.applied_version = version;
        self.settings = self.control.snapshot();
        self.normalizer_mix
            .set_target(if self.settings.normalize_volume { 1.0 } else { 0.0 });
        self.loudness_gain
            .set_target(millibels_to_gain(self.settings.loudness_gain_mb));
        self.balance
            .set_target(balance_value(self.settings.balance_centi));
    }

    fn process_block(&mut self, block: &mut [f32]) {
        let channels = self.channels;
        let frames = block.len() / channels;
        if frames == 0 {
            return;
        }
        self.equalizer.retune(self.settings.equalizer_levels_db());
        let (gain_start, gain_step) = self.advance_normalizer(frames);
        if self.is_transparent() {
            return;
        }
        let equalize = !self.equalizer.is_idle();
        let limit = self.needs_limiter();
        for (index, frame) in block.chunks_exact_mut(channels).enumerate() {
            let normalizer = gain_start + gain_step * (index + 1) as f64;
            let mix = self.normalizer_mix.next();
            let gain = (1.0 + (normalizer - 1.0) * mix) * self.loudness_gain.next();
            let (left, right) = balance_gains(self.balance.next());
            let fade = self.equalizer.advance_fade();
            let mut peak = 0.0f64;
            for ((channel, sample), side) in frame.iter_mut().enumerate().zip(&self.sides) {
                // 解码出的坏样本按静音处理：进了滤波器的延迟线，之后整段输出都会是 NaN
                let input = if sample.is_finite() { f64::from(*sample) } else { 0.0 };
                let filtered = if equalize {
                    self.equalizer.process(input, channel, fade)
                } else {
                    input
                };
                let side_gain = match side {
                    Side::Left => left,
                    Side::Right => right,
                    Side::Center => 1.0,
                };
                let value = filtered * gain * side_gain;
                peak = peak.max(value.abs());
                *sample = value as f32;
            }
            if limit {
                self.envelope[index] = if peak > PEAK_CEILING {
                    PEAK_CEILING / peak
                } else {
                    1.0
                };
            }
        }
        if limit {
            self.limit(block, frames);
        }
    }

    /// 推进音量均衡增益，返回本块起点与逐帧增量
    ///
    /// 与 Android 一样按块平滑、块内线性过渡；目标比当前低时 0.25 s 跟上，
    /// 比当前高时 4 s 慢慢抬。目标来自整首之后不再漂移，升降都按 0.5 s 走到位。
    fn advance_normalizer(&mut self, frames: usize) -> (f64, f64) {
        let Some(target) = self.track.target_gain() else {
            return (self.normalizer_gain, 0.0);
        };
        if !self.track.is_seeded() {
            // 新曲目第一次拿到统计：直接落位，不从 1 慢慢滑过去
            self.track.seeded.store(true, Ordering::Release);
            self.normalizer_gain = target;
        }
        let start = self.normalizer_gain;
        let next = if self.normalizer_mix.may_exceed(0.0) {
            let seconds = if self.track.covers_whole_track() {
                FINAL_GAIN_SECONDS
            } else if target < start {
                GAIN_REDUCTION_SECONDS
            } else {
                GAIN_INCREASE_SECONDS
            };
            start + (target - start) * smoothing_factor(frames as f64 / self.sample_rate, seconds)
        } else {
            // 关着时只跟着统计走，打开时直接从当前统计对应的增益淡入
            target
        };
        self.normalizer_gain = next;
        (start, (next - start) / frames as f64)
    }

    fn is_transparent(&self) -> bool {
        self.normalizer_mix.rests_at(0.0)
            && self.loudness_gain.rests_at(1.0)
            && self.equalizer.is_idle()
            && self.balance.rests_at(0.0)
            && self.limiter_gain >= 1.0
    }

    fn needs_limiter(&self) -> bool {
        self.normalizer_mix.may_exceed(0.0)
            || self.loudness_gain.may_exceed(1.0)
            || self.limiter_gain < 1.0
    }

    /// Android `buildLimiterEnvelope` 的包络：逐帧算出不过线所需的增益，
    /// 倒序一遍让增益在峰值前 5 ms 内提前压下，正序一遍限制回升速度（100 ms）
    fn limit(&mut self, block: &mut [f32], frames: usize) {
        let envelope = &mut self.envelope[..frames];
        for index in (0..frames.saturating_sub(1)).rev() {
            envelope[index] = envelope[index].min(envelope[index + 1] + self.attack_step);
        }
        let mut previous = self.limiter_gain;
        for gain in envelope.iter_mut() {
            *gain = gain.min(previous + self.release_step);
            previous = *gain;
        }
        self.limiter_gain = previous.min(1.0);
        for (frame, &gain) in block.chunks_exact_mut(self.channels).zip(envelope.iter()) {
            if gain < 1.0 {
                self.limited_frames += 1;
                for sample in frame {
                    *sample = (f64::from(*sample) * gain) as f32;
                }
            }
        }
    }
}

/// 声道平衡时一个输出声道归哪一侧
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Left,
    Right,
    /// 中置、低音与单声道：平衡不动它
    Center,
}

/// 按 Windows 多声道的标准顺序（FL FR FC LFE BL BR SL SR）分左右
fn channel_side(channel: usize, channels: usize) -> Side {
    use Side::{Center as C, Left as L, Right as R};
    let layout: &[Side] = match channels {
        1 => &[C],
        // 第三路是中置（3.0）或低音（2.1）
        3 => &[L, R, C],
        4 => &[L, R, L, R],
        5 => &[L, R, C, L, R],
        6 => &[L, R, C, C, L, R],
        // 6.1 的第五路是后中置
        7 => &[L, R, C, C, C, L, R],
        8 => &[L, R, C, C, L, R, L, R],
        _ => &[L, R],
    };
    layout.get(channel).copied().unwrap_or(Side::Center)
}

fn balance_value(balance_centi: i32) -> f64 {
    f64::from(balance_centi) / 100.0
}

/// Android `stereoBalanceGains`：偏向一侧时只把另一侧线性压低，偏向的一侧保持原样
fn balance_gains(balance: f64) -> (f64, f64) {
    let left = if balance > 0.0 { 1.0 - balance } else { 1.0 };
    let right = if balance < 0.0 { 1.0 + balance } else { 1.0 };
    (left, right)
}

fn smoothing_factor(duration_seconds: f64, time_constant_seconds: f64) -> f64 {
    if duration_seconds <= 0.0 {
        return 0.0;
    }
    if time_constant_seconds <= 0.0 {
        return 1.0;
    }
    (1.0 - (-duration_seconds / time_constant_seconds).exp()).clamp(0.0, 1.0)
}

/// 限幅增益每帧最多变化多少：在给定时长内走完整个增益范围
fn limiter_step(sample_rate: f64, seconds: f64) -> f64 {
    let frames = sample_rate * seconds;
    if frames <= 1.0 {
        MAX_GAIN - MIN_GAIN
    } else {
        (MAX_GAIN - MIN_GAIN) / frames
    }
}

/// 毫贝 -> 线性增益：10^(mB / 2000)
fn millibels_to_gain(millibels: i32) -> f64 {
    if millibels == 0 {
        1.0
    } else {
        10.0_f64.powf(f64::from(millibels) / 2_000.0)
    }
}

/// 线性增益 -> 毫贝，供指标与日志使用
pub fn gain_to_millibels(gain: f64) -> i64 {
    (2_000.0 * gain.max(1e-6).log10()).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn control_with(change: impl FnOnce(&mut EffectsSettings)) -> Arc<EffectsControl> {
        let control = EffectsControl::new_shared();
        control.update(change);
        control
    }

    fn processor(
        channels: usize,
        sample_rate: u32,
        control: &Arc<EffectsControl>,
        track: &Arc<TrackLoudness>,
    ) -> EffectsProcessor {
        EffectsProcessor::new(
            channels,
            sample_rate,
            Arc::clone(control),
            Arc::clone(track),
            4_096,
        )
    }

    fn analyzed_track(samples: &[f32]) -> Arc<TrackLoudness> {
        let track = TrackLoudness::new_shared();
        let mut meter = LoudnessMeter::new(Arc::clone(&track));
        meter.observe(samples);
        meter.flush();
        track
    }

    fn sine(frequency: f64, amplitude: f32, frames: usize, channels: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|index| {
                let value = amplitude
                    * (std::f64::consts::TAU * frequency * index as f64 / f64::from(RATE)).sin()
                        as f32;
                std::iter::repeat_n(value, channels)
            })
            .collect()
    }

    fn render(processor: &mut EffectsProcessor, input: &[f32], block_frames: usize) -> Vec<f32> {
        let mut output = input.to_vec();
        for block in output.chunks_mut(block_frames * processor.channels) {
            processor.process(block);
        }
        output
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0f32, |peak, sample| peak.max(sample.abs()))
    }

    fn rms(samples: &[f32]) -> f64 {
        let sum: f64 = samples.iter().map(|sample| f64::from(*sample).powi(2)).sum();
        (sum / samples.len().max(1) as f64).sqrt()
    }

    fn block(rms: f64, peak: f64, samples: usize) -> Block {
        Block { sum_squares: rms * rms * samples as f64, samples, peak }
    }

    fn fold_stats(track: &TrackLoudness, rms: f64, peak: f64, samples: usize) {
        track.fold(&block(rms, peak, samples));
    }

    fn stats_for_gain(gain: f64) -> LoudnessStats {
        let mut stats = LoudnessStats::default();
        assert!(stats.add(&block(TARGET_RMS / gain, 0.1, 96_000)));
        stats
    }

    /// 目标增益的三种典型情形与 Android `VolumeNormalizationAudioProcessorTest` 逐个对齐
    #[test]
    fn target_gain_matches_android_reference_values() {
        let quiet = TrackLoudness::new_shared();
        fold_stats(&quiet, 0.01, 0.02, 48_000);
        assert!((quiet.target_gain().unwrap() - 1.995_262_3).abs() < 1e-4, "安静曲最多抬 6 dB");

        let loud = TrackLoudness::new_shared();
        fold_stats(&loud, 0.5, 0.7, 48_000);
        assert!((loud.target_gain().unwrap() - 0.251_785).abs() < 1e-4, "响曲压向目标 RMS");

        let peaky = TrackLoudness::new_shared();
        fold_stats(&peaky, 0.08, 0.8, 48_000);
        assert!((peaky.target_gain().unwrap() - 0.992_910_3).abs() < 1e-4, "峰值封顶先于 RMS 目标");

        let silent = TrackLoudness::new_shared();
        fold_stats(&silent, 0.0001, 0.0002, 48_000);
        assert_eq!(silent.target_gain(), None, "静音不产生增益决策");
    }

    /// 音效全关时输出必须逐位等于输入，包括满幅样本
    #[test]
    fn disabled_effects_keep_samples_bit_exact() {
        let control = EffectsControl::new_shared();
        let track = analyzed_track(&sine(440.0, 0.05, 4_800, 2));
        let mut effects = processor(2, RATE, &control, &track);
        let mut input = sine(440.0, 0.9, 2_400, 2);
        input.extend_from_slice(&[1.0, -1.0, 0.5, -0.25]);

        let output = render(&mut effects, &input, 480);

        assert_eq!(output, input);
    }

    /// 安静的曲子被抬起来，后面突然来一个满幅峰也不能被推过 -2 dBFS
    #[test]
    fn quiet_track_is_boosted_and_sudden_peaks_stay_under_the_ceiling() {
        let control = control_with(|settings| settings.normalize_volume = true);
        let quiet = sine(440.0, 0.01, 48_000, 2);
        let track = analyzed_track(&quiet);
        let mut effects = processor(2, RATE, &control, &track);

        let boosted = render(&mut effects, &quiet[..9_600], 480);
        let gain = rms(&boosted) / rms(&quiet[..9_600]);
        assert!((gain - MAX_GAIN).abs() < 0.01, "安静曲应抬 +6 dB: {gain:.3}");

        let burst = vec![1.0f32; 960];
        let limited = render(&mut effects, &burst, 480);
        assert!(
            peak(&limited) <= PEAK_CEILING as f32 + 1e-6,
            "突发满幅峰被推过 -2 dBFS: {}",
            peak(&limited),
        );
        assert!(effects.take_limited_frames() > 0);
    }

    /// 块尾的峰只提前 5 ms 压下，块开头仍按均衡增益播放（Android 同名用例）
    #[test]
    fn future_peak_does_not_duck_the_start_of_the_block() {
        let control = control_with(|settings| settings.normalize_volume = true);
        let quiet = vec![320.0f32 / 32_768.0; 20_000];
        let track = analyzed_track(&quiet);
        let mut effects = processor(1, 1_000, &control, &track);
        render(&mut effects, &quiet, 100);

        let mut transition = vec![320.0f32 / 32_768.0; 99];
        transition.push(1.0);
        let output = render(&mut effects, &transition, 100);

        assert!(output[0] > 500.0 / 32_768.0, "块开头被提前压低: {}", output[0] * 32_768.0);
        assert!(peak(&output) <= PEAK_CEILING as f32 + 1e-6);
    }

    /// seek / 重建会话后接着上一个会话的增益走，不重新预热、不跳音量
    ///
    /// 旧实现每次重建都从目标位置之后 200 ms 重新估计增益：seek 到安静段
    /// 音量突然 +6 dB，seek 到副歌又被压下去。
    #[test]
    fn a_new_session_for_the_same_track_continues_the_applied_gain() {
        let control = control_with(|settings| settings.normalize_volume = true);
        let track = analyzed_track(&sine(440.0, 0.02, 48_000, 2));
        let mut first = processor(2, RATE, &control, &track);
        render(&mut first, &sine(440.0, 0.02, 4_800, 2), 480);
        // 解码线程读到了响段，目标增益大幅下调；第一个会话刚开始往下走
        let mut meter = LoudnessMeter::new(Arc::clone(&track));
        meter.observe(&sine(440.0, 0.6, 96_000, 2));
        meter.flush();
        render(&mut first, &sine(440.0, 0.02, 960, 2), 480);
        let before = first.normalization_gain();
        let target = track.target_gain().unwrap();
        assert!(before > target * 1.5, "前置条件：增益还在往目标走 {before:.3} -> {target:.3}");

        let mut second = processor(2, RATE, &control, &track);
        let input = sine(1_000.0, 0.02, 480, 2);
        let output = render(&mut second, &input, 480);
        let after = rms(&output) / rms(&input);

        assert!(
            (after / before - 1.0).abs() < 0.05,
            "重建后增益应从 {before:.3} 接着走，实际 {after:.3}",
        );
    }

    /// 播放中开关音量均衡：下一块开始在 30 ms 内平滑过渡，关掉后回到逐位直通
    #[test]
    fn toggling_normalization_ramps_smoothly_and_returns_to_bit_exact() {
        let control = EffectsControl::new_shared();
        let input = vec![0.01f32; 2 * 4_800];
        let track = analyzed_track(&input);
        let mut effects = processor(2, RATE, &control, &track);
        assert_eq!(render(&mut effects, &input[..960], 480), input[..960]);

        control.update(|settings| settings.normalize_volume = true);
        let rising = render(&mut effects, &input, 480);
        let largest_step = rising
            .chunks_exact(2)
            .zip(rising.chunks_exact(2).skip(1))
            .map(|(left, right)| (right[0] - left[0]).abs())
            .fold(0.0f32, f32::max);
        let ramp_frames = (f64::from(RATE) * PARAMETER_RAMP_SECONDS) as f32;
        assert!(largest_step <= 0.01 * (MAX_GAIN as f32 - 1.0) / ramp_frames * 1.5, "开启时增益跳变: {largest_step}");
        let settled = rising[rising.len() - 1] / 0.01;
        assert!((f64::from(settled) - MAX_GAIN).abs() < 0.01, "过渡后应到目标增益: {settled}");

        control.update(|settings| settings.normalize_volume = false);
        render(&mut effects, &input, 480);
        assert_eq!(render(&mut effects, &input[..960], 480), input[..960], "关掉后应逐位直通");
    }

    /// 拉高低频时整体下移：低频保持原样、其它频段被压低，不削波
    #[test]
    fn equalizer_boost_uses_headroom_instead_of_clipping() {
        let control = control_with(|settings| {
            settings.eq_enabled = true;
            settings.eq_band_levels_mb = [1_200, 0, 0, 0, 0];
        });
        let track = TrackLoudness::new_shared();
        let mut effects = processor(2, RATE, &control, &track);

        // 60 Hz 只受相邻 230 Hz 频段裙边的约 -1 dB 影响
        let bass = sine(60.0, 0.9, 48_000, 2);
        let bass_out = render(&mut effects, &bass, 480);
        let tail = &bass_out[bass_out.len() / 2..];
        assert!(peak(tail) <= 0.9, "低频被推过原峰值: {}", peak(tail));
        assert!(peak(tail) >= 0.75, "低频应基本保持: {}", peak(tail));

        // 3.6 kHz 落在 -12 dB 的频段中心，两侧频段的裙边再叠一点
        let treble = sine(3_600.0, 0.9, 48_000, 2);
        let treble_out = render(&mut effects, &treble, 480);
        let ratio = rms(&treble_out[treble_out.len() / 2..]) / rms(&treble[treble.len() / 2..]);
        assert!((0.15..0.3).contains(&ratio), "其它频段应下移约 12 dB: {ratio:.3}");
    }

    /// 播放中反复切换均衡器参数：交叉淡化让波形连续，没有咔哒
    #[test]
    fn equalizer_changes_crossfade_without_clicks() {
        let control = EffectsControl::new_shared();
        let track = TrackLoudness::new_shared();
        let mut effects = processor(1, RATE, &control, &track);
        let input = sine(910.0, 0.5, 48_000, 1);
        let natural_step = 0.5 * std::f64::consts::TAU * 910.0 / f64::from(RATE);

        let mut output = Vec::with_capacity(input.len());
        for (index, block) in input.chunks(480).enumerate() {
            if index % 5 == 0 {
                let enabled = index % 10 == 0;
                control.update(|settings| {
                    settings.eq_enabled = enabled;
                    settings.eq_band_levels_mb = [0, 0, -1_500, 0, 0];
                });
            }
            let mut chunk = block.to_vec();
            effects.process(&mut chunk);
            output.extend_from_slice(&chunk);
        }

        let largest_step = output
            .windows(2)
            .map(|pair| f64::from((pair[1] - pair[0]).abs()))
            .fold(0.0, f64::max);
        assert!(output.iter().all(|sample| sample.is_finite()));
        assert!(
            largest_step < natural_step * 1.3,
            "切换参数时波形跳变 {largest_step:.4}，正弦本身最大 {natural_step:.4}",
        );
    }

    /// 响度增益 +15 dB：更响，但限幅器保证峰值不过 -2 dBFS；归零后回到逐位直通
    #[test]
    fn loudness_gain_is_limited_and_releases_back_to_bit_exact() {
        let control = control_with(|settings| settings.loudness_gain_mb = 1_500);
        let track = TrackLoudness::new_shared();
        let mut effects = processor(2, RATE, &control, &track);
        let input = sine(440.0, 0.5, 48_000, 2);

        let output = render(&mut effects, &input, 480);
        assert!(peak(&output) <= PEAK_CEILING as f32 + 1e-6, "峰值过线: {}", peak(&output));
        assert!(rms(&output) > rms(&input) * 1.2, "响度增益应让声音更响");

        control.update(|settings| settings.loudness_gain_mb = 0);
        render(&mut effects, &input, 480);
        let quiet = sine(440.0, 0.3, 4_800, 2);
        assert_eq!(render(&mut effects, &quiet, 480), quiet, "归零并释放后应逐位直通");
    }

    /// 静音块不计入统计；曲尾不足一块的部分在 flush 时计入
    #[test]
    fn meter_ignores_silent_blocks_and_counts_the_tail() {
        let track = TrackLoudness::new_shared();
        let mut meter = LoudnessMeter::new(Arc::clone(&track));
        meter.observe(&vec![0.0; ANALYSIS_BLOCK_SAMPLES * 4]);
        assert_eq!(track.target_gain(), None);
        meter.observe(&vec![0.1; 100]);
        assert_eq!(track.analyzed_samples(), 0, "不足一块时先攒着");
        meter.flush();
        assert_eq!(track.analyzed_samples(), 100);
        assert!(track.target_gain().is_some());
    }

    /// 整首扫描和播放时的统计必须是同一个口径，否则扫完之后目标会跳一下
    #[test]
    fn scanner_and_live_meter_measure_the_same_way() {
        let mut samples = vec![0.0005f32; ANALYSIS_BLOCK_SAMPLES * 2];
        samples.extend(sine(440.0, 0.3, 30_000, 2));
        samples.extend(sine(90.0, 0.05, 7_777, 2));
        let track = TrackLoudness::new_shared();
        let mut meter = LoudnessMeter::new(Arc::clone(&track));
        meter.observe(&samples);
        meter.flush();

        let mut scanner = LoudnessScanner::default();
        scanner.observe(&samples);
        let stats = scanner.finish();

        assert_eq!(track.analyzed_samples(), stats.samples());
        assert_eq!(track.target_gain(), stats.target_gain());
        assert!(stats.samples() < samples.len() as u64, "开头的静音块不计入");
    }

    /// 扫描完成后换成整首的统计，之后播放时的统计不再改变目标
    #[test]
    fn completed_scan_replaces_live_stats_and_freezes_the_target() {
        let track = TrackLoudness::new_shared();
        fold_stats(&track, 0.02, 0.05, 48_000);
        let live = track.target_gain().unwrap();

        let mut scanner = LoudnessScanner::default();
        scanner.observe(&sine(440.0, 0.4, 48_000, 2));
        let stats = scanner.finish();
        track.complete(stats);
        let final_target = track.target_gain().unwrap();

        assert!(track.is_complete());
        assert_eq!(Some(final_target), stats.target_gain());
        assert!(final_target < live, "整首更响，目标应低于开头的估计");
        fold_stats(&track, 0.02, 0.05, 480_000);
        assert_eq!(track.target_gain(), Some(final_target), "扫描完成后目标固定");
        assert_eq!(track.analyzed_samples(), stats.samples());
    }

    /// 抽样估计先顶上，完整扫描到了再换；之后的估计和播放时的统计都不再改目标
    #[test]
    fn whole_track_estimate_holds_until_the_full_scan_replaces_it() {
        let track = TrackLoudness::new_shared();
        assert!(track.begin_scan());
        assert!(!track.begin_scan(), "每首歌只扫描一次");
        fold_stats(&track, 0.02, 0.05, 48_000);

        track.estimate(stats_for_gain(0.8));
        assert!(track.covers_whole_track() && !track.is_complete());
        assert!((track.target_gain().unwrap() - 0.8).abs() < 1e-9);
        fold_stats(&track, 0.5, 0.9, 480_000);
        assert!((track.target_gain().unwrap() - 0.8).abs() < 1e-9, "播放时的统计不再并入");

        track.complete(stats_for_gain(0.6));
        track.estimate(stats_for_gain(1.2));
        assert!(track.is_complete());
        assert!((track.target_gain().unwrap() - 0.6).abs() < 1e-9, "完整扫描之后不再被估计覆盖");
    }

    /// 抽样估计全落在静音里：不能换上空统计停掉播放时的统计，否则整首都没有目标增益
    #[test]
    fn an_empty_estimate_keeps_the_live_stats_running() {
        let track = TrackLoudness::new_shared();
        fold_stats(&track, 0.02, 0.05, 48_000);
        let quiet_start = track.target_gain().unwrap();
        assert!(!track.estimate(LoudnessScanner::default().finish()), "空估计不采用");
        assert!(!track.covers_whole_track());
        fold_stats(&track, 0.2, 0.5, 48_000);
        assert!(track.target_gain().unwrap() < quiet_start, "播放时的统计照常并入");
    }

    /// 解码出一个 NaN / inf：不能毒化整首共享的响度统计，也不能卡死均衡器的延迟线
    #[test]
    fn a_bad_sample_cannot_poison_the_track_stats_or_the_equalizer() {
        let control = control_with(|settings| {
            settings.eq_enabled = true;
            settings.eq_band_levels_mb = [600, 0, 0, 0, 0];
            settings.normalize_volume = true;
        });
        let mut input = sine(440.0, 0.3, 9_600, 2);
        input[1_000] = f32::NAN;
        input[1_001] = f32::INFINITY;
        let track = analyzed_track(&input);
        assert!(track.target_gain().is_some_and(f64::is_finite), "目标增益: {:?}", track.target_gain());

        let mut effects = processor(2, RATE, &control, &track);
        let output = render(&mut effects, &input, 480);
        assert!(output.iter().all(|sample| sample.is_finite()), "坏样本之后的输出必须都是有限值");
        assert!(peak(&output[output.len() / 2..]) > 0.1, "坏样本之后仍在出声");
    }

    /// 目标固定后升降都在约 2 秒内到位；还按 4 s 慢升的话 2 秒只走到四成
    #[test]
    fn final_target_is_reached_quickly_in_both_directions() {
        let control = control_with(|settings| settings.normalize_volume = true);
        let quiet = sine(440.0, 0.01, 96_000, 2);
        for (start, target) in [(0.5, 1.5), (1.5, 0.5)] {
            let track = TrackLoudness::new_shared();
            track.fold(&block(TARGET_RMS / start, 0.1, 96_000));
            let mut effects = processor(2, RATE, &control, &track);
            render(&mut effects, &quiet[..9_600], 480);
            assert!((effects.normalization_gain() - start).abs() < 1e-6, "先按开头的估计落位");

            track.complete(stats_for_gain(target));
            render(&mut effects, &quiet, 480);
            render(&mut effects, &quiet[..48_000], 480);

            let gain = effects.normalization_gain();
            assert!(
                (gain - target).abs() < 0.01,
                "{start} -> {target} 应在 2.5 秒内到位，实际 {gain:.3}",
            );
        }
    }

    /// 最后一个会话释放曲目时通知扫描线程停下
    #[test]
    fn releasing_the_last_session_cancels_the_scan() {
        let track = TrackLoudness::new_shared();
        let cancel = track.scan_cancel();
        let other_session = Arc::clone(&track);

        drop(track);
        assert!(!cancel.load(Ordering::Acquire), "还有会话在用这首歌");
        drop(other_session);
        assert!(cancel.load(Ordering::Acquire));
    }

    /// 回归：重置音效曾把音量均衡一起关掉，设置页的开关却还显示开着
    #[test]
    fn resetting_the_effects_panel_keeps_the_settings_page_values() {
        let control = control_with(|settings| {
            settings.loudness_gain_mb = 600;
            settings.eq_enabled = true;
            settings.eq_band_levels_mb = [300, 0, 0, 0, -300];
            settings.normalize_volume = true;
            settings.balance_centi = -35;
        });

        control.update(|settings| *settings = settings.reset_panel());

        assert_eq!(
            control.snapshot(),
            EffectsSettings {
                normalize_volume: true,
                balance_centi: -35,
                ..EffectsSettings::default()
            },
        );
    }

    /// 与 Android `stereoBalanceGains` 相同：只压低另一侧，偏向的一侧不变
    #[test]
    fn balance_gains_match_android() {
        assert_eq!(balance_gains(0.0), (1.0, 1.0));
        assert_eq!(balance_gains(0.35), (0.65, 1.0));
        assert_eq!(balance_gains(-0.35), (1.0, 0.65));
        assert_eq!(balance_gains(1.0), (0.0, 1.0));
        assert_eq!(balance_gains(-1.0), (1.0, 0.0));
        let control = control_with(|settings| settings.balance_centi = 150);
        assert_eq!(control.snapshot().balance_centi, 100, "越界值夹到满偏");
    }

    /// 拖动平衡平滑过渡、不爆音；回到居中后逐位直通
    #[test]
    fn balance_ramps_one_side_down_and_returns_to_bit_exact() {
        let control = EffectsControl::new_shared();
        let track = TrackLoudness::new_shared();
        let mut effects = processor(2, RATE, &control, &track);
        let input = vec![0.5f32; 2 * 4_800];

        control.update(|settings| settings.balance_centi = 50);
        let output = render(&mut effects, &input, 480);
        let ramp_frames = (f64::from(RATE) * PARAMETER_RAMP_SECONDS) as f32;
        let largest_step = output
            .chunks_exact(2)
            .zip(output.chunks_exact(2).skip(1))
            .map(|(previous, next)| (next[0] - previous[0]).abs())
            .fold((0.5 - output[0]).abs(), f32::max);
        assert!(largest_step <= 0.25 / ramp_frames * 1.5, "平衡跳变: {largest_step}");
        let last = &output[output.len() - 2..];
        assert!((last[0] - 0.25).abs() < 1e-6, "左声道应压到一半: {}", last[0]);
        assert_eq!(last[1], 0.5, "右声道不动");

        control.update(|settings| settings.balance_centi = 0);
        render(&mut effects, &input, 480);
        assert_eq!(render(&mut effects, &input[..960], 480), input[..960], "居中后应逐位直通");
    }

    /// 多声道设备上左侧各声道一起压低，中置与低音不动；单声道不受平衡影响
    #[test]
    fn balance_covers_every_speaker_on_the_attenuated_side() {
        use Side::{Center as C, Left as L, Right as R};
        let layout = |channels| (0..channels).map(|channel| channel_side(channel, channels)).collect::<Vec<_>>();
        assert_eq!(layout(1), [C]);
        assert_eq!(layout(2), [L, R]);
        assert_eq!(layout(6), [L, R, C, C, L, R]);
        assert_eq!(layout(8), [L, R, C, C, L, R, L, R]);

        let control = control_with(|settings| settings.balance_centi = 100);
        let track = TrackLoudness::new_shared();
        let mut effects = processor(6, RATE, &control, &track);
        let output = render(&mut effects, &vec![0.5f32; 6 * 480], 480);
        assert_eq!(&output[output.len() - 6..], [0.0f32, 0.5, 0.5, 0.5, 0.0, 0.5]);

        let mut mono = processor(1, RATE, &control, &track);
        let input = vec![0.5f32; 480];
        assert_eq!(render(&mut mono, &input, 480), input);
    }

    /// 写端 panic 毒化锁之后，后续设置仍能写入并被回调读到
    #[test]
    fn updates_still_apply_after_a_writer_panicked() {
        let control = EffectsControl::new_shared();
        let poisoner = Arc::clone(&control);
        let _ = std::thread::spawn(move || {
            poisoner.update(|_| panic!("simulated panic while updating effects"));
        })
        .join();
        let before = control.version();

        control.update(|settings| {
            settings.loudness_gain_mb = 9_000;
            settings.eq_band_levels_mb = [100, -9_000, 300, -400, 500];
            settings.normalize_volume = true;
        });

        let settings = control.snapshot();
        assert!(control.version() > before);
        assert_eq!(settings.loudness_gain_mb, MAX_LOUDNESS_GAIN_MB);
        assert_eq!(settings.eq_band_levels_mb, [100, -MAX_BAND_LEVEL_MB, 300, -400, 500]);
        assert!(settings.normalize_volume);
    }
}
