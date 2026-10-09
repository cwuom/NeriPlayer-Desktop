// 同步相关命令
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};
use crate::auth::state::{AuthState, BiliAuth, CookieEntry, NeteaseAuth, YouTubeAuth};
use crate::error::{AppError, AppResult};
use crate::settings::store::{self, AppSettings};
use crate::state::AppState;
use crate::sync::failure::{GitHubCooldown, SyncFailure};
use crate::sync::models::*;
use crate::sync::manager;
use crate::library::playlist;
use crate::security;
use tauri_plugin_store::StoreExt;

// 同步配置存储键
const GITHUB_CONFIG_KEY: &str = "githubSync";
const WEBDAV_CONFIG_KEY: &str = "webdavSync";
const SYNC_PREFERENCES_KEY: &str = "syncPreferences";
const SYNC_STORE: &str = "sync-config.json";
const CONFIG_FILE_KIND: &str = "moe.ouom.neriplayer.config";
const CONFIG_FILE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy)]
enum ConfigProvider {
    GitHub,
    WebDav,
}

#[derive(Debug, Clone, Copy)]
struct ConfigRequestToken {
    provider: ConfigProvider,
    generation: u64,
}

#[derive(Default)]
struct ConfigRequestGenerations {
    github: u64,
    webdav: u64,
    github_preferences: GitHubPreferenceRevisions,
    webdav_preferences: WebDavPreferenceRevisions,
    sync_preferences_revision: u64,
}

#[derive(Clone, Copy, Default)]
struct GitHubPreferenceRevisions {
    auto_sync: u64,
    data_saver: u64,
    silent_failures: u64,
    history_update_mode: u64,
}

#[derive(Clone, Copy, Default)]
struct WebDavPreferenceRevisions {
    auto_sync: u64,
    data_saver: u64,
}

fn advance_preference_revision(revision: &mut u64) -> AppResult<()> {
    *revision = revision
        .checked_add(1)
        .ok_or_else(|| AppError::Other("Sync preference revision exhausted".into()))?;
    Ok(())
}

impl ConfigRequestGenerations {
    fn snapshot(&self, provider: ConfigProvider) -> ConfigRequestToken {
        ConfigRequestToken {
            provider,
            generation: match provider {
                ConfigProvider::GitHub => self.github,
                ConfigProvider::WebDav => self.webdav,
            },
        }
    }

    fn begin(&mut self, provider: ConfigProvider) -> AppResult<ConfigRequestToken> {
        let generation = match provider {
            ConfigProvider::GitHub => &mut self.github,
            ConfigProvider::WebDav => &mut self.webdav,
        };
        *generation = generation
            .checked_add(1)
            .ok_or_else(|| AppError::Other("Sync configuration generation exhausted".into()))?;
        Ok(self.snapshot(provider))
    }

    fn invalidate(&mut self, provider: ConfigProvider) -> AppResult<()> {
        self.begin(provider).map(|_| ())
    }

    fn is_current(&self, token: ConfigRequestToken) -> bool {
        self.snapshot(token.provider).generation == token.generation
    }

    fn complete<R>(
        &self,
        token: ConfigRequestToken,
        apply: impl FnOnce() -> AppResult<R>,
    ) -> AppResult<R> {
        if !self.is_current(token) {
            return Err(AppError::Other(
                "Sync configuration request superseded by a newer action".into(),
            ));
        }
        apply()
    }
}

fn with_sync_config<R>(operation: impl FnOnce() -> R) -> R {
    with_config_generations(|_| operation())
}

fn with_config_generations<R>(operation: impl FnOnce(&mut ConfigRequestGenerations) -> R) -> R {
    // 配置变更和本地同步提交共用临界区，网络请求始终在锁外执行
    static CONFIG_LOCK: std::sync::OnceLock<parking_lot::Mutex<ConfigRequestGenerations>> =
        std::sync::OnceLock::new();
    let mut guard = CONFIG_LOCK
        .get_or_init(|| parking_lot::Mutex::new(ConfigRequestGenerations::default()))
        .lock();
    operation(&mut guard)
}

fn github_connection_preferences(
    started: GitHubPreferenceRevisions,
    latest: GitHubPreferenceRevisions,
    current: &GitHubSyncConfig,
    mut proposed: GitHubSyncConfig,
) -> GitHubSyncConfig {
    if latest.auto_sync != started.auto_sync {
        proposed.auto_sync = current.auto_sync;
    }
    if latest.data_saver != started.data_saver {
        proposed.data_saver = current.data_saver;
    }
    if latest.silent_failures != started.silent_failures {
        proposed.silent_failures = current.silent_failures;
    }
    if latest.history_update_mode != started.history_update_mode {
        proposed.history_update_mode = current.history_update_mode.clone();
    }
    proposed
}

fn webdav_connection_preferences(
    started: WebDavPreferenceRevisions,
    latest: WebDavPreferenceRevisions,
    current: &WebDavSyncConfig,
    mut proposed: WebDavSyncConfig,
) -> WebDavSyncConfig {
    if latest.auto_sync != started.auto_sync {
        proposed.auto_sync = current.auto_sync;
    }
    if latest.data_saver != started.data_saver {
        proposed.data_saver = current.data_saver;
    }
    proposed
}

fn validated_github_identity(
    mut config: GitHubSyncConfig,
    token: String,
    owner: String,
) -> GitHubSyncConfig {
    if config.token != token || config.owner != owner {
        config.repo.clear();
        config.last_remote_sha.clear();
        config.last_sync_time = 0;
        config.auto_sync = false;
    }
    config.token = token;
    config.owner = owner;
    config
}

fn github_sync_configured(config: &GitHubSyncConfig) -> bool {
    !config.token.trim().is_empty()
        && !config.owner.trim().is_empty()
        && !config.repo.trim().is_empty()
}

fn same_github_target(current: &GitHubSyncConfig, requested: &GitHubSyncConfig) -> bool {
    !current.token.is_empty()
        && current.owner == requested.owner
        && current.repo == requested.repo
        && current.token == requested.token
}

fn same_webdav_target(current: &WebDavSyncConfig, requested: &WebDavSyncConfig) -> bool {
    !current.server_url.is_empty()
        && current.server_url == requested.server_url
        && current.username == requested.username
        && current.password == requested.password
        && current.base_path == requested.base_path
}

fn complete_for_current_target<C, R>(
    current: &mut C,
    requested: &C,
    same_target: impl FnOnce(&C, &C) -> bool,
    apply: impl FnOnce() -> AppResult<R>,
    record: impl FnOnce(&mut C),
) -> AppResult<R> {
    if !same_target(current, requested) {
        return Err(AppError::Other(
            "Sync target or credentials changed; retry with the current configuration".into(),
        ));
    }
    let result = apply()?;
    record(current);
    Ok(result)
}

fn config_default_true() -> bool { true }
fn config_default_history_mode() -> String { "immediate".into() }

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
    #[serde(default = "config_default_data_saver")]
    data_saver: bool,
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
            data_saver: config.data_saver,
        }
    }
}

/// 配置文件缺 dataSaver 时按开启处理，与 Android 默认一致
fn config_default_data_saver() -> bool {
    true
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
            data_saver: self.data_saver,
        }
    }
}

// ---- Android 导出配置（AppConfigBackup.kt）兼容导入 ----

/// Android 端 settings 是 DataStore 类型分组快照（TypedPreferenceSnapshot）。
/// ints 分组暂无与桌面语义相同的键，不读取（serde 自动忽略未知键）。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AndroidTypedSettings {
    #[serde(default)]
    booleans: HashMap<String, bool>,
    #[serde(default)]
    floats: HashMap<String, f32>,
    #[serde(default)]
    longs: HashMap<String, i64>,
    #[serde(default)]
    strings: HashMap<String, String>,
}

