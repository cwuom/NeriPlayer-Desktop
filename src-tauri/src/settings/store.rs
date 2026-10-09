use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

use crate::error::{AppError, AppResult};

pub const SETTINGS_FORMAT_VERSION: u32 = 1;
pub const SETTINGS_STORE_FILE: &str = "settings.json";
pub const DEFAULT_DOWNLOAD_NAME_TEMPLATE: &str = "%title% - %artist% - %album% - %source%";
pub const MIN_DOWNLOAD_PARALLELISM: i32 = 1;
pub const MAX_DOWNLOAD_PARALLELISM: i32 = 8;
pub const DEFAULT_DOWNLOAD_PARALLELISM: i32 = 6;
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
    /// 关闭主窗口时收进托盘继续播放；关掉后关闭即退出
    pub close_to_tray: bool,
    pub show_cover_badge: bool,
    pub show_now_playing_title: bool,
    pub show_toolbar_dock: bool,
    pub show_quality_switch: bool,
    pub show_audio_codec: bool,
    pub show_audio_spec: bool,
    pub show_audio_bitrate: bool,
    pub show_audio_format: bool,
    pub show_audio_channels: bool,
    pub show_audio_sample_rate: bool,
    pub show_audio_bit_depth: bool,
    pub lyric_font_scale: f32,
    /// 旧版「无缝切换」，已并入 crossfade_next；保留字段兼容旧配置，规整后恒为 false
    pub crossfade: bool,
    pub normalize_volume: bool,
    /// 多声道（AC-3/E-AC-3）音轨保留码流自带的动态范围压缩；默认关闭，保留完整动态
    pub multichannel_drc: bool,
    /// 声道平衡，-1（只剩左声道）～1（只剩右声道），按 0.01 取整（对齐 Android）
    pub volume_balance: f32,
    pub fade_in: bool,
    #[serde(deserialize_with = "lenient_i32")]
    pub fade_in_duration: i32,
    #[serde(deserialize_with = "lenient_i32")]
    pub fade_out_duration: i32,
    pub crossfade_next: bool,
    #[serde(deserialize_with = "lenient_i32")]
    pub crossfade_in_duration: i32,
    #[serde(deserialize_with = "lenient_i32")]
    pub crossfade_out_duration: i32,
    pub keep_progress: bool,
    /// 15 分钟以上内容记住播放位置并随同步续播（对齐 Android remember_long_form_playback_progress）
    pub remember_long_form_progress: bool,
    pub keep_playback_mode: bool,
    pub show_translation: bool,
    /// 罗马音等音译；与 Android 一样默认不显示
    pub show_romanization: bool,
    pub lyric_blur: bool,
    pub lyric_blur_amount: f32,
    #[serde(deserialize_with = "lenient_i32")]
    pub cloud_music_offset: i32,
    #[serde(deserialize_with = "lenient_i32")]
    pub qq_music_offset: i32,
    /// 酷狗、LRCLIB、AMLL TTML 歌词的默认偏移（毫秒），与 Android 一样默认 0
    #[serde(deserialize_with = "lenient_i32")]
    pub kugou_offset: i32,
    #[serde(deserialize_with = "lenient_i32")]
    pub lrclib_offset: i32,
    #[serde(deserialize_with = "lenient_i32")]
    pub amll_ttml_offset: i32,
    pub cover_style: String,
    pub advanced_lyrics: bool,
    /// 有逐字结果时优先用；关闭后不再用 AMLL/酷狗补逐字（对齐 Android prefer_word_timed_lyrics）
    pub prefer_word_timed_lyrics: bool,
    /// 播放时优先尝试的歌词源，找不到时回退自动（对齐 Android default_lyric_source）
    pub default_lyric_source: String,
    /// 取色方式：system=跟随系统取色 / default=默认主题色 / cover=跟随封面动态取色
    pub color_mode: String,
    /// 旧版字段（封面取色开关），仅用于迁移，导出配置时不序列化
    #[serde(default, skip_serializing)]
    pub dynamic_color: bool,
    pub dynamic_background: bool,
    pub audio_reactive: bool,
    pub cover_blur_bg: bool,
    pub cover_blur_amount: f32,
    pub cover_blur_darken: f32,
    pub netease_quality: String,
    pub qq_music_quality: String,
    pub youtube_quality: String,
    pub bili_quality: String,
    pub youtube_playback_source: String,
    pub netease_auto_source_switch: bool,
    pub netease_local_source_fallback: bool,
    pub bypass_proxy: bool,
    pub internationalization_enabled: bool,
    /// 探索页显示并记录搜索关键词（对齐 Android explore_search_history_enabled）
    pub explore_search_history_enabled: bool,
    pub background_image_uri: String,
    pub background_image_blur: f32,
    pub background_image_alpha: f32,
    /// 背景图模式下卡片、搜索框等控件的实时玻璃模糊（对齐 Android enhanced_advanced_blur_enabled，PC 默认开）
    pub enhanced_advanced_blur: bool,
    /// 玻璃模糊半径（px），12–64 按 4 对齐（Android enhanced_advanced_blur_radius_dp）
    pub enhanced_advanced_blur_radius: f32,
    pub dev_mode_enabled: bool,
    pub log_to_file: bool,
    pub log_level: String,
    #[serde(deserialize_with = "lenient_i32")]
    pub max_cache_size: i32,
    pub download_name_template: String,
    pub download_dir: String,
    #[serde(deserialize_with = "lenient_i32")]
    pub download_parallelism: i32,
    pub download_auto_fill_metadata: bool,
    pub download_embed_lyrics: bool,
    pub download_follow_playback_quality: bool,
    pub download_netease_quality: String,
    pub download_qq_music_quality: String,
    pub download_youtube_quality: String,
    pub download_bili_quality: String,
    pub lt_server_url: String,
    pub lt_nickname: String,
    pub lt_allow_member_control: bool,
    pub lt_auto_pause_on_member_change: bool,
    pub lt_share_audio_links: bool,
    pub volume: f32,
    pub audio_output_device: String,
    pub playback_speed: f32,
    #[serde(deserialize_with = "lenient_i32")]
    pub loudness_gain_mb: i32,
    pub equalizer_enabled: bool,
    pub equalizer_preset_id: String,
    #[serde(deserialize_with = "lenient_i32_vec")]
    pub equalizer_bands: Vec<i32>,
    /// 桌面歌词外观（字体、颜色、布局、锁定、窗口位置……），逐项规整在前端
    /// normalizeDesktopLyricsStyle；这里只保证是对象且不过大
    pub desktop_lyrics: serde_json::Value,
}

