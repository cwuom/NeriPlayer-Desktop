//! 配置文件导入导出：桌面/Android 配置格式的解析、映射与凭据合并

use super::*;

fn config_default_true() -> bool {
    true
}
fn config_default_history_mode() -> String {
    "immediate".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConfigListenTogether {
    #[serde(default)]
    pub(super) user_uuid: String,
    #[serde(default)]
    pub(super) server_url: String,
    #[serde(default)]
    pub(super) nickname: String,
    #[serde(default = "config_default_true")]
    pub(super) allow_member_control: bool,
    #[serde(default = "config_default_true")]
    pub(super) auto_pause_on_member_change: bool,
    #[serde(default = "config_default_true")]
    pub(super) share_audio_links: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConfigLanguage {
    #[serde(default)]
    pub(super) code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConfigGitHubSync {
    #[serde(default)]
    pub(super) token: String,
    #[serde(default)]
    pub(super) owner: String,
    #[serde(default)]
    pub(super) repo: String,
    #[serde(default)]
    pub(super) last_remote_sha: String,
    #[serde(default)]
    pub(super) last_sync_time: i64,
    #[serde(default)]
    pub(super) auto_sync: bool,
    #[serde(default = "config_default_true")]
    pub(super) data_saver: bool,
    #[serde(default)]
    pub(super) silent_failures: bool,
    #[serde(default = "config_default_history_mode")]
    pub(super) history_update_mode: String,
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
pub(super) struct ConfigWebDavSync {
    #[serde(default)]
    pub(super) server_url: String,
    #[serde(default)]
    pub(super) username: String,
    #[serde(default)]
    pub(super) password: String,
    #[serde(default)]
    pub(super) base_path: String,
    #[serde(default)]
    pub(super) last_remote_fingerprint: String,
    #[serde(default)]
    pub(super) last_sync_time: i64,
    #[serde(default)]
    pub(super) auto_sync: bool,
    #[serde(default = "config_default_data_saver")]
    pub(super) data_saver: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConfigSyncPreferences {
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
pub(super) struct AndroidTypedSettings {
    #[serde(default)]
    pub(super) booleans: HashMap<String, bool>,
    #[serde(default)]
    pub(super) floats: HashMap<String, f32>,
    #[serde(default)]
    pub(super) longs: HashMap<String, i64>,
    #[serde(default)]
    pub(super) strings: HashMap<String, String>,
}

impl AndroidTypedSettings {
    /// 把 Android 分组偏好叠加到桌面设置上（仅映射两端语义相同的键，未知键忽略）
    pub(super) fn apply_to(&self, settings: &mut AppSettings) {
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
                "cloud_music_lyric_default_offset_ms" => {
                    settings.cloud_music_offset = *value as i32
                }
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
pub(super) struct AndroidListenTogether {
    #[serde(default)]
    pub(super) worker_base_url: String,
    #[serde(default)]
    pub(super) user_uuid: String,
    #[serde(default)]
    pub(super) nickname: String,
    #[serde(default = "config_default_true")]
    pub(super) allow_member_control: bool,
    #[serde(default = "config_default_true")]
    pub(super) auto_pause_on_member_change: bool,
    #[serde(default = "config_default_true")]
    pub(super) share_audio_links: bool,
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
pub(super) struct AndroidCookieAuth {
    #[serde(default)]
    pub(super) cookies: HashMap<String, String>,
}

/// YouTube 的登录快照，桌面恢复可持久化的 Cookie 和账号索引
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AndroidYouTubeAuth {
    #[serde(default)]
    pub(super) cookies: HashMap<String, String>,
    #[serde(default)]
    pub(super) cookie_header: String,
    #[serde(default)]
    pub(super) authorization: String,
    #[serde(default)]
    pub(super) x_goog_auth_user: String,
    #[serde(default)]
    pub(super) origin: String,
    #[serde(default)]
    pub(super) user_agent: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AndroidGitHubSync {
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
pub(super) struct AndroidWebDavSync {
    #[serde(default)]
    pub(super) server_url: String,
    #[serde(default)]
    pub(super) base_path: String,
    #[serde(default)]
    pub(super) username: String,
    #[serde(default)]
    pub(super) password: String,
    #[serde(default)]
    pub(super) auto_sync_enabled: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AndroidSyncPreferences {
    #[serde(default)]
    play_history_update_mode: String,
}

/// Android 端导出的配置备份（字段与 AppConfigBackup.kt 对齐）
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AndroidConfigFile {
    #[serde(default)]
    pub(super) kind: String,
    #[serde(default)]
    pub(super) format_version: u32,
    #[serde(default)]
    pub(super) settings: AndroidTypedSettings,
    #[serde(default)]
    pub(super) listen_together: Option<AndroidListenTogether>,
    #[serde(default)]
    pub(super) language: Option<ConfigLanguage>,
    #[serde(default)]
    pub(super) netease_auth: Option<AndroidCookieAuth>,
    #[serde(default)]
    pub(super) bili_auth: Option<AndroidCookieAuth>,
    #[serde(default)]
    pub(super) you_tube_auth: Option<AndroidYouTubeAuth>,
    #[serde(default)]
    pub(super) git_hub_sync: Option<AndroidGitHubSync>,
    #[serde(default)]
    pub(super) web_dav_sync: Option<AndroidWebDavSync>,
    #[serde(default)]
    pub(super) sync_preferences: Option<AndroidSyncPreferences>,
}

/// Android 与 PC 配置的判别：Android 的 settings 是类型分组
/// （booleans/floats/...），PC 是扁平 AppSettings，无 "booleans" 键
pub(super) fn is_android_config(root: &Value) -> bool {
    root.get("platform").and_then(Value::as_str) != Some("pc")
        && root.get("settings").is_some_and(|settings| {
            [
                "booleans",
                "floats",
                "ints",
                "longs",
                "strings",
                "stringSets",
            ]
            .iter()
            .any(|group| settings.get(group).is_some_and(Value::is_object))
        })
}

/// Android 只导出 name→value，域名按桌面手动粘贴 Cookie 的规则补齐（YouTube 还要补 google.com 的会话 Cookie）
pub(super) fn android_cookie_entries(
    cookies: &HashMap<String, String>,
    platform: &str,
) -> Vec<CookieEntry> {
    let raw = cookies
        .iter()
        .filter(|(name, value)| {
            neri_platform::auth::cookie_jar::cookie_name_allowed(name, platform)
                && !value.is_empty()
                && !value.contains(';')
                && !value.chars().any(char::is_control)
        })
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ");
    neri_platform::auth::cookie_jar::parse_raw_cookie_text(&raw, platform)
        .into_iter()
        .filter(|cookie| {
            neri_platform::auth::cookie_jar::cookie_name_allowed(&cookie.name, platform)
        })
        .collect()
}

/// Android 三平台 Cookie 快照 → 桌面 AuthState（不包含用户资料字段）
pub(super) fn map_android_auth(config: &AndroidConfigFile) -> Option<ImportedAuth> {
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
    let youtube = config.you_tube_auth.as_ref().and_then(|auth| {
        let cookies = if auth.cookies.is_empty() {
            neri_platform::auth::cookie_jar::parse_raw_cookie_text(&auth.cookie_header, "youtube")
        } else {
            android_cookie_entries(&auth.cookies, "youtube")
        };
        let imported = YouTubeAuth {
            cookies,
            nickname: None,
            avatar_url: None,
            auth_user: neri_platform::auth::state::youtube_auth_user(&auth.x_goog_auth_user)
                .map(String::from),
        };
        imported.has_login().then_some(imported)
    });
    // 没有 Cookie 的授权头无法恢复桌面会话，只有明确空段落表示清除该平台
    let replace = [
        config.netease_auth.is_some(),
        config.bili_auth.is_some(),
        config.you_tube_auth.as_ref().is_some_and(|auth| {
            youtube.is_some()
                || (auth.cookies.is_empty()
                    && auth.cookie_header.is_empty()
                    && auth.authorization.is_empty()
                    && auth.x_goog_auth_user.is_empty()
                    && auth.origin.is_empty()
                    && auth.user_agent.is_empty())
        }),
    ];
    if !replace.iter().any(|replace| *replace) {
        None
    } else {
        Some(ImportedAuth {
            state: AuthState {
                netease,
                bilibili,
                youtube,
            },
            replace,
        })
    }
}

/// Android 的非登录空分组不覆盖桌面配置，保留当前平台可用的设置
pub(super) fn map_android_language(language: Option<ConfigLanguage>) -> Option<ConfigLanguage> {
    language
        .filter(|language| !language.code.is_empty())
        .map(|mut language| {
            // Android 语言码是 zh/en/ja，桌面端是 zh-CN/zh-TW/en/ja
            if language.code == "zh" {
                language.code = "zh-CN".into();
            }
            language
        })
}

pub(super) fn map_android_listen_together(
    listen_together: Option<AndroidListenTogether>,
) -> Option<ConfigListenTogether> {
    listen_together
        .filter(|lt| {
            !lt.worker_base_url.is_empty() || !lt.user_uuid.is_empty() || !lt.nickname.is_empty()
        })
        .map(Into::into)
}

pub(super) fn map_android_github(github: Option<AndroidGitHubSync>) -> Option<ConfigGitHubSync> {
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

pub(super) fn map_android_webdav(webdav: Option<AndroidWebDavSync>) -> Option<ConfigWebDavSync> {
    webdav
        .filter(|webdav| {
            !webdav.server_url.is_empty()
                || !webdav.username.is_empty()
                || !webdav.password.is_empty()
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

pub(super) fn map_android_sync_preferences(
    preferences: Option<AndroidSyncPreferences>,
) -> Option<ConfigSyncPreferences> {
    preferences
        .filter(|preferences| !preferences.play_history_update_mode.is_empty())
        .map(|preferences| ConfigSyncPreferences {
            history_update_mode: normalize_history_update_mode(
                &preferences.play_history_update_mode,
            ),
        })
}

/// 归一化后的待导入配置（PC 与 Android 两种来源共用）
struct ImportedConfig {
    settings: AppSettings,
    listen_together: Option<ConfigListenTogether>,
    language: Option<ConfigLanguage>,
    auth: Option<ImportedAuth>,
    github_sync: Option<ConfigGitHubSync>,
    webdav_sync: Option<ConfigWebDavSync>,
    sync_preferences: Option<ConfigSyncPreferences>,
}

pub(super) struct ImportedAuth {
    pub(super) state: AuthState,
    pub(super) replace: [bool; 3],
}

pub(super) fn merge_imported_auth(previous: &AuthState, imported: ImportedAuth) -> AuthState {
    AuthState {
        netease: if imported.replace[0] {
            imported.state.netease
        } else {
            previous.netease.clone()
        },
        bilibili: if imported.replace[1] {
            imported.state.bilibili
        } else {
            previous.bilibili.clone()
        },
        youtube: if imported.replace[2] {
            imported.state.youtube
        } else {
            previous.youtube.clone()
        },
    }
}

/// 解析桌面端导出的配置文件
fn parse_pc_config(root: Value) -> AppResult<ImportedConfig> {
    let payload: DesktopConfigFile =
        serde_json::from_value(root).map_err(|_| AppError::Other("CONFIG_FILE_INVALID".into()))?;
    if payload.kind != CONFIG_FILE_KIND
        || (payload.format_version != 0 && payload.format_version > CONFIG_FILE_VERSION)
        || payload.platform != "pc"
    {
        return Err(AppError::Other("CONFIG_FILE_UNSUPPORTED".into()));
    }
    Ok(ImportedConfig {
        settings: payload.settings,
        listen_together: payload.listen_together,
        language: payload.language,
        auth: payload.auth.map(|state| ImportedAuth {
            state,
            replace: [true; 3],
        }),
        github_sync: payload.github_sync,
        webdav_sync: payload.webdav_sync,
        sync_preferences: payload.sync_preferences,
    })
}

fn deserialize_android_config(root: Value) -> AppResult<AndroidConfigFile> {
    let payload: AndroidConfigFile =
        serde_json::from_value(root).map_err(|_| AppError::Other("CONFIG_FILE_INVALID".into()))?;
    if payload.kind != CONFIG_FILE_KIND
        || (payload.format_version != 0 && payload.format_version > CONFIG_FILE_VERSION)
    {
        return Err(AppError::Other("CONFIG_FILE_UNSUPPORTED".into()));
    }
    Ok(payload)
}

/// 解析 Android 端导出的配置备份，偏好叠加到桌面当前设置上
fn parse_android_config(app: &AppHandle, payload: AndroidConfigFile) -> AppResult<ImportedConfig> {
    let mut settings = store::load_settings(app)?.settings;
    payload.settings.apply_to(&mut settings);

    let auth = map_android_auth(&payload);
    Ok(ImportedConfig {
        settings,
        listen_together: map_android_listen_together(payload.listen_together),
        language: map_android_language(payload.language),
        auth,
        github_sync: map_android_github(payload.git_hub_sync),
        webdav_sync: map_android_webdav(payload.web_dav_sync),
        sync_preferences: map_android_sync_preferences(payload.sync_preferences),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DesktopConfigFile {
    #[serde(default)]
    pub(super) kind: String,
    #[serde(default)]
    pub(super) format_version: u32,
    #[serde(default)]
    pub(super) platform: String,
    #[serde(default)]
    pub(super) platform_name: String,
    #[serde(default)]
    pub(super) exported_at: i64,
    #[serde(default, deserialize_with = "store::deserialize_app_settings")]
    pub(super) settings: AppSettings,
    #[serde(default)]
    pub(super) listen_together: Option<ConfigListenTogether>,
    #[serde(default)]
    pub(super) language: Option<ConfigLanguage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) auth: Option<AuthState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) github_sync: Option<ConfigGitHubSync>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) webdav_sync: Option<ConfigWebDavSync>,
    #[serde(default)]
    pub(super) sync_preferences: Option<ConfigSyncPreferences>,
}

/// 按用户选择导出登录凭据，并使用与 Android 相同的加密信封
#[tauri::command]
pub async fn export_config(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: AppSettings,
    listen_together_user_uuid: String,
    password: Option<String>,
    include_login_state: Option<bool>,
) -> AppResult<Value> {
    let include_login_state = include_login_state.unwrap_or(false);
    let auth = include_login_state.then(|| state.auth.lock().clone());
    let preferences = load_sync_preferences(&app);
    let github = include_login_state.then(|| {
        let mut github = load_github_config(&app);
        github.history_update_mode = preferences.history_update_mode.clone();
        ConfigGitHubSync::from(&github)
    });
    let webdav = include_login_state.then(|| ConfigWebDavSync::from(&load_webdav_config(&app)));
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
        auth,
        github_sync: github,
        webdav_sync: webdav,
        sync_preferences: Some(ConfigSyncPreferences::from(&preferences)),
    };
    let config =
        serde_json::to_value(config).map_err(|_| AppError::Other("CONFIG_FILE_INVALID".into()))?;
    let content = tauri::async_runtime::spawn_blocking(move || {
        config_backup::encode(&config, password.as_deref())
    })
    .await
    .map_err(|_| AppError::Other("CONFIG_ENCRYPTION_INVALID".into()))?
    .map_err(|error| AppError::Other(error.to_string()))?;
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

/// 先解密并确认 Android 兼容性，再恢复设置、登录状态和同步配置
#[tauri::command]
pub async fn import_config(
    app: AppHandle,
    state: State<'_, AppState>,
    password: Option<String>,
    confirmed_android: Option<bool>,
    file_path: Option<String>,
) -> AppResult<Value> {
    let file_path = if let Some(path) = file_path {
        std::path::PathBuf::from(path)
    } else {
        let Some(path) = pick_file_path(&app, None).await? else {
            return Ok(dialog_cancelled());
        };
        path
    };
    let selected_path = file_path.to_string_lossy().into_owned();
    let decoded = tauri::async_runtime::spawn_blocking(move || {
        let file = std::fs::File::open(&file_path)
            .map_err(|_| AppError::Other("CONFIG_FILE_READ_FAILED".into()))?;
        let mut content = Vec::new();
        file.take(config_backup::MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut content)
            .map_err(|_| AppError::Other("CONFIG_FILE_READ_FAILED".into()))?;
        Ok::<_, AppError>(config_backup::decode(&content, password.as_deref()))
    })
    .await
    .map_err(|_| AppError::Other("CONFIG_FILE_READ_FAILED".into()))??;
    let root = match decoded {
        Ok(root) => root,
        Err(ConfigBackupError::PasswordRequired) => {
            return Ok(serde_json::json!({
                "success": false,
                "requiresPassword": true,
                "filePath": selected_path,
            }))
        }
        Err(error) => return Err(AppError::Other(error.to_string())),
    };
    let android = is_android_config(&root);
    let imported = if android {
        let payload = deserialize_android_config(root)?;
        if !confirmed_android.unwrap_or(false) {
            return Ok(serde_json::json!({
                "success": false,
                "requiresConfirmation": true,
                "filePath": selected_path,
                "platform": "android",
            }));
        }
        parse_android_config(&app, payload)?
    } else {
        parse_pc_config(root)?
    };
    // 对话框、密码和格式预检都通过后才使在途同步配置请求失效
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

    let mut changed_auth_platforms = Vec::new();
    if let Some(imported_auth) = imported.auth {
        use neri_platform::auth::coordination::AuthPlatform;
        let mut auth_operations = state.auth_operations.lock();
        for (platform, replace) in [
            AuthPlatform::Netease,
            AuthPlatform::Bilibili,
            AuthPlatform::Youtube,
        ]
        .into_iter()
        .zip(imported_auth.replace)
        {
            if replace {
                auth_operations.begin(platform);
            }
        }
        let mut auth = state.auth.lock();
        let previous_auth = auth.clone();
        for (platform, replace) in ["netease", "bilibili", "youtube"]
            .into_iter()
            .zip(imported_auth.replace)
        {
            if !replace {
                continue;
            }
            changed_auth_platforms.push(platform);
            neri_platform::auth::cookie_jar::expire_platform_cookies(
                &state.cookie_jar,
                &previous_auth,
                platform,
            );
            let cookies = match platform {
                "netease" => imported_auth
                    .state
                    .netease
                    .as_ref()
                    .map(|auth| &auth.cookies),
                "bilibili" => imported_auth
                    .state
                    .bilibili
                    .as_ref()
                    .map(|auth| &auth.cookies),
                _ => imported_auth
                    .state
                    .youtube
                    .as_ref()
                    .map(|auth| &auth.cookies),
            };
            if let Some(cookies) = cookies {
                neri_platform::auth::cookie_jar::inject_cookies(&state.cookie_jar, cookies);
            }
        }
        *auth = merge_imported_auth(&previous_auth, imported_auth);
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
        "platform": if android { "android" } else { "pc" },
        "changedAuthPlatforms": changed_auth_platforms,
    }))
}
