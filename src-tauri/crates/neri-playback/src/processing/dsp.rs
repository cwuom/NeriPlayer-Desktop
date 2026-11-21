//! Android DSP 的桌面桥接，控制端规整设置，回调独占预分配引擎

use super::dsp_settings::AudioEffectsSettings;
use serde::Serialize;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

// 前 104 项对齐 Android c54884d 的 NeriDspParams，之后是桌面扩展；与 C++ neri_dsp_params.h 一致
pub(crate) mod params {
    pub const COUNT: usize = 106;
    pub const GRAPHIC_BAND_COUNT: usize = 10;
    pub const PARAMETRIC_BAND_COUNT: usize = 10;
    pub const PARAMETRIC_BAND_STRIDE: usize = 5;
    pub const MASTER_ENABLED: usize = 0;
    pub const QUALITY_MODE: usize = 1;
    pub const PREAMP_DB: usize = 2;
    pub const OUTPUT_GAIN_DB: usize = 3;
    pub const LIMITER_ENABLED: usize = 4;
    pub const LIMITER_CEILING_DB: usize = 5;
    pub const LIMITER_RELEASE_MS: usize = 6;
    pub const DITHER_ENABLED: usize = 7;
    pub const GRAPHIC_EQ_ENABLED: usize = 8;
    pub const GRAPHIC_EQ_GAIN_0: usize = 9;
    pub const PARAMETRIC_EQ_ENABLED: usize = 19;
    pub const PARAMETRIC_BAND_0: usize = 20;
    pub const BAND_ENABLED: usize = 0;
    pub const BAND_TYPE: usize = 1;
    pub const BAND_FREQUENCY_HZ: usize = 2;
    pub const BAND_GAIN_DB: usize = 3;
    pub const BAND_Q: usize = 4;
    pub const BASS_GAIN_DB: usize = 70;
    pub const BASS_FREQUENCY_HZ: usize = 71;
    pub const TREBLE_GAIN_DB: usize = 72;
    pub const TREBLE_FREQUENCY_HZ: usize = 73;
    pub const VIRTUAL_BASS_AMOUNT: usize = 74;
    pub const VIRTUAL_BASS_FREQUENCY_HZ: usize = 75;
    pub const WARMTH_AMOUNT: usize = 76;
    pub const EXCITER_AMOUNT: usize = 77;
    pub const EXCITER_FREQUENCY_HZ: usize = 78;
    pub const VOCAL_CLARITY_AMOUNT: usize = 79;
    pub const VOCAL_REMOVAL_AMOUNT: usize = 80;
    pub const STEREO_WIDTH: usize = 81;
    pub const MONO_BASS_FREQUENCY_HZ: usize = 82;
    pub const MONO_MIX: usize = 83;
    pub const CHANNEL_SWAP: usize = 84;
    pub const CROSSFEED_AMOUNT: usize = 85;
    pub const SURROUND_AMOUNT: usize = 86;
    pub const REVERB_AMOUNT: usize = 87;
    pub const REVERB_ROOM_SIZE: usize = 88;
    pub const REVERB_DAMPING: usize = 89;
    pub const REVERB_PRE_DELAY_MS: usize = 90;
    pub const COMPRESSOR_ENABLED: usize = 91;
    pub const COMPRESSOR_THRESHOLD_DB: usize = 92;
    pub const COMPRESSOR_RATIO: usize = 93;
    pub const COMPRESSOR_ATTACK_MS: usize = 94;
    pub const COMPRESSOR_RELEASE_MS: usize = 95;
    pub const COMPRESSOR_KNEE_DB: usize = 96;
    pub const COMPRESSOR_MAKEUP_DB: usize = 97;
    pub const SPEAKER_ENABLED: usize = 98;
    pub const SPEAKER_HIGH_PASS_HZ: usize = 99;
    pub const SPEAKER_BASS_HARMONICS: usize = 100;
    pub const SPEAKER_LOUDNESS: usize = 101;
    pub const SPEAKER_CLARITY: usize = 102;
    pub const SPEAKER_STEREO_EXPAND: usize = 103;
    pub const LOUDNESS_BASS_DB: usize = 104;
    pub const LOUDNESS_TREBLE_DB: usize = 105;
}