const MAX_DESKTOP_LYRICS_STYLE_BYTES: usize = 16 * 1024;

/// 前端数字输入可能带小数：整数字段四舍五入接收，单个字段不能让整份设置被拒绝
fn lenient_i32<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    round_to_i32(f64::deserialize(deserializer)?)
        .ok_or_else(|| serde::de::Error::custom("expected a finite number"))
}

fn lenient_i32_vec<'de, D>(deserializer: D) -> Result<Vec<i32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Vec::<f64>::deserialize(deserializer)?
        .into_iter()
        .map(|value| {
            round_to_i32(value).ok_or_else(|| serde::de::Error::custom("expected a finite number"))
        })
        .collect()
}

fn round_to_i32(value: f64) -> Option<i32> {
    value
        .is_finite()
        .then(|| value.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32)
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            format_version: SETTINGS_FORMAT_VERSION,
            dark_mode: "dark".into(),
            theme_color: "purple".into(),
            locale: "zh-CN".into(),
            default_screen: "home".into(),
            close_to_tray: true,
            show_cover_badge: true,
            show_now_playing_title: true,
            show_toolbar_dock: true,
            show_quality_switch: false,
            show_audio_codec: true,
            show_audio_spec: true,
            show_audio_bitrate: true,
            show_audio_format: true,
            show_audio_channels: false,
            show_audio_sample_rate: false,
            show_audio_bit_depth: false,
            lyric_font_scale: 1.0,
            crossfade: false,
            normalize_volume: false,
            multichannel_drc: false,
            volume_balance: 0.0,
            fade_in: false,
            fade_in_duration: 500,
            fade_out_duration: 500,
            crossfade_next: false,
            crossfade_in_duration: 500,
            crossfade_out_duration: 500,
            keep_progress: true,
            remember_long_form_progress: true,
            keep_playback_mode: true,
            show_translation: true,
            show_romanization: false,
            lyric_blur: true,
            lyric_blur_amount: 1.5,
            cloud_music_offset: 1000,
            qq_music_offset: 500,
            kugou_offset: 0,
            lrclib_offset: 0,
            amll_ttml_offset: 0,
            cover_style: "card".into(),
            advanced_lyrics: true,
            prefer_word_timed_lyrics: true,
            default_lyric_source: "automatic".into(),
            color_mode: "default".into(),
            dynamic_color: false,
            dynamic_background: true,
            audio_reactive: true,
            cover_blur_bg: false,
            cover_blur_amount: 1.5,
            cover_blur_darken: 0.2,
            netease_quality: "exhigh".into(),
            qq_music_quality: "high".into(),
            youtube_quality: "very_high".into(),
            bili_quality: "high".into(),
            youtube_playback_source: "automatic".into(),
            netease_auto_source_switch: false,
            netease_local_source_fallback: false,
            bypass_proxy: true,
            internationalization_enabled: false,
            explore_search_history_enabled: true,
            background_image_uri: String::new(),
            background_image_blur: 20.0,
            background_image_alpha: 0.3,
            enhanced_advanced_blur: true,
            enhanced_advanced_blur_radius: 36.0,
            dev_mode_enabled: false,
            log_to_file: false,
            log_level: "info".into(),
            max_cache_size: 1024,
            download_name_template: DEFAULT_DOWNLOAD_NAME_TEMPLATE.into(),
            download_dir: String::new(),
            download_parallelism: DEFAULT_DOWNLOAD_PARALLELISM,
            download_auto_fill_metadata: true,
            download_embed_lyrics: false,
            download_follow_playback_quality: true,
            download_netease_quality: "exhigh".into(),
            download_qq_music_quality: "high".into(),
            download_youtube_quality: "high".into(),
            download_bili_quality: "high".into(),
            lt_server_url: "https://neriplayer.hancat.work".into(),
            lt_nickname: String::new(),
            lt_allow_member_control: true,
            lt_auto_pause_on_member_change: true,
            lt_share_audio_links: true,
            volume: 1.0,
            audio_output_device: String::new(),
            playback_speed: 1.0,
            loudness_gain_mb: 0,
            equalizer_enabled: false,
            equalizer_preset_id: "flat".into(),
            equalizer_bands: vec![0; EQUALIZER_BAND_COUNT],
            desktop_lyrics: serde_json::Value::Object(serde_json::Map::new()),
        }
    }
}

