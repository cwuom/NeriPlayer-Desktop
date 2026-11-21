//! 同步配置的持久化：tauri store 与系统安全存储读写

use super::*;

/// 启动时迁移旧版同步凭据，避免只有打开设置页后才清理明文
pub fn initialize_secure_storage(app: &AppHandle) {
    let _ = load_github_config(app);
    let _ = load_webdav_config(app);
}

pub(super) fn load_github_config(app: &AppHandle) -> GitHubSyncConfig {
    with_sync_config(|| load_github_config_unlocked(app))
}

pub(super) fn load_github_config_unlocked(app: &AppHandle) -> GitHubSyncConfig {
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

pub(super) fn save_github_config_unlocked(app: &AppHandle, config: &GitHubSyncConfig) {
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

pub(super) fn load_webdav_config(app: &AppHandle) -> WebDavSyncConfig {
    with_sync_config(|| load_webdav_config_unlocked(app))
}

pub(super) fn load_webdav_config_unlocked(app: &AppHandle) -> WebDavSyncConfig {
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

pub(super) fn save_webdav_config_unlocked(app: &AppHandle, config: &WebDavSyncConfig) {
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

pub(super) fn load_sync_preferences(app: &AppHandle) -> SyncPreferencesConfig {
    with_sync_config(|| load_sync_preferences_unlocked(app))
}

pub(super) fn load_sync_preferences_unlocked(app: &AppHandle) -> SyncPreferencesConfig {
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

pub(super) fn save_sync_preferences_unlocked(app: &AppHandle, config: &SyncPreferencesConfig) {
    if let Ok(store) = app.store(SYNC_STORE) {
        store.set(SYNC_PREFERENCES_KEY, sync_preferences_store_value(config));
        let _ = store.save();
    }
}

pub(super) fn github_config_store_value(config: &GitHubSyncConfig) -> Value {
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

pub(super) fn sync_preferences_store_value(config: &SyncPreferencesConfig) -> Value {
    serde_json::json!({
        "historyUpdateMode": normalize_history_update_mode(&config.history_update_mode),
    })
}

pub(super) fn webdav_config_store_value(config: &WebDavSyncConfig) -> Value {
    serde_json::json!({
        "serverUrl": config.server_url,
        "username": config.username,
        "basePath": config.base_path,
        "lastRemoteFingerprint": config.last_remote_fingerprint,
        "lastSyncTime": config.last_sync_time,
        "autoSync": config.auto_sync,
    })
}