use params::*;
pub const PARAM_COUNT: usize = COUNT;

pub struct DspResolution {
    pub params: [f32; PARAM_COUNT],
    pub inactive_reason: u8,
}

pub fn neutral_params() -> [f32; PARAM_COUNT] {
    let mut p = [0.0; PARAM_COUNT];
    p[QUALITY_MODE] = 1.0;
    p[LIMITER_ENABLED] = 1.0;
    p[LIMITER_CEILING_DB] = -1.0;
    p[LIMITER_RELEASE_MS] = 80.0;
    for band in 0..PARAMETRIC_BAND_COUNT {
        p[PARAMETRIC_BAND_0 + BAND_FREQUENCY_HZ + band * PARAMETRIC_BAND_STRIDE] = 1000.0;
        p[PARAMETRIC_BAND_0 + BAND_Q + band * PARAMETRIC_BAND_STRIDE] = 0.707;
    }
    for (index, value) in [
        (BASS_FREQUENCY_HZ, 100.0),
        (TREBLE_FREQUENCY_HZ, 8000.0),
        (VIRTUAL_BASS_FREQUENCY_HZ, 100.0),
        (EXCITER_FREQUENCY_HZ, 5000.0),
        (STEREO_WIDTH, 1.0),
        (REVERB_ROOM_SIZE, 0.5),
        (REVERB_DAMPING, 0.5),
        (COMPRESSOR_THRESHOLD_DB, -18.0),
        (COMPRESSOR_RATIO, 2.0),
        (COMPRESSOR_ATTACK_MS, 10.0),
        (COMPRESSOR_RELEASE_MS, 150.0),
        (COMPRESSOR_KNEE_DB, 6.0),
        (SPEAKER_HIGH_PASS_HZ, 180.0),
    ] {
        p[index] = value;
    }
    p
}

#[repr(C)]
#[derive(Default)]
struct NativeStats {
    frames: i64,
    nanos: i64,
    limiter_reduction_db: f32,
    compressor_reduction_db: f32,
    idle: i32,
    sample_rate: i32,
}

extern "C" {
    fn neri_desktop_dsp_create(rate: i32, channels: i32, neutral: *const f32) -> *mut c_void;
    fn neri_desktop_dsp_destroy(engine: *mut c_void);
    fn neri_desktop_dsp_set_params(engine: *mut c_void, params: *const f32, count: i32) -> i32;
    fn neri_desktop_dsp_process(engine: *mut c_void, samples: *mut f32, count: usize) -> i32;
    fn neri_desktop_dsp_stats(engine: *mut c_void, stats: *mut NativeStats);
    fn neri_desktop_dsp_headroom(params: *const f32) -> f32;
}