impl AppSettings {
    pub fn normalize(&mut self) {
        self.format_version = SETTINGS_FORMAT_VERSION;
        self.dark_mode = normalize_choice(
            &self.dark_mode,
            &["system", "dark", "light"],
            "dark",
        );
        self.theme_color = normalize_theme_color(&self.theme_color);
        self.locale = normalize_choice(
            &self.locale,
            &["zh-CN", "zh-TW", "en", "ja"],
            "zh-CN",
        );
        self.default_screen = normalize_choice(
            &self.default_screen,
            &["home", "explore", "library"],
            "home",
        );
        self.cover_style = normalize_choice(&self.cover_style, &["disc", "card"], "card");
        self.default_lyric_source = normalize_choice(
            &self.default_lyric_source,
            &["automatic", "cloud_music", "kugou", "qq_music", "lrclib", "amll_ttml"],
            "automatic",
        );

        if self.crossfade {
            if !self.crossfade_next {
                self.crossfade_next = true;
                self.crossfade_in_duration = self.fade_in_duration;
                self.crossfade_out_duration = self.fade_out_duration;
            }
            self.crossfade = false;
        }

        self.lyric_font_scale = clamp_f32(self.lyric_font_scale, 0.5, 1.6, 1.0);
        self.fade_in_duration = self.fade_in_duration.clamp(0, 10_000);
        self.fade_out_duration = self.fade_out_duration.clamp(0, 10_000);
        self.crossfade_in_duration = self.crossfade_in_duration.clamp(0, 10_000);
        self.crossfade_out_duration = self.crossfade_out_duration.clamp(0, 10_000);
        self.lyric_blur_amount = clamp_f32(self.lyric_blur_amount, 0.0, 8.0, 1.5);
        for offset in [
            &mut self.cloud_music_offset,
            &mut self.qq_music_offset,
            &mut self.kugou_offset,
            &mut self.lrclib_offset,
            &mut self.amll_ttml_offset,
        ] {
            *offset = normalize_lyric_default_offset(*offset);
        }
        self.cover_blur_amount = clamp_f32(self.cover_blur_amount, 0.0, 8.0, 1.5);
        self.cover_blur_darken = clamp_f32(self.cover_blur_darken, 0.0, 1.0, 0.2);
        self.background_image_blur = clamp_f32(self.background_image_blur, 0.0, 100.0, 20.0);
        self.background_image_alpha = clamp_f32(self.background_image_alpha, 0.0, 1.0, 0.3);
        self.enhanced_advanced_blur_radius =
            (clamp_f32(self.enhanced_advanced_blur_radius, 12.0, 64.0, 36.0) / 4.0).round() * 4.0;
        self.max_cache_size = self
            .max_cache_size
            .clamp(MIN_MEDIA_CACHE_SIZE_MB, MAX_MEDIA_CACHE_SIZE_MB);
        self.download_parallelism = self
            .download_parallelism
            .clamp(MIN_DOWNLOAD_PARALLELISM, MAX_DOWNLOAD_PARALLELISM);
        self.volume = clamp_f32(self.volume, 0.0, 1.0, 1.0);
        self.playback_speed = clamp_f32(self.playback_speed, 0.25, 3.0, 1.0);
        self.loudness_gain_mb = self.loudness_gain_mb.clamp(0, 1_500);
        self.volume_balance = (clamp_f32(self.volume_balance, -1.0, 1.0, 0.0) * 100.0).round() / 100.0;
        let style_bytes = serde_json::to_vec(&self.desktop_lyrics).map_or(usize::MAX, |bytes| bytes.len());
        if !self.desktop_lyrics.is_object() || style_bytes > MAX_DESKTOP_LYRICS_STYLE_BYTES {
            self.desktop_lyrics = serde_json::Value::Object(serde_json::Map::new());
        }

        if self.netease_quality.trim() == "high" {
            self.netease_quality = "higher".into();
        }
        self.netease_quality = normalize_choice(
            &self.netease_quality,
            &[
                "standard",
                "higher",
                "exhigh",
                "lossless",
                "hires",
                "jyeffect",
                "sky",
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
        self.youtube_playback_source =
            normalize_youtube_playback_source(&self.youtube_playback_source);
        if self.download_netease_quality.trim() == "high" {
            self.download_netease_quality = "higher".into();
        }
        self.download_netease_quality = normalize_choice(
            &self.download_netease_quality,
            &[
                "standard",
                "higher",
                "exhigh",
                "lossless",
                "hires",
                "jyeffect",
                "sky",
                "jymaster",
            ],
            "exhigh",
        );
        self.download_qq_music_quality = normalize_choice(
            &self.download_qq_music_quality,
            &["standard", "high", "lossless"],
            "high",
        );
        self.download_youtube_quality = normalize_choice(
            &self.download_youtube_quality,
            &["low", "medium", "high", "very_high"],
            "high",
        );
        self.download_bili_quality = normalize_choice(
            &self.download_bili_quality,
            &["low", "medium", "high", "dolby", "lossless", "hires"],
            "high",
        );
        self.equalizer_preset_id = normalize_equalizer_preset(&self.equalizer_preset_id);
        self.log_level = normalize_choice(
            &self.log_level,
            &["off", "error", "warn", "info", "debug", "trace"],
            "info",
        );

        // 取色方式：旧版 dynamic_color（封面取色开关）迁移到 color_mode；
        // Android 导入语义不同（dynamic_color=true=跟随系统），由导入路径直接写 color_mode
        if self.dynamic_color && (self.color_mode.is_empty() || self.color_mode == "default") {
            self.color_mode = "cover".into();
        }
        self.color_mode = normalize_choice(
            &self.color_mode,
            &["system", "default", "cover"],
            "default",
        );
        self.dynamic_color = false;

        self.background_image_uri = self.background_image_uri.trim().into();
        self.download_name_template = non_empty_or_default(
            &self.download_name_template,
            DEFAULT_DOWNLOAD_NAME_TEMPLATE,
        );
        self.download_dir = self.download_dir.trim().into();
        self.audio_output_device = self.audio_output_device.trim().into();
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

fn migrate_audio_display_settings(value: &mut serde_json::Value) -> bool {
    let Some(settings) = value.as_object_mut() else {
        return false;
    };
    let mut migrated = false;
    for (key, legacy_key) in [
        ("showAudioFormat", "showAudioCodec"),
        ("showAudioChannels", "showAudioSpec"),
        ("showAudioSampleRate", "showAudioSpec"),
        ("showAudioBitDepth", "showAudioSpec"),
    ] {
        if !settings.contains_key(key) {
            if let Some(enabled) = settings.get(legacy_key).and_then(serde_json::Value::as_bool) {
                settings.insert(key.into(), serde_json::Value::Bool(enabled));
                migrated = true;
            }
        }
    }
    migrated
}

pub fn deserialize_app_settings<'de, D>(deserializer: D) -> Result<AppSettings, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let mut value = serde_json::Value::deserialize(deserializer)?;
    migrate_audio_display_settings(&mut value);
    serde_json::from_value(value).map_err(serde::de::Error::custom)
}

pub fn load_settings(app: &AppHandle) -> AppResult<SettingsLoadResult> {
    let store = app
        .store(SETTINGS_STORE_FILE)
        .map_err(|error| AppError::Other(error.to_string()))?;
    let Some(mut value) = store.get(SETTINGS_STORE_KEY) else {
        return Ok(SettingsLoadResult {
            settings: AppSettings::default(),
            persisted: false,
        });
    };

    let migrated = migrate_audio_display_settings(&mut value);
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
    if migrated || settings != before_normalize {
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
    crate::logging::set_runtime_level(crate::logging::parse_level(&normalized.log_level));
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

/// 歌词来源的默认偏移：按 50ms 对齐并夹到 ±5000ms（Android normalizeLyricDefaultOffsetMs）
///
/// 半步向正无穷取整，与 Kotlin roundToLong、前端 Math.round 一致（-75 → -50）。
fn normalize_lyric_default_offset(value: i32) -> i32 {
    const STEP: f64 = 50.0;
    const RANGE: i32 = 5_000;
    let aligned = (f64::from(value) / STEP + 0.5).floor() * STEP;
    aligned.clamp(f64::from(-RANGE), f64::from(RANGE)) as i32
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
        "purple" | "teal" | "blue" | "rose" | "olive" | "brown" | "orange" | "green" => {
            normalized
        }
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
            "loudness",
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

fn normalize_youtube_playback_source(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase();
    let canonical = match normalized.as_str() {
        "vision_os" => "visionos",
        "androidvr" => "android_vr",
        "creator" => "web_creator",
        value => value,
    };
    normalize_choice(
        canonical,
        &[
            "automatic",
            "visionos",
            "android_vr",
            "web_remix",
            "tv_html5",
            "web_creator",
        ],
        "automatic",
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
    #[test]
    fn audio_display_settings_new_defaults_show_bitrate_and_format() {
        let settings: AppSettings = serde_json::from_str("{}").expect("new settings");
        assert!(settings.show_audio_bitrate);
        assert!(settings.show_audio_format);
        assert!(!settings.show_audio_channels);
        assert!(!settings.show_audio_sample_rate);
        assert!(!settings.show_audio_bit_depth);
        assert!(!settings.show_quality_switch);
    }

    #[test]
    fn audio_display_settings_preserve_legacy_groups_and_explicit_new_values() {
        for enabled in [false, true] {
            let mut value = serde_json::json!({
                "showAudioCodec": enabled,
                "showAudioSpec": enabled,
                "showQualitySwitch": enabled,
            });
            assert!(super::migrate_audio_display_settings(&mut value));
            let settings: AppSettings = serde_json::from_value(value).expect("legacy settings");
            assert!(settings.show_audio_bitrate);
            assert_eq!(settings.show_audio_format, enabled);
            assert_eq!(settings.show_audio_channels, enabled);
            assert_eq!(settings.show_audio_sample_rate, enabled);
            assert_eq!(settings.show_audio_bit_depth, enabled);
            assert_eq!(settings.show_quality_switch, enabled);
        }
        let mut value = serde_json::json!({
            "showAudioCodec": true, "showAudioSpec": true,
            "showAudioBitrate": false, "showAudioFormat": false,
            "showAudioChannels": false, "showAudioSampleRate": true,
            "showAudioBitDepth": false,
        });
        assert!(!super::migrate_audio_display_settings(&mut value));
        let settings: AppSettings = serde_json::from_value(value.clone()).expect("new settings");
        assert!(!settings.show_audio_bitrate);
        assert!(!settings.show_audio_format);
        assert!(!settings.show_audio_channels);
        assert!(settings.show_audio_sample_rate);
        assert!(!settings.show_audio_bit_depth);
        let serialized = serde_json::to_value(settings).expect("serialized settings");
        for key in ["showAudioBitrate", "showAudioFormat", "showAudioChannels", "showAudioSampleRate", "showAudioBitDepth"] {
            assert_eq!(serialized[key], value[key]);
        }
    }

    #[test]
    fn audio_display_settings_config_deserialization_migrates_old_preferences() {
        #[derive(serde::Deserialize)]
        struct Config {
            #[serde(deserialize_with = "super::deserialize_app_settings")]
            settings: AppSettings,
        }
        let config: Config = serde_json::from_value(serde_json::json!({
            "settings": { "showAudioCodec": true, "showAudioSpec": false, "showQualitySwitch": true }
        })).expect("legacy config");
        assert!(config.settings.show_audio_format);
        assert!(!config.settings.show_audio_channels);
        assert!(!config.settings.show_audio_sample_rate);
        assert!(!config.settings.show_audio_bit_depth);
        assert!(config.settings.show_quality_switch);
    }

    #[test]
    fn download_settings_adopt_android_defaults_for_old_snapshots() {
        let settings: AppSettings = serde_json::from_str("{}").expect("old settings");
        assert_eq!(settings.download_parallelism, 6);
        assert!(settings.download_auto_fill_metadata);
        assert!(!settings.download_embed_lyrics);
        assert!(settings.download_follow_playback_quality);
        assert_eq!(settings.download_netease_quality, "exhigh");
        assert_eq!(settings.download_youtube_quality, "high");
        assert_eq!(settings.download_bili_quality, "high");
        assert_eq!(
            settings.download_name_template,
            "%title% - %artist% - %album% - %source%"
        );
    }

    #[test]
    fn download_settings_normalize_and_round_trip() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "downloadParallelism": 99,
            "downloadAutoFillMetadata": false,
            "downloadEmbedLyrics": true,
            "downloadFollowPlaybackQuality": false,
            "downloadNeteaseQuality": " lossless ",
            "downloadYoutubeQuality": "invalid",
            "downloadBiliQuality": "hires",
            "downloadNameTemplate": " {artist} - {title} ",
            "downloadDir": " E:\\Music ",
        }))
        .expect("download settings");
        let json = serde_json::to_value(settings.normalized()).expect("settings");
        assert_eq!(json["downloadParallelism"], 8);
        assert_eq!(json["downloadAutoFillMetadata"], false);
        assert_eq!(json["downloadEmbedLyrics"], true);
        assert_eq!(json["downloadFollowPlaybackQuality"], false);
        assert_eq!(json["downloadNeteaseQuality"], "lossless");
        assert_eq!(json["downloadYoutubeQuality"], "high");
        assert_eq!(json["downloadBiliQuality"], "hires");
        assert_eq!(json["downloadNameTemplate"], "{artist} - {title}");
        assert_eq!(json["downloadDir"], "E:\\Music");
        let restored: AppSettings = serde_json::from_value(json.clone()).expect("restored settings");
        assert_eq!(serde_json::to_value(restored).expect("restored JSON"), json);
        let mut settings = AppSettings {
            download_parallelism: 0,
            ..AppSettings::default()
        };
        settings.normalize();
        assert_eq!(settings.download_parallelism, 1);
    }

    #[test]
    fn download_netease_quality_alias_normalizes_to_android_canonical_value() {
        for value in ["high", " high ", "higher", " higher "] {
            let mut settings = AppSettings {
                download_netease_quality: value.into(),
                ..AppSettings::default()
            };
            settings.normalize();
            assert_eq!(settings.download_netease_quality, "higher");
        }
    }

    #[test]
    fn android_alignment_output_device_settings_are_backward_compatible() {
        let mut settings: AppSettings = serde_json::from_str("{}").expect("old settings");
        assert_eq!(settings.audio_output_device, "");
        settings.audio_output_device = "  Headphones  ".into();
        settings.normalize();
        assert_eq!(settings.audio_output_device, "Headphones");
        let json = serde_json::to_value(&settings).expect("settings");
        assert_eq!(json["audioOutputDevice"], "Headphones");
    }

    use super::{AppSettings, MAX_MEDIA_CACHE_SIZE_MB, MIN_MEDIA_CACHE_SIZE_MB};

    #[test]
    fn playback_source_settings_are_backward_compatible() {
        let old: AppSettings = serde_json::from_str("{}").expect("old settings");
        assert_eq!(old.youtube_playback_source, "automatic");
        assert!(!old.netease_auto_source_switch);
        assert!(!old.netease_local_source_fallback);

        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "youtubePlaybackSource": " Web_Remix ",
            "neteaseAutoSourceSwitch": true,
            "neteaseLocalSourceFallback": true,
        }))
        .expect("playback source settings");
        let json = serde_json::to_value(settings.normalized()).expect("settings");
        assert_eq!(json["youtubePlaybackSource"], "web_remix");
        assert_eq!(json["neteaseAutoSourceSwitch"], true);
        assert_eq!(json["neteaseLocalSourceFallback"], true);
    }

