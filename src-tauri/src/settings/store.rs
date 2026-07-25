use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

use crate::error::{AppError, AppResult};

pub const SETTINGS_FORMAT_VERSION: u32 = 1;
pub const SETTINGS_STORE_FILE: &str = "settings.json";
pub const DEFAULT_DOWNLOAD_NAME_TEMPLATE: &str = "{source} - {artist} - {title}";
pub const MIN_MEDIA_CACHE_SIZE_MB: i32 = 256;
pub const MAX_MEDIA_CACHE_SIZE_MB: i32 = 512 * 1024;

const SETTINGS_STORE_KEY: &str = "appSettings";
const EQUALIZER_BAND_COUNT: usize = 5;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsLoadResult {
    pub settings: AppSettings,
    pub persisted: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub format_version: u32,
    pub dark_mode: String,
    pub theme_color: String,
    pub locale: String,
    pub default_screen: String,
    pub show_cover_badge: bool,
    pub show_now_playing_title: bool,
    pub show_toolbar_dock: bool,
    pub show_quality_switch: bool,
    pub show_audio_codec: bool,
    pub show_audio_spec: bool,
    pub lyric_font_scale: f32,
    pub crossfade: bool,
    pub normalize_volume: bool,
    pub fade_in: bool,
    pub fade_in_duration: i32,
    pub fade_out_duration: i32,
    pub crossfade_next: bool,
    pub crossfade_in_duration: i32,
    pub crossfade_out_duration: i32,
    pub keep_progress: bool,
    pub keep_playback_mode: bool,
    pub show_translation: bool,
    pub lyric_blur: bool,
    pub lyric_blur_amount: f32,
    pub cloud_music_offset: i32,
    pub qq_music_offset: i32,
    pub cover_style: String,
    pub advanced_lyrics: bool,
    pub dynamic_color: bool,
    pub dynamic_background: bool,
    pub audio_reactive: bool,
    pub cover_blur_bg: bool,
    pub cover_blur_amount: f32,
    pub cover_blur_darken: f32,
    pub netease_quality: String,
    pub netease_auto_source_switch: bool,
    pub qq_music_quality: String,
    pub youtube_quality: String,
    pub bili_quality: String,
    pub bypass_proxy: bool,
    pub internationalization_enabled: bool,
    pub background_image_uri: String,
    pub background_image_blur: f32,
    pub background_image_alpha: f32,
    pub dev_mode_enabled: bool,
    pub log_to_file: bool,
    pub log_level: String,
    pub max_cache_size: i32,
    pub download_name_template: String,
    pub download_dir: String,
    pub lt_server_url: String,
    pub lt_nickname: String,
    pub lt_allow_member_control: bool,
    pub lt_auto_pause_on_member_change: bool,
    pub lt_share_audio_links: bool,
    pub volume: f32,
    pub playback_speed: f32,
    pub loudness_gain_mb: i32,
    pub equalizer_enabled: bool,
    pub equalizer_preset_id: String,
    pub equalizer_bands: Vec<i32>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            format_version: SETTINGS_FORMAT_VERSION,
            dark_mode: "dark".into(),
            theme_color: "purple".into(),
            locale: "zh-CN".into(),
            default_screen: "home".into(),
            show_cover_badge: true,
            show_now_playing_title: true,
            show_toolbar_dock: true,
            show_quality_switch: true,
            show_audio_codec: true,
            show_audio_spec: true,
            lyric_font_scale: 1.0,
            crossfade: false,
            normalize_volume: false,
            fade_in: false,
            fade_in_duration: 500,
            fade_out_duration: 500,
            crossfade_next: false,
            crossfade_in_duration: 500,
            crossfade_out_duration: 500,
            keep_progress: true,
            keep_playback_mode: true,
            show_translation: true,
            lyric_blur: true,
            lyric_blur_amount: 1.5,
            cloud_music_offset: 500,
            qq_music_offset: 500,
            cover_style: "card".into(),
            advanced_lyrics: true,
            dynamic_color: false,
            dynamic_background: true,
            audio_reactive: true,
            cover_blur_bg: false,
            cover_blur_amount: 1.5,
            cover_blur_darken: 0.2,
            netease_quality: "exhigh".into(),
            netease_auto_source_switch: true,
            qq_music_quality: "high".into(),
            youtube_quality: "very_high".into(),
            bili_quality: "high".into(),
            bypass_proxy: true,
            internationalization_enabled: false,
            background_image_uri: String::new(),
            background_image_blur: 20.0,
            background_image_alpha: 0.3,
            dev_mode_enabled: false,
            log_to_file: false,
            log_level: "info".into(),
            max_cache_size: 1024,
            download_name_template: DEFAULT_DOWNLOAD_NAME_TEMPLATE.into(),
            download_dir: String::new(),
            lt_server_url: "https://neriplayer.hancat.work".into(),
            lt_nickname: String::new(),
            lt_allow_member_control: true,
            lt_auto_pause_on_member_change: true,
            lt_share_audio_links: true,
            volume: 1.0,
            playback_speed: 1.0,
            loudness_gain_mb: 0,
            equalizer_enabled: false,
            equalizer_preset_id: "flat".into(),
            equalizer_bands: vec![0; EQUALIZER_BAND_COUNT],
        }
    }
}

