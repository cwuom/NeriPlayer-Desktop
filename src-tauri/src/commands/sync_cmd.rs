// 同步相关命令
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};
use crate::error::{AppError, AppResult};
use crate::settings::store::{self, AppSettings};
use crate::state::AppState;
use crate::sync::models::*;
use crate::sync::manager;
use crate::security;
use tauri_plugin_store::StoreExt;

// 同步配置存储键
const GITHUB_CONFIG_KEY: &str = "githubSync";
const WEBDAV_CONFIG_KEY: &str = "webdavSync";
const SYNC_PREFERENCES_KEY: &str = "syncPreferences";
const SYNC_STORE: &str = "sync-config.json";
const CONFIG_FILE_KIND: &str = "moe.ouom.neriplayer.config";
const CONFIG_FILE_VERSION: u32 = 1;

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

/// 启动时迁移旧版同步凭据，避免只有打开设置页后才清理明文
pub fn initialize_secure_storage(app: &AppHandle) {
    let _ = load_github_config(app);
    let _ = load_webdav_config(app);
}

fn load_github_config(app: &AppHandle) -> GitHubSyncConfig {
    let mut config: GitHubSyncConfig = app.store(SYNC_STORE).ok()
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

fn save_github_config(app: &AppHandle, config: &GitHubSyncConfig) {
    if config.token.is_empty() {
        let _ = security::delete_secret(security::GITHUB_TOKEN_KEY);
    } else if !security::set_secret(security::GITHUB_TOKEN_KEY, &config.token) {
        log::error!(target: "sync", "凭据存储不可用，GitHub Token 未持久化");
    }

    if let Ok(s) = app.store(SYNC_STORE) {
        let _ = s.set(GITHUB_CONFIG_KEY, github_config_store_value(config));
        let _ = s.save();
    }
}

fn load_webdav_config(app: &AppHandle) -> WebDavSyncConfig {
    let mut config: WebDavSyncConfig = app.store(SYNC_STORE).ok()
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

fn save_webdav_config(app: &AppHandle, config: &WebDavSyncConfig) {
    if config.password.is_empty() {
        let _ = security::delete_secret(security::WEBDAV_PASSWORD_KEY);
    } else if !security::set_secret(security::WEBDAV_PASSWORD_KEY, &config.password) {
        log::error!(target: "sync", "凭据存储不可用，WebDAV 密码未持久化");
    }

    if let Ok(s) = app.store(SYNC_STORE) {
        let _ = s.set(WEBDAV_CONFIG_KEY, webdav_config_store_value(config));
        let _ = s.save();
    }
}

fn load_sync_preferences(app: &AppHandle) -> SyncPreferencesConfig {
    let stored = app.store(SYNC_STORE).ok()
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

fn save_sync_preferences(app: &AppHandle, config: &SyncPreferencesConfig) {
    if let Ok(store) = app.store(SYNC_STORE) {
        let _ = store.set(SYNC_PREFERENCES_KEY, sync_preferences_store_value(config));
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
        data_saver: true,  // 默认开启省流
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
        data_saver: true,  // 默认开启省流
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

    let config = GitHubSyncConfig {
        token,
        owner: owner.clone(),
        repo: repo.clone(),
        auto_sync: true,
        data_saver: true,  // 默认开启省流
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
    let mut config = load_github_config(&app);
    if config.token.is_empty() {
        return Err(AppError::Api("GitHub sync not configured".into()));
    }

    let local_data = manager::build_local_sync_data(
        &app,
        history_entries.as_deref(),
        history_deletions.as_deref(),
    );
    let result = manager::sync_github(&state.http(), &mut config, &local_data).await?;
    save_github_config(&app, &config);
    // 通知前端歌单数据可能已变更
    let _ = app.emit("playlists-changed", ());
    Ok(result)
}

/// 断开 GitHub 同步
#[tauri::command]
pub async fn disconnect_github_sync(app: AppHandle) -> AppResult<()> {
    save_github_config(&app, &GitHubSyncConfig::default());
    Ok(())
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
    let bp = base_path.unwrap_or_default();
    let api = crate::sync::webdav_api::WebDavApiClient::new(
        &state.http(), &server_url, &username, &password, &bp,
    );
    api.validate_connection().await?;

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
    let mut config = load_webdav_config(&app);
    if config.server_url.is_empty() {
        return Err(AppError::Api("WebDAV sync not configured".into()));
    }

    let local_data = manager::build_local_sync_data(
        &app,
        history_entries.as_deref(),
        history_deletions.as_deref(),
    );
    let result = manager::sync_webdav(&state.http(), &mut config, &local_data).await?;
    save_webdav_config(&app, &config);
    let _ = app.emit("playlists-changed", ());
    Ok(result)
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
    let mut config = load_github_config(&app);
    if config.token.is_empty() {
        return Err(AppError::Api("GitHub sync not configured".into()));
    }
    if let Some(v) = auto_sync { config.auto_sync = v; }
    if let Some(v) = data_saver { config.data_saver = v; }
    if let Some(v) = silent_failures { config.silent_failures = v; }
    if let Some(v) = history_update_mode {
        let normalized = normalize_history_update_mode(&v);
        config.history_update_mode = normalized.clone();
        save_sync_preferences(&app, &SyncPreferencesConfig {
            history_update_mode: normalized,
        });
    }
    save_github_config(&app, &config);
    Ok(())
}

/// 更新 WebDAV 同步子设置
#[tauri::command]
pub async fn update_webdav_sync_settings(
    app: AppHandle,
    auto_sync: Option<bool>,
) -> AppResult<()> {
    let mut config = load_webdav_config(&app);
    if config.server_url.is_empty() {
        return Err(AppError::Api("WebDAV sync not configured".into()));
    }
    if let Some(v) = auto_sync { config.auto_sync = v; }
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
                if let Ok(size) = dir_size(&path) { cleared += size; }
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

    use tauri_plugin_dialog::DialogExt;
    let path = app.dialog().file()
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

/// 导入播放列表 JSON（兼容 Android BackupData 和 Desktop 两种格式）
#[tauri::command]
pub async fn import_playlists(app: AppHandle) -> AppResult<Value> {
    use crate::library::playlist::{PlaylistStore, Playlist};
    use crate::sync::models::{SyncPlaylist, SyncData};
    use crate::sync::manager::save_synced_playlists;
    use tauri_plugin_dialog::DialogExt;

    let path = app.dialog().file()
        .add_filter("JSON", &["json"])
        .blocking_pick_file();

    match path {
        Some(p) => {
            let data = std::fs::read_to_string(p.as_path().unwrap())
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

                // 通过 save_synced_playlists 转换并回写（复用已有的去重 + 转换逻辑）
                let sync_data = SyncData {
                    playlists: sync_playlists,
                    ..Default::default()
                };
                save_synced_playlists(&sync_data);
            } else if parsed.is_array() {
                // 尝试 Desktop 格式
                if let Ok(imported) = serde_json::from_value::<Vec<Playlist>>(parsed.clone()) {
                    count = imported.len();
                    let playlists_path = {
                        let mut path = dirs_next::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
                        path.push("NeriPlayer");
                        path.push("playlists.json");
                        path
                    };
                    let mut store = PlaylistStore::load(&playlists_path);
                    for pl in imported {
                        if !store.playlists.iter().any(|p| p.name == pl.name) {
                            store.playlists.push(pl);
                        }
                    }
                    store.save(&playlists_path)?;
                } else {
                    // 可能是 SyncPlaylist 数组（无外层包装）
                    let sync_playlists: Vec<SyncPlaylist> = serde_json::from_value(parsed)
                        .map_err(|e| AppError::Other(format!("Parse playlists array: {}", e)))?;
                    count = sync_playlists.len();
                    let sync_data = SyncData {
                        playlists: sync_playlists,
                        ..Default::default()
                    };
                    save_synced_playlists(&sync_data);
                }
            } else {
                return Err(AppError::Other("Unrecognized playlist format".into()));
            }

            let _ = app.emit("playlists-changed", ());
            Ok(serde_json::json!({ "success": true, "imported": count }))
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

/// 导入 PC 配置文件并恢复设置、登录状态和同步配置
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
    let payload: DesktopConfigFile = serde_json::from_str(&content)
        .map_err(|e| AppError::Other(format!("Parse config failed: {}", e)))?;
    if payload.kind != CONFIG_FILE_KIND
        || (payload.format_version != 0 && payload.format_version > CONFIG_FILE_VERSION)
        || payload.platform != "pc"
    {
        return Err(AppError::Other("Unsupported config file".into()));
    }

    let imported_listen_together = payload.listen_together;
    let imported_language = payload.language;
    let mut settings = payload.settings;
    if let Some(language) = imported_language.as_ref().filter(|language| !language.code.is_empty()) {
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

    if let Some(imported_auth) = payload.auth {
        let _cookie_guard = state.auth_cookie_gate.lock().await;
        {
            let mut auth = state.auth.lock();
            let previous_auth = auth.clone();
            for platform in ["netease", "bilibili", "youtube"] {
                crate::auth::cookies::expire_platform_cookies(
                    &state.cookie_jar,
                    &previous_auth,
                    platform,
                );
            }
            *auth = imported_auth;
            crate::auth::cookies::save_auth(&app, &auth);
        }

        crate::commands::auth_cmd::clear_and_reinject_webview_cookies(&app, &state).await?;
    }
    let legacy_history_mode = payload
        .github_sync
        .as_ref()
        .map(|github| github.history_update_mode.clone());
    if let Some(preferences) = payload.sync_preferences {
        save_sync_preferences(&app, &preferences.into_config());
    } else if let Some(mode) = legacy_history_mode {
        save_sync_preferences(&app, &SyncPreferencesConfig {
            history_update_mode: normalize_history_update_mode(&mode),
        });
    }
    if let Some(github) = payload.github_sync {
        save_github_config(&app, &github.into_config());
    }
    if let Some(webdav) = payload.webdav_sync {
        save_webdav_config(&app, &webdav.into_config());
    }

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
    save_webdav_config(&app, &WebDavSyncConfig::default());
    Ok(())
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
}
