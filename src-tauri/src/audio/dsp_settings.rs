use super::dsp::params::*;
use super::dsp::{neutral_params, DspResolution, PARAM_COUNT};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ParametricEqBand {
    pub enabled: bool,
    pub r#type: String,
    pub frequency_hz: f32,
    pub gain_db: f32,
    pub q: f32,
}
impl Default for ParametricEqBand {
    fn default() -> Self {
        Self {
            enabled: true,
            r#type: "peak".into(),
            frequency_hz: 1_000.0,
            gain_db: 0.0,
            q: 0.707,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioEffectsSound {
    pub preamp_db: f32,
    pub equalizer_enabled: bool,
    pub equalizer_bands_db: Vec<f32>,
    /// 桌面扩展：补偿相邻频段叠加，让各中心频率的实际频响等于滑块值
    pub equalizer_accurate: bool,
    pub parametric_enabled: bool,
    pub parametric_bands: Vec<ParametricEqBand>,
    pub bass_db: f32,
    pub bass_frequency_hz: f32,
    pub treble_db: f32,
    pub treble_frequency_hz: f32,
    /// 桌面扩展：随播放器音量变化的等响补偿强度，0–1
    pub loudness_compensation: f32,
    pub virtual_bass: f32,
    pub virtual_bass_frequency_hz: f32,
    pub warmth: f32,
    pub clarity: f32,
    pub clarity_frequency_hz: f32,
    pub vocal: f32,
    pub vocal_removal: f32,
    pub stereo_width: f32,
    pub mono_bass_hz: f32,
    pub mono: bool,
    pub swap_channels: bool,
    pub crossfeed: f32,
    pub surround: f32,
    pub reverb: f32,
    pub reverb_room: f32,
    pub reverb_damping: f32,
    pub reverb_pre_delay_ms: f32,
    pub compressor_enabled: bool,
    pub compressor_threshold_db: f32,
    pub compressor_ratio: f32,
    pub compressor_attack_ms: f32,
    pub compressor_release_ms: f32,
    pub compressor_knee_db: f32,
    pub compressor_makeup_db: f32,
    pub output_gain_db: f32,
    pub limiter_enabled: bool,
    pub limiter_ceiling_db: f32,
    pub limiter_release_ms: f32,
    pub tone_enabled: bool,
    pub space_enabled: bool,
    pub dynamics_enabled: bool,
}
impl Default for AudioEffectsSound {
    fn default() -> Self {
        Self {
            preamp_db: 0.0,
            equalizer_enabled: true,
            equalizer_bands_db: vec![0.0; GRAPHIC_BAND_COUNT],
            equalizer_accurate: false,
            parametric_enabled: false,
            parametric_bands: Vec::new(),
            bass_db: 0.0,
            bass_frequency_hz: 100.0,
            treble_db: 0.0,
            treble_frequency_hz: 8_000.0,
            loudness_compensation: 0.0,
            virtual_bass: 0.0,
            virtual_bass_frequency_hz: 100.0,
            warmth: 0.0,
            clarity: 0.0,
            clarity_frequency_hz: 5_000.0,
            vocal: 0.0,
            vocal_removal: 0.0,
            stereo_width: 1.0,
            mono_bass_hz: 0.0,
            mono: false,
            swap_channels: false,
            crossfeed: 0.0,
            surround: 0.0,
            reverb: 0.0,
            reverb_room: 0.5,
            reverb_damping: 0.5,
            reverb_pre_delay_ms: 10.0,
            compressor_enabled: false,
            compressor_threshold_db: -18.0,
            compressor_ratio: 2.0,
            compressor_attack_ms: 10.0,
            compressor_release_ms: 150.0,
            compressor_knee_db: 6.0,
            compressor_makeup_db: 0.0,
            output_gain_db: 0.0,
            limiter_enabled: true,
            limiter_ceiling_db: -1.0,
            limiter_release_ms: 80.0,
            tone_enabled: true,
            space_enabled: true,
            dynamics_enabled: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioEffectsProfile {
    pub enabled: bool,
    pub preset_id: String,
    pub sound: AudioEffectsSound,
}
impl Default for AudioEffectsProfile {
    fn default() -> Self {
        Self {
            enabled: false,
            preset_id: "flat".into(),
            sound: AudioEffectsSound::default(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioEffectsUserPreset {
    pub id: String,
    pub name: String,
    pub sound: AudioEffectsSound,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SpeakerOptimizerSettings {
    pub enabled: bool,
    pub size: String,
    pub protection_hz: f32,
    pub bass_harmonics: f32,
    pub loudness: f32,
    pub clarity: f32,
    pub stereo_expand: f32,
}
impl Default for SpeakerOptimizerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            size: "phone".into(),
            protection_hz: 0.0,
            bass_harmonics: 0.5,
            loudness: 0.4,
            clarity: 0.3,
            stereo_expand: 0.3,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioEffectsSettings {
    pub version: u32,
    pub main: AudioEffectsProfile,
    pub per_output_enabled: bool,
    pub output_profiles: BTreeMap<String, AudioEffectsProfile>,
    pub user_presets: Vec<AudioEffectsUserPreset>,
    pub speaker: SpeakerOptimizerSettings,
    pub power_mode: String,
    pub auto_headroom: bool,
}
impl Default for AudioEffectsSettings {
    fn default() -> Self {
        Self {
            version: 2,
            main: AudioEffectsProfile::default(),
            per_output_enabled: false,
            output_profiles: BTreeMap::new(),
            user_presets: Vec::new(),
            speaker: SpeakerOptimizerSettings::default(),
            power_mode: "balanced".into(),
            auto_headroom: true,
        }
    }
}

impl ParametricEqBand {
    pub fn normalize(&mut self) {
        self.frequency_hz = finite(self.frequency_hz, 20.0, 20000.0, 1000.0);
        self.gain_db = finite(self.gain_db, -30.0, 24.0, 0.0);
        self.q = finite(self.q, 0.1, 20.0, 0.707);
        if ![
            "peak",
            "low_shelf",
            "high_shelf",
            "low_pass",
            "high_pass",
            "notch",
            "band_pass",
        ]
        .contains(&self.r#type.as_str())
        {
            self.r#type = "peak".into();
        }
    }
}

impl AudioEffectsSound {
    pub fn normalize(&mut self) {
        self.preamp_db = finite(self.preamp_db, -24.0, 6.0, 0.0);
        self.bass_db = finite(self.bass_db, -12.0, 12.0, 0.0);
        self.bass_frequency_hz = finite(self.bass_frequency_hz, 40.0, 250.0, 100.0);
        self.treble_db = finite(self.treble_db, -12.0, 12.0, 0.0);
        self.treble_frequency_hz = finite(self.treble_frequency_hz, 2000.0, 14000.0, 8000.0);
        self.loudness_compensation = finite(self.loudness_compensation, 0.0, 1.0, 0.0);
        self.virtual_bass = finite(self.virtual_bass, 0.0, 1.0, 0.0);
        self.virtual_bass_frequency_hz = finite(self.virtual_bass_frequency_hz, 50.0, 200.0, 100.0);
        self.warmth = finite(self.warmth, 0.0, 1.0, 0.0);
        self.clarity = finite(self.clarity, 0.0, 1.0, 0.0);
        self.clarity_frequency_hz = finite(self.clarity_frequency_hz, 2000.0, 10000.0, 5000.0);
        self.vocal = finite(self.vocal, 0.0, 1.0, 0.0);
        self.vocal_removal = finite(self.vocal_removal, 0.0, 1.0, 0.0);
        self.stereo_width = finite(self.stereo_width, 0.0, 2.0, 1.0);
        self.crossfeed = finite(self.crossfeed, 0.0, 1.0, 0.0);
        self.surround = finite(self.surround, 0.0, 1.0, 0.0);
        self.reverb = finite(self.reverb, 0.0, 1.0, 0.0);
        self.reverb_room = finite(self.reverb_room, 0.0, 1.0, 0.5);
        self.reverb_damping = finite(self.reverb_damping, 0.0, 1.0, 0.5);
        self.reverb_pre_delay_ms = finite(self.reverb_pre_delay_ms, 0.0, 100.0, 10.0);
        self.compressor_threshold_db = finite(self.compressor_threshold_db, -50.0, 0.0, -18.0);
        self.compressor_ratio = finite(self.compressor_ratio, 1.0, 20.0, 2.0);
        self.compressor_attack_ms = finite(self.compressor_attack_ms, 0.5, 100.0, 10.0);
        self.compressor_release_ms = finite(self.compressor_release_ms, 20.0, 1000.0, 150.0);
        self.compressor_knee_db = finite(self.compressor_knee_db, 0.0, 18.0, 6.0);
        self.compressor_makeup_db = finite(self.compressor_makeup_db, 0.0, 18.0, 0.0);
        self.output_gain_db = finite(self.output_gain_db, -12.0, 12.0, 0.0);
        self.limiter_ceiling_db = finite(self.limiter_ceiling_db, -6.0, 0.0, -1.0);
        self.limiter_release_ms = finite(self.limiter_release_ms, 20.0, 500.0, 80.0);
        self.equalizer_bands_db.truncate(GRAPHIC_BAND_COUNT);
        self.equalizer_bands_db.resize(GRAPHIC_BAND_COUNT, 0.0);
        for gain in &mut self.equalizer_bands_db {
            *gain = finite(*gain, -12.0, 12.0, 0.0);
        }
        self.parametric_bands.truncate(PARAMETRIC_BAND_COUNT);
        for band in &mut self.parametric_bands {
            band.normalize();
        }
        self.mono_bass_hz = optional_frequency(self.mono_bass_hz, 250.0);
    }
}

impl SpeakerOptimizerSettings {
    pub fn normalize(&mut self) {
        self.bass_harmonics = finite(self.bass_harmonics, 0.0, 1.0, 0.5);
        self.loudness = finite(self.loudness, 0.0, 1.0, 0.4);
        self.clarity = finite(self.clarity, 0.0, 1.0, 0.3);
        self.stereo_expand = finite(self.stereo_expand, 0.0, 1.0, 0.3);
        if !["phone", "tablet", "large"].contains(&self.size.as_str()) {
            self.size = "phone".into();
        }
        self.protection_hz = optional_frequency(self.protection_hz, 400.0);
    }
}

fn finite(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}
fn optional_frequency(value: f32, max: f32) -> f32 {
    if value.is_finite() && value >= 40.0 {
        value.min(max)
    } else {
        0.0
    }
}

impl AudioEffectsSettings {
    pub fn normalized(mut self) -> Self {
        self.version = 2;
        self.main.normalize();
        self.output_profiles = self
            .output_profiles
            .into_iter()
            .take(64)
            .map(|(name, mut profile)| {
                profile.normalize();
                (output_key(&name), profile)
            })
            .collect();
        let mut ids = BTreeSet::new();
        self.user_presets = self
            .user_presets
            .into_iter()
            .filter_map(|mut preset| {
                preset.id = preset.id.trim().chars().take(128).collect();
                preset.name = preset.name.trim().chars().take(24).collect();
                if preset.id.is_empty() || preset.name.is_empty() || !ids.insert(preset.id.clone())
                {
                    return None;
                }
                preset.sound.normalize();
                Some(preset)
            })
            .take(50)
            .collect();
        self.speaker.normalize();
        if !["eco", "balanced", "high"].contains(&self.power_mode.as_str()) {
            self.power_mode = "balanced".into();
        }
        self
    }

    fn profile_for(&self, output_device: &str) -> &AudioEffectsProfile {
        if self.per_output_enabled {
            self.output_profiles
                .get(&output_key(output_device))
                .or_else(|| self.output_profiles.get("system_default"))
                .unwrap_or(&self.main)
        } else {
            &self.main
        }
    }

    pub fn uses_loudness_compensation(&self, output_device: &str) -> bool {
        let profile = self.profile_for(output_device);
        profile.enabled && profile.sound.effective().loudness_compensation > 0.0
    }

    pub fn resolve(&self, output_device: &str) -> DspResolution {
        self.resolve_with_volume(output_device, 1.0)
    }

    pub fn resolve_with_volume(&self, output_device: &str, volume: f32) -> DspResolution {
        let profile = self.profile_for(output_device);
        let effective = profile.sound.effective();
        let sound_active = profile.enabled && !effective.is_neutral();
        let speaker_active = profile.enabled && self.speaker.enabled;
        let mut params = neutral_params();
        params[QUALITY_MODE] = match self.power_mode.as_str() {
            "eco" => 0.0,
            "high" => 2.0,
            _ => 1.0,
        };
        if !sound_active && !speaker_active {
            return DspResolution {
                params,
                inactive_reason: if profile.enabled { 2 } else { 1 },
            };
        }
        params[MASTER_ENABLED] = 1.0;
        params[DITHER_ENABLED] = u8::from(self.power_mode != "eco") as f32;
        let sound = if sound_active {
            effective
        } else {
            AudioEffectsSound::default()
        };
        sound.write_params(&mut params);
        let (loudness_bass, loudness_treble) =
            loudness_compensation_db(sound.loudness_compensation, volume);
        params[LOUDNESS_BASS_DB] = loudness_bass;
        params[LOUDNESS_TREBLE_DB] = loudness_treble;
        if self.auto_headroom {
            params[PREAMP_DB] -= super::dsp::headroom(&params);
        }
        if speaker_active {
            params[SPEAKER_ENABLED] = 1.0;
            params[SPEAKER_HIGH_PASS_HZ] = if self.speaker.protection_hz >= 40.0 {
                self.speaker.protection_hz
            } else {
                match self.speaker.size.as_str() {
                    "tablet" => 120.0,
                    "large" => 70.0,
                    _ => 180.0,
                }
            };
            params[SPEAKER_BASS_HARMONICS] = self.speaker.bass_harmonics;
            params[SPEAKER_LOUDNESS] = self.speaker.loudness;
            params[SPEAKER_CLARITY] = self.speaker.clarity;
            params[SPEAKER_STEREO_EXPAND] = self.speaker.stereo_expand;
        }
        DspResolution {
            params,
            inactive_reason: 0,
        }
    }
}

pub fn output_key(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        "system_default".into()
    } else {
        name.chars().take(256).collect()
    }
}

impl AudioEffectsProfile {
    fn normalize(&mut self) {
        self.preset_id = self.preset_id.trim().chars().take(128).collect();
        if self.preset_id.is_empty() {
            self.preset_id = "flat".into();
        }
        self.sound.normalize();
    }
}

impl AudioEffectsSound {
    fn effective(&self) -> Self {
        let mut sound = self.clone();
        let defaults = Self::default();
        if !self.equalizer_enabled {
            sound.preamp_db = defaults.preamp_db;
            sound.equalizer_enabled = defaults.equalizer_enabled;
            sound.equalizer_bands_db = defaults.equalizer_bands_db;
            sound.equalizer_accurate = defaults.equalizer_accurate;
            sound.parametric_enabled = defaults.parametric_enabled;
            sound.parametric_bands = defaults.parametric_bands;
        }
        if !self.tone_enabled {
            sound.tone_enabled = defaults.tone_enabled;
            sound.bass_db = defaults.bass_db;
            sound.bass_frequency_hz = defaults.bass_frequency_hz;
            sound.treble_db = defaults.treble_db;
            sound.treble_frequency_hz = defaults.treble_frequency_hz;
            sound.loudness_compensation = defaults.loudness_compensation;
            sound.virtual_bass = defaults.virtual_bass;
            sound.virtual_bass_frequency_hz = defaults.virtual_bass_frequency_hz;
            sound.warmth = defaults.warmth;
            sound.clarity = defaults.clarity;
            sound.clarity_frequency_hz = defaults.clarity_frequency_hz;
            sound.vocal = defaults.vocal;
            sound.vocal_removal = defaults.vocal_removal;
        }
        if !self.space_enabled {
            sound.space_enabled = defaults.space_enabled;
            sound.stereo_width = defaults.stereo_width;
            sound.mono_bass_hz = defaults.mono_bass_hz;
            sound.mono = defaults.mono;
            sound.swap_channels = defaults.swap_channels;
            sound.crossfeed = defaults.crossfeed;
            sound.surround = defaults.surround;
            sound.reverb = defaults.reverb;
            sound.reverb_room = defaults.reverb_room;
            sound.reverb_damping = defaults.reverb_damping;
            sound.reverb_pre_delay_ms = defaults.reverb_pre_delay_ms;
        }
        if !self.dynamics_enabled {
            sound.dynamics_enabled = defaults.dynamics_enabled;
            sound.compressor_enabled = defaults.compressor_enabled;
            sound.compressor_threshold_db = defaults.compressor_threshold_db;
            sound.compressor_ratio = defaults.compressor_ratio;
            sound.compressor_attack_ms = defaults.compressor_attack_ms;
            sound.compressor_release_ms = defaults.compressor_release_ms;
            sound.compressor_knee_db = defaults.compressor_knee_db;
            sound.compressor_makeup_db = defaults.compressor_makeup_db;
            sound.output_gain_db = defaults.output_gain_db;
            sound.limiter_enabled = defaults.limiter_enabled;
            sound.limiter_ceiling_db = defaults.limiter_ceiling_db;
            sound.limiter_release_ms = defaults.limiter_release_ms;
        }
        sound
    }

    fn is_neutral(&self) -> bool {
        let zero = |value: f32| value.abs() < 0.001;
        (!self.equalizer_enabled || self.equalizer_bands_db.iter().copied().all(zero))
            && (!self.parametric_enabled
                || self.parametric_bands.iter().all(|band| {
                    !band.enabled
                        || (["peak", "low_shelf", "high_shelf"].contains(&band.r#type.as_str())
                            && zero(band.gain_db))
                }))
            && [
                self.preamp_db,
                self.bass_db,
                self.treble_db,
                self.loudness_compensation,
                self.virtual_bass,
                self.warmth,
                self.clarity,
                self.vocal,
                self.vocal_removal,
                self.stereo_width - 1.0,
                self.mono_bass_hz,
                self.crossfeed,
                self.surround,
                self.reverb,
                self.output_gain_db,
            ]
            .into_iter()
            .all(zero)
            && !self.mono
            && !self.swap_channels
            && !self.compressor_enabled
            && (!self.limiter_enabled
                || (zero(self.limiter_ceiling_db + 1.0) && zero(self.limiter_release_ms - 80.0)))
    }

    fn write_params(&self, p: &mut [f32; PARAM_COUNT]) {
        p[PREAMP_DB] = self.preamp_db;
        p[OUTPUT_GAIN_DB] = self.output_gain_db;
        p[LIMITER_ENABLED] = u8::from(self.limiter_enabled) as f32;
        p[LIMITER_CEILING_DB] = self.limiter_ceiling_db;
        p[LIMITER_RELEASE_MS] = self.limiter_release_ms;
        p[GRAPHIC_EQ_ENABLED] = u8::from(self.equalizer_enabled) as f32;
        for (index, gain) in graphic_eq_design_gains(&self.equalizer_bands_db, self.equalizer_accurate)
            .into_iter()
            .enumerate()
        {
            p[GRAPHIC_EQ_GAIN_0 + index] = gain;
        }
        p[PARAMETRIC_EQ_ENABLED] = u8::from(self.parametric_enabled) as f32;
        for (index, band) in self
            .parametric_bands
            .iter()
            .take(PARAMETRIC_BAND_COUNT)
            .enumerate()
        {
            let base = PARAMETRIC_BAND_0 + index * PARAMETRIC_BAND_STRIDE;
            p[base + BAND_ENABLED] = u8::from(band.enabled) as f32;
            p[base + BAND_TYPE] = match band.r#type.as_str() {
                "low_shelf" => 1.0,
                "high_shelf" => 2.0,
                "low_pass" => 3.0,
                "high_pass" => 4.0,
                "notch" => 5.0,
                "band_pass" => 6.0,
                _ => 0.0,
            };
            p[base + BAND_FREQUENCY_HZ] = band.frequency_hz;
            p[base + BAND_GAIN_DB] = band.gain_db;
            p[base + BAND_Q] = band.q;
        }
        p[BASS_GAIN_DB] = self.bass_db;
        p[BASS_FREQUENCY_HZ] = self.bass_frequency_hz;
        p[TREBLE_GAIN_DB] = self.treble_db;
        p[TREBLE_FREQUENCY_HZ] = self.treble_frequency_hz;
        p[VIRTUAL_BASS_AMOUNT] = self.virtual_bass;
        p[VIRTUAL_BASS_FREQUENCY_HZ] = self.virtual_bass_frequency_hz;
        p[WARMTH_AMOUNT] = self.warmth;
        p[EXCITER_AMOUNT] = self.clarity;
        p[EXCITER_FREQUENCY_HZ] = self.clarity_frequency_hz;
        p[VOCAL_CLARITY_AMOUNT] = self.vocal;
        p[VOCAL_REMOVAL_AMOUNT] = self.vocal_removal;
        p[STEREO_WIDTH] = self.stereo_width;
        p[MONO_BASS_FREQUENCY_HZ] = self.mono_bass_hz;
        p[MONO_MIX] = u8::from(self.mono) as f32;
        p[CHANNEL_SWAP] = u8::from(self.swap_channels) as f32;
        p[CROSSFEED_AMOUNT] = self.crossfeed;
        p[SURROUND_AMOUNT] = self.surround;
        p[REVERB_AMOUNT] = self.reverb;
        p[REVERB_ROOM_SIZE] = self.reverb_room;
        p[REVERB_DAMPING] = self.reverb_damping;
        p[REVERB_PRE_DELAY_MS] = self.reverb_pre_delay_ms;
        p[COMPRESSOR_ENABLED] = u8::from(self.compressor_enabled) as f32;
        p[COMPRESSOR_THRESHOLD_DB] = self.compressor_threshold_db;
        p[COMPRESSOR_RATIO] = self.compressor_ratio;
        p[COMPRESSOR_ATTACK_MS] = self.compressor_attack_ms;
        p[COMPRESSOR_RELEASE_MS] = self.compressor_release_ms;
        p[COMPRESSOR_KNEE_DB] = self.compressor_knee_db;
        p[COMPRESSOR_MAKEUP_DB] = self.compressor_makeup_db;
    }
}

// 以下与前端 src/modules/audioEffects/model.ts 的同名算法逐项一致，频响图与实际处理才对得上
const GRAPHIC_EQ_FREQUENCIES_HZ: [f64; GRAPHIC_BAND_COUNT] = [
    31.25, 62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];
const GRAPHIC_EQ_Q: f64 = 1.41;
const GRAPHIC_EQ_NATIVE_LIMIT_DB: f64 = 15.0;
const GRAPHIC_EQ_PROBE_DB: f64 = 6.0;
const GRAPHIC_EQ_REGULARIZATION: f64 = 0.01;
const GRAPHIC_EQ_REFINEMENTS: usize = 3;
const RESPONSE_SAMPLE_RATE: f64 = 48_000.0;

type Matrix = [[f64; GRAPHIC_BAND_COUNT]; GRAPHIC_BAND_COUNT];

fn peak_magnitude_db(center_hz: f64, gain_db: f64, q: f64, frequency_hz: f64) -> f64 {
    let center = center_hz.clamp(5.0, RESPONSE_SAMPLE_RATE * 0.45);
    let omega = std::f64::consts::TAU * center / RESPONSE_SAMPLE_RATE;
    let (sin, cos) = omega.sin_cos();
    let alpha = sin / (2.0 * q.clamp(0.1, 24.0));
    let a = 10f64.powf(gain_db / 40.0);
    let a0 = 1.0 + alpha / a;
    let (b0, b1, b2) = ((1.0 + alpha * a) / a0, -2.0 * cos / a0, (1.0 - alpha * a) / a0);
    let (a1, a2) = (-2.0 * cos / a0, (1.0 - alpha / a) / a0);
    let w = std::f64::consts::TAU * frequency_hz / RESPONSE_SAMPLE_RATE;
    let (w1s, w1c, w2s, w2c) = (w.sin(), w.cos(), (2.0 * w).sin(), (2.0 * w).cos());
    let numerator = (b0 + b1 * w1c + b2 * w2c).powi(2) + (b1 * w1s + b2 * w2s).powi(2);
    let denominator = (1.0 + a1 * w1c + a2 * w2c).powi(2) + (a1 * w1s + a2 * w2s).powi(2);
    if numerator <= 0.0 || denominator <= 0.0 {
        -120.0
    } else {
        10.0 * (numerator / denominator).log10()
    }
}

fn graphic_eq_interaction() -> &'static Matrix {
    static INTERACTION: std::sync::OnceLock<Matrix> = std::sync::OnceLock::new();
    INTERACTION.get_or_init(|| {
        std::array::from_fn(|center| {
            std::array::from_fn(|band| {
                peak_magnitude_db(
                    GRAPHIC_EQ_FREQUENCIES_HZ[band],
                    GRAPHIC_EQ_PROBE_DB,
                    GRAPHIC_EQ_Q,
                    GRAPHIC_EQ_FREQUENCIES_HZ[center],
                ) / GRAPHIC_EQ_PROBE_DB
            })
        })
    })
}

fn graphic_center_response(gains: &[f64; GRAPHIC_BAND_COUNT]) -> [f64; GRAPHIC_BAND_COUNT] {
    std::array::from_fn(|center| {
        gains
            .iter()
            .enumerate()
            .map(|(band, gain)| {
                peak_magnitude_db(
                    GRAPHIC_EQ_FREQUENCIES_HZ[band],
                    *gain,
                    GRAPHIC_EQ_Q,
                    GRAPHIC_EQ_FREQUENCIES_HZ[center],
                )
            })
            .sum()
    })
}

// 正则化最小二乘：(BᵀB + λI)x = Bᵀr，λ 防止相邻段为凑精度互相拉出极端的正负增益
fn solve_graphic_correction(residual: &[f64; GRAPHIC_BAND_COUNT]) -> [f64; GRAPHIC_BAND_COUNT] {
    const N: usize = GRAPHIC_BAND_COUNT;
    let interaction = graphic_eq_interaction();
    let mut matrix = [[0.0f64; N + 1]; N];
    for row in 0..N {
        for column in 0..N {
            let value: f64 = (0..N).map(|k| interaction[k][row] * interaction[k][column]).sum();
            matrix[row][column] = if row == column {
                value + GRAPHIC_EQ_REGULARIZATION
            } else {
                value
            };
        }
        matrix[row][N] = (0..N).map(|k| interaction[k][row] * residual[k]).sum();
    }
    for pivot in 0..N {
        let best = (pivot..N)
            .max_by(|a, b| matrix[*a][pivot].abs().total_cmp(&matrix[*b][pivot].abs()))
            .unwrap_or(pivot);
        matrix.swap(pivot, best);
        for row in 0..N {
            if row == pivot {
                continue;
            }
            let factor = matrix[row][pivot] / matrix[pivot][pivot];
            for column in pivot..=N {
                matrix[row][column] -= factor * matrix[pivot][column];
            }
        }
    }
    std::array::from_fn(|index| matrix[index][N] / matrix[index][index])
}

/// 送进原生图形均衡的实际增益；精确模式下让各中心频率的合成频响等于滑块值
pub fn graphic_eq_design_gains(targets: &[f32], accurate: bool) -> [f32; GRAPHIC_BAND_COUNT] {
    let gains: [f64; GRAPHIC_BAND_COUNT] = std::array::from_fn(|index| {
        targets
            .get(index)
            .copied()
            .filter(|gain| gain.is_finite())
            .map_or(0.0, f64::from)
    });
    if !accurate || gains.iter().all(|gain| *gain == 0.0) {
        return gains.map(|gain| gain as f32);
    }
    let clamp = |value: f64| value.clamp(-GRAPHIC_EQ_NATIVE_LIMIT_DB, GRAPHIC_EQ_NATIVE_LIMIT_DB);
    let mut design = solve_graphic_correction(&gains).map(clamp);
    // 峰值滤波器的 dB 响应只是近似线性，用真实频响再修正几轮
    for _ in 0..GRAPHIC_EQ_REFINEMENTS {
        let response = graphic_center_response(&design);
        let residual = std::array::from_fn(|index| gains[index] - response[index]);
        let correction = solve_graphic_correction(&residual);
        design = std::array::from_fn(|index| clamp(design[index] + correction[index]));
    }
    design.map(|gain| gain as f32)
}

/// 响度补偿：音量每降 1 dB，低频补约 0.3 dB、高频补约 0.1 dB，分别封顶 10 dB / 4 dB
pub fn loudness_compensation_db(amount: f32, volume: f32) -> (f32, f32) {
    let strength = if amount.is_finite() { amount.clamp(0.0, 1.0) } else { 0.0 };
    if strength <= 0.0 {
        return (0.0, 0.0);
    }
    let linear = if volume.is_finite() { volume.clamp(0.001, 1.0) } else { 1.0 };
    let attenuation = (-20.0 * linear.log10()).clamp(0.0, 40.0);
    (
        strength * (attenuation * 0.3).min(10.0),
        strength * (attenuation * 0.1).min(4.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_clamps_all_native_boundaries_and_keeps_saved_sections() {
        let mut settings = AudioEffectsSettings {
            version: 1,
            ..AudioEffectsSettings::default()
        };
        settings.main.sound.preamp_db = f32::INFINITY;
        settings.main.sound.equalizer_bands_db = vec![30.0, f32::NAN, -30.0];
        settings.main.sound.parametric_bands = vec![
            ParametricEqBand {
                frequency_hz: 90_000.0,
                gain_db: -100.0,
                q: 0.0,
                r#type: "unknown".into(),
                ..ParametricEqBand::default()
            };
            12
        ];
        settings.main.sound.mono_bass_hz = 20.0;
        settings.main.sound.bass_db = 7.0;
        settings.main.sound.tone_enabled = false;
        settings.speaker.protection_hz = 1000.0;
        settings.power_mode = "unknown".into();
        let normalized = settings.normalized();
        let sound = &normalized.main.sound;
        assert_eq!(normalized.version, 2);
        assert_eq!(normalized.power_mode, "balanced");
        assert_eq!(sound.preamp_db, 0.0);
        assert_eq!(&sound.equalizer_bands_db[..3], &[12.0, 0.0, -12.0]);
        assert_eq!(sound.equalizer_bands_db.len(), 10);
        assert_eq!(sound.parametric_bands.len(), 10);
        assert_eq!(sound.parametric_bands[0].frequency_hz, 20_000.0);
        assert_eq!(sound.parametric_bands[0].gain_db, -30.0);
        assert_eq!(sound.parametric_bands[0].q, 0.1);
        assert_eq!(sound.parametric_bands[0].r#type, "peak");
        assert_eq!(normalized.speaker.protection_hz, 400.0);
        assert_eq!(sound.mono_bass_hz, 0.0);
        assert_eq!(sound.bass_db, 7.0);
        assert_eq!(sound.effective().bass_db, 0.0);
    }

    #[test]
    fn each_disabled_section_restores_android_effective_defaults() {
        let mut sound = AudioEffectsSound {
            equalizer_enabled: false,
            preamp_db: 4.0,
            equalizer_bands_db: vec![6.0; 10],
            parametric_enabled: true,
            parametric_bands: vec![ParametricEqBand::default()],
            tone_enabled: false,
            bass_db: 6.0,
            bass_frequency_hz: 200.0,
            virtual_bass: 1.0,
            vocal_removal: 1.0,
            space_enabled: false,
            stereo_width: 0.0,
            reverb: 1.0,
            dynamics_enabled: false,
            output_gain_db: 12.0,
            compressor_enabled: true,
            compressor_threshold_db: -40.0,
            limiter_ceiling_db: -6.0,
            ..AudioEffectsSound::default()
        };
        let effective = sound.effective();
        assert_eq!(effective, AudioEffectsSound::default());
        assert_eq!(sound.bass_db, 6.0);
        sound.space_enabled = true;
        assert_eq!(sound.effective().reverb, 1.0);
    }

    #[test]
    fn output_profiles_select_actual_name_then_system_default_then_main() {
        let mut settings = AudioEffectsSettings::default();
        settings.main.enabled = true;
        settings.main.sound.output_gain_db = -3.0;
        settings.per_output_enabled = true;
        let mut system = settings.main.clone();
        system.sound.output_gain_db = -6.0;
        let mut named = settings.main.clone();
        named.sound.output_gain_db = -9.0;
        settings
            .output_profiles
            .insert("system_default".into(), system);
        settings.output_profiles.insert("Headphones".into(), named);
        assert_eq!(settings.resolve("Headphones").params[OUTPUT_GAIN_DB], -9.0);
        assert_eq!(settings.resolve("Speakers").params[OUTPUT_GAIN_DB], -6.0);
        settings.output_profiles.clear();
        assert_eq!(settings.resolve("Headphones").params[OUTPUT_GAIN_DB], -3.0);
    }

    #[test]
    fn bass_headroom_matches_android_without_reducing_static_cuts() {
        let mut settings = AudioEffectsSettings::default();
        settings.main.enabled = true;
        settings.main.sound.bass_db = 6.0;
        let boosted = settings.resolve("").params;
        assert!(
            (boosted[PREAMP_DB] + 1.25).abs() < 0.15,
            "preamp {}",
            boosted[PREAMP_DB]
        );
        assert_eq!(boosted[BASS_GAIN_DB], 6.0);

        settings.main.sound.bass_db = -6.0;
        let cut = settings.resolve("").params;
        assert_eq!(cut[PREAMP_DB], 0.0);
        assert_eq!(cut[BASS_GAIN_DB], -6.0);

        settings.main.sound.bass_db = 6.0;
        settings.main.sound.loudness_compensation = 1.0;
        let quiet = settings.resolve_with_volume("", 0.1).params;
        assert!(quiet[PREAMP_DB] < boosted[PREAMP_DB] - 4.0);
        assert_eq!(quiet[LOUDNESS_BASS_DB], 6.0);
        settings.auto_headroom = false;
        assert_eq!(settings.resolve("").params[PREAMP_DB], 0.0);
    }

    #[test]
    fn auto_headroom_uses_the_real_combined_filter_response() {
        let mut settings = AudioEffectsSettings::default();
        settings.main.enabled = true;
        settings.main.sound.equalizer_bands_db[5] = 6.0;
        let resolved = settings.resolve("");
        assert_eq!(resolved.params.len(), PARAM_COUNT);
        assert!(resolved.params.iter().all(|value| value.is_finite()));
        assert!(
            (resolved.params[PREAMP_DB] + 5.1).abs() < 0.2,
            "preamp {}",
            resolved.params[PREAMP_DB]
        );
        settings.main.sound.tone_enabled = false;
        settings.main.sound.bass_db = 12.0;
        assert_eq!(settings.resolve("").params[BASS_GAIN_DB], 0.0);
        assert_eq!(settings.main.sound.bass_db, 12.0);
    }

    #[test]
    fn explicit_speaker_processing_works_with_the_master_sound_disabled() {
        let mut settings = AudioEffectsSettings::default();
        settings.main.enabled = true;
        settings.speaker.enabled = true;
        let resolved = settings.resolve("Desk speakers");
        assert_eq!(resolved.params[MASTER_ENABLED], 1.0);
        assert_eq!(resolved.params[SPEAKER_ENABLED], 1.0);
        assert_eq!(resolved.params[SPEAKER_HIGH_PASS_HZ], 180.0);
        assert_eq!(resolved.inactive_reason, 0);
    }

    #[test]
    fn master_disabled_is_original_audio_even_when_speaker_processing_is_saved() {
        let mut settings = AudioEffectsSettings::default();
        settings.speaker.enabled = true;
        assert_eq!(
            settings.resolve("Desk speakers").params[MASTER_ENABLED],
            0.0
        );
        assert!(settings.speaker.enabled);
    }

    #[test]
    fn accurate_graphic_eq_hits_every_band_center_and_stays_off_by_default() {
        let targets = [4.5, 4.0, 2.5, 1.0, -0.5, -0.5, 1.0, 1.0, 1.5, 1.0];
        assert_eq!(graphic_eq_design_gains(&targets, false), targets);
        let design = graphic_eq_design_gains(&targets, true);
        let response = graphic_center_response(&design.map(f64::from));
        for (actual, target) in response.iter().zip(targets) {
            assert!((actual - f64::from(target)).abs() < 0.05, "{response:?}");
        }
        let flat = graphic_eq_design_gains(&[12.0; 10], true);
        let summed = graphic_center_response(&[12.0; 10]);
        assert!(summed[5] > 18.0, "overlapping bands add up without correction");
        let corrected = graphic_center_response(&flat.map(f64::from));
        assert!(corrected.iter().all(|gain| (gain - 12.0).abs() < 0.1), "{corrected:?}");
        let extreme = graphic_eq_design_gains(&[12.0, -12.0, 12.0, -12.0, 12.0, -12.0, 12.0, -12.0, 12.0, -12.0], true);
        assert!(extreme.iter().all(|gain| gain.abs() <= 15.0));

        let mut settings = AudioEffectsSettings::default();
        settings.main.enabled = true;
        settings.main.sound.equalizer_bands_db = vec![12.0; 10];
        assert_eq!(settings.resolve("").params[GRAPHIC_EQ_GAIN_0 + 5], 12.0);
        settings.main.sound.equalizer_accurate = true;
        let accurate = settings.resolve("").params;
        assert!(accurate[GRAPHIC_EQ_GAIN_0 + 5] < 9.0);
        assert!(accurate[PREAMP_DB] > -11.0, "headroom follows the corrected response");
        let json: AudioEffectsSound = serde_json::from_str(r#"{"bassDb":2}"#).unwrap();
        assert!(!json.equalizer_accurate, "Android sounds keep their summed response");
    }

    #[test]
    fn loudness_compensation_follows_player_volume_only_when_enabled() {
        assert_eq!(loudness_compensation_db(1.0, 1.0), (0.0, 0.0));
        let (bass, treble) = loudness_compensation_db(1.0, 0.1);
        assert!((bass - 6.0).abs() < 1e-4 && (treble - 2.0).abs() < 1e-4);
        assert_eq!(loudness_compensation_db(1.0, 0.0), (10.0, 4.0));
        assert_eq!(loudness_compensation_db(0.0, 0.0), (0.0, 0.0));

        let mut settings = AudioEffectsSettings::default();
        settings.main.enabled = true;
        settings.main.sound.loudness_compensation = 0.5;
        assert!(settings.uses_loudness_compensation(""));
        let quiet = settings.resolve_with_volume("", 0.1).params;
        assert_eq!(quiet[MASTER_ENABLED], 1.0, "a loudness-only profile still runs the DSP");
        assert!((quiet[LOUDNESS_BASS_DB] - 3.0).abs() < 1e-4);
        assert!((quiet[LOUDNESS_TREBLE_DB] - 1.0).abs() < 1e-4);
        assert!(quiet[PREAMP_DB] < 0.0, "automatic headroom covers the low-frequency lift");
        assert_eq!(settings.resolve_with_volume("", 1.0).params[LOUDNESS_BASS_DB], 0.0);
        settings.main.sound.tone_enabled = false;
        assert!(!settings.uses_loudness_compensation(""));
        assert_eq!(settings.resolve_with_volume("", 0.1).params[LOUDNESS_BASS_DB], 0.0);
    }

    #[test]
    fn user_presets_are_trimmed_deduplicated_and_bounded() {
        let mut settings = AudioEffectsSettings {
            user_presets: (0..70)
                .map(|index| AudioEffectsUserPreset {
                    id: format!(" preset-{index} "),
                    name: "  这是长度超过二十四个字符的用户自定义音效预设名称会截断  ".into(),
                    sound: AudioEffectsSound::default(),
                })
                .collect(),
            ..AudioEffectsSettings::default()
        };
        settings
            .user_presets
            .insert(0, settings.user_presets[0].clone());
        settings
            .user_presets
            .insert(0, AudioEffectsUserPreset::default());
        let normalized = settings.normalized();
        assert_eq!(normalized.user_presets.len(), 50);
        assert_eq!(normalized.user_presets[0].id, "preset-0");
        assert_eq!(normalized.user_presets[0].name.chars().count(), 24);
    }
}