pub fn headroom(params: &[f32; PARAM_COUNT]) -> f32 {
    // 参数经过 Rust 规整，C++ 只读取固定 PARAM_COUNT 项，计算发生在控制端
    unsafe { neri_desktop_dsp_headroom(params.as_ptr()) }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioEffectsStats {
    pub output_device: String,
    pub active: bool,
    pub cpu_load_percent: f32,
    pub limiter_reduction_db: f32,
    pub compressor_reduction_db: f32,
    pub sample_rate: u32,
    pub native_available: bool,
    pub inactive_reason: Option<&'static str>,
}

pub struct DspControl {
    settings: Mutex<AudioEffectsSettings>,
    output_device: Mutex<String>,
    /// 播放器音量（线性），响度补偿据此计算
    volume: AtomicU32,
    version: AtomicU64,
    params: [AtomicU32; PARAM_COUNT],
    requested_reason: AtomicU8,
    active: AtomicBool,
    cpu: AtomicU32,
    limiter: AtomicU32,
    compressor: AtomicU32,
    sample_rate: AtomicU32,
    native_available: AtomicBool,
    reason: AtomicU8,
    next_processor: AtomicU64,
    current_processor: AtomicU64,
}

impl DspControl {
    pub fn new_shared() -> Arc<Self> {
        let p = neutral_params();
        Arc::new(Self {
            settings: Mutex::new(AudioEffectsSettings::default()),
            output_device: Mutex::new(String::new()),
            volume: AtomicU32::new(1.0f32.to_bits()),
            version: AtomicU64::new(0),
            params: std::array::from_fn(|index| AtomicU32::new(p[index].to_bits())),
            requested_reason: AtomicU8::new(1),
            active: AtomicBool::new(false),
            cpu: AtomicU32::new(0),
            limiter: AtomicU32::new(0),
            compressor: AtomicU32::new(0),
            sample_rate: AtomicU32::new(0),
            native_available: AtomicBool::new(true),
            reason: AtomicU8::new(1),
            next_processor: AtomicU64::new(0),
            current_processor: AtomicU64::new(0),
        })
    }

    pub fn set_settings(
        &self,
        settings: AudioEffectsSettings,
        output_device: &str,
    ) -> AudioEffectsSettings {
        let normalized = settings.normalized();
        let mut stored = self.settings.lock().unwrap_or_else(PoisonError::into_inner);
        let actual_output = self
            .output_device
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let selected = if actual_output.is_empty() {
            output_device
        } else {
            actual_output.as_str()
        };
        let resolution = normalized.resolve_with_volume(selected, self.volume());
        self.publish(resolution);
        *stored = normalized.clone();
        normalized
    }

    pub fn select_output(&self, output_device: &str) {
        let stored = self.settings.lock().unwrap_or_else(PoisonError::into_inner);
        *self
            .output_device
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = output_device.into();
        self.publish(stored.resolve_with_volume(output_device, self.volume()));
    }

    fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }

    /// 只有当前配置用到响度补偿时才重新下发参数，拖动音量条不会白白打断其它模块的平滑
    pub fn set_volume(&self, volume: f32) {
        let volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let previous = self.volume.swap(volume.to_bits(), Ordering::Relaxed);
        if previous == volume.to_bits() {
            return;
        }
        let stored = self.settings.lock().unwrap_or_else(PoisonError::into_inner);
        let output = self
            .output_device
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if stored.uses_loudness_compensation(&output) {
            self.publish(stored.resolve_with_volume(&output, volume));
        }
    }

    pub fn reset_preserving_user_presets(&self) {
        let mut stored = self.settings.lock().unwrap_or_else(PoisonError::into_inner);
        let reset = AudioEffectsSettings {
            user_presets: std::mem::take(&mut stored.user_presets),
            ..AudioEffectsSettings::default()
        };
        self.publish(reset.resolve(""));
        *stored = reset;
    }

    fn publish(&self, resolution: DspResolution) {
        self.version.fetch_add(1, Ordering::SeqCst);
        for (slot, value) in self.params.iter().zip(resolution.params) {
            slot.store(value.to_bits(), Ordering::SeqCst);
        }
        self.requested_reason
            .store(resolution.inactive_reason, Ordering::SeqCst);
        self.version.fetch_add(1, Ordering::SeqCst);
        if self.sample_rate.load(Ordering::Relaxed) == 0 {
            self.reason
                .store(resolution.inactive_reason, Ordering::Relaxed);
        }
    }

    // 有界读取，撞上控制端更新就沿用旧参数，下一块再读
    fn snapshot(&self, previous: u64) -> Option<(u64, DspResolution)> {
        let version = self.version.load(Ordering::SeqCst);
        if version == previous || version & 1 != 0 {
            return None;
        }
        let params =
            std::array::from_fn(|index| f32::from_bits(self.params[index].load(Ordering::SeqCst)));
        let inactive_reason = self.requested_reason.load(Ordering::SeqCst);
        if self.version.load(Ordering::SeqCst) != version {
            return None;
        }
        Some((
            version,
            DspResolution {
                params,
                inactive_reason,
            },
        ))
    }

    pub fn stats(&self) -> AudioEffectsStats {
        AudioEffectsStats {
            output_device: self
                .output_device
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            active: self.active.load(Ordering::Relaxed),
            cpu_load_percent: f32::from_bits(self.cpu.load(Ordering::Relaxed)),
            limiter_reduction_db: f32::from_bits(self.limiter.load(Ordering::Relaxed)),
            compressor_reduction_db: f32::from_bits(self.compressor.load(Ordering::Relaxed)),
            sample_rate: self.sample_rate.load(Ordering::Relaxed),
            native_available: self.native_available.load(Ordering::Relaxed),
            inactive_reason: match self.reason.load(Ordering::Relaxed) {
                1 => Some("disabled"),
                2 => Some("neutral"),
                3 => Some("unsupported_format"),
                4 => Some("native_unavailable"),
                _ => None,
            },
        }
    }
}