impl AndroidTypedSettings {
    /// 把 Android 分组偏好叠加到桌面设置上（仅映射两端语义相同的键，未知键忽略）
    fn apply_to(&self, settings: &mut AppSettings) {
        for (key, value) in &self.booleans {
            match key.as_str() {
                "dev_mode_enabled" => settings.dev_mode_enabled = *value,
                "internationalization_enabled" => settings.internationalization_enabled = *value,
                "bypass_proxy" => settings.bypass_proxy = *value,
                // Android 的 dynamic_color = Material You 跟随系统取色，
                // 语义与桌面旧 dynamic_color（跟随封面）不同，直接映射 color_mode
                "dynamic_color" => {
                    settings.color_mode = if *value { "system" } else { "default" }.into()
                }
                "nowplaying_audio_reactive_enabled" => settings.audio_reactive = *value,
                "nowplaying_dynamic_background_enabled" => settings.dynamic_background = *value,
                "nowplaying_cover_blur_background_enabled" => settings.cover_blur_bg = *value,
                "nowplaying_toolbar_dock_enabled" => settings.show_toolbar_dock = *value,
                _ => {}
            }
        }
        for (key, value) in &self.floats {
            match key.as_str() {
                "lyric_font_scale" => settings.lyric_font_scale = *value,
                "lyric_blur_amount" => settings.lyric_blur_amount = *value,
                "nowplaying_cover_blur_amount" => settings.cover_blur_amount = *value,
                _ => {}
            }
        }
        for (key, value) in &self.longs {
            match key.as_str() {
                "cloud_music_lyric_default_offset_ms" => settings.cloud_music_offset = *value as i32,
                // Android 以字节存储缓存上限，桌面端以 MB 为单位
                "max_cache_size_bytes" => settings.max_cache_size = (*value / 1024 / 1024) as i32,
                _ => {}
            }
        }
        for (key, value) in &self.strings {
            match key.as_str() {
                "audio_quality" => settings.netease_quality = value.clone(),
                "youtube_audio_quality" => settings.youtube_quality = value.clone(),
                "bili_audio_quality" => settings.bili_quality = value.clone(),
                _ => {}
            }
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AndroidListenTogether {
    #[serde(default)]
    worker_base_url: String,
    #[serde(default)]
    user_uuid: String,
    #[serde(default)]
    nickname: String,
    #[serde(default = "config_default_true")]
    allow_member_control: bool,
    #[serde(default = "config_default_true")]
    auto_pause_on_member_change: bool,
    #[serde(default = "config_default_true")]
    share_audio_links: bool,
}

impl From<AndroidListenTogether> for ConfigListenTogether {
    fn from(value: AndroidListenTogether) -> Self {
        Self {
            user_uuid: value.user_uuid,
            server_url: value.worker_base_url,
            nickname: value.nickname,
            allow_member_control: value.allow_member_control,
            auto_pause_on_member_change: value.auto_pause_on_member_change,
            share_audio_links: value.share_audio_links,
        }
    }
}

/// 网易云 / B站 的 Cookie 快照（SavedCookieConfigSnapshot）
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AndroidCookieAuth {
    #[serde(default)]
    cookies: HashMap<String, String>,
}

/// YouTube 的 Cookie 快照（YouTubeAuthConfigSnapshot，只取 cookies）
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AndroidYouTubeAuth {
    #[serde(default)]
    cookies: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AndroidGitHubSync {
    #[serde(default)]
    token: String,
    #[serde(default)]
    repo_owner: String,
    #[serde(default)]
    repo_name: String,
    #[serde(default)]
    auto_sync_enabled: bool,
    #[serde(default)]
    play_history_update_mode: String,
    #[serde(default = "config_default_true")]
    data_saver_mode: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AndroidWebDavSync {
    #[serde(default)]
    server_url: String,
    #[serde(default)]
    base_path: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    auto_sync_enabled: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AndroidSyncPreferences {
    #[serde(default)]
    play_history_update_mode: String,
}

/// Android 端导出的配置备份（字段与 AppConfigBackup.kt 对齐）
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AndroidConfigFile {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    format_version: u32,
    #[serde(default)]
    settings: AndroidTypedSettings,
    #[serde(default)]
    listen_together: Option<AndroidListenTogether>,
    #[serde(default)]
    language: Option<ConfigLanguage>,
    #[serde(default)]
    netease_auth: Option<AndroidCookieAuth>,
    #[serde(default)]
    bili_auth: Option<AndroidCookieAuth>,
    #[serde(default)]
    you_tube_auth: Option<AndroidYouTubeAuth>,
    #[serde(default)]
    git_hub_sync: Option<AndroidGitHubSync>,
    #[serde(default)]
    web_dav_sync: Option<AndroidWebDavSync>,
    #[serde(default)]
    sync_preferences: Option<AndroidSyncPreferences>,
}

/// Android 与 PC 配置的判别：Android 的 settings 是类型分组
/// （booleans/floats/...），PC 是扁平 AppSettings，无 "booleans" 键
fn is_android_config(root: &Value) -> bool {
    root.get("settings")
        .and_then(|settings| settings.get("booleans"))
        .is_some()
}

/// Android 只导出 name→value，域名按桌面手动粘贴 Cookie 的规则补齐（YouTube 还要补 google.com 的会话 Cookie）
fn android_cookie_entries(cookies: &HashMap<String, String>, platform: &str) -> Vec<CookieEntry> {
    let raw = cookies
        .iter()
        .filter(|(name, value)| !name.is_empty() && !value.is_empty())
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ");
    crate::auth::cookies::parse_raw_cookie_text(&raw, platform)
}

/// Android 三平台 Cookie 快照 → 桌面 AuthState（不包含用户资料字段）
fn map_android_auth(config: &AndroidConfigFile) -> Option<AuthState> {
    let netease = config
        .netease_auth
        .as_ref()
        .filter(|auth| !auth.cookies.is_empty())
        .map(|auth| NeteaseAuth {
            cookies: android_cookie_entries(&auth.cookies, "netease"),
            user_id: None,
            nickname: None,
            avatar_url: None,
        });
    let bilibili = config
        .bili_auth
        .as_ref()
        .filter(|auth| !auth.cookies.is_empty())
        .map(|auth| BiliAuth {
            cookies: android_cookie_entries(&auth.cookies, "bilibili"),
            mid: None,
            nickname: None,
            avatar_url: None,
        });
    let youtube = config
        .you_tube_auth
        .as_ref()
        .filter(|auth| !auth.cookies.is_empty())
        .map(|auth| YouTubeAuth {
            cookies: android_cookie_entries(&auth.cookies, "youtube"),
            nickname: None,
            avatar_url: None,
        });
    if netease.is_none() && bilibili.is_none() && youtube.is_none() {
        None
    } else {
        Some(AuthState {
            netease,
            bilibili,
            youtube,
        })
    }
}

/// Android 导出总带全部分组；空段落不覆盖桌面已有配置（避免误清凭据）。
/// 以下映射函数在有实际内容时才返回 Some。
fn map_android_language(language: Option<ConfigLanguage>) -> Option<ConfigLanguage> {
    language.filter(|language| !language.code.is_empty()).map(|mut language| {
        // Android 语言码是 zh/en/ja，桌面端是 zh-CN/zh-TW/en/ja
        if language.code == "zh" {
            language.code = "zh-CN".into();
        }
        language
    })
}

fn map_android_listen_together(
    listen_together: Option<AndroidListenTogether>,
) -> Option<ConfigListenTogether> {
    listen_together
        .filter(|lt| {
            !lt.worker_base_url.is_empty() || !lt.user_uuid.is_empty() || !lt.nickname.is_empty()
        })
        .map(Into::into)
}

fn map_android_github(github: Option<AndroidGitHubSync>) -> Option<ConfigGitHubSync> {
    github
        .filter(|github| {
            !github.token.is_empty()
                || !github.repo_owner.is_empty()
                || !github.repo_name.is_empty()
                || !github.play_history_update_mode.is_empty()
        })
        .map(|github| ConfigGitHubSync {
            token: github.token,
            owner: github.repo_owner,
            repo: github.repo_name,
            last_remote_sha: String::new(),
            last_sync_time: 0,
            auto_sync: github.auto_sync_enabled,
            data_saver: github.data_saver_mode,
            silent_failures: false,
            history_update_mode: normalize_history_update_mode(&github.play_history_update_mode),
        })
}

fn map_android_webdav(webdav: Option<AndroidWebDavSync>) -> Option<ConfigWebDavSync> {
    webdav
        .filter(|webdav| {
            !webdav.server_url.is_empty() || !webdav.username.is_empty() || !webdav.password.is_empty()
        })
        .map(|webdav| ConfigWebDavSync {
            server_url: webdav.server_url,
            username: webdav.username,
            password: webdav.password,
            base_path: webdav.base_path,
            last_remote_fingerprint: String::new(),
            last_sync_time: 0,
            auto_sync: webdav.auto_sync_enabled,
            data_saver: true,
        })
}

fn map_android_sync_preferences(
    preferences: Option<AndroidSyncPreferences>,
) -> Option<ConfigSyncPreferences> {
    preferences
        .filter(|preferences| !preferences.play_history_update_mode.is_empty())
        .map(|preferences| ConfigSyncPreferences {
            history_update_mode: normalize_history_update_mode(&preferences.play_history_update_mode),
        })
}

/// 归一化后的待导入配置（PC 与 Android 两种来源共用）
struct ImportedConfig {
    settings: AppSettings,
    listen_together: Option<ConfigListenTogether>,
    language: Option<ConfigLanguage>,
    auth: Option<AuthState>,
    /// Android 备份只带有 Cookie 的平台，按平台合并；桌面配置是完整快照，整体替换
    merge_auth: bool,
    github_sync: Option<ConfigGitHubSync>,
    webdav_sync: Option<ConfigWebDavSync>,
    sync_preferences: Option<ConfigSyncPreferences>,
}

fn merge_imported_auth(previous: &AuthState, imported: AuthState) -> AuthState {
    AuthState {
        netease: imported.netease.or_else(|| previous.netease.clone()),
        bilibili: imported.bilibili.or_else(|| previous.bilibili.clone()),
        youtube: imported.youtube.or_else(|| previous.youtube.clone()),
    }
}

/// 解析桌面端导出的配置文件
fn parse_pc_config(root: Value) -> AppResult<ImportedConfig> {
    let payload: DesktopConfigFile = serde_json::from_value(root)
        .map_err(|e| AppError::Other(format!("Parse config failed: {}", e)))?;
    if payload.kind != CONFIG_FILE_KIND
        || (payload.format_version != 0 && payload.format_version > CONFIG_FILE_VERSION)
        || payload.platform != "pc"
    {
        return Err(AppError::Other("Unsupported config file".into()));
    }
    Ok(ImportedConfig {
        settings: payload.settings,
        listen_together: payload.listen_together,
        language: payload.language,
        auth: payload.auth,
        merge_auth: false,
        github_sync: payload.github_sync,
        webdav_sync: payload.webdav_sync,
        sync_preferences: payload.sync_preferences,
    })
}

/// 解析 Android 端导出的配置备份，偏好叠加到桌面当前设置上
fn parse_android_config(app: &AppHandle, root: Value) -> AppResult<ImportedConfig> {
    let payload: AndroidConfigFile = serde_json::from_value(root)
        .map_err(|e| AppError::Other(format!("Parse android config failed: {}", e)))?;
    if payload.kind != CONFIG_FILE_KIND
        || (payload.format_version != 0 && payload.format_version > CONFIG_FILE_VERSION)
    {
        return Err(AppError::Other("Unsupported config file".into()));
    }

    let mut settings = store::load_settings(app)?.settings;
    payload.settings.apply_to(&mut settings);

    let auth = map_android_auth(&payload);
    Ok(ImportedConfig {
        settings,
        listen_together: map_android_listen_together(payload.listen_together),
        language: map_android_language(payload.language),
        auth,
        merge_auth: true,
        github_sync: map_android_github(payload.git_hub_sync),
        webdav_sync: map_android_webdav(payload.web_dav_sync),
        sync_preferences: map_android_sync_preferences(payload.sync_preferences),
    })
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
    #[serde(default, deserialize_with = "store::deserialize_app_settings")]
    settings: AppSettings,
    #[serde(default)]
    listen_together: Option<ConfigListenTogether>,
    #[serde(default)]
    language: Option<ConfigLanguage>,
    #[serde(default)]
    auth: Option<AuthState>,
    #[serde(default)]
    github_sync: Option<ConfigGitHubSync>,
    #[serde(default)]
    webdav_sync: Option<ConfigWebDavSync>,
    #[serde(default)]
    sync_preferences: Option<ConfigSyncPreferences>,
}

/// 启动时迁移旧版同步凭据，避免只有打开设置页后才清理明文
pub fn initialize_secure_storage(app: &AppHandle) {
    let _ = load_github_config(app);
    let _ = load_webdav_config(app);
}

fn load_github_config(app: &AppHandle) -> GitHubSyncConfig {
    with_sync_config(|| load_github_config_unlocked(app))
}

fn load_github_config_unlocked(app: &AppHandle) -> GitHubSyncConfig {
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
            save_github_config_unlocked(app, &config);
        }
        return config;
    }

    if legacy_token.is_empty() {
        return config;
    }

    if security::set_secret(security::GITHUB_TOKEN_KEY, &legacy_token) {
        config.token = legacy_token;
        save_github_config_unlocked(app, &config);
    } else {
        // 安全存储不可用时清除旧明文，不让凭据继续留在配置文件
        log::error!(target: "sync", "旧版 GitHub Token 迁移到凭据存储失败，已清除明文凭据");
        config.token.clear();
        save_github_config_unlocked(app, &config);
    }
    config
}

fn save_github_config_unlocked(app: &AppHandle, config: &GitHubSyncConfig) {
    if config.token.is_empty() {
        let _ = security::delete_secret(security::GITHUB_TOKEN_KEY);
    } else if !security::set_secret(security::GITHUB_TOKEN_KEY, &config.token) {
        log::error!(target: "sync", "凭据存储不可用，GitHub Token 未持久化");
    }

    if let Ok(s) = app.store(SYNC_STORE) {
        s.set(GITHUB_CONFIG_KEY, github_config_store_value(config));
        let _ = s.save();
    }
}

fn load_webdav_config(app: &AppHandle) -> WebDavSyncConfig {
    with_sync_config(|| load_webdav_config_unlocked(app))
}

fn load_webdav_config_unlocked(app: &AppHandle) -> WebDavSyncConfig {
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
            save_webdav_config_unlocked(app, &config);
        }
        return config;
    }

    if legacy_password.is_empty() {
        return config;
    }

    if security::set_secret(security::WEBDAV_PASSWORD_KEY, &legacy_password) {
        config.password = legacy_password;
        save_webdav_config_unlocked(app, &config);
    } else {
        // 安全存储不可用时清除旧明文，不让凭据继续留在配置文件
        log::error!(target: "sync", "旧版 WebDAV 密码迁移到凭据存储失败，已清除明文凭据");
        config.password.clear();
        save_webdav_config_unlocked(app, &config);
    }
    config
}

fn save_webdav_config_unlocked(app: &AppHandle, config: &WebDavSyncConfig) {
    if config.password.is_empty() {
        let _ = security::delete_secret(security::WEBDAV_PASSWORD_KEY);
    } else if !security::set_secret(security::WEBDAV_PASSWORD_KEY, &config.password) {
        log::error!(target: "sync", "凭据存储不可用，WebDAV 密码未持久化");
    }

    if let Ok(s) = app.store(SYNC_STORE) {
        s.set(WEBDAV_CONFIG_KEY, webdav_config_store_value(config));
        let _ = s.save();
    }
}

fn load_sync_preferences(app: &AppHandle) -> SyncPreferencesConfig {
    with_sync_config(|| load_sync_preferences_unlocked(app))
}

fn load_sync_preferences_unlocked(app: &AppHandle) -> SyncPreferencesConfig {
    let stored = app
        .store(SYNC_STORE)
        .ok()
        .and_then(|s| s.get(SYNC_PREFERENCES_KEY))
        .and_then(|v| serde_json::from_value::<SyncPreferencesConfig>(v.clone()).ok());
    let has_stored = stored.is_some();

    let mut config = stored.unwrap_or_else(|| {
        // 旧版本把频率放在 GitHub 配置中，首次读取时迁移到全局偏好
        let github = load_github_config_unlocked(app);
        SyncPreferencesConfig {
            history_update_mode: github.history_update_mode,
        }
    });
    config.history_update_mode = normalize_history_update_mode(&config.history_update_mode);
    if !has_stored {
        save_sync_preferences_unlocked(app, &config);
    }
    config
}

fn save_sync_preferences_unlocked(app: &AppHandle, config: &SyncPreferencesConfig) {
    if let Ok(store) = app.store(SYNC_STORE) {
        store.set(SYNC_PREFERENCES_KEY, sync_preferences_store_value(config));
        let _ = store.save();
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
pub async fn get_github_sync_config(app: AppHandle) -> AppResult<Value> {
    let config = load_github_config(&app);
    let preferences = load_sync_preferences(&app);
    Ok(serde_json::json!({
        "configured": github_sync_configured(&config),
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
pub async fn get_sync_preferences(app: AppHandle) -> AppResult<Value> {
    let preferences = load_sync_preferences(&app);
    Ok(serde_json::json!({
        "historyUpdateMode": preferences.history_update_mode,
    }))
}

/// 更新全局同步偏好，兼容 Android 的频率枚举和旧版 batched 值
#[tauri::command]
pub async fn update_sync_preferences(
    app: AppHandle,
    history_update_mode: Option<String>,
) -> AppResult<()> {
    with_config_generations(|generations| {
        let mut preferences = load_sync_preferences_unlocked(&app);
        if let Some(mode) = history_update_mode {
            advance_preference_revision(&mut generations.sync_preferences_revision)?;
            preferences.history_update_mode = normalize_history_update_mode(&mode);
        }
        save_sync_preferences_unlocked(&app, &preferences);
        Ok(())
    })
}

/// 验证 GitHub token，返回用户名
#[tauri::command]
pub async fn validate_github_token(
    app: AppHandle,
    state: State<'_, AppState>,
    token: String,
) -> AppResult<Value> {
    let request = with_config_generations(|generations| generations.begin(ConfigProvider::GitHub))?;
    let api = crate::sync::github_api::GitHubApiClient::new(&state.sync_http(), &token);
    let username = api.validate_token().await?;

    // 暂存 token（还没配置完，只保存 token 和 owner）
    with_config_generations(|generations| {
        generations.complete(request, || {
            let config = validated_github_identity(
                load_github_config_unlocked(&app),
                token,
                username.clone(),
            );
            save_github_config_unlocked(&app, &config);
            Ok(())
        })
    })?;

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
    let (config, request, preferences) = with_config_generations(|generations| {
        let request = generations.begin(ConfigProvider::GitHub)?;
        Ok::<_, AppError>((
            load_github_config_unlocked(&app),
            request,
            generations.github_preferences,
        ))
    })?;
    if config.token.is_empty() {
        return Err(AppError::Api("Token not validated yet".into()));
    }

    let api = crate::sync::github_api::GitHubApiClient::new(&state.sync_http(), &config.token);
    api.create_repository(&repo_name).await?;

    let updated = GitHubSyncConfig {
        token: config.token.clone(),
        owner: config.owner.clone(),
        repo: repo_name.clone(),
        auto_sync: true,
        data_saver: true, // 默认开启省流
        ..Default::default()
    };
    with_config_generations(|generations| {
        generations.complete(request, || {
            let current = load_github_config_unlocked(&app);
            let updated = github_connection_preferences(
                preferences,
                generations.github_preferences,
                &current,
                updated,
            );
            save_github_config_unlocked(&app, &updated);
            Ok(())
        })
    })?;

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
    let (config, request, preferences) = with_config_generations(|generations| {
        let request = generations.begin(ConfigProvider::GitHub)?;
        Ok::<_, AppError>((
            load_github_config_unlocked(&app),
            request,
            generations.github_preferences,
        ))
    })?;
    if config.token.is_empty() {
        return Err(AppError::Api("Token not validated yet".into()));
    }

    let api = crate::sync::github_api::GitHubApiClient::new(&state.sync_http(), &config.token);
    let _branch = api.check_repository(&owner, &repo).await?;

    let updated = GitHubSyncConfig {
        token: config.token.clone(),
        owner: owner.clone(),
        repo: repo.clone(),
        auto_sync: true,
        data_saver: true, // 默认开启省流
        ..Default::default()
    };
    with_config_generations(|generations| {
        generations.complete(request, || {
            let current = load_github_config_unlocked(&app);
            let updated = github_connection_preferences(
                preferences,
                generations.github_preferences,
                &current,
                updated,
            );
            save_github_config_unlocked(&app, &updated);
            Ok(())
        })
    })?;

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
    let (request, preferences) = with_config_generations(|generations| {
        let request = generations.begin(ConfigProvider::GitHub)?;
        Ok::<_, AppError>((request, generations.github_preferences))
    })?;
    let api = crate::sync::github_api::GitHubApiClient::new(&state.sync_http(), &token);
    let owner = api.validate_token().await?;

    match api.check_repository(&owner, &repo).await {
        Ok(_) => {}
        Err(error) if error.is_not_found() => api.create_repository(&repo).await?,
        Err(error) => return Err(error.into()),
    }

    let config = GitHubSyncConfig {
        token,
        owner: owner.clone(),
        repo: repo.clone(),
        auto_sync: true,
        data_saver: true, // 默认开启省流
        ..Default::default()
    };
    with_config_generations(|generations| {
        generations.complete(request, || {
            let current = load_github_config_unlocked(&app);
            let config = github_connection_preferences(
                preferences,
                generations.github_preferences,
                &current,
                config,
            );
            save_github_config_unlocked(&app, &config);
            Ok(())
        })
    })?;

    Ok(serde_json::json!({
        "success": true,
        "owner": owner,
        "repo": repo,
    }))
}

/// 读取本地播放统计快照注入同步信封
fn local_stats_payload(state: &State<'_, AppState>) -> manager::SyncStatsPayload {
    let store = state.stats.lock();
    let (stats, buckets, cleared_at) = store.sync_snapshot();
    manager::SyncStatsPayload { stats, buckets, cleared_at }
}

fn build_local_sync_snapshot(
    app: &AppHandle,
    history_entries: Option<&[manager::SyncHistoryEntry]>,
    history_deletions: Option<&[manager::SyncHistoryDeletion]>,
    stats: manager::SyncStatsPayload,
) -> AppResult<(SyncData, u64)> {
    // 快照与 epoch 必须在同一把歌单锁内取得。否则等待全局同步锁期间的
    // 本地编辑会让旧快照搭配新 epoch，随后通过检查并覆盖刚保存的歌单
    let _playlist_guard = playlist::lock_io();
    let local_data = manager::build_local_sync_data(
        app,
        history_entries,
        history_deletions,
        Some(stats),
    )?;
    Ok((local_data, playlist::io_epoch()))
}

/// 写回完整统计来源，本机增量由 StatsStore 单独维护
fn apply_merged_stats(
    app: &AppHandle,
    state: &State<'_, AppState>,
    merged: &SyncData,
) -> AppResult<()> {
    let device_id = manager::get_or_create_device_id_pub(app);
    let mut store = state.stats.lock();
    store.apply_merged(
        &merged.playback_stats,
        &merged.playback_stat_buckets,
        merged.playback_stats_cleared_at,
        &device_id,
    );
    crate::stats::save_checked(&store)
}

/// 执行 GitHub 同步
#[tauri::command]
pub async fn approve_sync_protocol_upgrade(
    app: AppHandle,
    state: State<'_, AppState>,
    challenge: crate::sync::archive::approval::SyncProtocolUpgrade,
    all_devices_updated: bool,
) -> AppResult<()> {
    ensure_all_devices_updated(all_devices_updated)?;
    let (current, request) = match challenge.backend.as_str() {
        "github" => {
            let (config, request) = with_config_generations(|generations| {
                (
                    load_github_config_unlocked(&app),
                    generations.snapshot(ConfigProvider::GitHub),
                )
            });
            (
                crate::sync::cloud::inspect_github_upgrade(&state.sync_http(), &config).await?,
                request,
            )
        }
        "webdav" => {
            let (config, request) = with_config_generations(|generations| {
                (
                    load_webdav_config_unlocked(&app),
                    generations.snapshot(ConfigProvider::WebDav),
                )
            });
            (
                crate::sync::cloud::inspect_webdav_upgrade(&state.sync_http(), &config).await?,
                request,
            )
        }
        _ => return Err(AppError::Other("Invalid sync upgrade provider".into())),
    };
    with_config_generations(|generations| {
        generations.complete(request, || {
            crate::sync::archive::approval::approve_verified(&challenge, &current)
        })
    })
}

/// 旧客户端读不懂升级后的归档，必须由用户确认所有设备都已更新（对齐 Android SyncProtocolUpgradeRepository）
fn ensure_all_devices_updated(confirmed: bool) -> AppResult<()> {
    if confirmed {
        Ok(())
    } else {
        Err(AppError::Other(
            "Confirm that every device runs a compatible version before upgrading the sync format".into(),
        ))
    }
}

#[tauri::command]
pub async fn sync_github(
    app: AppHandle,
    state: State<'_, AppState>,
    history_entries: Option<Vec<manager::SyncHistoryEntry>>,
    history_deletions: Option<Vec<manager::SyncHistoryDeletion>>,
) -> AppResult<SyncResult> {
    let (config, request) = with_config_generations(|generations| {
        (
            load_github_config_unlocked(&app),
            generations.snapshot(ConfigProvider::GitHub),
        )
    });
    if !github_sync_configured(&config) {
        return Err(AppError::Api("GitHub sync not configured".into()));
    }
    // 冷却期内不发任何请求：GitHub 对限流期间的继续请求会延长封禁
    let cooldown_target = github_cooldown_target(&config);
    let previous_cooldown = load_github_cooldown(&app);
    if let Some(cooldown) = previous_cooldown.as_ref().filter(|cooldown| {
        cooldown.blocks(&cooldown_target, chrono::Utc::now().timestamp_millis())
    }) {
        return Err(cooldown.failure().into());
    }

    let (local_data, playlist_epoch) = build_local_sync_snapshot(
        &app,
        history_entries.as_deref(),
        history_deletions.as_deref(),
        local_stats_payload(&state),
    )?;
    let outcome = deferred_on_local_change(manager::sync_github(
        &state.sync_http(),
        &config,
        &local_data,
        playlist_epoch,
        |completed| {
            with_config_generations(|generations| {
                generations.complete(request, || {
                    let mut current = load_github_config_unlocked(&app);
                    let outcome = complete_for_current_target(
                        &mut current,
                        &config,
                        same_github_target,
                        || {
                            let outcome = manager::complete_cloud_sync(
                                &completed,
                                &local_data,
                                playlist_epoch,
                            )?;
                            apply_merged_stats(&app, &state, &outcome.merged)?;
                            Ok(outcome)
                        },
                        |current| {
                            current.last_remote_sha = completed.version.clone();
                            current.last_sync_time = chrono::Utc::now().timestamp_millis();
                        },
                    )?;
                    save_github_config_unlocked(&app, &current);
                    Ok(outcome)
                })
            })
        },
    )
    .await)
    .map_err(|error| {
        github_sync_failed(&app, &config, previous_cooldown.as_ref(), &cooldown_target, error)
    })?;
    if previous_cooldown.is_some() {
        save_github_cooldown(&app, None);
    }
    let Some(outcome) = outcome else {
        return Ok(deferred_sync_result());
    };
    // 仅本地数据确有变化时通知前端刷新；"sync" 标记让 App.vue 不再为这次写回触发自动同步
    if outcome.local_changed {
        let _ = app.emit("playlists-changed", SYNC_WRITE_BACK);
    }
    let _ = app.emit("playback-stats-changed", ());
    Ok(outcome.result)
}

/// `playlists-changed` 的载荷，表示这次变化来自同步写回而不是用户编辑
const SYNC_WRITE_BACK: &str = "sync";

/// 本地歌单在同步期间被修改时什么都没有写回，这不是失败：转成 `None` 交给前端补一轮同步
fn deferred_on_local_change<T>(result: AppResult<T>) -> AppResult<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if manager::is_local_change_conflict(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn deferred_sync_result() -> SyncResult {
    SyncResult {
        message: "Local data changed during sync; another sync will follow".into(),
        deferred: true,
        ..Default::default()
    }
}

const GITHUB_COOLDOWN_KEY: &str = "githubRateLimit";

fn github_cooldown_target(config: &GitHubSyncConfig) -> String {
    format!("{}/{}", config.owner.trim(), config.repo.trim()).to_ascii_lowercase()
}

fn load_github_cooldown(app: &AppHandle) -> Option<GitHubCooldown> {
    app.store(SYNC_STORE)
        .ok()?
        .get(GITHUB_COOLDOWN_KEY)
        .and_then(|value| serde_json::from_value(value).ok())
}

fn save_github_cooldown(app: &AppHandle, cooldown: Option<&GitHubCooldown>) {
    let Ok(store) = app.store(SYNC_STORE) else {
        return;
    };
    match cooldown {
        Some(cooldown) => store.set(GITHUB_COOLDOWN_KEY, serde_json::json!(cooldown)),
        None => {
            store.delete(GITHUB_COOLDOWN_KEY);
        }
    }
    let _ = store.save();
}

/// 限流时记一次冷却，交给前端的错误带上冷却后的重试时间和是否还自动重试
fn classify_github_sync_failure(
    error: AppError,
    previous: Option<&GitHubCooldown>,
    target: &str,
    now_ms: i64,
) -> (AppError, Option<GitHubCooldown>) {
    match error {
        AppError::Sync(SyncFailure::GitHubRateLimited { status, retry_at_ms, .. }) => {
            let cooldown = GitHubCooldown::record(previous, target, status, retry_at_ms, now_ms);
            (cooldown.failure().into(), Some(cooldown))
        }
        error => (error, None),
    }
}

/// 只清掉这次被拒的 token：同步途中用户可能已经换上了新 token。owner/repo 保留，方便重新登录
fn clear_rejected_github_token(current: &mut GitHubSyncConfig, rejected: &GitHubSyncConfig) -> bool {
    if current.token.is_empty() || current.token != rejected.token {
        return false;
    }
    current.token.clear();
    true
}

fn github_sync_failed(
    app: &AppHandle,
    config: &GitHubSyncConfig,
    previous: Option<&GitHubCooldown>,
    target: &str,
    error: AppError,
) -> AppError {
    if matches!(error, AppError::Sync(SyncFailure::GitHubTokenExpired)) {
        // 对齐 Android：失效 token 不会自己恢复，清掉后自动同步不再反复请求
        let cleared = with_config_generations(|generations| {
            let mut current = load_github_config_unlocked(app);
            if !clear_rejected_github_token(&mut current, config) {
                return Ok(());
            }
            generations.invalidate(ConfigProvider::GitHub)?;
            save_github_config_unlocked(app, &current);
            Ok::<_, AppError>(())
        });
        if let Err(clear_error) = cleared {
            log::warn!(target: "sync", "clearing the rejected GitHub token failed: {clear_error}");
        }
        return error;
    }
    let (error, cooldown) = classify_github_sync_failure(
        error,
        previous,
        target,
        chrono::Utc::now().timestamp_millis(),
    );
    if let Some(cooldown) = cooldown {
        save_github_cooldown(app, Some(&cooldown));
    }
    error
}

/// 断开 GitHub 同步
#[tauri::command]
pub async fn disconnect_github_sync(app: AppHandle) -> AppResult<()> {
    with_config_generations(|generations| {
        generations.invalidate(ConfigProvider::GitHub)?;
        save_github_config_unlocked(&app, &GitHubSyncConfig::default());
        Ok(())
    })
}

/// 获取 WebDAV 同步配置
#[tauri::command]
pub async fn get_webdav_sync_config(app: AppHandle) -> AppResult<Value> {
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
    // 对齐 Android WebDavStorage：服务器、用户名、密码缺一不可
    if server_url.trim().is_empty() || username.trim().is_empty() || password.is_empty() {
        return Err(AppError::Other(
            "WebDAV server, username and password are required".into(),
        ));
    }
    let (request, preferences) = with_config_generations(|generations| {
        let request = generations.begin(ConfigProvider::WebDav)?;
        Ok::<_, AppError>((request, generations.webdav_preferences))
    })?;
    let bp = base_path.unwrap_or_default();
    let config = WebDavSyncConfig {
        server_url: server_url.clone(),
        username,
        password,
        base_path: bp,
        auto_sync: true,
        ..Default::default()
    };
    crate::sync::webdav_archive::WebDavArchiveClient::new(&state.sync_http(), &config)?
        .validate_connection()
        .await?;
    with_config_generations(|generations| {
        generations.complete(request, || {
            let current = load_webdav_config_unlocked(&app);
            let config = webdav_connection_preferences(
                preferences,
                generations.webdav_preferences,
                &current,
                config,
            );
            save_webdav_config_unlocked(&app, &config);
            Ok(())
        })
    })?;

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
    let (config, request) = with_config_generations(|generations| {
        (
            load_webdav_config_unlocked(&app),
            generations.snapshot(ConfigProvider::WebDav),
        )
    });
    if config.server_url.is_empty() {
        return Err(AppError::Api("WebDAV sync not configured".into()));
    }

    let (local_data, playlist_epoch) = build_local_sync_snapshot(
        &app,
        history_entries.as_deref(),
        history_deletions.as_deref(),
        local_stats_payload(&state),
    )?;
    let outcome = deferred_on_local_change(manager::sync_webdav(
        &state.sync_http(),
        &config,
        &local_data,
        playlist_epoch,
        |completed| {
            with_config_generations(|generations| {
                generations.complete(request, || {
                    let mut current = load_webdav_config_unlocked(&app);
                    let outcome = complete_for_current_target(
                        &mut current,
                        &config,
                        same_webdav_target,
                        || {
                            let outcome = manager::complete_cloud_sync(
                                &completed,
                                &local_data,
                                playlist_epoch,
                            )?;
                            apply_merged_stats(&app, &state, &outcome.merged)?;
                            Ok(outcome)
                        },
                        |current| {
                            current.last_remote_fingerprint = completed.version.clone();
                            current.last_sync_time = chrono::Utc::now().timestamp_millis();
                        },
                    )?;
                    save_webdav_config_unlocked(&app, &current);
                    Ok(outcome)
                })
            })
        },
    )
    .await)?;
    let Some(outcome) = outcome else {
        return Ok(deferred_sync_result());
    };
    // 与 sync_github 同理
    if outcome.local_changed {
        let _ = app.emit("playlists-changed", SYNC_WRITE_BACK);
    }
    let _ = app.emit("playback-stats-changed", ());
    Ok(outcome.result)
}

/// 更新 GitHub 同步子设置（不影响 token/owner/repo）
#[tauri::command]
pub async fn update_github_sync_settings(
    app: AppHandle,
    auto_sync: Option<bool>,
    data_saver: Option<bool>,
    silent_failures: Option<bool>,
    history_update_mode: Option<String>,
) -> AppResult<()> {
    with_config_generations(|generations| {
        let mut config = load_github_config_unlocked(&app);
        if config.token.is_empty() {
            return Err(AppError::Api("GitHub sync not configured".into()));
        }
        if let Some(v) = auto_sync {
            advance_preference_revision(&mut generations.github_preferences.auto_sync)?;
            config.auto_sync = v;
        }
        if let Some(v) = data_saver {
            advance_preference_revision(&mut generations.github_preferences.data_saver)?;
            config.data_saver = v;
        }
        if let Some(v) = silent_failures {
            advance_preference_revision(&mut generations.github_preferences.silent_failures)?;
            config.silent_failures = v;
        }
        if let Some(v) = history_update_mode {
            advance_preference_revision(&mut generations.github_preferences.history_update_mode)?;
            advance_preference_revision(&mut generations.sync_preferences_revision)?;
            let normalized = normalize_history_update_mode(&v);
            config.history_update_mode = normalized.clone();
            save_sync_preferences_unlocked(
                &app,
                &SyncPreferencesConfig {
                    history_update_mode: normalized,
                },
            );
        }
        save_github_config_unlocked(&app, &config);
        Ok(())
    })
}

/// 更新 WebDAV 同步子设置
#[tauri::command]
pub async fn update_webdav_sync_settings(app: AppHandle, auto_sync: Option<bool>) -> AppResult<()> {
    with_config_generations(|generations| {
        let mut config = load_webdav_config_unlocked(&app);
        if config.server_url.is_empty() {
            return Err(AppError::Api("WebDAV sync not configured".into()));
        }
        if let Some(v) = auto_sync {
            advance_preference_revision(&mut generations.webdav_preferences.auto_sync)?;
            config.auto_sync = v;
        }
        save_webdav_config_unlocked(&app, &config);
        Ok(())
    })
}

/// 导出播放列表为 JSON（Android BackupData 兼容格式）
#[tauri::command]
pub async fn export_playlists(app: AppHandle) -> AppResult<Value> {
    use crate::library::playlist::PlaylistStore;
    let store = PlaylistStore::load()?;

    // 转换为 SyncPlaylist 格式（Android 兼容）
    let sync_playlists: Vec<crate::sync::models::SyncPlaylist> = store.playlists.iter().map(|pl| {
        crate::sync::models::SyncPlaylist {
            id: pl.id.to_string(),
            name: pl.name.clone(),
            songs: crate::sync::manager::tracks_to_sync_songs_pub(&pl.tracks),
            created_at: pl.modified_at as i64,
            modified_at: pl.modified_at as i64,
            is_deleted: false,
            song_order_version: 1,
        }
    }).collect();

    let backup_data = serde_json::json!({
        "version": "2.0",
        "timestamp": chrono::Utc::now().timestamp_millis(),
        "exportDate": chrono::Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string(),
        "playlists": sync_playlists,
    });

    let json_data = serde_json::to_string_pretty(&backup_data)
        .map_err(|e| AppError::Other(format!("Serialize failed: {}", e)))?;

    let Some(path) = pick_file_path(&app, Some("neriplayer-playlists.json".into())).await? else {
        return Ok(dialog_cancelled());
    };
    std::fs::write(&path, &json_data)
        .map_err(|e| AppError::Other(format!("Write failed: {}", e)))?;
    Ok(serde_json::json!({ "success": true, "count": store.playlists.len() }))
}

/// 弹出文件对话框，且不占住 async 运行时
///
/// `blocking_save_file` / `blocking_pick_file` 会一直阻塞到用户选完，
/// 可能是几分钟。直接在 `#[tauri::command] async fn` 里调用等于霸占一个
/// tokio worker，插件文档也明确警告这么用会死锁。挪到阻塞线程池里执行。
///
/// 返回的 `FilePath` 可能是 `Url`（`file://`、Android `content://`），
/// `as_path()` 对它返回 `None`——之前那里直接 unwrap，用户从虚拟位置选文件
/// 就会 panic。`into_path()` 能把 `file://` 正确还原成路径。
async fn pick_file_path(
    app: &AppHandle,
    save_as: Option<String>,
) -> AppResult<Option<std::path::PathBuf>> {
    use tauri_plugin_dialog::DialogExt;

    let dialog = app.dialog().clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let builder = dialog.file().add_filter("JSON", &["json"]);
        match save_as {
            Some(name) => builder.set_file_name(name).blocking_save_file(),
            None => builder.blocking_pick_file(),
        }
    })
    .await
    .map_err(|error| AppError::Other(format!("File dialog failed: {error}")))?;

    let Some(picked) = picked else {
        return Ok(None);
    };
    picked
        .into_path()
        .map(Some)
        .map_err(|error| AppError::Other(format!("Unsupported file location: {error}")))
}

/// 取消选择时统一的返回体，避免各处自己拼 JSON 拼歪
fn dialog_cancelled() -> Value {
    serde_json::json!({ "success": false, "reason": "cancelled" })
}

/// 合并导入歌单到本地快照（修复 SY-1）
///
/// 旧实现把导入的歌单当作全量集合直接喂给 save_synced_playlists，而后者会以
/// merged.playlists 全量替换 store.playlists、以 merged.favorite_playlists 覆盖
/// favorites 文件、以 merged.playlist_song_deletions 覆盖删除墓碑——传入的
/// SyncData 除 playlists 外全为空，于是本地独有歌单 / 收藏 / 删除墓碑被一并清空。
///
/// 正确语义：以本地完整快照为基底，导入项按身份（sync id / 名称）覆盖匹配项，
/// 本地独有歌单必须保留；导入不传播删除墓碑，避免误删本地歌单。
fn build_import_merged_playlists(
    app: &AppHandle,
    imported: Vec<crate::sync::models::SyncPlaylist>,
) -> AppResult<crate::sync::models::SyncData> {
    let mut merged = crate::sync::manager::build_local_sync_data(app, None, None, None)?;
    for imp in imported {
        if imp.is_deleted {
            continue;
        }
        let pos = merged
            .playlists
            .iter()
            .position(|p| !p.is_deleted && (p.id == imp.id || p.name == imp.name));
        match pos {
            Some(i) => merged.playlists[i] = imp,
            None => merged.playlists.push(imp),
        }
    }
    Ok(merged)
}

/// 合并桌面格式导出的歌单：同名歌单保留本地版本，新歌单 ID 与本地冲突时换发新 ID
fn merge_desktop_playlists(
    store: &mut crate::library::playlist::PlaylistStore,
    imported: Vec<crate::library::playlist::Playlist>,
) -> bool {
    let mut changed = false;
    for mut playlist in imported {
        if store.playlists.iter().any(|existing| existing.name == playlist.name) {
            continue;
        }
        if store.playlists.iter().any(|existing| existing.id == playlist.id) {
            playlist.id = store.allocate_id();
        }
        store.deleted_playlist_ids.retain(|id| *id != playlist.id);
        store.playlists.push(playlist);
        changed = true;
    }
    if changed {
        store.fix_next_id();
    }
    changed
}

/// 导入播放列表 JSON（兼容 Android BackupData 和 Desktop 两种格式）
#[tauri::command]
pub async fn import_playlists(app: AppHandle) -> AppResult<Value> {
    use crate::library::playlist::{PlaylistStore, Playlist};
    use crate::sync::models::SyncPlaylist;
    use crate::sync::manager::save_synced_playlists;
    let Some(path) = pick_file_path(&app, None).await? else {
        return Ok(dialog_cancelled());
    };
    {
            let data = std::fs::read_to_string(&path)
                .map_err(|e| AppError::Other(format!("Read failed: {}", e)))?;

            let parsed: serde_json::Value = serde_json::from_str(&data)
                .map_err(|e| AppError::Other(format!("Parse failed: {}", e)))?;

            // 检测格式：Android BackupData 有 "playlists" 顶层数组（每项有 "songs"）
            //           Desktop 格式是直接的 Playlist 数组（每项有 "tracks"）
            let count;

            if parsed.is_object() && parsed.get("playlists").is_some() {
                // Android BackupData 格式：{ version, playlists: [SyncPlaylist] }
                let sync_playlists: Vec<SyncPlaylist> = serde_json::from_value(
                    parsed["playlists"].clone()
                ).map_err(|e| AppError::Other(format!("Parse sync playlists: {}", e)))?;
                count = sync_playlists.len();

                // 并入本地快照后回写：保留本地独有歌单 / 收藏 / 删除墓碑（修复 SY-1）
                let sync_data = build_import_merged_playlists(&app, sync_playlists)?;
                save_synced_playlists(&sync_data)?;
            } else if parsed.is_array() {
                // 尝试 Desktop 格式
                if let Ok(imported) = serde_json::from_value::<Vec<Playlist>>(parsed.clone()) {
                    count = imported.len();
                    PlaylistStore::update(|store| {
                        let changed = merge_desktop_playlists(store, imported);
                        Ok(((), changed))
                    })?;
                } else {
                    // 可能是 SyncPlaylist 数组（无外层包装）
                    let sync_playlists: Vec<SyncPlaylist> = serde_json::from_value(parsed)
                        .map_err(|e| AppError::Other(format!("Parse playlists array: {}", e)))?;
                    count = sync_playlists.len();
                    // 并入本地快照后回写（修复 SY-1）
                    let sync_data = build_import_merged_playlists(&app, sync_playlists)?;
                    save_synced_playlists(&sync_data)?;
                }
            } else {
                return Err(AppError::Other("Unrecognized playlist format".into()));
            }

            let _ = app.emit("playlists-changed", ());
            Ok(serde_json::json!({ "success": true, "imported": count }))
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
        language: Some(ConfigLanguage { code: settings.locale.clone() }),
        settings,
        auth: Some(auth),
        github_sync: Some(ConfigGitHubSync::from(&github)),
        webdav_sync: Some(ConfigWebDavSync::from(&webdav)),
        sync_preferences: Some(ConfigSyncPreferences::from(&preferences)),
    };
    let content = serde_json::to_string_pretty(&config)
        .map_err(|e| AppError::Other(format!("Serialize config failed: {}", e)))?;
    let file_name = format!(
        "neriplayer-desktop-config-{}.json",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );
    let path = pick_file_path(&app, Some(file_name)).await?;

    match path {
        Some(path) => {
            std::fs::write(&path, content)
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

/// 导入 PC 配置文件并恢复设置、登录状态和同步配置
#[tauri::command]
pub async fn import_config(app: AppHandle, state: State<'_, AppState>) -> AppResult<Value> {
    let (
        github_request,
        webdav_request,
        github_preferences,
        webdav_preferences,
        sync_preferences_revision,
    ) = with_config_generations(|generations| {
        advance_preference_revision(&mut generations.sync_preferences_revision)?;
        Ok::<_, AppError>((
            generations.begin(ConfigProvider::GitHub)?,
            generations.begin(ConfigProvider::WebDav)?,
            generations.github_preferences,
            generations.webdav_preferences,
            generations.sync_preferences_revision,
        ))
    })?;
    let Some(file_path) = pick_file_path(&app, None).await? else {
        return Ok(dialog_cancelled());
    };
    let file_path = file_path.as_path();
    let metadata = std::fs::metadata(file_path)
        .map_err(|e| AppError::Other(format!("Read config metadata failed: {}", e)))?;
    if metadata.len() > 2 * 1024 * 1024 {
        return Err(AppError::Other("Config file is too large".into()));
    }
    let content = std::fs::read_to_string(file_path)
        .map_err(|e| AppError::Other(format!("Read config failed: {}", e)))?;
    let root: Value = serde_json::from_str(&content)
        .map_err(|e| AppError::Other(format!("Parse config failed: {}", e)))?;

    // 归一化 PC 与 Android 两种导出格式（parse_pc_config 内部做 kind/platform 校验）
    let imported = if is_android_config(&root) {
        parse_android_config(&app, root)?
    } else {
        parse_pc_config(root)?
    };

    let imported_listen_together = imported.listen_together;
    let imported_language = imported.language;
    let mut settings = imported.settings;
    if let Some(language) = imported_language
        .as_ref()
        .filter(|language| !language.code.is_empty())
    {
        settings.locale = language.code.clone();
    }
    if let Some(listen_together) = imported_listen_together.as_ref() {
        if !listen_together.server_url.is_empty() {
            settings.lt_server_url = listen_together.server_url.clone();
        }
        settings.lt_nickname = listen_together.nickname.clone();
        settings.lt_allow_member_control = listen_together.allow_member_control;
        settings.lt_auto_pause_on_member_change = listen_together.auto_pause_on_member_change;
        settings.lt_share_audio_links = listen_together.share_audio_links;
    }
    let settings = store::save_settings(&app, settings)?;
    state.rebuild_http(settings.bypass_proxy);

    if let Some(imported_auth) = imported.auth {
        let mut auth = state.auth.lock();
        let previous_auth = auth.clone();
        for platform in ["netease", "bilibili", "youtube"] {
            crate::auth::cookies::expire_platform_cookies(
                &state.cookie_jar,
                &previous_auth,
                platform,
            );
        }
        *auth = if imported.merge_auth {
            merge_imported_auth(&previous_auth, imported_auth)
        } else {
            imported_auth
        };
        crate::auth::cookies::inject_all(&state.cookie_jar, &auth);
        crate::auth::cookies::save_auth(&app, &auth);
    }
    let legacy_history_mode = imported
        .github_sync
        .as_ref()
        .map(|github| github.history_update_mode.clone());
    with_config_generations(|generations| {
        if generations.sync_preferences_revision == sync_preferences_revision {
            if let Some(preferences) = imported.sync_preferences {
                save_sync_preferences_unlocked(&app, &preferences.into_config());
            } else if let Some(mode) = legacy_history_mode {
                save_sync_preferences_unlocked(
                    &app,
                    &SyncPreferencesConfig {
                        history_update_mode: normalize_history_update_mode(&mode),
                    },
                );
            }
        }
        if generations.is_current(github_request) {
            if let Some(github) = imported.github_sync {
                let current = load_github_config_unlocked(&app);
                let github = github_connection_preferences(
                    github_preferences,
                    generations.github_preferences,
                    &current,
                    github.into_config(),
                );
                save_github_config_unlocked(&app, &github);
            }
        }
        if generations.is_current(webdav_request) {
            if let Some(webdav) = imported.webdav_sync {
                let current = load_webdav_config_unlocked(&app);
                let webdav = webdav_connection_preferences(
                    webdav_preferences,
                    generations.webdav_preferences,
                    &current,
                    webdav.into_config(),
                );
                save_webdav_config_unlocked(&app, &webdav);
            }
        }
    });

    Ok(serde_json::json!({
        "success": true,
        "settings": settings,
        "listenTogetherUserUuid": imported_listen_together
            .map(|listen_together| listen_together.user_uuid)
            .unwrap_or_default(),
        "platform": "pc",
    }))
}

/// 断开 WebDAV 同步
#[tauri::command]
pub async fn disconnect_webdav_sync(app: AppHandle) -> AppResult<()> {
    with_config_generations(|generations| {
        generations.invalidate(ConfigProvider::WebDav)?;
        save_webdav_config_unlocked(&app, &WebDavSyncConfig::default());
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::{
        github_config_store_value, webdav_config_store_value, ConfigGitHubSync,
        ConfigSyncPreferences, ConfigWebDavSync, DesktopConfigFile, CONFIG_FILE_KIND,
        CONFIG_FILE_VERSION,
    };
    use crate::auth::state::AuthState;
    use crate::settings::store::AppSettings;
    use crate::sync::models::{GitHubSyncConfig, SyncPreferencesConfig, WebDavSyncConfig};

    #[test]
    fn github_rate_limits_persist_a_cooldown_but_token_expiry_does_not() {
        use crate::error::AppError;
        use crate::sync::failure::SyncFailure;
        let now = 1_800_000_000_000;
        let limited = AppError::Sync(SyncFailure::GitHubRateLimited {
            status: 429,
            retry_at_ms: now + 30_000,
            automatic: true,
        });
        let (error, cooldown) = super::classify_github_sync_failure(limited, None, "owner/repo", now);
        let cooldown = cooldown.expect("a rate limit must persist a cooldown");
        assert_eq!(cooldown.retry_at_ms, now + 60_000, "the first backoff is one minute");
        assert_eq!(error.to_string(), format!("GITHUB_RATE_LIMITED:{}:1", now + 60_000));

        let (error, cooldown) = super::classify_github_sync_failure(
            AppError::Sync(SyncFailure::GitHubTokenExpired),
            Some(&cooldown),
            "owner/repo",
            now,
        );
        assert!(cooldown.is_none());
        assert_eq!(error.to_string(), "GITHUB_TOKEN_EXPIRED");
        assert_eq!(
            super::github_cooldown_target(&GitHubSyncConfig {
                owner: " Owner ".into(),
                repo: "Repo".into(),
                ..Default::default()
            }),
            "owner/repo"
        );
    }

    #[test]
    fn upgrade_approval_requires_the_all_devices_confirmation() {
        assert!(super::ensure_all_devices_updated(false).is_err());
        assert!(super::ensure_all_devices_updated(true).is_ok());
    }

    #[test]
    fn only_the_rejected_github_token_is_cleared() {
        let rejected = GitHubSyncConfig {
            token: "old".into(),
            owner: "owner".into(),
            repo: "repo".into(),
            auto_sync: true,
            ..Default::default()
        };
        let mut current = rejected.clone();
        assert!(super::clear_rejected_github_token(&mut current, &rejected));
        assert!(current.token.is_empty());
        assert_eq!((current.owner.as_str(), current.repo.as_str()), ("owner", "repo"));
        assert!(!super::github_sync_configured(&current));

        let mut replaced = GitHubSyncConfig {
            token: "new".into(),
            ..rejected.clone()
        };
        assert!(!super::clear_rejected_github_token(&mut replaced, &rejected));
        assert_eq!(replaced.token, "new", "a token replaced during the sync must survive");
    }

    #[test]
    fn validated_token_without_a_repository_is_not_a_configured_sync_target() {
        let mut config = GitHubSyncConfig::default();
        assert!(!super::github_sync_configured(&config));
        config.token = "fixture-token".into();
        config.owner = "owner".into();
        assert!(!super::github_sync_configured(&config));
        config.repo = "repo".into();
        assert!(super::github_sync_configured(&config));
        config.owner = " ".into();
        assert!(!super::github_sync_configured(&config));
    }

    #[test]
    fn connection_preference_aba_preserves_last_user_choice() {
        let mut generations = super::ConfigRequestGenerations::default();
        let started = generations.github_preferences;
        let request = generations.begin(super::ConfigProvider::GitHub).unwrap();
        let mut current = GitHubSyncConfig {
            auto_sync: false,
            ..Default::default()
        };
        super::advance_preference_revision(&mut generations.github_preferences.auto_sync).unwrap();
        current.auto_sync = true;
        super::advance_preference_revision(&mut generations.github_preferences.auto_sync).unwrap();
        current.auto_sync = false;
        let proposed = GitHubSyncConfig {
            auto_sync: true,
            data_saver: true,
            ..Default::default()
        };
        let result = super::github_connection_preferences(
            started,
            generations.github_preferences,
            &current,
            proposed,
        );
        assert!(!result.auto_sync);
        generations.complete(request, || Ok(())).unwrap();
        let started = generations.webdav_preferences;
        let request = generations.begin(super::ConfigProvider::WebDav).unwrap();
        let mut current = WebDavSyncConfig {
            auto_sync: false,
            ..Default::default()
        };
        super::advance_preference_revision(&mut generations.webdav_preferences.auto_sync).unwrap();
        current.auto_sync = true;
        super::advance_preference_revision(&mut generations.webdav_preferences.auto_sync).unwrap();
        current.auto_sync = false;
        let proposed = WebDavSyncConfig {
            auto_sync: true,
            ..Default::default()
        };
        let result = super::webdav_connection_preferences(
            started,
            generations.webdav_preferences,
            &current,
            proposed,
        );
        assert!(!result.auto_sync);
        generations.complete(request, || Ok(())).unwrap();
    }

    #[test]
    fn token_validation_clears_previous_target_only_when_identity_changes() {
        let original = GitHubSyncConfig {
            token: "old token".into(),
            owner: "old owner".into(),
            repo: "old repo".into(),
            last_remote_sha: "old sha".into(),
            last_sync_time: 42,
            auto_sync: true,
            data_saver: true,
            silent_failures: true,
            history_update_mode: "every_15_minutes".into(),
        };
        for (token, owner, changed) in [
            ("old token", "old owner", false),
            ("new token", "old owner", true),
            ("old token", "new owner", true),
        ] {
            let validated =
                super::validated_github_identity(original.clone(), token.into(), owner.into());
            assert_eq!(validated.token, token);
            assert_eq!(validated.owner, owner);
            assert_eq!(validated.repo, if changed { "" } else { "old repo" });
            assert_eq!(
                validated.last_remote_sha,
                if changed { "" } else { "old sha" }
            );
            assert_eq!(validated.last_sync_time, if changed { 0 } else { 42 });
            assert_eq!(validated.auto_sync, !changed);
            assert!(validated.data_saver);
            assert!(validated.silent_failures);
            assert_eq!(validated.history_update_mode, "every_15_minutes");
        }
    }

    #[test]
    fn new_connection_defaults_are_preserved_without_preference_changes() {
        let started = GitHubSyncConfig::default();
        let proposed = GitHubSyncConfig {
            token: "new".into(),
            owner: "new".into(),
            repo: "new".into(),
            auto_sync: true,
            data_saver: true,
            ..Default::default()
        };
        let result = super::github_connection_preferences(
            super::GitHubPreferenceRevisions::default(),
            super::GitHubPreferenceRevisions::default(),
            &started,
            proposed,
        );
        assert!(result.auto_sync);
        assert!(result.data_saver);
        let started = WebDavSyncConfig::default();
        let proposed = WebDavSyncConfig {
            server_url: "https://new.invalid/".into(),
            auto_sync: true,
            ..Default::default()
        };
        let result = super::webdav_connection_preferences(
            super::WebDavPreferenceRevisions::default(),
            super::WebDavPreferenceRevisions::default(),
            &started,
            proposed,
        );
        assert!(result.auto_sync);
        assert!(!result.data_saver);
    }

    #[test]
    fn connection_completion_preserves_changed_preferences_and_new_identity() {
        let started = GitHubSyncConfig {
            auto_sync: true,
            data_saver: true,
            history_update_mode: "immediate".into(),
            ..Default::default()
        };
        let current = GitHubSyncConfig {
            auto_sync: false,
            data_saver: false,
            silent_failures: true,
            history_update_mode: "every_15_minutes".into(),
            ..started.clone()
        };
        let proposed = GitHubSyncConfig {
            token: "new".into(),
            owner: "new owner".into(),
            repo: "new repo".into(),
            auto_sync: true,
            data_saver: true,
            ..Default::default()
        };
        let revisions = super::GitHubPreferenceRevisions {
            auto_sync: 1,
            data_saver: 1,
            silent_failures: 1,
            history_update_mode: 1,
        };
        let result = super::github_connection_preferences(
            super::GitHubPreferenceRevisions::default(),
            revisions,
            &current,
            proposed,
        );
        assert!(!result.auto_sync);
        assert!(!result.data_saver);
        assert!(result.silent_failures);
        assert_eq!(result.history_update_mode, "every_15_minutes");
        assert_eq!(result.token, "new");
        assert_eq!(result.owner, "new owner");
        assert_eq!(result.repo, "new repo");
        let started = WebDavSyncConfig {
            auto_sync: true,
            ..Default::default()
        };
        let current = WebDavSyncConfig {
            auto_sync: false,
            data_saver: true,
            ..started.clone()
        };
        let proposed = WebDavSyncConfig {
            server_url: "https://new.invalid/".into(),
            username: "new".into(),
            password: "new".into(),
            auto_sync: true,
            ..Default::default()
        };
        let revisions = super::WebDavPreferenceRevisions {
            auto_sync: 1,
            data_saver: 1,
        };
        let result = super::webdav_connection_preferences(
            super::WebDavPreferenceRevisions::default(),
            revisions,
            &current,
            proposed,
        );
        assert!(!result.auto_sync);
        assert!(result.data_saver);
        assert_eq!(result.server_url, "https://new.invalid/");
        assert_eq!(result.username, "new");
        assert_eq!(result.password, "new");
    }

    #[test]
    fn config_requests_reverse_completion_rejects_old_save() {
        let mut generations = super::ConfigRequestGenerations::default();
        let old = generations.begin(super::ConfigProvider::GitHub).unwrap();
        let new = generations.begin(super::ConfigProvider::GitHub).unwrap();
        let mut stored = "initial";
        generations
            .complete(new, || {
                stored = "new";
                Ok(())
            })
            .unwrap();
        assert!(generations
            .complete(old, || {
                stored = "old";
                Ok(())
            })
            .is_err());
        assert_eq!(stored, "new");
    }

    #[test]
    fn config_request_after_disconnect_cannot_restore_identity() {
        for provider in [super::ConfigProvider::GitHub, super::ConfigProvider::WebDav] {
            let mut generations = super::ConfigRequestGenerations::default();
            let pending = generations.begin(provider).unwrap();
            generations.invalidate(provider).unwrap();
            let mut stored = None;
            assert!(generations
                .complete(pending, || {
                    stored = Some("old credential");
                    Ok(())
                })
                .is_err());
            assert!(stored.is_none());
        }
    }

    #[test]
    fn config_import_invalidates_both_and_preserves_newer_provider_intent() {
        let mut generations = super::ConfigRequestGenerations::default();
        let pending_github = generations.begin(super::ConfigProvider::GitHub).unwrap();
        let pending_webdav = generations.begin(super::ConfigProvider::WebDav).unwrap();
        let import_github = generations.begin(super::ConfigProvider::GitHub).unwrap();
        let import_webdav = generations.begin(super::ConfigProvider::WebDav).unwrap();
        let mut github = "old";
        let mut webdav = "old";
        assert!(generations
            .complete(pending_github, || {
                github = "pending";
                Ok(())
            })
            .is_err());
        assert!(generations
            .complete(pending_webdav, || {
                webdav = "pending";
                Ok(())
            })
            .is_err());
        let newer = generations.begin(super::ConfigProvider::GitHub).unwrap();
        generations
            .complete(newer, || {
                github = "newer";
                Ok(())
            })
            .unwrap();
        assert!(generations
            .complete(import_github, || {
                github = "imported";
                Ok(())
            })
            .is_err());
        generations
            .complete(import_webdav, || {
                webdav = "imported";
                Ok(())
            })
            .unwrap();
        assert_eq!(github, "newer");
        assert_eq!(webdav, "imported");
    }

    #[test]
    fn config_providers_are_independent_and_preferences_do_not_invalidate_requests() {
        let mut generations = super::ConfigRequestGenerations::default();
        let github = generations.begin(super::ConfigProvider::GitHub).unwrap();
        let webdav = generations.begin(super::ConfigProvider::WebDav).unwrap();
        let github_read = generations.snapshot(super::ConfigProvider::GitHub);
        let webdav_read = generations.snapshot(super::ConfigProvider::WebDav);
        let auto_sync = false;
        super::advance_preference_revision(&mut generations.sync_preferences_revision).unwrap();
        assert_eq!(generations.sync_preferences_revision, 1);
        generations
            .complete(github, || {
                assert!(!auto_sync);
                Ok(())
            })
            .unwrap();
        generations
            .complete(webdav, || {
                assert!(!auto_sync);
                Ok(())
            })
            .unwrap();
        generations.complete(github_read, || Ok(())).unwrap();
        generations.complete(webdav_read, || Ok(())).unwrap();
    }

    #[test]
    fn config_sync_and_upgrade_tickets_reject_disconnect_reconnect_aba() {
        for provider in [super::ConfigProvider::GitHub, super::ConfigProvider::WebDav] {
            let mut generations = super::ConfigRequestGenerations::default();
            let sync = generations.snapshot(provider);
            let upgrade = generations.snapshot(provider);
            generations.invalidate(provider).unwrap();
            let reconnect = generations.begin(provider).unwrap();
            let mut identity = "same target and credential";
            generations
                .complete(reconnect, || {
                    identity = "same target and credential";
                    Ok(())
                })
                .unwrap();
            assert_eq!(identity, "same target and credential");
            assert!(generations
                .complete(sync, || -> crate::error::AppResult<()> {
                    panic!("ABA sync must not apply local data")
                })
                .is_err());
            assert!(generations
                .complete(upgrade, || -> crate::error::AppResult<()> {
                    panic!("ABA upgrade must not authorize old target")
                })
                .is_err());
        }
    }

    #[test]
    fn completion_preserves_preferences_changed_during_network_request() {
        let requested = GitHubSyncConfig {
            token: "fixture-token".into(),
            owner: "owner".into(),
            repo: "repo".into(),
            auto_sync: true,
            ..Default::default()
        };
        let mut current = requested.clone();
        current.auto_sync = false;
        current.data_saver = false;
        current.silent_failures = true;
        current.history_update_mode = "every_15_minutes".into();
        let result = super::complete_for_current_target(
            &mut current,
            &requested,
            super::same_github_target,
            || Ok(7),
            |current| {
                current.last_remote_sha = "completed".into();
                current.last_sync_time = 20;
            },
        )
        .unwrap();
        assert_eq!(result, 7);
        assert!(!current.auto_sync);
        assert!(!current.data_saver);
        assert!(current.silent_failures);
        assert_eq!(current.history_update_mode, "every_15_minutes");
        assert_eq!(current.last_remote_sha, "completed");
        assert_eq!(current.last_sync_time, 20);

        let requested = WebDavSyncConfig {
            server_url: "https://fixture.invalid/".into(),
            username: "fixture".into(),
            password: "fixture-password".into(),
            base_path: "backup".into(),
            auto_sync: true,
            ..Default::default()
        };
        let mut current = requested.clone();
        current.auto_sync = false;
        current.data_saver = false;
        super::complete_for_current_target(
            &mut current,
            &requested,
            super::same_webdav_target,
            || Ok(()),
            |current| {
                current.last_remote_fingerprint = "completed".into();
                current.last_sync_time = 20;
            },
        )
        .unwrap();
        assert!(!current.auto_sync);
        assert!(!current.data_saver);
        assert_eq!(current.last_remote_fingerprint, "completed");
        assert_eq!(current.last_sync_time, 20);
    }

    #[test]
    fn changed_target_credentials_and_disconnect_reject_before_any_local_apply() {
        let requested = GitHubSyncConfig {
            token: "fixture-token".into(),
            owner: "owner".into(),
            repo: "repo".into(),
            ..Default::default()
        };
        for field in ["owner", "repo", "token", "disconnect"] {
            let mut current = requested.clone();
            match field {
                "owner" => current.owner = "different".into(),
                "repo" => current.repo = "different".into(),
                "token" => current.token = "different".into(),
                _ => current = GitHubSyncConfig::default(),
            }
            let before = super::github_config_store_value(&current);
            let result = super::complete_for_current_target(
                &mut current,
                &requested,
                super::same_github_target,
                || -> crate::error::AppResult<()> {
                    panic!("stale completion must not apply local data")
                },
                |_| panic!("stale completion must not record progress"),
            );
            assert!(result.is_err(), "{field}");
            assert_eq!(super::github_config_store_value(&current), before);
            if field == "disconnect" {
                assert!(current.token.is_empty());
            }
        }
        let requested = WebDavSyncConfig {
            server_url: "https://fixture.invalid/".into(),
            username: "fixture".into(),
            password: "fixture-password".into(),
            base_path: "backup".into(),
            ..Default::default()
        };
        for field in ["server", "username", "password", "path", "disconnect"] {
            let mut current = requested.clone();
            match field {
                "server" => current.server_url = "https://different.invalid/".into(),
                "username" => current.username = "different".into(),
                "password" => current.password = "different".into(),
                "path" => current.base_path = "different".into(),
                _ => current = WebDavSyncConfig::default(),
            }
            let before = super::webdav_config_store_value(&current);
            let result = super::complete_for_current_target(
                &mut current,
                &requested,
                super::same_webdav_target,
                || -> crate::error::AppResult<()> {
                    panic!("stale completion must not apply local data")
                },
                |_| panic!("stale completion must not record progress"),
            );
            assert!(result.is_err(), "{field}");
            assert_eq!(super::webdav_config_store_value(&current), before);
            if field == "disconnect" {
                assert!(current.password.is_empty());
            }
        }
    }

    #[test]
    fn failed_local_apply_does_not_advance_sync_progress() {
        let requested = GitHubSyncConfig {
            token: "fixture-token".into(),
            owner: "owner".into(),
            repo: "repo".into(),
            last_sync_time: 10,
            ..Default::default()
        };
        let mut current = requested.clone();
        let result = super::complete_for_current_target(
            &mut current,
            &requested,
            super::same_github_target,
            || -> crate::error::AppResult<()> {
                Err(crate::error::AppError::Other(
                    "fixture write failure".into(),
                ))
            },
            |_| panic!("failed local apply must not advance progress"),
        );
        assert!(result.is_err());
        assert_eq!(current.last_sync_time, 10);
    }

    #[test]
    fn disconnect_serializes_with_completion_and_is_not_revived() {
        use std::sync::{mpsc, Arc};
        let requested = GitHubSyncConfig {
            token: "fixture-token".into(),
            owner: "owner".into(),
            repo: "repo".into(),
            ..Default::default()
        };
        let stored = Arc::new(parking_lot::Mutex::new(requested.clone()));
        let (entered, entered_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let completion_store = stored.clone();
        let completion = std::thread::spawn(move || {
            super::with_sync_config(|| {
                let mut current = completion_store.lock().clone();
                super::complete_for_current_target(
                    &mut current,
                    &requested,
                    super::same_github_target,
                    || {
                        entered.send(()).unwrap();
                        release_rx
                            .recv_timeout(std::time::Duration::from_secs(5))
                            .unwrap();
                        Ok(())
                    },
                    |current| current.last_sync_time = 20,
                )
                .unwrap();
                *completion_store.lock() = current;
            })
        });
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let (started, started_rx) = mpsc::channel();
        let (done, done_rx) = mpsc::channel();
        let disconnect_store = stored.clone();
        let disconnect = std::thread::spawn(move || {
            started.send(()).unwrap();
            super::with_sync_config(|| *disconnect_store.lock() = GitHubSyncConfig::default());
            done.send(()).unwrap();
        });
        started_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(matches!(done_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
        release.send(()).unwrap();
        completion.join().unwrap();
        disconnect.join().unwrap();
        let current = stored.lock();
        assert!(current.token.is_empty());
        assert!(current.owner.is_empty());
        assert_eq!(current.last_sync_time, 0);
    }

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
        assert!(serde_json::to_value(&config).unwrap().get("token").is_none());
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
        assert!(serde_json::to_value(&config).unwrap().get("password").is_none());
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
            github_sync: Some(ConfigGitHubSync { token: "token".into(), ..Default::default() }),
            webdav_sync: Some(ConfigWebDavSync { password: "password".into(), ..Default::default() }),
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
    fn android_settings_apply_maps_known_keys_only() {
        use super::AndroidTypedSettings;
        let android = AndroidTypedSettings {
            booleans: [("bypass_proxy".into(), true), ("usb_exclusive_playback".into(), true)]
                .into_iter()
                .collect(),
            floats: [("lyric_font_scale".into(), 1.25)].into_iter().collect(),
            longs: [
                ("cloud_music_lyric_default_offset_ms".into(), 250),
                ("max_cache_size_bytes".into(), 10_737_418_240),
            ]
            .into_iter()
            .collect(),
            strings: [
                ("audio_quality".into(), "jymaster".into()),
                ("youtube_audio_quality".into(), "low".into()),
                ("theme_color_spec".into(), "SPEC_2025".into()),
            ]
            .into_iter()
            .collect(),
        };
        let mut settings = AppSettings::default();
        android.apply_to(&mut settings);

        assert!(settings.bypass_proxy);
        assert!((settings.lyric_font_scale - 1.25).abs() < f32::EPSILON);
        assert_eq!(settings.cloud_music_offset, 250);
        assert_eq!(settings.max_cache_size, 10_240);
        assert_eq!(settings.netease_quality, "jymaster");
        assert_eq!(settings.youtube_quality, "low");
        // 未知键与桌面无关键不产生副作用
        assert!(settings.dynamic_background);
        assert_eq!(settings.theme_color, "purple");
    }

    #[test]
    fn android_dynamic_color_maps_to_system_mode() {
        use super::AndroidTypedSettings;
        let android = AndroidTypedSettings {
            booleans: [("dynamic_color".into(), true)].into_iter().collect(),
            floats: Default::default(),
            longs: Default::default(),
            strings: Default::default(),
        };
        let mut settings = AppSettings::default();
        android.apply_to(&mut settings);
        // Android 的 dynamic_color = Material You 跟随系统取色
        assert_eq!(settings.color_mode, "system");

        let android = AndroidTypedSettings {
            booleans: [("dynamic_color".into(), false)].into_iter().collect(),
            floats: Default::default(),
            longs: Default::default(),
            strings: Default::default(),
        };
        let mut settings = AppSettings::default();
        android.apply_to(&mut settings);
        assert_eq!(settings.color_mode, "default");
    }

    #[test]
    fn android_config_file_parses_and_maps() {
        use super::{
            is_android_config, map_android_auth, AndroidConfigFile, ConfigListenTogether,
        };
        let json = r#"{
            "kind": "moe.ouom.neriplayer.config",
            "formatVersion": 1,
            "exportedAt": 1787906559291,
            "settings": { "booleans": { "bypass_proxy": true }, "floats": {}, "ints": {}, "longs": {}, "strings": {} },
            "listenTogether": { "workerBaseUrl": "https://lt.example", "userUuid": "abc", "nickname": "me" },
            "language": { "code": "zh" },
            "neteaseAuth": { "cookies": { "MUSIC_U": "v1", "__csrf": "" }, "savedAt": 1 },
            "biliAuth": { "cookies": {}, "savedAt": 0 },
            "youTubeAuth": { "cookies": { "SAPISID": "s1" }, "authorization": "SAPISIDHASH x", "savedAt": 1 },
            "gitHubSync": { "token": "", "repoOwner": "", "repoName": "", "autoSyncEnabled": true, "playHistoryUpdateMode": "", "dataSaverMode": true },
            "webDavSync": { "serverUrl": "https://dav.example", "basePath": "", "username": "u", "password": "p", "autoSyncEnabled": true },
            "syncPreferences": { "playHistoryUpdateMode": "IMMEDIATE" }
        }"#;
        let root: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(is_android_config(&root));

        let config: AndroidConfigFile = serde_json::from_value(root).unwrap();
        assert_eq!(config.kind, CONFIG_FILE_KIND);
        assert_eq!(config.format_version, 1);
        assert_eq!(
            config.listen_together.as_ref().unwrap().worker_base_url,
            "https://lt.example"
        );
        assert_eq!(config.language.as_ref().unwrap().code, "zh");
        assert!(config.netease_auth.as_ref().unwrap().cookies.contains_key("MUSIC_U"));
        assert!(config.bili_auth.as_ref().unwrap().cookies.is_empty());
        assert!(config.git_hub_sync.is_some());
        assert_eq!(config.web_dav_sync.as_ref().unwrap().server_url, "https://dav.example");

        let lt: ConfigListenTogether = config.listen_together.clone().unwrap().into();
        assert_eq!(lt.server_url, "https://lt.example");
        assert_eq!(lt.user_uuid, "abc");

        let auth = map_android_auth(&config).unwrap();
        assert!(auth.netease.is_some());
        assert!(auth.bilibili.is_none());
        assert!(auth.youtube.is_some());
        // 空值 Cookie 被过滤
        assert_eq!(auth.netease.as_ref().unwrap().cookies.len(), 1);
        assert_eq!(auth.netease.as_ref().unwrap().cookies[0].domain, "music.163.com");
        // YouTube 会话 Cookie 要同时落在 youtube.com 与 google.com 上，播放和账号接口才都认
        let youtube = &auth.youtube.as_ref().unwrap().cookies;
        assert!(youtube.iter().any(|c| c.name == "SAPISID" && c.domain == ".youtube.com"));
        assert!(youtube.iter().any(|c| c.name == "SAPISID" && c.domain == ".google.com"));
    }

    #[test]
    fn android_auth_merges_per_platform() {
        use super::merge_imported_auth;
        use crate::auth::state::{AuthState, BiliAuth, CookieEntry, NeteaseAuth};
        let cookie = |value: &str| CookieEntry {
            name: "SESSDATA".into(),
            value: value.into(),
            domain: ".bilibili.com".into(),
        };
        let previous = AuthState {
            netease: Some(NeteaseAuth {
                cookies: vec![],
                user_id: Some(1),
                nickname: None,
                avatar_url: None,
            }),
            bilibili: Some(BiliAuth { cookies: vec![cookie("old")], mid: None, nickname: None, avatar_url: None }),
            youtube: None,
        };
        let imported = AuthState {
            netease: None,
            bilibili: Some(BiliAuth { cookies: vec![cookie("new")], mid: None, nickname: None, avatar_url: None }),
            youtube: None,
        };
        let merged = merge_imported_auth(&previous, imported);
        // 备份里没有的平台保留原登录，有的平台用备份覆盖
        assert_eq!(merged.netease.unwrap().user_id, Some(1));
        assert_eq!(merged.bilibili.unwrap().cookies[0].value, "new");
        assert!(merged.youtube.is_none());
    }

    #[test]
    fn android_empty_sections_do_not_clobber_existing_config() {
        use super::{
            map_android_auth, map_android_github, map_android_language,
            map_android_listen_together, map_android_sync_preferences, map_android_webdav,
            AndroidConfigFile,
        };
        let json = r#"{
            "kind": "moe.ouom.neriplayer.config",
            "formatVersion": 1,
            "settings": { "booleans": {}, "floats": {}, "ints": {}, "longs": {}, "strings": {} },
            "listenTogether": { "workerBaseUrl": "", "userUuid": "", "nickname": "" },
            "language": { "code": "" },
            "gitHubSync": { "token": "", "repoOwner": "", "repoName": "", "autoSyncEnabled": true, "playHistoryUpdateMode": "", "dataSaverMode": true },
            "webDavSync": { "serverUrl": "", "basePath": "", "username": "", "password": "", "autoSyncEnabled": true },
            "syncPreferences": { "playHistoryUpdateMode": "" }
        }"#;
        let config: AndroidConfigFile = serde_json::from_value(serde_json::from_str(json).unwrap())
            .unwrap();
        assert!(map_android_listen_together(config.listen_together.clone()).is_none());
        assert!(map_android_language(config.language.clone()).is_none());
        assert!(map_android_github(config.git_hub_sync.clone()).is_none());
        assert!(map_android_webdav(config.web_dav_sync.clone()).is_none());
        assert!(map_android_sync_preferences(config.sync_preferences.clone()).is_none());
        assert!(map_android_auth(&config).is_none());
    }
}
