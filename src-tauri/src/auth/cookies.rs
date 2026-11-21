// Cookie 持久化：Release 使用系统钥匙串，Debug 使用随机路径明文文件
// tauri-plugin-store 仅负责旧数据迁移；Jar 操作在 neri_platform::auth::cookie_jar
use neri_common::security;
use neri_platform::auth::state::AuthState;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

const STORE_FILE: &str = "auth.json";
const STORE_KEY: &str = "auth_state";

/// 持久化 AuthState
pub fn save_auth(app: &AppHandle, auth: &AuthState) {
    if !has_any_auth(auth) {
        if !delete_persisted_auth(app) {
            log::error!(target: "auth", "登录凭据存储不可用，无法清除登录凭据");
        }
        return;
    }

    let Ok(serialized) = serde_json::to_string(auth) else {
        log::error!(target: "auth", "登录凭据序列化失败，已跳过持久化");
        return;
    };

    if !security::set_secret(security::AUTH_STATE_KEY, &serialized) {
        // 写入失败通常是磁盘或钥匙串的瞬时问题，删除旧密文或迁移源会把
        // 原本可恢复的登录直接变成永久登出，保留它们供下次启动重试
        log::error!(target: "auth", "登录凭据存储不可用，已保留现有凭据等待重试");
        return;
    }

    clear_legacy_auth(app);
}

/// 启动时恢复 AuthState，并迁移旧版明文数据
pub fn load_auth(app: &AppHandle) -> AuthState {
    if let Some(serialized) = security::get_secret(security::AUTH_STATE_KEY) {
        clear_legacy_auth(app);
        return match serde_json::from_str(&serialized) {
            Ok(auth) => auth,
            Err(_) => {
                log::error!(target: "auth", "登录凭据格式无效，已清除");
                let _ = security::delete_secret(security::AUTH_STATE_KEY);
                AuthState::default()
            }
        };
    }

    let legacy = load_legacy_auth(app);
    if !has_any_auth(&legacy) {
        clear_legacy_auth(app);
        return AuthState::default();
    }

    let Ok(serialized) = serde_json::to_string(&legacy) else {
        clear_legacy_auth(app);
        return AuthState::default();
    };

    if security::set_secret(security::AUTH_STATE_KEY, &serialized) {
        clear_legacy_auth(app);
        return legacy;
    }

    // 目标存储瞬时不可用（如磁盘满）时销毁有效旧凭据会造成不可恢复的掉登录:
    // 保留明文供下次启动重试迁移（代价: 一个启动周期的明文暴露窗口）, 本次先用起来（AU-12）
    log::error!(target: "auth", "旧版登录凭据迁移失败，暂保留明文等待下次启动重试");
    legacy
}

/// 删除所有持久化登录凭据，包括旧版明文数据
pub fn delete_persisted_auth(app: &AppHandle) -> bool {
    let deleted = security::delete_secret(security::AUTH_STATE_KEY);
    clear_legacy_auth(app);
    deleted
}

fn load_legacy_auth(app: &AppHandle) -> AuthState {
    let Ok(store) = app.store(STORE_FILE) else {
        return AuthState::default();
    };
    store
        .get(STORE_KEY)
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default()
}

fn clear_legacy_auth(app: &AppHandle) {
    if let Ok(store) = app.store(STORE_FILE) {
        let _ = store.delete(STORE_KEY);
        let _ = store.save();
    }
}

fn has_any_auth(auth: &AuthState) -> bool {
    auth.netease.is_some() || auth.bilibili.is_some() || auth.youtube.is_some()
}
