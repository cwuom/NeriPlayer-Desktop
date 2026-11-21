use serde::Serialize;
use std::sync::{Arc, Mutex};

/// 音频电平分析数据，推送到前端用于可视化
#[derive(Debug, Clone, Serialize)]
pub struct AudioLevelData {
    pub level: f32,
    pub beat_impulse: f32,
}

/// 共享音频电平数据，供 main.rs ticker 线程读取并发射事件
/// 仅包含两个 f32，锁持有时间 <1μs
#[derive(Debug, Clone)]
pub struct SharedAudioLevel {
    pub level: f32,
    pub beat_impulse: f32,
}

impl SharedAudioLevel {
    pub fn new() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self {
            level: 0.0,
            beat_impulse: 0.0,
        }))
    }

    pub fn reset(shared: &Arc<Mutex<Self>>) {
        if let Ok(mut lock) = shared.lock() {
            lock.level = 0.0;
            lock.beat_impulse = 0.0;
        }
    }

    pub fn try_update(shared: &Arc<Mutex<Self>>, level: f32, beat_impulse: f32) -> bool {
        let Ok(mut lock) = shared.try_lock() else {
            return false;
        };
        lock.level = level;
        lock.beat_impulse = beat_impulse;
        true
    }

    pub fn try_snapshot(shared: &Arc<Mutex<Self>>) -> Option<(f32, f32)> {
        let lock = shared.try_lock().ok()?;
        Some((lock.level, lock.beat_impulse))
    }
}

/// PCM 音频分析器：精确对齐 Android `AudioReactive.kt`
///
/// 算法流程：
/// 计算 RMS（线性幅值，0..1）
/// 双 EMA 包络：emaFast(α=0.5) 和 emaSlow(α=0.05) 跟踪 RMS
/// 正向能量增量：delta = max(0, emaFast - emaSlow)
/// 自适应噪声底限：noiseEma(α=0.02) 跟踪 delta
/// Beat 检测：delta > 3 * (noiseEma + EPS)，最小间隔 120ms
/// Beat 帧直接置 1.0，非 beat 帧衰减 *= 0.90
/// 感知响度：sqrt(rms)，beat 时加 0.08 boost
pub struct AudioAnalyzer {
    /// 快速 EMA（α=0.5，跟踪 RMS 攻速）
    ema_fast: f64,
    /// 慢速 EMA（α=0.05，跟踪 RMS 释速）
    ema_slow: f64,
    /// 自适应噪声底限 EMA
    noise_ema: f64,
    /// 上次 beat 触发的帧号
    last_beat_frame: u64,
    /// 当前帧号
    frame_count: u64,
    /// 每帧对应的纳秒数（用于最小 beat 间隔检查）
    ns_per_frame: f64,
    /// 当前 beat 脉冲值
    beat_impulse: f32,
}

// 精确对齐 Android AudioReactive.kt 的常量
const EMA_FAST_ALPHA: f64 = 0.5;
const EMA_SLOW_ALPHA: f64 = 0.05;
const NOISE_EMA_ALPHA: f64 = 0.02;
const BEAT_THRESHOLD_MULT: f64 = 3.0;
const MIN_BEAT_GAP_NS: f64 = 120_000_000.0; // 120ms
const DECAY_PER_CALL: f32 = 0.90;
const BEAT_LEVEL_BOOST: f32 = 0.08;
const EPS: f64 = 1e-9;

impl Default for AudioAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioAnalyzer {
    pub fn new() -> Self {
        Self {
            ema_fast: 0.0,
            ema_slow: 0.0,
            noise_ema: 0.0,
            last_beat_frame: 0,
            frame_count: 0,
            // 默认：2048 样本 @44100Hz ≈ 46.4ms
            ns_per_frame: 2048.0 / 44100.0 * 1_000_000_000.0,
            beat_impulse: 0.0,
        }
    }