impl AppSettings {
    pub fn normalize(&mut self) {
        self.format_version = SETTINGS_FORMAT_VERSION;
        self.dark_mode = normalize_choice(&self.dark_mode, &["system", "dark", "light"], "dark");
        self.theme_color = normalize_theme_color(&self.theme_color);
        self.locale = normalize_choice(&self.locale, &["zh-CN", "zh-TW", "en", "ja"], "zh-CN");
        self.default_screen = normalize_choice(
            &self.default_screen,
            &["home", "explore", "library"],
            "home",
        );
        self.cover_style = normalize_choice(&self.cover_style, &["disc", "card"], "card");

        self.lyric_font_scale = clamp_f32(self.lyric_font_scale, 0.5, 1.5, 1.0);
        self.fade_in_duration = self.fade_in_duration.clamp(0, 10_000);
        self.fade_out_duration = self.fade_out_duration.clamp(0, 10_000);
        self.crossfade_in_duration = self.crossfade_in_duration.clamp(0, 10_000);
        self.crossfade_out_duration = self.crossfade_out_duration.clamp(0, 10_000);
        self.lyric_blur_amount = clamp_f32(self.lyric_blur_amount, 0.0, 8.0, 1.5);
        self.cloud_music_offset = self.cloud_music_offset.clamp(-30_000, 30_000);
        self.qq_music_offset = self.qq_music_offset.clamp(-30_000, 30_000);
        self.cover_blur_amount = clamp_f32(self.cover_blur_amount, 0.0, 100.0, 1.5);
        self.cover_blur_darken = clamp_f32(self.cover_blur_darken, 0.0, 1.0, 0.2);
        self.background_image_blur = clamp_f32(self.background_image_blur, 0.0, 100.0, 20.0);
        self.background_image_alpha = clamp_f32(self.background_image_alpha, 0.0, 1.0, 0.3);
        self.max_cache_size = self
            .max_cache_size
            .clamp(MIN_MEDIA_CACHE_SIZE_MB, MAX_MEDIA_CACHE_SIZE_MB);
        self.volume = clamp_f32(self.volume, 0.0, 1.0, 1.0);
        self.playback_speed = clamp_f32(self.playback_speed, 0.25, 3.0, 1.0);
        self.loudness_gain_mb = self.loudness_gain_mb.clamp(0, 1_500);

        self.netease_quality = normalize_choice(
            &self.netease_quality,
            &[
                "standard", "higher", "high", "exhigh", "lossless", "hires", "jyeffect", "sky",
                "jymaster",
            ],
            "exhigh",
        );
        self.qq_music_quality = normalize_choice(
            &self.qq_music_quality,
            &["standard", "high", "lossless"],
            "high",
        );
        self.youtube_quality = normalize_choice(
            &self.youtube_quality,
            &["low", "medium", "high", "very_high"],
            "very_high",
        );
        self.bili_quality = normalize_choice(
            &self.bili_quality,
            &["low", "medium", "high", "dolby", "lossless", "hires"],
            "high",
        );
        self.equalizer_preset_id = normalize_equalizer_preset(&self.equalizer_preset_id);
        self.log_level = normalize_choice(
            &self.log_level,
            &["off", "error", "warn", "info", "debug", "trace"],
            "info",
        );

        self.background_image_uri = self.background_image_uri.trim().into();
        self.download_name_template =
            non_empty_or_default(&self.download_name_template, DEFAULT_DOWNLOAD_NAME_TEMPLATE);
        self.download_dir = self.download_dir.trim().into();
        self.lt_server_url = self.lt_server_url.trim().trim_end_matches('/').into();
        self.lt_nickname = self.lt_nickname.trim().into();

        self.equalizer_bands = self
            .equalizer_bands
            .iter()
            .take(EQUALIZER_BAND_COUNT)
            .map(|value| (*value).clamp(-1_500, 1_500))
            .collect();
        self.equalizer_bands.resize(EQUALIZER_BAND_COUNT, 0);
    }