pub struct DspProcessor {
    engine: Option<NonNull<c_void>>,
    control: Arc<DspControl>,
    version: u64,
    requested_reason: u8,
    sample_rate: u32,
    channels: usize,
    stats_frames: usize,
    token: u64,
}

// 引擎从构造后只被所属输出回调使用，不在其它线程共享
unsafe impl Send for DspProcessor {}

impl DspProcessor {
    pub fn new(channels: usize, sample_rate: u32, control: Arc<DspControl>) -> Self {
        let supported = (1..=2).contains(&channels) && (8000..=768000).contains(&sample_rate);
        let engine = if supported {
            unsafe {
                NonNull::new(neri_desktop_dsp_create(
                    sample_rate as i32,
                    channels as i32,
                    neutral_params().as_ptr(),
                ))
            }
        } else {
            None
        };
        let token = control.next_processor.fetch_add(1, Ordering::Relaxed) + 1;
        control
            .current_processor
            .fetch_max(token, Ordering::Relaxed);
        control.sample_rate.store(sample_rate, Ordering::Relaxed);
        control
            .native_available
            .store(!supported || engine.is_some(), Ordering::Relaxed);
        if engine.is_none() {
            control.active.store(false, Ordering::Relaxed);
            control.cpu.store(0, Ordering::Relaxed);
            control.limiter.store(0, Ordering::Relaxed);
            control.compressor.store(0, Ordering::Relaxed);
            control
                .reason
                .store(if supported { 4 } else { 3 }, Ordering::Relaxed);
        }
        Self {
            engine,
            control,
            version: u64::MAX,
            requested_reason: 1,
            sample_rate,
            channels,
            stats_frames: 0,
            token,
        }
    }