    /// 设置帧参数（每次音频源变化时调用）
    pub fn configure(&mut self, sample_rate: u32, frame_size: usize) {
        if sample_rate > 0 && frame_size > 0 {
            self.ns_per_frame = frame_size as f64 / sample_rate as f64 * 1_000_000_000.0;
        }
    }

    /// 重置所有状态（切歌/seek 时调用）
    pub fn reset(&mut self) {
        self.ema_fast = 0.0;
        self.ema_slow = 0.0;
        self.noise_ema = 0.0;
        self.last_beat_frame = 0;
        self.frame_count = 0;
        self.beat_impulse = 0.0;
    }

    /// 分析一帧 PCM 样本（f32, -1.0..1.0），返回电平和 beat 脉冲
    ///
    /// 精确对齐 Android AudioReactive.teeSink.handleBuffer()
    pub fn analyze_frame(&mut self, samples: &[f32]) -> AudioLevelData {
        self.frame_count += 1;

        if samples.is_empty() {
            self.beat_impulse *= DECAY_PER_CALL;
            return AudioLevelData {
                level: 0.0,
                beat_impulse: self.beat_impulse,
            };
        }

        // 计算 RMS（线性幅值，0..1）
        let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
        let rms = (sum_sq / samples.len() as f64).sqrt(); // 0..1 线性

        // 双 EMA 包络：对齐 Android emaFast/emaSlow
        self.ema_fast = EMA_FAST_ALPHA * rms + (1.0 - EMA_FAST_ALPHA) * self.ema_fast;
        self.ema_slow = EMA_SLOW_ALPHA * rms + (1.0 - EMA_SLOW_ALPHA) * self.ema_slow;

        // 正向能量增量：对齐 Android delta = max(0, emaFast - emaSlow)
        let delta = (self.ema_fast - self.ema_slow).max(0.0);

        // 自适应噪声底限：对齐 Android noiseEma
        self.noise_ema = NOISE_EMA_ALPHA * delta + (1.0 - NOISE_EMA_ALPHA) * self.noise_ema;
        let threshold = BEAT_THRESHOLD_MULT * (self.noise_ema + EPS);

        // Beat 检测：对齐 Android: delta > threshold && 间隔 > 120ms
        let frames_since_beat = self.frame_count - self.last_beat_frame;
        let ns_since_beat = frames_since_beat as f64 * self.ns_per_frame;

        let new_beat = delta > threshold && ns_since_beat > MIN_BEAT_GAP_NS;

        // Beat 脉冲更新：对齐 Android: beat 帧置 1.0，否则衰减
        if new_beat {
            self.last_beat_frame = self.frame_count;
            self.beat_impulse = 1.0;
        } else {
            self.beat_impulse *= DECAY_PER_CALL;
        }

        // 感知响度：对齐 Android: sqrt(clamp(rms, 0, 1))
        let perceptual = rms.clamp(0.0, 1.0).sqrt() as f32;
        let level = if new_beat {
            // 对齐 Android: max(perceptual, min(1, perceptual + 0.08))
            perceptual.max((perceptual + BEAT_LEVEL_BOOST).min(1.0))
        } else {
            perceptual
        };

        AudioLevelData {
            level: level.clamp(0.0, 1.0),
            beat_impulse: self.beat_impulse.clamp(0.0, 1.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SharedAudioLevel;

    #[test]
    fn visualization_updates_do_not_wait_for_a_busy_consumer() {
        let shared = SharedAudioLevel::new();
        let guard = shared.lock().expect("shared audio level lock");

        assert!(!SharedAudioLevel::try_update(&shared, 0.4, 0.8));
        assert!(SharedAudioLevel::try_snapshot(&shared).is_none());

        drop(guard);
        assert!(SharedAudioLevel::try_update(&shared, 0.4, 0.8));
        assert_eq!(SharedAudioLevel::try_snapshot(&shared), Some((0.4, 0.8)));
    }
}
