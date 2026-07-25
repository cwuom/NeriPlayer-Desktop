// Cookie 持久化：Release 使用系统钥匙串，Debug 使用随机路径明文文件
// tauri-plugin-store 仅负责旧数据迁移
use reqwest::cookie::Jar;
use reqwest::Url;
use std::sync::Arc;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

use super::state::{AuthState, CookieEntry};
use crate::error::{AppError, AppResult};
use crate::security;

const STORE_FILE: &str = "auth.json";
const STORE_KEY: &str = "auth_state";

/// 持久化 AuthState
/// Persist AuthState and surface serialization and credential-storage failures.
pub fn save_auth_strict(app: &AppHandle, auth: &AuthState) -> AppResult<()> {
    let Some(serialized) = auth_persistence_payload(auth)? else {
        if delete_persisted_auth(app) {
            return Ok(());
        }
        return Err(AppError::Other(
            "Failed to delete persisted authentication credentials".into(),
        ));
    };

    if !security::set_secret(security::AUTH_STATE_KEY, &serialized) {
        clear_legacy_auth(app);
        return Err(AppError::Other(
            "Failed to write authentication credentials".into(),
        ));
    }

    clear_legacy_auth(app);
    Ok(())
}

fn auth_persistence_payload(auth: &AuthState) -> AppResult<Option<String>> {
    if !has_any_auth(auth) {
        return Ok(None);
    }
    Ok(Some(serde_json::to_string(auth)?))
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

    // 目标存储不可用时不继续使用旧明文凭据，避免下次启动再次暴露
    log::error!(target: "auth", "旧版登录凭据迁移失败，已清除明文凭据");
    clear_legacy_auth(app);
    AuthState::default()
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

/// 将所有已登录平台的 Cookie 注入 Jar
pub fn inject_all(jar: &Arc<Jar>, auth: &AuthState) {
    if let Some(ref netease) = auth.netease {
        inject_cookies(jar, &netease.cookies);
    }
    if let Some(ref bilibili) = auth.bilibili {
        inject_cookies(jar, &bilibili.cookies);
    }
    if let Some(ref youtube) = auth.youtube {
        inject_cookies(jar, &youtube.cookies);
    }
}

/// 将 Cookie 列表注入 Jar（包含 Domain 属性，确保子域名可用）
pub fn inject_cookies(jar: &Arc<Jar>, entries: &[CookieEntry]) {
    for entry in entries {
        let url = domain_to_url(&entry.domain);
        if let Ok(url) = url.parse::<Url>() {
            // 必须设置 Domain 属性，否则 reqwest 按精确域名匹配，子域名 API 拿不到 cookie
            jar.add_cookie_str(
                &format!(
                    "{}={}; Domain={}; Path=/",
                    entry.name, entry.value, entry.domain
                ),
                &url,
            );
        }
    }
}

/// 登出时过期指定平台的 Cookie
pub fn expire_platform_cookies(jar: &Arc<Jar>, auth: &AuthState, platform: &str) {
    let entries = match platform {
        "netease" => auth.netease.as_ref().map(|a| &a.cookies),
        "bilibili" => auth.bilibili.as_ref().map(|a| &a.cookies),
        "youtube" => auth.youtube.as_ref().map(|a| &a.cookies),
        _ => None,
    };

    if let Some(entries) = entries {
        for entry in entries {
            let url = domain_to_url(&entry.domain);
            if let Ok(url) = url.parse::<Url>() {
                // 必须带 Domain + Path 属性，与注入时一致，才能正确覆盖并过期
                jar.add_cookie_str(
                    &format!(
                        "{}=deleted; Domain={}; Path=/; Max-Age=0",
                        entry.name, entry.domain
                    ),
                    &url,
                );
            }
        }
    }
}

/// 解析 document.cookie 字符串为 CookieEntry 列表
pub fn parse_document_cookies(cookie_str: &str, domain: &str) -> Vec<CookieEntry> {
    cookie_str
        .split(';')
        .filter_map(|pair| {
            let pair = pair.trim();
            let (name, value) = pair.split_once('=')?;
            let name = name.trim();
            let value = value.trim();
            if name.is_empty() {
                return None;
            }
            Some(CookieEntry {
                name: name.to_string(),
                value: value.to_string(),
                domain: domain.to_string(),
            })
        })
        .collect()
}

/// 解析用户粘贴的原始 Cookie 文本（对齐 Android RawCookieTextParser）
/// 支持分号、换行、回车分隔
pub fn parse_raw_cookie_text(raw: &str, platform: &str) -> Vec<CookieEntry> {
    let domain = match platform {
        "netease" => "music.163.com",
        "bilibili" => ".bilibili.com",
        "youtube" => ".youtube.com",
        _ => "unknown",
    };

    let mut entries = Vec::new();
    // 按 ; \r \n 分割
    for segment in raw.split([';', '\r', '\n']) {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        if let Some((name, value)) = segment.split_once('=') {
            let name = name.trim().to_string();
            let value = value.trim().to_string();
            if !name.is_empty() {
                entries.push(CookieEntry {
                    name,
                    value,
                    domain: domain.to_string(),
                });
            }
        }
    }

    // YouTube 需要额外为 google.com 注入部分 Cookie
    if platform == "youtube" {
        let google_entries: Vec<CookieEntry> = entries
            .iter()
            .filter(|c| {
                matches!(
                    c.name.as_str(),
                    "SID" | "HSID" | "SSID" | "APISID" | "SAPISID" | "LSID" | "SIDCC"
                )
            })
            .map(|c| CookieEntry {
                name: c.name.clone(),
                value: c.value.clone(),
                domain: ".google.com".into(),
            })
            .collect();
        entries.extend(google_entries);
    }

    entries
}

/// 域名转 URL（用于 Jar.add_cookie_str）
fn domain_to_url(domain: &str) -> String {
    let d = domain.trim_start_matches('.');
    format!("https://{}", d)
}

#[cfg(test)]
mod tests {
    use super::{auth_persistence_payload, parse_raw_cookie_text};
    use crate::auth::state::{AuthState, NeteaseAuth};

    #[test]
    fn raw_cookie_parser_accepts_all_supported_separators() {
        let entries = parse_raw_cookie_text("first=1;second=2\rthird=3\nfourth=4", "netease");
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();

        assert_eq!(names, ["first", "second", "third", "fourth"]);
    }

    #[test]
    fn empty_auth_state_requests_persisted_credential_deletion() {
        assert_eq!(
            auth_persistence_payload(&AuthState::default()).expect("build persistence payload"),
            None
        );
    }

    #[test]
    fn authenticated_state_serializes_for_strict_persistence() {
        let auth = AuthState {
            netease: Some(NeteaseAuth {
                cookies: vec![],
                user_id: Some(42),
                nickname: Some("Neri".into()),
                avatar_url: None,
            }),
            ..Default::default()
        };

        let serialized = auth_persistence_payload(&auth)
            .expect("serialize auth state")
            .expect("non-empty auth payload");
        let restored: AuthState =
            serde_json::from_str(&serialized).expect("deserialize auth state");

        assert_eq!(restored.netease.and_then(|state| state.user_id), Some(42));
    }
}