    pub fn process(&mut self, samples: &mut [f32]) {
        let owner = self
            .control
            .current_processor
            .fetch_max(self.token, Ordering::Relaxed);
        if owner <= self.token {
            self.control
                .sample_rate
                .store(self.sample_rate, Ordering::Relaxed);
        }
        let Some(engine) = self.engine else {
            return;
        };
        if let Some((version, resolution)) = self.control.snapshot(self.version) {
            // C++ 实例已按质量预建，setParams 不会调用 configure 或重建容量
            let accepted = unsafe {
                neri_desktop_dsp_set_params(
                    engine.as_ptr(),
                    resolution.params.as_ptr(),
                    PARAM_COUNT as i32,
                )
            };
            if accepted != 0 {
                self.version = version;
                self.requested_reason = resolution.inactive_reason;
            }
        }
        let status = unsafe {
            neri_desktop_dsp_process(engine.as_ptr(), samples.as_mut_ptr(), samples.len())
        };
        if self.control.current_processor.load(Ordering::Relaxed) != self.token {
            return;
        }
        let active = status >= 0 && status & 1 == 0;
        self.control.active.store(active, Ordering::Relaxed);
        self.control.reason.store(
            if active { 0 } else { self.requested_reason },
            Ordering::Relaxed,
        );
        self.stats_frames += samples.len() / self.channels.max(1);
        if self.stats_frames >= self.sample_rate.max(1) as usize || !active {
            let mut stats = NativeStats::default();
            unsafe { neri_desktop_dsp_stats(engine.as_ptr(), &mut stats) };
            let cpu = if stats.frames > 0 {
                (stats.nanos.max(0) as f64 * f64::from(self.sample_rate)
                    / (stats.frames as f64 * 10_000_000.0)) as f32
            } else {
                0.0
            };
            self.control.cpu.store(cpu.to_bits(), Ordering::Relaxed);
            self.control
                .limiter
                .store(stats.limiter_reduction_db.to_bits(), Ordering::Relaxed);
            self.control
                .compressor
                .store(stats.compressor_reduction_db.to_bits(), Ordering::Relaxed);
            self.stats_frames = 0;
        }
    }
}