    pub fn normalized(mut self) -> Self {
        self.normalize();
        self
    }
}

pub fn load_settings(app: &AppHandle) -> AppResult<SettingsLoadResult> {
    let store = app
        .store(SETTINGS_STORE_FILE)
        .map_err(|error| AppError::Other(error.to_string()))?;
    let Some(value) = store.get(SETTINGS_STORE_KEY) else {
        return Ok(SettingsLoadResult {
            settings: AppSettings::default(),
            persisted: false,
        });
    };

    let mut settings = match serde_json::from_value::<AppSettings>(value) {
        Ok(settings) => settings,
        Err(error) => {
            log::warn!(target: "settings", "普通设置格式无效，回退到默认值: {}", error);
            return Ok(SettingsLoadResult {
                settings: AppSettings::default(),
                persisted: false,
            });
        }
    };
    let before_normalize = settings.clone();
    settings.normalize();
    if settings != before_normalize {
        persist_settings(&store, &settings)?;
    }
    Ok(SettingsLoadResult {
        settings,
        persisted: true,
    })
}

pub fn save_settings(app: &AppHandle, settings: AppSettings) -> AppResult<AppSettings> {
    let normalized = settings.normalized();
    let store = app
        .store(SETTINGS_STORE_FILE)
        .map_err(|error| AppError::Other(error.to_string()))?;
    persist_settings(&store, &normalized)?;
    // 日志级别可运行时即时调整；文件开关受插件限制需重启生效
    log::set_max_level(crate::logging::parse_level(&normalized.log_level));
    Ok(normalized)
}

fn persist_settings<R: tauri::Runtime>(
    store: &tauri_plugin_store::Store<R>,
    settings: &AppSettings,
) -> AppResult<()> {
    let value = serde_json::to_value(settings)?;
    store.set(SETTINGS_STORE_KEY, value);
    store
        .save()
        .map_err(|error| AppError::Other(error.to_string()))?;
    Ok(())
}

fn clamp_f32(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

fn normalize_choice(value: &str, allowed: &[&str], fallback: &str) -> String {
    let normalized = value.trim();
    if allowed.contains(&normalized) {
        normalized.into()
    } else {
        fallback.into()
    }
}

fn normalize_theme_color(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "purple" | "teal" | "blue" | "rose" | "olive" | "brown" | "orange" | "green" => normalized,
        "#6750a4" => "purple".into(),
        _ => "purple".into(),
    }
}

fn normalize_equalizer_preset(value: &str) -> String {
    normalize_choice(
        value,
        &[
            "flat",
            "acoustic",
            "bass_boost",
            "bass_reduce",
            "classical",
            "dance",
            "deep",
            "electronic",
            "hip_hop",
            "jazz",
            "latin",
            "lounge",
            "piano",
            "pop",
            "rnb",
            "rock",
            "small_speakers",
            "spoken_word",
            "treble_boost",
            "treble_reduce",
            "vocal_boost",
            "custom",
        ],
        "flat",
    )
}

fn non_empty_or_default(value: &str, fallback: &str) -> String {
    let normalized = value.trim();
    if normalized.is_empty() {
        fallback.into()
    } else {
        normalized.into()
    }
}

#[cfg(test)]
mod tests {
    use super::{AppSettings, MAX_MEDIA_CACHE_SIZE_MB, MIN_MEDIA_CACHE_SIZE_MB};

    #[test]
    fn media_cache_is_mandatory_and_capped_at_512_gib() {
        let mut disabled = AppSettings {
            max_cache_size: 0,
            ..AppSettings::default()
        };
        disabled.normalize();
        assert_eq!(disabled.max_cache_size, MIN_MEDIA_CACHE_SIZE_MB);

        let mut oversized = AppSettings {
            max_cache_size: i32::MAX,
            ..AppSettings::default()
        };
        oversized.normalize();
        assert_eq!(oversized.max_cache_size, MAX_MEDIA_CACHE_SIZE_MB);
    }
}