    #[test]
    fn youtube_playback_source_normalization_matches_android() {
        for (value, expected) in [
            ("automatic", "automatic"),
            (" VISION_OS ", "visionos"),
            ("visionos", "visionos"),
            ("AndroidVR", "android_vr"),
            ("android_vr", "android_vr"),
            ("web_remix", "web_remix"),
            ("tv_html5", "tv_html5"),
            ("Creator", "web_creator"),
            ("web_creator", "web_creator"),
            ("invalid", "automatic"),
            ("", "automatic"),
        ] {
            let mut settings = AppSettings {
                youtube_playback_source: value.into(),
                ..AppSettings::default()
            };
            settings.normalize();
            assert_eq!(settings.youtube_playback_source, expected, "{value}");
        }
    }

    #[test]
    fn fractional_integer_settings_are_rounded_instead_of_rejecting_the_save() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "fadeInDuration": 500.5,
            "fadeOutDuration": 249.4,
            "crossfadeInDuration": 1200.6,
            "cloudMusicOffset": -12.5,
            "maxCacheSize": 1049.2,
            "loudnessGainMb": 10.7,
            "equalizerBands": [1.4, -2.6, 0, 3.5, 4],
            "lyricFontScale": 1.25,
        }))
        .expect("fractional numbers must not reject the whole settings payload");
        assert_eq!(settings.fade_in_duration, 501);
        assert_eq!(settings.fade_out_duration, 249);
        assert_eq!(settings.crossfade_in_duration, 1201);
        assert_eq!(settings.cloud_music_offset, -13);
        assert_eq!(settings.max_cache_size, 1049);
        assert_eq!(settings.loudness_gain_mb, 11);
        assert_eq!(settings.equalizer_bands, vec![1, -3, 0, 4, 4]);
        assert!(serde_json::from_value::<AppSettings>(serde_json::json!({ "fadeInDuration": "fast" })).is_err());
    }

    #[test]
    fn lyric_default_offsets_follow_android_step_and_range() {
        let mut settings = AppSettings {
            cloud_music_offset: 1_024,
            qq_music_offset: -26,
            kugou_offset: 9_000,
            lrclib_offset: -7_500,
            amll_ttml_offset: -75,
            ..AppSettings::default()
        };
        settings.normalize();
        assert_eq!(settings.cloud_music_offset, 1_000);
        assert_eq!(settings.qq_music_offset, -50);
        assert_eq!(settings.kugou_offset, 5_000);
        assert_eq!(settings.lrclib_offset, -5_000);
        assert_eq!(settings.amll_ttml_offset, -50, "半步向正无穷取整，与 Android/前端一致");
        let missing: AppSettings = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(
            (missing.kugou_offset, missing.lrclib_offset, missing.amll_ttml_offset),
            (0, 0, 0),
            "旧配置没有这些字段时按 Android 默认 0",
        );
    }

    #[test]
    fn volume_balance_is_clamped_and_rounded_like_android() {
        for (value, expected) in [(0.354f32, 0.35f32), (-0.35, -0.35), (1.7, 1.0), (-3.0, -1.0), (f32::NAN, 0.0)] {
            let mut settings = AppSettings { volume_balance: value, ..AppSettings::default() };
            settings.normalize();
            assert_eq!(settings.volume_balance, expected, "{value}");
        }
        let missing: AppSettings = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(missing.volume_balance, 0.0, "旧配置没有这个字段时居中");
    }

    #[test]
    fn desktop_lyrics_style_is_kept_as_an_object_and_bounded() {
        let missing: AppSettings = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(missing.desktop_lyrics, serde_json::json!({}), "旧配置没有这个字段时为空对象，前端补默认值");
        let mut kept: AppSettings = serde_json::from_value(serde_json::json!({
            "desktopLyrics": { "layout": "double", "fontSize": 40, "bounds": { "x": 10, "y": 20, "width": 900, "height": 180 } }
        }))
        .unwrap();
        kept.normalize();
        assert_eq!(kept.desktop_lyrics["layout"], "double");
        assert_eq!(kept.desktop_lyrics["bounds"]["width"], 900);
        for bad in [serde_json::json!("double"), serde_json::json!({ "fontFamily": "x".repeat(20_000) })] {
            let mut settings = AppSettings { desktop_lyrics: bad, ..AppSettings::default() };
            settings.normalize();
            assert_eq!(settings.desktop_lyrics, serde_json::json!({}));
        }
    }

    #[test]
    fn romanization_is_its_own_switch_and_off_by_default_like_android() {
        let missing: AppSettings = serde_json::from_value(serde_json::json!({ "showTranslation": true })).unwrap();
        assert!(missing.show_translation);
        assert!(!missing.show_romanization, "旧配置没有这个字段时不显示音译");
        let saved = serde_json::to_value(AppSettings { show_romanization: true, ..AppSettings::default() }).unwrap();
        assert_eq!(saved["showRomanization"], true);
    }

    #[test]
    fn legacy_crossfade_switch_merges_into_next_track_crossfade() {
        let mut settings = AppSettings {
            crossfade: true,
            crossfade_next: false,
            fade_in_duration: 800,
            fade_out_duration: 300,
            ..AppSettings::default()
        };
        settings.normalize();
        assert!(!settings.crossfade);
        assert!(settings.crossfade_next);
        assert_eq!(settings.crossfade_in_duration, 800);
        assert_eq!(settings.crossfade_out_duration, 300);

        let mut explicit = AppSettings {
            crossfade: true,
            crossfade_next: true,
            crossfade_in_duration: 1500,
            ..AppSettings::default()
        };
        explicit.normalize();
        assert!(!explicit.crossfade);
        assert_eq!(explicit.crossfade_in_duration, 1500, "existing crossfade durations win");
    }

    #[test]
    fn playback_quality_and_display_ranges_match_android() {
        let mut settings = AppSettings {
            netease_quality: " high ".into(),
            lyric_font_scale: 1.6,
            cover_blur_amount: 500.0,
            ..AppSettings::default()
        };
        settings.normalize();
        assert_eq!(settings.netease_quality, "higher");
        assert_eq!(settings.lyric_font_scale, 1.6);
        assert_eq!(settings.cover_blur_amount, 8.0);
        assert_eq!(AppSettings::default().cloud_music_offset, 1000);
    }

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