impl Drop for DspProcessor {
    fn drop(&mut self) {
        if let Some(engine) = self.engine {
            unsafe { neri_desktop_dsp_destroy(engine.as_ptr()) };
        }
        if self
            .control
            .current_processor
            .compare_exchange(self.token, 0, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            self.control.sample_rate.store(0, Ordering::Relaxed);
            self.control.active.store(false, Ordering::Relaxed);
            self.control.cpu.store(0, Ordering::Relaxed);
            self.control.limiter.store(0, Ordering::Relaxed);
            self.control.compressor.store(0, Ordering::Relaxed);
            self.control
                .reason
                .store(self.requested_reason, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp_settings::{AudioEffectsProfile, AudioEffectsSound, ParametricEqBand};

    extern "C" {
        fn neri_desktop_dsp_run_host_tests() -> i32;
    }

    fn active_settings() -> AudioEffectsSettings {
        AudioEffectsSettings {
            main: AudioEffectsProfile {
                enabled: true,
                sound: AudioEffectsSound {
                    limiter_enabled: false,
                    ..AudioEffectsSound::default()
                },
                ..AudioEffectsProfile::default()
            },
            auto_headroom: false,
            ..AudioEffectsSettings::default()
        }
    }

    #[test]
    fn parameter_snapshots_never_mix_concurrent_control_updates() {
        let control = DspControl::new_shared();
        let writer = Arc::clone(&control);
        let thread = std::thread::spawn(move || {
            for iteration in 0..1000 {
                let value = if iteration % 2 == 0 { -6.0 } else { 6.0 };
                let mut settings = active_settings();
                settings.main.sound.equalizer_bands_db.fill(value);
                settings.main.sound.output_gain_db = value;
                writer.set_settings(settings, "");
            }
        });
        for _ in 0..5000 {
            if let Some((_, resolved)) = control.snapshot(u64::MAX) {
                let gain = resolved.params[OUTPUT_GAIN_DB];
                assert!(resolved.params[GRAPHIC_EQ_GAIN_0..PARAMETRIC_EQ_ENABLED]
                    .iter()
                    .all(|band| *band == gain));
            }
        }
        thread.join().unwrap();
        let (_, final_packet) = control.snapshot(u64::MAX).unwrap();
        assert_eq!(final_packet.params[OUTPUT_GAIN_DB], 6.0);
    }

    #[test]
    fn previous_callbacks_cannot_clear_a_newer_output_chain_stats() {
        let control = DspControl::new_shared();
        let mut settings = active_settings();
        settings.main.sound.bass_db = 6.0;
        control.set_settings(settings, "");
        let mut previous = DspProcessor::new(2, 44_100, Arc::clone(&control));
        previous.process(&mut [0.1; 960]);
        let mut current = DspProcessor::new(2, 48_000, Arc::clone(&control));
        current.process(&mut [0.1; 960]);
        previous.process(&mut [0.1; 960]);
        drop(previous);
        current.process(&mut [0.1; 960]);
        assert_eq!(control.stats().sample_rate, 48_000);
        assert!(control.stats().active);
    }

    #[test]
    fn settings_updates_prefer_the_selected_actual_output_over_stale_ui_names() {
        let control = DspControl::new_shared();
        let mut settings = active_settings();
        settings.main.sound.output_gain_db = -3.0;
        settings.per_output_enabled = true;
        let mut current = settings.main.clone();
        current.sound.output_gain_db = -9.0;
        settings.output_profiles.insert("Current".into(), current);
        control.select_output("Current");
        control.set_settings(settings, "Previous");
        let (_, resolved) = control.snapshot(u64::MAX).unwrap();
        assert_eq!(resolved.params[OUTPUT_GAIN_DB], -9.0);
    }

    #[test]
    fn reset_preserves_user_presets_and_restores_neutral_processing() {
        let control = DspControl::new_shared();
        let mut settings = active_settings();
        settings.main.sound.bass_db = 12.0;
        settings
            .user_presets
            .push(crate::dsp_settings::AudioEffectsUserPreset {
                id: "saved".into(),
                name: "Saved".into(),
                sound: settings.main.sound.clone(),
            });
        control.set_settings(settings, "");
        control.reset_preserving_user_presets();
        let (_, reset) = control.snapshot(u64::MAX).unwrap();
        assert_eq!(reset.params[MASTER_ENABLED], 0.0);
        assert_eq!(control.settings.lock().unwrap().user_presets[0].id, "saved");
    }

    #[test]
    fn native_packet_validation_rejects_wrong_count_and_nonfinite_parameters() {
        let mut params = neutral_params();
        let engine = unsafe { neri_desktop_dsp_create(48_000, 2, params.as_ptr()) };
        assert!(!engine.is_null());
        assert_eq!(
            unsafe { neri_desktop_dsp_set_params(engine, params.as_ptr(), PARAM_COUNT as i32 - 1) },
            0
        );
        params[PREAMP_DB] = f32::NAN;
        assert_eq!(
            unsafe { neri_desktop_dsp_set_params(engine, params.as_ptr(), PARAM_COUNT as i32) },
            0
        );
        params[PREAMP_DB] = 0.0;
        assert_eq!(
            unsafe { neri_desktop_dsp_set_params(engine, params.as_ptr(), PARAM_COUNT as i32) },
            1
        );
        unsafe { neri_desktop_dsp_destroy(engine) };
    }

    #[test]
    fn preamp_keeps_deep_autoeq_attenuation_after_automatic_headroom() {
        let mut settings = active_settings();
        settings.auto_headroom = true;
        settings.main.sound.preamp_db = -24.0;
        settings.main.sound.equalizer_bands_db[5] = 12.0;
        let normalized = settings.clone().normalized();
        assert_eq!(normalized.main.sound.preamp_db, -24.0);
        let input = sine(1000.0, 1);
        let (output, _) = process(settings, &input, 1);
        let gain = 20.0 * (rms(&output[24_000..]) / rms(&input[24_000..])).log10();
        assert!(
            (gain + 22.2).abs() < 0.6,
            "preamp -24 + EQ12 - headroom10.2: actual {gain}"
        );
    }

    fn sine(frequency: f64, channels: usize) -> Vec<f32> {
        (0..48_000)
            .flat_map(|frame| {
                let value = (0.05
                    * (std::f64::consts::TAU * frequency * frame as f64 / 48_000.0).sin())
                    as f32;
                std::iter::repeat_n(value, channels)
            })
            .collect()
    }

    fn rms(samples: &[f32]) -> f64 {
        (samples
            .iter()
            .map(|sample| f64::from(*sample).powi(2))
            .sum::<f64>()
            / samples.len() as f64)
            .sqrt()
    }

    fn process(
        settings: AudioEffectsSettings,
        input: &[f32],
        channels: usize,
    ) -> (Vec<f32>, AudioEffectsStats) {
        let control = DspControl::new_shared();
        control.set_settings(settings, "");
        let mut processor = DspProcessor::new(channels, 48_000, Arc::clone(&control));
        let mut output = input.to_vec();
        for block in output.chunks_mut(480 * channels) {
            processor.process(block);
        }
        (output, control.stats())
    }

    #[test]
    fn android_host_dsp_regressions_run_through_the_desktop_build() {
        assert_eq!(unsafe { neri_desktop_dsp_run_host_tests() }, 0);
    }

    #[test]
    fn enabled_neutral_and_disabled_settings_are_bit_exact() {
        for enabled in [false, true] {
            let mut settings = AudioEffectsSettings::default();
            settings.main.enabled = enabled;
            let input = sine(900.0, 2);
            let (output, stats) = process(settings, &input, 2);
            assert!(input
                .iter()
                .zip(output)
                .all(|(a, b)| a.to_bits() == b.to_bits()));
            assert!(!stats.active);
            assert_eq!(
                stats.inactive_reason,
                Some(if enabled { "neutral" } else { "disabled" })
            );
        }
    }

    #[test]
    fn ten_band_equalizer_and_parametric_peak_change_real_pcm_spectra() {
        for parametric in [false, true] {
            for channels in [1, 2] {
                let mut settings = active_settings();
                if parametric {
                    settings.main.sound.parametric_enabled = true;
                    settings.main.sound.parametric_bands = vec![ParametricEqBand {
                        gain_db: 6.0,
                        ..ParametricEqBand::default()
                    }];
                } else {
                    settings.main.sound.equalizer_bands_db[5] = 6.0;
                }
                let input = sine(1000.0, channels);
                let (output, stats) = process(settings, &input, channels);
                let tail = 24_000 * channels;
                let gain = 20.0 * (rms(&output[tail..]) / rms(&input[tail..])).log10();
                assert!(
                    (gain - 6.0).abs() < 0.6,
                    "parametric={parametric}, channels={channels}, gain={gain}"
                );
                assert!(stats.active && stats.native_available);
                assert!(stats.cpu_load_percent.is_finite());
                assert_eq!(stats.sample_rate, 48_000);
            }
        }
    }

    #[test]
    fn parametric_eq_preserves_legacy_thirty_db_cuts_in_real_pcm() {
        let mut settings = active_settings();
        settings.main.sound.parametric_enabled = true;
        settings.main.sound.parametric_bands = vec![ParametricEqBand {
            gain_db: -30.0,
            ..ParametricEqBand::default()
        }];
        let input = sine(1000.0, 2);
        let (output, _) = process(settings, &input, 2);
        assert!(output.iter().all(|sample| sample.is_finite()));
        let gain = 20.0 * (rms(&output[48_000..]) / rms(&input[48_000..])).log10();
        assert!((gain + 30.0).abs() < 0.6, "legacy cut actual {gain} dB");
    }

    #[test]
    fn each_parametric_filter_type_affects_the_expected_frequency() {
        for (kind, frequency, minimum, maximum) in [
            ("peak", 1000.0, 5.5, 6.5),
            ("low_shelf", 50.0, 5.0, 7.0),
            ("high_shelf", 10_000.0, 5.0, 7.0),
            ("low_pass", 8000.0, -100.0, -20.0),
            ("high_pass", 50.0, -100.0, -20.0),
            ("notch", 1000.0, f64::NEG_INFINITY, -30.0),
            ("band_pass", 50.0, -100.0, -15.0),
        ] {
            let mut settings = active_settings();
            settings.main.sound.parametric_enabled = true;
            settings.main.sound.parametric_bands = vec![ParametricEqBand {
                r#type: kind.into(),
                gain_db: 6.0,
                ..ParametricEqBand::default()
            }];
            let input = sine(frequency, 1);
            let (output, _) = process(settings, &input, 1);
            let gain = 20.0 * (rms(&output[24_000..]) / rms(&input[24_000..])).log10();
            assert!((minimum..=maximum).contains(&gain), "{kind}: {gain} dB");
        }
    }

    #[test]
    fn disabling_the_master_fades_then_reaches_exact_bypass() {
        let control = DspControl::new_shared();
        let mut settings = active_settings();
        settings.main.sound.bass_db = 6.0;
        settings.power_mode = "high".into();
        control.set_settings(settings.clone(), "");
        let mut processor = DspProcessor::new(2, 48_000, Arc::clone(&control));
        for _ in 0..50 {
            processor.process(&mut [0.1; 960]);
        }
        settings.main.enabled = false;
        control.set_settings(settings, "");
        processor.process(&mut [0.1; 960]);
        assert!(control.stats().active, "禁用后仍处理短淡出");
        for _ in 0..100 {
            processor.process(&mut [0.1; 960]);
        }
        let mut input = [0.125; 960];
        processor.process(&mut input);
        assert_eq!(input, [0.125; 960]);
        assert!(!control.stats().active);
        assert_eq!(control.stats().inactive_reason, Some("disabled"));
    }

    #[test]
    fn switching_quality_does_not_replay_previous_reverb_tails() {
        let control = DspControl::new_shared();
        let mut settings = active_settings();
        settings.main.sound.reverb = 1.0;
        settings.main.sound.surround = 1.0;
        control.set_settings(settings.clone(), "");
        let mut processor = DspProcessor::new(2, 48_000, Arc::clone(&control));
        for quality in ["balanced", "eco", "high", "balanced"] {
            for _ in 0..30 {
                processor.process(&mut [0.1; 960]);
            }
            settings.power_mode = quality.into();
            control.set_settings(settings.clone(), "");
            let mut zeros = [0.0; 960];
            processor.process(&mut zeros);
            // 第一次相同模式没有重置，后面的真实切换必须清空历史
            if quality != "balanced" || processor.version > 4 {
                assert!(zeros.iter().all(|sample| *sample == 0.0));
            }
        }
    }

    #[test]
    fn unsupported_multichannel_is_bit_exact_and_reports_why() {
        let mut settings = active_settings();
        settings.main.sound.bass_db = 12.0;
        let input = sine(100.0, 6);
        let (output, stats) = process(settings, &input, 6);
        assert_eq!(output, input);
        assert_eq!(stats.inactive_reason, Some("unsupported_format"));
        assert!(!stats.active && stats.native_available);
    }

    #[test]
    fn limiter_and_compressor_report_actual_gain_reduction() {
        let mut settings = active_settings();
        settings.main.sound.output_gain_db = 12.0;
        settings.main.sound.limiter_enabled = true;
        settings.main.sound.limiter_ceiling_db = -3.0;
        settings.main.sound.compressor_enabled = true;
        settings.main.sound.compressor_threshold_db = -40.0;
        settings.main.sound.compressor_ratio = 10.0;
        settings.main.sound.compressor_makeup_db = 18.0;
        let input: Vec<_> = sine(400.0, 2)
            .into_iter()
            .map(|sample| sample * 10.0)
            .collect();
        let (output, stats) = process(settings, &input, 2);
        assert!(
            output
                .iter()
                .all(|sample| sample.is_finite()
                    && sample.abs() <= 10.0f32.powf(-3.0 / 20.0) + 0.0001)
        );
        assert!(stats.limiter_reduction_db > 0.0 && stats.compressor_reduction_db > 0.0);
    }
}
