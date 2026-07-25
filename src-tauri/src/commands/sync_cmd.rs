// 同步相关命令
use crate::error::{AppError, AppResult};
use crate::security;
use crate::settings::store::{self, AppSettings};
use crate::state::AppState;
use crate::sync::manager;
use crate::sync::models::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_store::StoreExt;

// 同步配置存储键
const GITHUB_CONFIG_KEY: &str = "githubSync";
const WEBDAV_CONFIG_KEY: &str = "webdavSync";
const SYNC_PREFERENCES_KEY: &str = "syncPreferences";
const SYNC_STORE: &str = "sync-config.json";
const CONFIG_FILE_KIND: &str = "moe.ouom.neriplayer.config";
const CONFIG_FILE_VERSION: u32 = 1;

fn config_default_true() -> bool {
    true
}
fn config_default_history_mode() -> String {
    "immediate".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ConfigListenTogether {
    #[serde(default)]
    user_uuid: String,
    #[serde(default)]
    server_url: String,
    #[serde(default)]
    nickname: String,
    #[serde(default = "config_default_true")]
    allow_member_control: bool,
    #[serde(default = "config_default_true")]
    auto_pause_on_member_change: bool,
    #[serde(default = "config_default_true")]
    share_audio_links: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ConfigLanguage {
    #[serde(default)]
    code: String,
}

impl ConfigLanguage {
    fn has_value(&self) -> bool {
        !self.code.trim().is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ConfigGitHubSync {
    #[serde(default)]
    token: String,
    #[serde(default)]
    owner: String,
    #[serde(default)]
    repo: String,
    #[serde(default)]
    last_remote_sha: String,
    #[serde(default)]
    last_sync_time: i64,
    #[serde(default)]
    auto_sync: bool,
    #[serde(default = "config_default_true")]
    data_saver: bool,
    #[serde(default)]
    silent_failures: bool,
    #[serde(default = "config_default_history_mode")]
    history_update_mode: String,
}

impl From<&GitHubSyncConfig> for ConfigGitHubSync {
    fn from(config: &GitHubSyncConfig) -> Self {
        Self {
            token: config.token.clone(),
            owner: config.owner.clone(),
            repo: config.repo.clone(),
            last_remote_sha: config.last_remote_sha.clone(),
            last_sync_time: config.last_sync_time,
            auto_sync: config.auto_sync,
            data_saver: config.data_saver,
            silent_failures: config.silent_failures,
            history_update_mode: config.history_update_mode.clone(),
        }
    }
}

impl ConfigGitHubSync {
    fn into_config(self) -> GitHubSyncConfig {
        GitHubSyncConfig {
            token: self.token,
            owner: self.owner,
            repo: self.repo,
            last_remote_sha: self.last_remote_sha,
            last_sync_time: self.last_sync_time,
            auto_sync: self.auto_sync,
            data_saver: self.data_saver,
            silent_failures: self.silent_failures,
            history_update_mode: self.history_update_mode,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ConfigWebDavSync {
    #[serde(default)]
    server_url: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    base_path: String,
    #[serde(default)]
    last_remote_fingerprint: String,
    #[serde(default)]
    last_sync_time: i64,
    #[serde(default)]
    auto_sync: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ConfigSyncPreferences {
    #[serde(default = "config_default_history_mode")]
    history_update_mode: String,
}

impl From<&SyncPreferencesConfig> for ConfigSyncPreferences {
    fn from(config: &SyncPreferencesConfig) -> Self {
        Self {
            history_update_mode: normalize_history_update_mode(&config.history_update_mode),
        }
    }
}

impl ConfigSyncPreferences {
    fn into_config(self) -> SyncPreferencesConfig {
        SyncPreferencesConfig {
            history_update_mode: normalize_history_update_mode(&self.history_update_mode),
        }
    }
}

impl From<&WebDavSyncConfig> for ConfigWebDavSync {
    fn from(config: &WebDavSyncConfig) -> Self {
        Self {
            server_url: config.server_url.clone(),
            username: config.username.clone(),
            password: config.password.clone(),
            base_path: config.base_path.clone(),
            last_remote_fingerprint: config.last_remote_fingerprint.clone(),
            last_sync_time: config.last_sync_time,
            auto_sync: config.auto_sync,
        }
    }
}

impl ConfigWebDavSync {
    fn into_config(self) -> WebDavSyncConfig {
        WebDavSyncConfig {
            server_url: self.server_url,
            username: self.username,
            password: self.password,
            base_path: self.base_path,
            last_remote_fingerprint: self.last_remote_fingerprint,
            last_sync_time: self.last_sync_time,
            auto_sync: self.auto_sync,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DesktopConfigFile {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    format_version: u32,
    #[serde(default)]
    platform: String,
    #[serde(default)]
    platform_name: String,
    #[serde(default)]
    exported_at: i64,
    #[serde(default)]
    settings: AppSettings,
    #[serde(default)]
    listen_together: Option<ConfigListenTogether>,
    #[serde(default)]
    language: Option<ConfigLanguage>,
    #[serde(default)]
    auth: Option<crate::auth::state::AuthState>,
    #[serde(default)]
    github_sync: Option<ConfigGitHubSync>,
    #[serde(default)]
    webdav_sync: Option<ConfigWebDavSync>,
    #[serde(default)]
    sync_preferences: Option<ConfigSyncPreferences>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct AndroidTypedPreferenceSnapshot {
    booleans: HashMap<String, bool>,
    floats: HashMap<String, f32>,
    ints: HashMap<String, i32>,
    longs: HashMap<String, i64>,
    strings: HashMap<String, String>,
}

impl AndroidTypedPreferenceSnapshot {
    fn has_data(&self) -> bool {
        !self.booleans.is_empty()
            || !self.floats.is_empty()
            || !self.ints.is_empty()
            || !self.longs.is_empty()
            || !self.strings.is_empty()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct AndroidListenTogetherConfig {
    worker_base_url: String,
    worker_base_url_input: String,
    user_uuid: String,
    nickname: String,
    allow_member_control: bool,
    auto_pause_on_member_change: bool,
    share_audio_links: bool,
}

impl Default for AndroidListenTogetherConfig {
    fn default() -> Self {
        Self {
            allow_member_control: true,
            auto_pause_on_member_change: true,
            share_audio_links: true,
            worker_base_url: String::new(),
            worker_base_url_input: String::new(),
            user_uuid: String::new(),
            nickname: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct AndroidSavedCookieConfig {
    cookies: HashMap<String, String>,
}

impl AndroidSavedCookieConfig {
    fn has_data(&self) -> bool {
        !self.cookies.is_empty()
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct AndroidYouTubeAuthConfig {
    cookie_header: String,
    cookies: HashMap<String, String>,
    authorization: String,
    x_goog_auth_user: String,
    origin: String,
    user_agent: String,
}

impl AndroidYouTubeAuthConfig {
    fn has_data(&self) -> bool {
        !self.cookie_header.trim().is_empty()
            || !self.cookies.is_empty()
            || !self.authorization.trim().is_empty()
            || !self.x_goog_auth_user.trim().is_empty()
            || !self.origin.trim().is_empty()
            || !self.user_agent.trim().is_empty()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct AndroidGitHubSyncConfig {
    token: String,
    repo_owner: String,
    repo_name: String,
    auto_sync_enabled: bool,
    play_history_update_mode: String,
    data_saver_mode: bool,
}

impl Default for AndroidGitHubSyncConfig {
    fn default() -> Self {
        Self {
            token: String::new(),
            repo_owner: String::new(),
            repo_name: String::new(),
            auto_sync_enabled: false,
            play_history_update_mode: String::new(),
            data_saver_mode: true,
        }
    }
}

impl AndroidGitHubSyncConfig {
    fn has_data(&self) -> bool {
        !self.token.trim().is_empty()
            || !self.repo_owner.trim().is_empty()
            || !self.repo_name.trim().is_empty()
            || self.auto_sync_enabled
            || !self.play_history_update_mode.trim().is_empty()
            || !self.data_saver_mode
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct AndroidWebDavSyncConfig {
    server_url: String,
    base_path: String,
    username: String,
    password: String,
    auto_sync_enabled: bool,
}

impl AndroidWebDavSyncConfig {
    fn has_data(&self) -> bool {
        !self.server_url.trim().is_empty()
            || !self.base_path.trim().is_empty()
            || !self.username.trim().is_empty()
            || !self.password.trim().is_empty()
            || self.auto_sync_enabled
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct AndroidSyncPreferencesConfig {
    play_history_update_mode: String,
}

impl AndroidSyncPreferencesConfig {
    fn has_data(&self) -> bool {
        !self.play_history_update_mode.trim().is_empty()
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct AndroidConfigFile {
    kind: String,
    format_version: u32,
    settings: AndroidTypedPreferenceSnapshot,
    listen_together: AndroidListenTogetherConfig,
    language: ConfigLanguage,
    netease_auth: AndroidSavedCookieConfig,
    bili_auth: AndroidSavedCookieConfig,
    you_tube_auth: AndroidYouTubeAuthConfig,
    git_hub_sync: AndroidGitHubSyncConfig,
    web_dav_sync: AndroidWebDavSyncConfig,
    sync_preferences: AndroidSyncPreferencesConfig,
}

impl AndroidConfigFile {
    fn has_restorable_content(&self, has_listen_together_section: bool) -> bool {
        self.settings.has_data()
            || has_listen_together_section
            || self.language.has_value()
            || self.netease_auth.has_data()
            || self.bili_auth.has_data()
            || self.you_tube_auth.has_data()
            || self.git_hub_sync.has_data()
            || self.web_dav_sync.has_data()
            || self.sync_preferences.has_data()
    }
}

struct ParsedConfigImport {
    settings: AppSettings,
    listen_together: Option<ConfigListenTogether>,
    language: Option<ConfigLanguage>,
    auth: Option<crate::auth::state::AuthState>,
    github_sync: Option<ConfigGitHubSync>,
    webdav_sync: Option<ConfigWebDavSync>,
    sync_preferences: Option<ConfigSyncPreferences>,
    auth_platforms: Vec<&'static str>,
    platform: &'static str,
    warnings: Vec<String>,
}

fn android_settings_to_desktop(
    snapshot: &AndroidTypedPreferenceSnapshot,
    current_settings: AppSettings,
) -> AppSettings {
    let mut settings = current_settings;
    macro_rules! apply_bool {
        ($key:literal, $field:ident) => {
            if let Some(value) = snapshot.booleans.get($key) {
                settings.$field = *value;
            }
        };
    }
    macro_rules! apply_float {
        ($key:literal, $field:ident) => {
            if let Some(value) = snapshot.floats.get($key) {
                settings.$field = *value;
            }
        };
    }
    macro_rules! apply_int {
        ($key:literal, $field:ident) => {
            if let Some(value) = snapshot.ints.get($key) {
                settings.$field = *value;
            }
        };
    }
    macro_rules! apply_string {
        ($key:literal, $field:ident) => {
            if let Some(value) = snapshot.strings.get($key) {
                settings.$field = value.clone();
            }
        };
    }

    if snapshot.booleans.get("follow_system_dark") == Some(&true) {
        settings.dark_mode = "system".into();
    } else if let Some(force_dark) = snapshot.booleans.get("force_dark") {
        settings.dark_mode = if *force_dark { "dark" } else { "light" }.into();
    }
    apply_bool!("dynamic_color", dynamic_color);
    apply_bool!("dev_mode_enabled", dev_mode_enabled);
    apply_bool!("always_record_logs_enabled", log_to_file);
    apply_bool!("internationalization_enabled", internationalization_enabled);
    apply_bool!("show_cover_source_badge", show_cover_badge);
    apply_bool!("nowplaying_show_title", show_now_playing_title);
    apply_bool!("nowplaying_toolbar_dock_enabled", show_toolbar_dock);
    apply_bool!(
        "nowplaying_progress_show_quality_switch",
        show_quality_switch
    );
    apply_bool!("nowplaying_progress_show_audio_codec", show_audio_codec);
    apply_bool!("nowplaying_progress_show_audio_spec", show_audio_spec);
    apply_bool!("show_lyric_translation", show_translation);
    apply_bool!("advanced_lyrics_enabled", advanced_lyrics);
    apply_bool!("nowplaying_audio_reactive_enabled", audio_reactive);
    apply_bool!("nowplaying_dynamic_background_enabled", dynamic_background);
    apply_bool!("nowplaying_cover_blur_background_enabled", cover_blur_bg);
    apply_bool!("lyric_blur_enabled", lyric_blur);
    apply_bool!("bypass_proxy", bypass_proxy);
    apply_bool!("playback_fade_in", fade_in);
    apply_bool!("playback_crossfade_next", crossfade_next);
    apply_bool!("playback_volume_normalization_enabled", normalize_volume);
    apply_bool!("keep_last_playback_progress", keep_progress);
    apply_bool!("netease_auto_source_switch", netease_auto_source_switch);
    apply_bool!("keep_playback_mode_state", keep_playback_mode);
    apply_bool!("playback_equalizer_enabled", equalizer_enabled);

    apply_float!("lyric_font_scale", lyric_font_scale);
    apply_float!("background_image_blur", background_image_blur);
    apply_float!("background_image_alpha", background_image_alpha);
    apply_float!("nowplaying_cover_blur_amount", cover_blur_amount);
    apply_float!("nowplaying_cover_blur_darken", cover_blur_darken);
    apply_float!("lyric_blur_amount", lyric_blur_amount);
    apply_float!("playback_speed", playback_speed);

    apply_int!("playback_loudness_gain_mb", loudness_gain_mb);

    for (key, target) in [
        (
            "cloud_music_lyric_default_offset_ms",
            &mut settings.cloud_music_offset,
        ),
        (
            "qq_music_lyric_default_offset_ms",
            &mut settings.qq_music_offset,
        ),
    ] {
        let value = snapshot
            .longs
            .get(key)
            .copied()
            .or_else(|| snapshot.ints.get(key).copied().map(i64::from));
        if let Some(value) = value {
            *target = i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX });
        }
    }

    for (key, target) in [
        (
            "playback_fade_in_duration_ms",
            &mut settings.fade_in_duration,
        ),
        (
            "playback_fade_out_duration_ms",
            &mut settings.fade_out_duration,
        ),
        (
            "playback_crossfade_in_duration_ms",
            &mut settings.crossfade_in_duration,
        ),
        (
            "playback_crossfade_out_duration_ms",
            &mut settings.crossfade_out_duration,
        ),
    ] {
        if let Some(value) = snapshot.longs.get(key) {
            *target = i32::try_from(*value).unwrap_or(if *value < 0 { i32::MIN } else { i32::MAX });
        }
    }

    if let Some(bytes) = snapshot
        .longs
        .get("max_cache_size_bytes")
        .filter(|bytes| **bytes > 0)
    {
        let mib = (*bytes / (1024 * 1024)).max(1);
        settings.max_cache_size = i32::try_from(mib).unwrap_or(i32::MAX);
    }

    apply_string!("audio_quality", netease_quality);
    apply_string!("youtube_audio_quality", youtube_quality);
    apply_string!("bili_audio_quality", bili_quality);
    apply_string!("download_file_name_template", download_name_template);
    if let Some(value) = snapshot.strings.get("theme_seed_color") {
        settings.theme_color = match value
            .trim()
            .trim_start_matches('#')
            .to_ascii_uppercase()
            .as_str()
        {
            "0061A4" | "2196F3" => "blue".into(),
            "006E6D" | "009688" => "teal".into(),
            "6750A4" | "9C27B0" => "purple".into(),
            "B3261E" | "E91E63" => "rose".into(),
            _ => value.clone(),
        };
    }
    if let Some(value) = snapshot.strings.get("default_start_destination") {
        settings.default_screen = match value.trim() {
            "settings" => "home".into(),
            value => value.into(),
        };
    }
    if let Some(value) = snapshot.strings.get("playback_equalizer_preset") {
        settings.equalizer_preset_id = match value.trim() {
            "club" => "dance".into(),
            "folk" => "acoustic".into(),
            value => value.into(),
        };
    }

    if settings.equalizer_preset_id == "bass_reducer" {
        settings.equalizer_preset_id = "bass_reduce".into();
    } else if settings.equalizer_preset_id == "treble_reducer" {
        settings.equalizer_preset_id = "treble_reduce".into();
    }
    if let Some(levels) = snapshot
        .strings
        .get("playback_equalizer_custom_band_levels")
    {
        settings.equalizer_bands = levels
            .split(',')
            .filter_map(|value| value.trim().parse::<i32>().ok())
            .collect();
    }

    settings.normalized()
}

fn cookie_entries_from_map(
    cookies: &HashMap<String, String>,
    domain: &str,
) -> Vec<crate::auth::state::CookieEntry> {
    let mut entries: Vec<_> = cookies
        .iter()
        .filter_map(|(name, value)| {
            let name = name.trim();
            let value = value.trim();
            (!name.is_empty() && !value.is_empty() && !value.contains(';')).then(|| {
                crate::auth::state::CookieEntry {
                    name: name.into(),
                    value: value.into(),
                    domain: domain.into(),
                }
            })
        })
        .collect();
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    entries
}

fn android_auth_to_desktop(
    payload: &AndroidConfigFile,
    current: &crate::auth::state::AuthState,
) -> (Option<crate::auth::state::AuthState>, Vec<&'static str>) {
    use crate::auth::state::{AuthState, BiliAuth, NeteaseAuth, YouTubeAuth};

    let mut auth = AuthState {
        netease: current.netease.clone(),
        bilibili: current.bilibili.clone(),
        youtube: current.youtube.clone(),
    };
    let mut changed = false;
    let mut imported_platforms = Vec::new();
    let netease_entries = cookie_entries_from_map(&payload.netease_auth.cookies, "music.163.com");
    let current_netease = current.netease.as_ref().filter(|auth| {
        let imported_music_u = netease_entries.iter().find(|entry| entry.name == "MUSIC_U");
        imported_music_u.is_some_and(|imported| {
            auth.cookies
                .iter()
                .any(|saved| saved.name == "MUSIC_U" && saved.value == imported.value)
        })
    });
    if !netease_entries.is_empty() {
        auth.netease = Some(NeteaseAuth {
            cookies: netease_entries,
            user_id: current_netease.and_then(|auth| auth.user_id),
            nickname: current_netease.and_then(|auth| auth.nickname.clone()),
            avatar_url: current_netease.and_then(|auth| auth.avatar_url.clone()),
        });
        changed = true;
        imported_platforms.push("netease");
    }

    let bilibili_entries = cookie_entries_from_map(&payload.bili_auth.cookies, ".bilibili.com");
    if !bilibili_entries.is_empty() {
        let mid = bilibili_entries
            .iter()
            .find(|entry| entry.name == "DedeUserID")
            .and_then(|entry| entry.value.parse().ok());
        let current_bilibili = current.bilibili.as_ref().filter(|saved| saved.mid == mid);
        auth.bilibili = Some(BiliAuth {
            mid,
            cookies: bilibili_entries,
            nickname: current_bilibili.and_then(|saved| saved.nickname.clone()),
            avatar_url: current_bilibili.and_then(|saved| saved.avatar_url.clone()),
        });
        changed = true;
        imported_platforms.push("bilibili");
    }

    let youtube = if payload.you_tube_auth.cookies.is_empty() {
        crate::auth::cookies::parse_raw_cookie_text(&payload.you_tube_auth.cookie_header, "youtube")
    } else {
        let raw = payload
            .you_tube_auth
            .cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join(";");
        crate::auth::cookies::parse_raw_cookie_text(&raw, "youtube")
    };

    if !youtube.is_empty() {
        auth.youtube = Some(YouTubeAuth {
            cookies: youtube,
            nickname: None,
            avatar_url: None,
        });
        changed = true;
        imported_platforms.push("youtube");
    }

    (changed.then_some(auth), imported_platforms)
}

fn merge_imported_auth_platforms(
    mut current: crate::auth::state::AuthState,
    imported: crate::auth::state::AuthState,
    platforms: &[&str],
) -> crate::auth::state::AuthState {
    for platform in platforms {
        match *platform {
            "netease" => current.netease = imported.netease.clone(),
            "bilibili" => current.bilibili = imported.bilibili.clone(),
            "youtube" => current.youtube = imported.youtube.clone(),
            _ => {}
        }
    }
    current
}

fn parse_config_import(
    content: &str,
    current_settings: AppSettings,
    current_auth: &crate::auth::state::AuthState,
) -> AppResult<ParsedConfigImport> {
    let value: Value = serde_json::from_str(content)
        .map_err(|error| AppError::Other(format!("Parse config failed: {error}")))?;
    let platform = value
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or_default();

    if platform == "pc" {
        let payload: DesktopConfigFile = serde_json::from_value(value)
            .map_err(|error| AppError::Other(format!("Parse config failed: {error}")))?;
        if payload.kind != CONFIG_FILE_KIND
            || (payload.format_version != 0 && payload.format_version > CONFIG_FILE_VERSION)
            || payload.platform != "pc"
        {
            return Err(AppError::Other("Unsupported config file".into()));
        }
        let auth_platforms = if payload.auth.is_some() {
            vec!["netease", "bilibili", "youtube"]
        } else {
            Vec::new()
        };
        return Ok(ParsedConfigImport {
            settings: payload.settings,
            listen_together: payload.listen_together,
            language: payload.language,
            auth: payload.auth,
            github_sync: payload.github_sync,
            webdav_sync: payload.webdav_sync,
            sync_preferences: payload.sync_preferences,
            auth_platforms,
            platform: "pc",
            warnings: Vec::new(),
        });
    }

    let has_listen_together_section = value.get("listenTogether").is_some_and(Value::is_object);
    let payload: AndroidConfigFile = serde_json::from_value(value)
        .map_err(|error| AppError::Other(format!("Parse Android config failed: {error}")))?;
    if payload.kind != CONFIG_FILE_KIND
        || payload.format_version == 0
        || payload.format_version > CONFIG_FILE_VERSION
    {
        return Err(AppError::Other("Unsupported config file".into()));
    }
    if !payload.has_restorable_content(has_listen_together_section) {
        return Err(AppError::Other(
            "Config backup has no restorable content".into(),
        ));
    }

    let settings = android_settings_to_desktop(&payload.settings, current_settings);
    let (auth, auth_platforms) = android_auth_to_desktop(&payload, current_auth);
    let worker_base_url = payload.listen_together.worker_base_url.trim();
    let worker_base_url_input = payload.listen_together.worker_base_url_input.trim();
    let fallback_worker_url = url::Url::parse(worker_base_url_input)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .map(|url| url.to_string())
        .unwrap_or_default();
    let listen_together = ConfigListenTogether {
        user_uuid: payload.listen_together.user_uuid.clone(),
        server_url: if worker_base_url.is_empty() {
            fallback_worker_url
        } else {
            worker_base_url.to_string()
        },
        nickname: payload.listen_together.nickname.clone(),
        allow_member_control: payload.listen_together.allow_member_control,
        auto_pause_on_member_change: payload.listen_together.auto_pause_on_member_change,
        share_audio_links: payload.listen_together.share_audio_links,
    };
    let history_update_mode = if payload
        .sync_preferences
        .play_history_update_mode
        .trim()
        .is_empty()
    {
        payload.git_hub_sync.play_history_update_mode.clone()
    } else {
        payload.sync_preferences.play_history_update_mode.clone()
    };

    let mut warnings = Vec::new();
    if worker_base_url.is_empty()
        && !worker_base_url_input.is_empty()
        && listen_together.server_url.is_empty()
    {
        warnings.push("listen_together_url_invalid".into());
    }
    if payload.you_tube_auth.cookies.is_empty()
        && payload.you_tube_auth.cookie_header.trim().is_empty()
        && !payload.you_tube_auth.authorization.trim().is_empty()
    {
        warnings.push("youtube_authorization_unsupported".into());
    }
    if !payload.you_tube_auth.x_goog_auth_user.trim().is_empty()
        && payload.you_tube_auth.x_goog_auth_user.trim() != "0"
    {
        warnings.push("youtube_multi_account_unsupported".into());
    }
    let _ = (
        &payload.you_tube_auth.origin,
        &payload.you_tube_auth.user_agent,
    );

    let has_github_sync = payload.git_hub_sync.has_data();
    let has_webdav_sync = payload.web_dav_sync.has_data();
    let has_sync_preferences = payload.sync_preferences.has_data()
        || !payload
            .git_hub_sync
            .play_history_update_mode
            .trim()
            .is_empty();

    Ok(ParsedConfigImport {
        settings,
        listen_together: has_listen_together_section.then_some(listen_together),
        language: payload
            .language
            .has_value()
            .then(|| payload.language.clone()),
        auth,
        github_sync: has_github_sync.then_some(ConfigGitHubSync {
            token: payload.git_hub_sync.token,
            owner: payload.git_hub_sync.repo_owner,
            repo: payload.git_hub_sync.repo_name,
            auto_sync: payload.git_hub_sync.auto_sync_enabled,
            data_saver: payload.git_hub_sync.data_saver_mode,
            silent_failures: payload
                .settings
                .booleans
                .get("silent_github_sync_failure")
                .copied()
                .unwrap_or(false),
            history_update_mode: history_update_mode.clone(),
            ..Default::default()
        }),
        webdav_sync: has_webdav_sync.then_some(ConfigWebDavSync {
            server_url: payload.web_dav_sync.server_url,
            username: payload.web_dav_sync.username,
            password: payload.web_dav_sync.password,
            base_path: payload.web_dav_sync.base_path,
            auto_sync: payload.web_dav_sync.auto_sync_enabled,
            ..Default::default()
        }),
        sync_preferences: has_sync_preferences.then_some(ConfigSyncPreferences {
            history_update_mode,
        }),
        auth_platforms,
        platform: "android",
        warnings,
    })
}

async fn hydrate_android_netease_auth(payload: &mut ParsedConfigImport, state: &AppState) {
    if payload.platform != "android" {
        return;
    }
    let Some(auth) = payload.auth.as_mut().and_then(|auth| auth.netease.as_mut()) else {
        return;
    };
    if auth.user_id.is_some() || !auth.cookies.iter().any(|cookie| cookie.name == "MUSIC_U") {
        return;
    }

    let candidate_jar = std::sync::Arc::new(reqwest::cookie::Jar::default());
    crate::auth::cookies::inject_cookies(&candidate_jar, &auth.cookies);
    let profile = state
        .http_with_cookie_jar(candidate_jar)
        .map_err(|error| AppError::Other(error.to_string()))
        .and_then(|http| Ok(crate::api::netease::client::NeteaseClient::new(&http)));
    let result = match profile {
        Ok(client) => tokio::time::timeout(
            crate::auth::netease_hydration::REQUEST_TIMEOUT,
            client.get_user_account(),
        )
        .await
        .map_err(|_| AppError::Other("NetEase account verification timed out".into()))
        .and_then(|result| result)
        .and_then(|body| crate::api::netease::client::parse_netease_account_profile(&body)),
        Err(error) => Err(error),
    };

    match result {
        Ok(profile) => {
            auth.user_id = Some(profile.user_id);
            auth.nickname = profile.nickname;
            auth.avatar_url = profile.avatar_url;
        }
        Err(error) => {
            log::warn!(target: "config-import", "NetEase account verification failed: {error}");
            payload
                .warnings
                .push("netease_auth_verification_failed".into());
        }
    }
}

/// 启动时迁移旧版同步凭据，避免只有打开设置页后才清理明文
pub fn initialize_secure_storage(app: &AppHandle) {
    let _ = load_github_config(app);
    let _ = load_webdav_config(app);
}

fn load_github_config(app: &AppHandle) -> GitHubSyncConfig {
    let mut config: GitHubSyncConfig = app
        .store(SYNC_STORE)
        .ok()
        .and_then(|s| s.get(GITHUB_CONFIG_KEY))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    config.history_update_mode = normalize_history_update_mode(&config.history_update_mode);
    let legacy_token = std::mem::take(&mut config.token);

    if let Some(token) = security::get_secret(security::GITHUB_TOKEN_KEY) {
        config.token = token;
        if !legacy_token.is_empty() {
            save_github_config(app, &config);
        }
        return config;
    }

    if legacy_token.is_empty() {
        return config;
    }

    if security::set_secret(security::GITHUB_TOKEN_KEY, &legacy_token) {
        config.token = legacy_token;
        save_github_config(app, &config);
    } else {
        // 安全存储不可用时清除旧明文，不让凭据继续留在配置文件
        log::error!(target: "sync", "旧版 GitHub Token 迁移到凭据存储失败，已清除明文凭据");
        config.token.clear();
        save_github_config(app, &config);
    }
    config
}

fn persist_sync_secret(key: &str, value: &str, label: &str) -> AppResult<()> {
    let current = security::get_secret(key);
    if value.is_empty() {
        if current.is_some() && !security::delete_secret(key) {
            return Err(AppError::Other(format!(
                "Failed to remove {label} from credential storage"
            )));
        }
    } else if current.as_deref() != Some(value) && !security::set_secret(key, value) {
        return Err(AppError::Other(format!(
            "Failed to save {label} to credential storage"
        )));
    }
    Ok(())
}

fn persist_sync_store_value(app: &AppHandle, key: &str, value: Value) -> AppResult<()> {
    let store = app
        .store(SYNC_STORE)
        .map_err(|error| AppError::Other(error.to_string()))?;
    let previous = store.get(key);
    store.set(key, value);
    if let Err(error) = store.save() {
        match previous {
            Some(value) => store.set(key, value),
            None => {
                store.delete(key);
            }
        }
        let _ = store.save();
        return Err(AppError::Other(error.to_string()));
    }
    Ok(())
}

fn save_github_config_checked(app: &AppHandle, config: &GitHubSyncConfig) -> AppResult<()> {
    let previous_secret = security::get_secret(security::GITHUB_TOKEN_KEY);
    persist_sync_secret(security::GITHUB_TOKEN_KEY, &config.token, "GitHub token")?;
    if let Err(error) =
        persist_sync_store_value(app, GITHUB_CONFIG_KEY, github_config_store_value(config))
    {
        if let Err(rollback_error) = persist_sync_secret(
            security::GITHUB_TOKEN_KEY,
            previous_secret.as_deref().unwrap_or_default(),
            "previous GitHub token",
        ) {
            log::error!(target: "sync", "GitHub credential rollback failed: {rollback_error}");
        }
        return Err(error);
    }
    Ok(())
}

fn save_github_config(app: &AppHandle, config: &GitHubSyncConfig) {
    if let Err(error) = save_github_config_checked(app, config) {
        log::error!(target: "sync", "GitHub sync config persistence failed: {error}");
    }
}

fn load_webdav_config(app: &AppHandle) -> WebDavSyncConfig {
    let mut config: WebDavSyncConfig = app
        .store(SYNC_STORE)
        .ok()
        .and_then(|s| s.get(WEBDAV_CONFIG_KEY))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let legacy_password = std::mem::take(&mut config.password);

    if let Some(password) = security::get_secret(security::WEBDAV_PASSWORD_KEY) {
        config.password = password;
        if !legacy_password.is_empty() {
            save_webdav_config(app, &config);
        }
        return config;
    }

    if legacy_password.is_empty() {
        return config;
    }

    if security::set_secret(security::WEBDAV_PASSWORD_KEY, &legacy_password) {
        config.password = legacy_password;
        save_webdav_config(app, &config);
    } else {
        // 安全存储不可用时清除旧明文，不让凭据继续留在配置文件
        log::error!(target: "sync", "旧版 WebDAV 密码迁移到凭据存储失败，已清除明文凭据");
        config.password.clear();
        save_webdav_config(app, &config);
    }
    config
}

fn save_webdav_config_checked(app: &AppHandle, config: &WebDavSyncConfig) -> AppResult<()> {
    let previous_secret = security::get_secret(security::WEBDAV_PASSWORD_KEY);
    persist_sync_secret(
        security::WEBDAV_PASSWORD_KEY,
        &config.password,
        "WebDAV password",
    )?;
    if let Err(error) =
        persist_sync_store_value(app, WEBDAV_CONFIG_KEY, webdav_config_store_value(config))
    {
        if let Err(rollback_error) = persist_sync_secret(
            security::WEBDAV_PASSWORD_KEY,
            previous_secret.as_deref().unwrap_or_default(),
            "previous WebDAV password",
        ) {
            log::error!(target: "sync", "WebDAV credential rollback failed: {rollback_error}");
        }
        return Err(error);
    }
    Ok(())
}

fn save_webdav_config(app: &AppHandle, config: &WebDavSyncConfig) {
    if let Err(error) = save_webdav_config_checked(app, config) {
        log::error!(target: "sync", "WebDAV sync config persistence failed: {error}");
    }
}

fn load_sync_preferences(app: &AppHandle) -> SyncPreferencesConfig {
    let stored = app
        .store(SYNC_STORE)
        .ok()
        .and_then(|s| s.get(SYNC_PREFERENCES_KEY))
        .and_then(|v| serde_json::from_value::<SyncPreferencesConfig>(v.clone()).ok());
    let has_stored = stored.is_some();

    let mut config = stored.unwrap_or_else(|| {
        // 旧版本把频率放在 GitHub 配置中，首次读取时迁移到全局偏好
        let github = load_github_config(app);
        SyncPreferencesConfig {
            history_update_mode: github.history_update_mode,
        }
    });
    config.history_update_mode = normalize_history_update_mode(&config.history_update_mode);
    if !has_stored {
        save_sync_preferences(app, &config);
    }
    config
}

fn save_sync_preferences_checked(app: &AppHandle, config: &SyncPreferencesConfig) -> AppResult<()> {
    persist_sync_store_value(
        app,
        SYNC_PREFERENCES_KEY,
        sync_preferences_store_value(config),
    )
}

fn save_sync_preferences(app: &AppHandle, config: &SyncPreferencesConfig) {
    if let Err(error) = save_sync_preferences_checked(app, config) {
        log::error!(target: "sync", "Sync preferences persistence failed: {error}");
    }
}

fn github_config_store_value(config: &GitHubSyncConfig) -> Value {
    serde_json::json!({
        "owner": config.owner,
        "repo": config.repo,
        "lastRemoteSha": config.last_remote_sha,
        "lastSyncTime": config.last_sync_time,
        "autoSync": config.auto_sync,
        "dataSaver": config.data_saver,
        "silentFailures": config.silent_failures,
        "historyUpdateMode": normalize_history_update_mode(&config.history_update_mode),
    })
}

fn sync_preferences_store_value(config: &SyncPreferencesConfig) -> Value {
    serde_json::json!({
        "historyUpdateMode": normalize_history_update_mode(&config.history_update_mode),
    })
}

fn webdav_config_store_value(config: &WebDavSyncConfig) -> Value {
    serde_json::json!({
        "serverUrl": config.server_url,
        "username": config.username,
        "basePath": config.base_path,
        "lastRemoteFingerprint": config.last_remote_fingerprint,
        "lastSyncTime": config.last_sync_time,
        "autoSync": config.auto_sync,
    })
}

/// 获取 GitHub 同步配置（不含 token 明文）
#[tauri::command]
pub async fn get_github_sync_config(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Value> {
    let _config_guard = state.config_persistence_gate.lock().await;
    let config = load_github_config(&app);
    let preferences = load_sync_preferences(&app);
    Ok(serde_json::json!({
        "configured": !config.token.is_empty(),
        "owner": config.owner,
        "repo": config.repo,
        "autoSync": config.auto_sync,
        "lastSyncTime": config.last_sync_time,
        "dataSaver": config.data_saver,
        "silentFailures": config.silent_failures,
        "historyUpdateMode": preferences.history_update_mode,
    }))
}

/// 获取全局同步偏好
#[tauri::command]
pub async fn get_sync_preferences(app: AppHandle, state: State<'_, AppState>) -> AppResult<Value> {
    let _config_guard = state.config_persistence_gate.lock().await;
    let preferences = load_sync_preferences(&app);
    Ok(serde_json::json!({
        "historyUpdateMode": preferences.history_update_mode,
    }))
}

/// 更新全局同步偏好，兼容 Android 的频率枚举和旧版 batched 值
#[tauri::command]
pub async fn update_sync_preferences(
    app: AppHandle,
    state: State<'_, AppState>,
    history_update_mode: Option<String>,
) -> AppResult<()> {
    let _config_guard = state.config_persistence_gate.lock().await;
    let mut preferences = load_sync_preferences(&app);
    if let Some(mode) = history_update_mode {
        preferences.history_update_mode = normalize_history_update_mode(&mode);
    }
    save_sync_preferences(&app, &preferences);
    Ok(())
}

/// 验证 GitHub token，返回用户名
#[tauri::command]
pub async fn validate_github_token(
    app: AppHandle,
    state: State<'_, AppState>,
    token: String,
) -> AppResult<Value> {
    let api = crate::sync::github_api::GitHubApiClient::new(&state.http(), &token);
    let username = api.validate_token().await?;

    let _config_guard = state.config_persistence_gate.lock().await;
    // 暂存 token（还没配置完，只保存 token 和 owner）
    let mut config = load_github_config(&app);
    config.token = token;
    config.owner = username.clone();
    save_github_config(&app, &config);

    Ok(serde_json::json!({
        "success": true,
        "username": username,
    }))
}

/// 创建新仓库
#[tauri::command]
pub async fn create_github_repo(
    app: AppHandle,
    state: State<'_, AppState>,
    repo_name: String,
) -> AppResult<Value> {
    let _config_guard = state.config_persistence_gate.lock().await;
    let config = load_github_config(&app);
    if config.token.is_empty() {
        return Err(AppError::Api("Token not validated yet".into()));
    }

    let api = crate::sync::github_api::GitHubApiClient::new(&state.http(), &config.token);
    api.create_repository(&repo_name).await?;

    let updated = GitHubSyncConfig {
        token: config.token,
        owner: config.owner.clone(),
        repo: repo_name.clone(),
        auto_sync: true,
        data_saver: true, // 默认开启省流
        ..Default::default()
    };
    save_github_config(&app, &updated);

    Ok(serde_json::json!({
        "success": true,
        "owner": config.owner,
        "repo": repo_name,
    }))
}

/// 使用已有仓库
#[tauri::command]
pub async fn use_existing_github_repo(
    app: AppHandle,
    state: State<'_, AppState>,
    owner: String,
    repo: String,
) -> AppResult<Value> {
    let _config_guard = state.config_persistence_gate.lock().await;
    let config = load_github_config(&app);
    if config.token.is_empty() {
        return Err(AppError::Api("Token not validated yet".into()));
    }

    let api = crate::sync::github_api::GitHubApiClient::new(&state.http(), &config.token);
    let _branch = api.check_repository(&owner, &repo).await?;

    let updated = GitHubSyncConfig {
        token: config.token,
        owner: owner.clone(),
        repo: repo.clone(),
        auto_sync: true,
        data_saver: true, // 默认开启省流
        ..Default::default()
    };
    save_github_config(&app, &updated);

    Ok(serde_json::json!({
        "success": true,
        "owner": owner,
        "repo": repo,
    }))
}

/// 配置 GitHub 同步（保留兼容，内部调用两阶段）
#[tauri::command]
pub async fn configure_github_sync(
    app: AppHandle,
    state: State<'_, AppState>,
    token: String,
    repo: String,
) -> AppResult<Value> {
    let api = crate::sync::github_api::GitHubApiClient::new(&state.http(), &token);
    let owner = api.validate_token().await?;

    match api.check_repository(&owner, &repo).await {
        Ok(_) => {}
        Err(error) if error.is_not_found() => api.create_repository(&repo).await?,
        Err(error) => return Err(error.into()),
    }

    let _config_guard = state.config_persistence_gate.lock().await;
    let config = GitHubSyncConfig {
        token,
        owner: owner.clone(),
        repo: repo.clone(),
        auto_sync: true,
        data_saver: true, // 默认开启省流
        ..Default::default()
    };
    save_github_config(&app, &config);

    Ok(serde_json::json!({
        "success": true,
        "owner": owner,
        "repo": repo,
    }))
}

/// 执行 GitHub 同步
#[tauri::command]
pub async fn sync_github(
    app: AppHandle,
    state: State<'_, AppState>,
    history_entries: Option<Vec<manager::SyncHistoryEntry>>,
    history_deletions: Option<Vec<manager::SyncHistoryDeletion>>,
) -> AppResult<SyncResult> {
    let _config_guard = state.config_persistence_gate.lock().await;
    let mut config = load_github_config(&app);
    if config.token.is_empty() {
        return Err(AppError::Api("GitHub sync not configured".into()));
    }

    let result = manager::sync_github(
        &state.http(),
        &mut config,
        &app,
        history_entries.as_deref(),
        history_deletions.as_deref(),
    )
    .await?;
    save_github_config(&app, &config);
    // 通知前端歌单数据可能已变更
    let _ = app.emit("playlists-changed", ());
    let _ = app.emit("favorite-playlists-changed", ());
    Ok(result)
}

/// 断开 GitHub 同步
#[tauri::command]
pub async fn disconnect_github_sync(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let _config_guard = state.config_persistence_gate.lock().await;
    save_github_config(&app, &GitHubSyncConfig::default());
    Ok(())
}

/// 获取 WebDAV 同步配置
#[tauri::command]
pub async fn get_webdav_sync_config(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Value> {
    let _config_guard = state.config_persistence_gate.lock().await;
    let config = load_webdav_config(&app);
    let preferences = load_sync_preferences(&app);
    Ok(serde_json::json!({
        "configured": !config.server_url.is_empty(),
        "serverUrl": config.server_url,
        "basePath": config.base_path,
        "autoSync": config.auto_sync,
        "lastSyncTime": config.last_sync_time,
        "historyUpdateMode": preferences.history_update_mode,
    }))
}

/// 配置 WebDAV 同步
#[tauri::command]
pub async fn configure_webdav_sync(
    app: AppHandle,
    state: State<'_, AppState>,
    server_url: String,
    username: String,
    password: String,
    base_path: Option<String>,
) -> AppResult<Value> {
    let bp = base_path.unwrap_or_default();
    let api = crate::sync::webdav_api::WebDavApiClient::new(
        &state.http(),
        &server_url,
        &username,
        &password,
        &bp,
    );
    api.validate_connection().await?;

    let _config_guard = state.config_persistence_gate.lock().await;
    let config = WebDavSyncConfig {
        server_url: server_url.clone(),
        username,
        password,
        base_path: bp,
        auto_sync: true,
        ..Default::default()
    };
    save_webdav_config(&app, &config);

    Ok(serde_json::json!({
        "success": true,
        "serverUrl": server_url,
    }))
}

/// 执行 WebDAV 同步
#[tauri::command]
pub async fn sync_webdav(
    app: AppHandle,
    state: State<'_, AppState>,
    history_entries: Option<Vec<manager::SyncHistoryEntry>>,
    history_deletions: Option<Vec<manager::SyncHistoryDeletion>>,
) -> AppResult<SyncResult> {
    let _config_guard = state.config_persistence_gate.lock().await;
    let mut config = load_webdav_config(&app);
    if config.server_url.is_empty() {
        return Err(AppError::Api("WebDAV sync not configured".into()));
    }

    let result = manager::sync_webdav(
        &state.http(),
        &mut config,
        &app,
        history_entries.as_deref(),
        history_deletions.as_deref(),
    )
    .await?;
    save_webdav_config(&app, &config);
    let _ = app.emit("playlists-changed", ());
    let _ = app.emit("favorite-playlists-changed", ());
    Ok(result)
}

/// 更新 GitHub 同步子设置（不影响 token/owner/repo）
#[tauri::command]
pub async fn update_github_sync_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    auto_sync: Option<bool>,
    data_saver: Option<bool>,
    silent_failures: Option<bool>,
    history_update_mode: Option<String>,
) -> AppResult<()> {
    let _config_guard = state.config_persistence_gate.lock().await;
    let mut config = load_github_config(&app);
    if config.token.is_empty() {
        return Err(AppError::Api("GitHub sync not configured".into()));
    }
    if let Some(v) = auto_sync {
        config.auto_sync = v;
    }
    if let Some(v) = data_saver {
        config.data_saver = v;
    }
    if let Some(v) = silent_failures {
        config.silent_failures = v;
    }
    if let Some(v) = history_update_mode {
        let normalized = normalize_history_update_mode(&v);
        config.history_update_mode = normalized.clone();
        save_sync_preferences(
            &app,
            &SyncPreferencesConfig {
                history_update_mode: normalized,
            },
        );
    }
    save_github_config(&app, &config);
    Ok(())
}

/// 更新 WebDAV 同步子设置
#[tauri::command]
pub async fn update_webdav_sync_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    auto_sync: Option<bool>,
) -> AppResult<()> {
    let _config_guard = state.config_persistence_gate.lock().await;
    let mut config = load_webdav_config(&app);
    if config.server_url.is_empty() {
        return Err(AppError::Api("WebDAV sync not configured".into()));
    }
    if let Some(v) = auto_sync {
        config.auto_sync = v;
    }
    save_webdav_config(&app, &config);
    Ok(())
}

/// 清除应用缓存（音频/图片缓存目录 + app_data_dir 下的临时缓存子目录）
#[tauri::command]
pub async fn clear_app_cache(app: AppHandle) -> AppResult<Value> {
    let mut cleared: u64 = 0;
    let mut failed: u64 = 0;

    // 清理 app_cache_dir
    if let Ok(cache_dir) = app.path().app_cache_dir() {
        let (c, f) = clear_directory_contents(&cache_dir);
        cleared += c;
        failed += f;
    }

    // 清理 app_data_dir 下的缓存子目录（covers, temp 等）
    if let Ok(data_dir) = app.path().app_data_dir() {
        for sub in &["covers", "temp", "cache", "thumbnails"] {
            let sub_dir = data_dir.join(sub);
            if sub_dir.exists() && sub_dir.is_dir() {
                let (c, f) = clear_directory_contents(&sub_dir);
                cleared += c;
                failed += f;
            }
        }
    }

    log::info!(target: "sync", "clear_app_cache: cleared {} bytes, {} failures", cleared, failed);
    Ok(serde_json::json!({ "clearedBytes": cleared, "failedCount": failed }))
}

/// 清除目录下所有内容，返回 (cleared_bytes, failed_count)
fn clear_directory_contents(dir: &std::path::Path) -> (u64, u64) {
    let mut cleared: u64 = 0;
    let mut failed: u64 = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                if std::fs::remove_file(&path).is_ok() {
                    cleared += size;
                } else {
                    failed += 1;
                }
            } else if path.is_dir() {
                if let Ok(size) = dir_size(&path) {
                    cleared += size;
                }
                if std::fs::remove_dir_all(&path).is_err() {
                    failed += 1;
                }
            }
        }
    }
    (cleared, failed)
}

fn dir_size(path: &std::path::Path) -> std::io::Result<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_file() {
            total += meta.len();
        } else if meta.is_dir() {
            total += dir_size(&entry.path())?;
        }
    }
    Ok(total)
}

/// 导出播放列表为 JSON（Android BackupData 兼容格式）
#[tauri::command]
pub async fn export_playlists(app: AppHandle) -> AppResult<Value> {
    use crate::library::playlist::PlaylistStore;
    let playlists_path = {
        let mut path = dirs_next::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        path.push("NeriPlayer");
        path.push("playlists.json");
        path
    };
    let store = PlaylistStore::load(&playlists_path);

    // 转换为 SyncPlaylist 格式（Android 兼容）
    let sync_playlists: Vec<crate::sync::models::SyncPlaylist> = store
        .playlists
        .iter()
        .map(|pl| crate::sync::models::SyncPlaylist {
            id: pl.id.to_string(),
            name: pl.name.clone(),
            songs: crate::sync::manager::tracks_to_sync_songs_pub(&pl.tracks),
            created_at: pl.modified_at as i64,
            modified_at: pl.modified_at as i64,
            is_deleted: false,
            song_order_version: 1,
        })
        .collect();

    let backup_data = serde_json::json!({
        "version": "2.0",
        "timestamp": chrono::Utc::now().timestamp_millis(),
        "exportDate": chrono::Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string(),
        "playlists": sync_playlists,
    });

    let json_data = serde_json::to_string_pretty(&backup_data)
        .map_err(|e| AppError::Other(format!("Serialize failed: {}", e)))?;

    use tauri_plugin_dialog::DialogExt;
    let path = app
        .dialog()
        .file()
        .set_file_name("neriplayer-playlists.json")
        .add_filter("JSON", &["json"])
        .blocking_save_file();

    match path {
        Some(p) => {
            std::fs::write(p.as_path().unwrap(), &json_data)
                .map_err(|e| AppError::Other(format!("Write failed: {}", e)))?;
            Ok(serde_json::json!({ "success": true, "count": store.playlists.len() }))
        }
        None => Ok(serde_json::json!({ "success": false, "reason": "cancelled" })),
    }
}

/// 导入播放列表（兼容 Android BackupData、完整同步快照和 Desktop 格式）
#[tauri::command]
pub async fn import_playlists(app: AppHandle) -> AppResult<Value> {
    use crate::library::playlist::Playlist;
    use crate::sync::models::{SyncData, SyncPlaylist};
    use tauri_plugin_dialog::DialogExt;

    let path = app
        .dialog()
        .file()
        .add_filter("NeriPlayer backup", &["json", "bin"])
        .blocking_pick_file();

    match path {
        Some(p) => {
            let file_path = p.as_path().unwrap();
            let is_binary = file_path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("bin"));
            let metadata = std::fs::metadata(file_path)
                .map_err(|e| AppError::Other(format!("Read metadata failed: {}", e)))?;
            let max_input_bytes = if is_binary {
                24 * 1024 * 1024
            } else {
                12 * 1024 * 1024
            };
            if metadata.len() > max_input_bytes {
                return Err(AppError::Other("Playlist backup is too large".into()));
            }
            let data = std::fs::read_to_string(file_path)
                .map_err(|e| AppError::Other(format!("Read failed: {}", e)))?;
            let count;
            let mut imported_favorites = 0usize;
            let mut favorites_changed = false;
            let online_favorites_available;

            if is_binary {
                let sync_data = crate::sync::serializer::deserialize(&data, true)?;
                count = sync_data.playlists.len();
                online_favorites_available = true;
                favorites_changed = sync_data
                    .favorite_playlists
                    .iter()
                    .any(|favorite| !favorite.is_deleted);
                imported_favorites = manager::save_imported_playlist_backup(&sync_data).await?;
            } else {
                let parsed: serde_json::Value = serde_json::from_str(&data)
                    .map_err(|e| AppError::Other(format!("Parse failed: {}", e)))?;

                // Android BackupData / SyncData 使用对象，Desktop 旧格式使用数组。
                online_favorites_available = parsed
                    .as_object()
                    .is_some_and(|object| object.contains_key("favoritePlaylists"));
                if parsed.is_object() && parsed.get("playlists").is_some() {
                    // Android BackupData / SyncData：完整解析以保留可选的 favoritePlaylists。
                    let sync_data: SyncData = serde_json::from_value(parsed)
                        .map_err(|e| AppError::Other(format!("Parse Android playlists: {}", e)))?;
                    count = sync_data.playlists.len();
                    favorites_changed = sync_data
                        .favorite_playlists
                        .iter()
                        .any(|favorite| !favorite.is_deleted);
                    imported_favorites = manager::save_imported_playlist_backup(&sync_data).await?;
                } else if parsed.is_array() {
                    if let Ok(imported) = serde_json::from_value::<Vec<Playlist>>(parsed.clone()) {
                        count = imported.len();
                        manager::save_imported_desktop_playlists(imported).await?;
                    } else {
                        let sync_playlists: Vec<SyncPlaylist> = serde_json::from_value(parsed)
                            .map_err(|e| {
                                AppError::Other(format!("Parse playlists array: {}", e))
                            })?;
                        count = sync_playlists.len();
                        let sync_data = SyncData {
                            playlists: sync_playlists,
                            ..Default::default()
                        };
                        manager::save_imported_playlists(&sync_data).await?;
                    }
                } else {
                    return Err(AppError::Other("Unrecognized playlist format".into()));
                }
            }

            let _ = app.emit("playlists-changed", ());
            if favorites_changed {
                let _ = app.emit("favorite-playlists-changed", ());
            }
            Ok(serde_json::json!({
                "success": true,
                "imported": count,
                "importedFavorites": imported_favorites,
                "onlineFavoritesAvailable": online_favorites_available,
            }))
        }
        None => Ok(serde_json::json!({ "success": false, "reason": "cancelled" })),
    }
}

/// 导出 PC 配置文件，配置中包含登录凭据和同步密钥
#[tauri::command]
pub async fn export_config(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: AppSettings,
    listen_together_user_uuid: String,
) -> AppResult<Value> {
    use tauri_plugin_dialog::DialogExt;

    let _config_guard = state.config_persistence_gate.lock().await;
    let auth = state.auth.lock().clone();
    let preferences = load_sync_preferences(&app);
    let mut github = load_github_config(&app);
    github.history_update_mode = preferences.history_update_mode.clone();
    let webdav = load_webdav_config(&app);
    let config = DesktopConfigFile {
        kind: CONFIG_FILE_KIND.into(),
        format_version: CONFIG_FILE_VERSION,
        platform: "pc".into(),
        platform_name: "NeriPlayer Desktop".into(),
        exported_at: chrono::Utc::now().timestamp_millis(),
        listen_together: Some(ConfigListenTogether {
            user_uuid: listen_together_user_uuid,
            server_url: settings.lt_server_url.clone(),
            nickname: settings.lt_nickname.clone(),
            allow_member_control: settings.lt_allow_member_control,
            auto_pause_on_member_change: settings.lt_auto_pause_on_member_change,
            share_audio_links: settings.lt_share_audio_links,
        }),
        language: Some(ConfigLanguage {
            code: settings.locale.clone(),
        }),
        settings,
        auth: Some(auth),
        github_sync: Some(ConfigGitHubSync::from(&github)),
        webdav_sync: Some(ConfigWebDavSync::from(&webdav)),
        sync_preferences: Some(ConfigSyncPreferences::from(&preferences)),
    };
    let content = serde_json::to_string_pretty(&config)
        .map_err(|e| AppError::Other(format!("Serialize config failed: {}", e)))?;
    drop(_config_guard);
    let file_name = format!(
        "neriplayer-desktop-config-{}.json",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );
    let path = app
        .dialog()
        .file()
        .set_file_name(file_name)
        .add_filter("JSON", &["json"])
        .blocking_save_file();

    match path {
        Some(path) => {
            std::fs::write(path.as_path().unwrap(), content)
                .map_err(|e| AppError::Other(format!("Write config failed: {}", e)))?;
            Ok(serde_json::json!({
                "success": true,
                "platform": "pc",
                "platformName": "NeriPlayer Desktop",
            }))
        }
        None => Ok(serde_json::json!({ "success": false, "reason": "cancelled" })),
    }
}

fn rollback_config_import_persistence(
    app: &AppHandle,
    previous_settings: &AppSettings,
    previous_auth: Option<&crate::auth::state::AuthState>,
    previous_preferences: Option<&SyncPreferencesConfig>,
    previous_github: Option<&GitHubSyncConfig>,
    previous_webdav: Option<&WebDavSyncConfig>,
) {
    if let Some(auth) = previous_auth {
        if let Err(error) = crate::auth::cookies::save_auth_strict(app, auth) {
            log::error!(target: "config-import", "Authentication rollback failed: {error}");
        }
    }
    if let Err(error) = store::save_settings(app, previous_settings.clone()) {
        log::error!(target: "config-import", "Settings rollback failed: {error}");
    }
    if let Some(webdav) = previous_webdav {
        if let Err(error) = save_webdav_config_checked(app, webdav) {
            log::error!(target: "config-import", "WebDAV config rollback failed: {error}");
        }
    }
    if let Some(github) = previous_github {
        if let Err(error) = save_github_config_checked(app, github) {
            log::error!(target: "config-import", "GitHub config rollback failed: {error}");
        }
    }
    if let Some(preferences) = previous_preferences {
        if let Err(error) = save_sync_preferences_checked(app, preferences) {
            log::error!(target: "config-import", "Sync preferences rollback failed: {error}");
        }
    }
}

/// 导入 PC 或 Android 配置文件并恢复设置、登录状态和同步配置
#[tauri::command]
pub async fn import_config(app: AppHandle, state: State<'_, AppState>) -> AppResult<Value> {
    use tauri_plugin_dialog::DialogExt;

    let path = app
        .dialog()
        .file()
        .add_filter("JSON", &["json"])
        .blocking_pick_file();
    let Some(path) = path else {
        return Ok(serde_json::json!({ "success": false, "reason": "cancelled" }));
    };
    let file_path = path.as_path().unwrap();
    let metadata = std::fs::metadata(file_path)
        .map_err(|e| AppError::Other(format!("Read config metadata failed: {}", e)))?;
    if metadata.len() > 2 * 1024 * 1024 {
        return Err(AppError::Other("Config file is too large".into()));
    }
    let content = std::fs::read_to_string(file_path)
        .map_err(|e| AppError::Other(format!("Read config failed: {}", e)))?;
    let _config_guard = state.config_persistence_gate.lock().await;
    let _cookie_guard = state.auth_cookie_gate.lock().await;
    let current_settings = store::load_settings(&app)?.settings;
    let current_auth = state.auth.lock().clone();
    let mut payload = parse_config_import(&content, current_settings.clone(), &current_auth)?;

    let mut settings = payload.settings.clone();
    if let Some(language) = payload
        .language
        .as_ref()
        .filter(|language| !language.code.is_empty())
    {
        settings.locale = language.code.clone();
    }
    if let Some(listen_together) = payload.listen_together.as_ref() {
        if !listen_together.server_url.is_empty() {
            settings.lt_server_url = listen_together.server_url.clone();
        }
        settings.lt_nickname = listen_together.nickname.clone();
        settings.lt_allow_member_control = listen_together.allow_member_control;
        settings.lt_auto_pause_on_member_change = listen_together.auto_pause_on_member_change;
        settings.lt_share_audio_links = listen_together.share_audio_links;
    }
    state.rebuild_http(settings.bypass_proxy);
    hydrate_android_netease_auth(&mut payload, &state).await;

    let imported_platform = payload.platform;
    let imported_listen_together = payload.listen_together;
    let legacy_history_mode = payload
        .github_sync
        .as_ref()
        .map(|github| github.history_update_mode.clone());
    let imported_preferences = if let Some(preferences) = payload.sync_preferences {
        Some(preferences.into_config())
    } else if let Some(mode) = legacy_history_mode {
        Some(SyncPreferencesConfig {
            history_update_mode: normalize_history_update_mode(&mode),
        })
    } else {
        None
    };
    let imported_github = payload.github_sync.map(ConfigGitHubSync::into_config);
    let imported_webdav = payload.webdav_sync.map(ConfigWebDavSync::into_config);
    let auth_platforms = payload.auth_platforms;
    let imported_auth = payload.auth;

    let previous_auth = state.auth.lock().clone();
    let imported_auth = imported_auth
        .map(|auth| merge_imported_auth_platforms(previous_auth.clone(), auth, &auth_platforms));
    let previous_preferences = imported_preferences
        .as_ref()
        .map(|_| load_sync_preferences(&app));
    let previous_github = imported_github.as_ref().map(|_| load_github_config(&app));
    let previous_webdav = imported_webdav.as_ref().map(|_| load_webdav_config(&app));

    let persistence_result = (|| -> AppResult<AppSettings> {
        if let Some(preferences) = imported_preferences.as_ref() {
            save_sync_preferences_checked(&app, preferences)?;
        }
        if let Some(github) = imported_github.as_ref() {
            save_github_config_checked(&app, github)?;
        }
        if let Some(webdav) = imported_webdav.as_ref() {
            save_webdav_config_checked(&app, webdav)?;
        }
        let persisted_settings = store::save_settings(&app, settings.clone())?;
        if let Some(auth) = imported_auth.as_ref() {
            crate::auth::cookies::save_auth_strict(&app, auth)?;
        }
        Ok(persisted_settings)
    })();

    let settings = match persistence_result {
        Ok(settings) => settings,
        Err(error) => {
            rollback_config_import_persistence(
                &app,
                &current_settings,
                imported_auth.as_ref().map(|_| &previous_auth),
                previous_preferences.as_ref(),
                previous_github.as_ref(),
                previous_webdav.as_ref(),
            );
            state.rebuild_http(current_settings.bypass_proxy);
            return Err(AppError::Other(format!(
                "Config import persistence failed: {error}"
            )));
        }
    };

    if let Some(imported_auth) = imported_auth {
        for platform in &auth_platforms {
            crate::auth::cookies::expire_platform_cookies(
                &state.cookie_jar,
                &previous_auth,
                platform,
            );
        }
        *state.auth.lock() = imported_auth.clone();

        if let Err(error) =
            crate::commands::auth_cmd::clear_and_reinject_webview_cookies(&app, &state).await
        {
            *state.auth.lock() = previous_auth.clone();
            for platform in &auth_platforms {
                crate::auth::cookies::expire_platform_cookies(
                    &state.cookie_jar,
                    &imported_auth,
                    platform,
                );
            }
            crate::auth::cookies::inject_all(&state.cookie_jar, &previous_auth);
            rollback_config_import_persistence(
                &app,
                &current_settings,
                Some(&previous_auth),
                previous_preferences.as_ref(),
                previous_github.as_ref(),
                previous_webdav.as_ref(),
            );
            state.rebuild_http(current_settings.bypass_proxy);
            let _ =
                crate::commands::auth_cmd::clear_and_reinject_webview_cookies(&app, &state).await;
            return Err(error);
        }
        state.netease_hydration.lock().reset();
    }

    Ok(serde_json::json!({
        "success": true,
        "settings": settings,
        "listenTogetherUserUuid": imported_listen_together
            .map(|listen_together| listen_together.user_uuid)
            .unwrap_or_default(),
        "platform": imported_platform,
        "warnings": payload.warnings,
    }))
}

/// 断开 WebDAV 同步
#[tauri::command]
pub async fn disconnect_webdav_sync(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let _config_guard = state.config_persistence_gate.lock().await;
    save_webdav_config(&app, &WebDavSyncConfig::default());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        github_config_store_value, parse_config_import, webdav_config_store_value,
        ConfigGitHubSync, ConfigSyncPreferences, ConfigWebDavSync, DesktopConfigFile,
        CONFIG_FILE_KIND, CONFIG_FILE_VERSION,
    };
    use crate::auth::state::{AuthState, CookieEntry, YouTubeAuth};
    use crate::settings::store::AppSettings;
    use crate::sync::models::{GitHubSyncConfig, SyncPreferencesConfig, WebDavSyncConfig};

    #[test]
    fn github_store_value_excludes_token() {
        let config = GitHubSyncConfig {
            token: "secret-token".into(),
            owner: "owner".into(),
            repo: "repo".into(),
            ..Default::default()
        };
        let value = github_config_store_value(&config);

        assert!(value.get("token").is_none());
        assert!(serde_json::to_value(&config)
            .unwrap()
            .get("token")
            .is_none());
        assert_eq!(value["owner"], "owner");
    }

    #[test]
    fn webdav_store_value_excludes_password() {
        let config = WebDavSyncConfig {
            server_url: "https://dav.example.test".into(),
            username: "user".into(),
            password: "secret-password".into(),
            ..Default::default()
        };
        let value = webdav_config_store_value(&config);

        assert!(value.get("password").is_none());
        assert!(serde_json::to_value(&config)
            .unwrap()
            .get("password")
            .is_none());
        assert_eq!(value["serverUrl"], "https://dav.example.test");
    }

    #[test]
    fn desktop_config_marks_pc_and_contains_sensitive_sections() {
        let value = serde_json::to_value(DesktopConfigFile {
            kind: CONFIG_FILE_KIND.into(),
            format_version: CONFIG_FILE_VERSION,
            platform: "pc".into(),
            platform_name: "NeriPlayer Desktop".into(),
            exported_at: 1,
            settings: AppSettings::default(),
            listen_together: Some(Default::default()),
            language: Some(Default::default()),
            auth: Some(AuthState::default()),
            github_sync: Some(ConfigGitHubSync {
                token: "token".into(),
                ..Default::default()
            }),
            webdav_sync: Some(ConfigWebDavSync {
                password: "password".into(),
                ..Default::default()
            }),
            sync_preferences: Some(ConfigSyncPreferences::default()),
        })
        .unwrap();

        assert_eq!(value["platform"], "pc");
        assert_eq!(value["platformName"], "NeriPlayer Desktop");
        assert_eq!(value["githubSync"]["token"], "token");
        assert_eq!(value["webdavSync"]["password"], "password");
    }

    #[test]
    fn sync_preferences_store_value_uses_canonical_mode() {
        let value = super::sync_preferences_store_value(&SyncPreferencesConfig {
            history_update_mode: "EVERY_15_MINUTES".into(),
        });

        assert_eq!(value["historyUpdateMode"], "every_15_minutes");
    }

    #[test]
    fn android_config_maps_mobile_sections_to_desktop() {
        let content = serde_json::json!({
            "kind": CONFIG_FILE_KIND,
            "formatVersion": CONFIG_FILE_VERSION,
            "settings": {
                "booleans": {
                    "follow_system_dark": false,
                    "force_dark": false,
                    "netease_auto_source_switch": false
                },
                "floats": { "playback_speed": 1.25 },
                "ints": {},
                "longs": {
                    "cloud_music_lyric_default_offset_ms": 750,
                    "max_cache_size_bytes": 536870912
                },
                "strings": {
                    "audio_quality": "lossless",
                    "download_file_name_template": "{artist} - {title}"
                }
            },
            "listenTogether": {
                "workerBaseUrl": "https://worker.example",
                "userUuid": "mobile-user",
                "nickname": "Mobile"
            },
            "language": { "code": "en" },
            "neteaseAuth": { "cookies": { "MUSIC_U": "netease-cookie" } },
            "biliAuth": { "cookies": { "DedeUserID": "123" } },
            "youTubeAuth": { "cookieHeader": "SID=youtube-cookie" },
            "gitHubSync": {
                "token": "github-token",
                "repoOwner": "owner",
                "repoName": "repo",
                "autoSyncEnabled": true,
                "playHistoryUpdateMode": "EVERY_15_MINUTES"
            },
            "webDavSync": {
                "serverUrl": "https://dav.example",
                "basePath": "/neri",
                "username": "user",
                "password": "password",
                "autoSyncEnabled": true
            },
            "syncPreferences": { "playHistoryUpdateMode": "EVERY_30_MINUTES" }
        })
        .to_string();

        let mut current_settings = AppSettings::default();
        current_settings.download_dir = "D:/desktop-music".into();
        current_settings.volume = 0.42;
        current_settings.log_level = "debug".into();
        let parsed =
            parse_config_import(&content, current_settings, &AuthState::default()).unwrap();

        assert_eq!(parsed.platform, "android");
        assert_eq!(parsed.settings.dark_mode, "light");
        assert!(!parsed.settings.netease_auto_source_switch);
        assert_eq!(parsed.settings.playback_speed, 1.25);
        assert_eq!(parsed.settings.cloud_music_offset, 750);
        assert_eq!(parsed.settings.max_cache_size, 512);
        assert_eq!(parsed.settings.netease_quality, "lossless");
        assert_eq!(parsed.settings.download_dir, "D:/desktop-music");
        assert_eq!(parsed.settings.volume, 0.42);
        assert_eq!(parsed.settings.log_level, "debug");
        assert_eq!(parsed.language.unwrap().code, "en");
        assert_eq!(parsed.listen_together.unwrap().user_uuid, "mobile-user");
        assert_eq!(parsed.auth.unwrap().bilibili.unwrap().mid, Some(123));
        assert_eq!(parsed.github_sync.unwrap().owner, "owner");
        assert_eq!(parsed.webdav_sync.unwrap().base_path, "/neri");
        assert_eq!(
            parsed.sync_preferences.unwrap().history_update_mode,
            "EVERY_30_MINUTES"
        );
    }

    #[test]
    fn android_config_preserves_unconverted_platform_auth() {
        let content = serde_json::json!({
            "kind": CONFIG_FILE_KIND,
            "formatVersion": CONFIG_FILE_VERSION,
            "settings": {
                "booleans": { "netease_auto_source_switch": false }
            },
            "neteaseAuth": { "cookies": { "MUSIC_U": "phone-session" } },
            "youTubeAuth": { "authorization": "SAPISIDHASH unsupported-on-desktop" }
        })
        .to_string();
        let current_auth = AuthState {
            youtube: Some(YouTubeAuth {
                cookies: vec![CookieEntry {
                    name: "SAPISID".into(),
                    value: "desktop-session".into(),
                    domain: ".youtube.com".into(),
                }],
                nickname: Some("Desktop account".into()),
                avatar_url: None,
            }),
            ..Default::default()
        };

        let parsed = parse_config_import(&content, AppSettings::default(), &current_auth).unwrap();
        let imported_auth = parsed.auth.expect("NetEase cookies should be imported");

        assert_eq!(
            imported_auth.youtube.unwrap().cookies[0].value,
            "desktop-session"
        );
        assert_eq!(
            imported_auth.netease.unwrap().cookies[0].value,
            "phone-session"
        );
        assert!(parsed
            .warnings
            .iter()
            .any(|warning| warning == "youtube_authorization_unsupported"));
    }

    #[test]
    fn android_config_rejects_a_header_without_restorable_sections() {
        let content = serde_json::json!({
            "kind": CONFIG_FILE_KIND,
            "formatVersion": CONFIG_FILE_VERSION
        })
        .to_string();

        let error = parse_config_import(&content, AppSettings::default(), &AuthState::default())
            .err()
            .expect("empty Android config must be rejected");

        assert!(error
            .to_string()
            .contains("Config backup has no restorable content"));
    }
}
