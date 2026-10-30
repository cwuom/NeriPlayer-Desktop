// Cookie 持久化：Release 使用系统钥匙串，Debug 使用随机路径明文文件
// tauri-plugin-store 仅负责旧数据迁移
use std::collections::HashMap;
use std::sync::Arc;
use reqwest::cookie::{CookieStore, Jar};
use reqwest::Url;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

use super::state::{AuthState, CookieEntry};
use crate::security;

const STORE_FILE: &str = "auth.json";
const STORE_KEY: &str = "auth_state";

const NETEASE_COOKIE_DOMAINS: &[&str] = &["music.163.com", "interface.music.163.com"];
const NETEASE_COOKIE_KEYS: &[&str] = &[
    "MUSIC_U",
    "MUSIC_A",
    "__csrf",
    "NMTID",
    "__remember_me",
    "ntes_utid",
    "os",
    "appver",
    "channel",
    "playerid",
];
const BILIBILI_COOKIE_DOMAINS: &[&str] = &[
    "www.bilibili.com",
    "api.bilibili.com",
    "passport.bilibili.com",
];
const BILIBILI_COOKIE_KEYS: &[&str] = &[
    "SESSDATA",
    "bili_jct",
    "DedeUserID",
    "DedeUserID__ckMd5",
    "sid",
    "buvid3",
    "buvid4",
    "b_nut",
    "b_lsid",
    "bili_ticket",
    "bili_ticket_expires",
];
const YOUTUBE_COOKIE_DOMAINS: &[&str] = &[
    "music.youtube.com",
    "www.youtube.com",
    "accounts.google.com",
    "www.google.com",
];

// 轮换会话令牌（与 api::youtube::session::ROTATING_SESSION_COOKIE_KEYS 一致）:
// 一旦本地已有值就不从共享 Jar 收编轮换值, 把轮换主导权让给主力设备,
// 避免与 session 侧 merge 策略打架（AU-06）
const ROTATING_SESSION_COOKIE_KEYS: &[&str] = &["__Secure-1PSIDTS", "__Secure-3PSIDTS"];

fn is_rotating_session_cookie(name: &str) -> bool {
    ROTATING_SESSION_COOKIE_KEYS
        .iter()
        .any(|key| key.eq_ignore_ascii_case(name))
}

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
                &format!("{}={}; Domain={}; Path=/", entry.name, entry.value, entry.domain),
                &url,
            );
        }
    }
}

/// 使指定 Cookie 失效，供粘贴登录失败时回滚共享 Jar
pub fn expire_cookie_entries(jar: &Arc<Jar>, entries: &[CookieEntry]) {
    for entry in entries {
        let url = domain_to_url(&entry.domain);
        if let Ok(url) = url.parse::<Url>() {
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

/// 从共享 Jar 回收服务端轮换后的 Cookie 值
///
/// 服务端会在响应的 Set-Cookie 中轮换会话令牌，reqwest 会把新值写进 Jar，
/// 但 AuthState 只在登录时落盘。重启后 inject_all 会把登录当天的旧值重新写回
/// Jar，对实现了令牌轮换 + 重放检测的服务端就等于提交了一个已作废的令牌，
/// 整条会话链会被判为异常 —— 表现就是「另一台设备登录后这边掉登录」。
///
/// 返回是否发生了变更，调用方据此决定要不要重新落盘。
pub fn sync_auth_from_jar(jar: &Arc<Jar>, auth: &mut AuthState) -> bool {
    let mut cache: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut changed = false;
    if let Some(netease) = auth.netease.as_mut() {
        changed |= refresh_platform_auth_entries(
            jar,
            &mut netease.cookies,
            "netease",
            NETEASE_COOKIE_DOMAINS,
            &mut cache,
        );
    }
    if let Some(bilibili) = auth.bilibili.as_mut() {
        changed |= refresh_platform_auth_entries(
            jar,
            &mut bilibili.cookies,
            "bilibili",
            BILIBILI_COOKIE_DOMAINS,
            &mut cache,
        );
    }
    if let Some(youtube) = auth.youtube.as_mut() {
        changed |= refresh_platform_auth_entries(
            jar,
            &mut youtube.cookies,
            "youtube",
            YOUTUBE_COOKIE_DOMAINS,
            &mut cache,
        );
    }
    changed
}

fn refresh_platform_auth_entries(
    jar: &Arc<Jar>,
    entries: &mut Vec<CookieEntry>,
    platform: &str,
    domains: &[&str],
    cache: &mut HashMap<String, HashMap<String, String>>,
) -> bool {
    let mut changed = false;
    for entry in entries.iter_mut() {
        // SIDTS 家族与 session 侧策略统一: 已持有值就保留登录当天的那份, 不从 Jar 收编轮换值
        if is_rotating_session_cookie(&entry.name) {
            continue;
        }
        let values = cache
            .entry(entry.domain.clone())
            .or_insert_with(|| read_jar_cookies(jar, &entry.domain));
        let Some(current) = values.get(&entry.name) else {
            continue;
        };
        if current.is_empty() || current == &entry.value {
            continue;
        }
        entry.value = current.clone();
        changed = true;
    }

    let mut existing_names: std::collections::HashSet<String> =
        entries.iter().map(|entry| entry.name.clone()).collect();
    for domain in domains {
        let values = cache
            .entry((*domain).to_string())
            .or_insert_with(|| read_jar_cookies(jar, domain));
        for (name, value) in values.iter() {
            if value.is_empty()
                || existing_names.contains(name)
                || !persisted_cookie_allowed(platform, name)
            {
                continue;
            }
            entries.push(CookieEntry {
                name: name.clone(),
                value: value.clone(),
                domain: format!(".{domain}"),
            });
            existing_names.insert(name.clone());
            changed = true;
        }
    }
    changed
}

fn persisted_cookie_allowed(platform: &str, name: &str) -> bool {
    cookie_name_allowed(name, platform)
        && (platform != "youtube" || !is_rotating_session_cookie(name))
}

/// 配置导入和平台会话回收共用凭据白名单
pub fn cookie_name_allowed(name: &str, platform: &str) -> bool {
    match platform {
        "netease" => NETEASE_COOKIE_KEYS.iter().any(|key| key.eq_ignore_ascii_case(name)),
        "bilibili" => BILIBILI_COOKIE_KEYS.iter().any(|key| key.eq_ignore_ascii_case(name)),
        "youtube" => crate::api::youtube::session::cookie_key_allowed(name),
        _ => false,
    }
}

fn read_jar_cookies(jar: &Arc<Jar>, domain: &str) -> HashMap<String, String> {
    let Ok(url) = domain_to_url(domain).parse::<Url>() else {
        return HashMap::new();
    };
    let Some(header) = jar.cookies(&url) else {
        return HashMap::new();
    };
    let Ok(text) = header.to_str() else {
        return HashMap::new();
    };
    text.split(';')
        .filter_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            Some((name.to_string(), value.trim().to_string()))
        })
        .collect()
}

/// 读取网易云 __csrf Cookie, 供 WEAPI 请求作为 csrf_token 查询参数
/// 对齐 Android NeteaseClient: 所有 WEAPI 调用都附加 csrf_token(缺失时为空串)
pub fn read_netease_csrf(jar: &Arc<Jar>) -> String {
    read_jar_cookies(jar, "music.163.com")
        .get("__csrf")
        .cloned()
        .unwrap_or_default()
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
        // 必须带 Domain + Path 属性，与注入时一致，才能正确覆盖并过期
        expire_cookie_entries(jar, entries);
    }

    let domains = match platform {
        "netease" => NETEASE_COOKIE_DOMAINS,
        "bilibili" => BILIBILI_COOKIE_DOMAINS,
        "youtube" => YOUTUBE_COOKIE_DOMAINS,
        _ => return,
    };
    let mut discovered = Vec::new();
    for domain in domains {
        for (name, value) in read_jar_cookies(jar, domain) {
            if persisted_cookie_allowed(platform, &name) && !value.is_empty() {
                discovered.push(CookieEntry {
                    name,
                    value,
                    domain: format!(".{domain}"),
                });
            }
        }
    }
    expire_cookie_entries(jar, &discovered);
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
        _ => return Vec::new(),
    };

    let raw = raw.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim().eq_ignore_ascii_case("cookie").then_some(value)
    }).unwrap_or(raw);
    let mut entries: Vec<CookieEntry> = Vec::new();
    for segment in raw.split([';', '\r', '\n']) {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        if let Some((name, value)) = segment.split_once('=') {
            let name = name.trim();
            let value = value.trim();
            if name.is_empty() || value.is_empty()
                || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+.^_`|~-".contains(&byte))
                || value.chars().any(char::is_control)
                || (platform == "youtube" && !cookie_name_allowed(name, platform))
            {
                continue;
            }
            if let Some(existing) = entries.iter_mut().find(|entry| entry.name == name) {
                existing.value = value.to_string();
            } else {
                entries.push(CookieEntry { name: name.to_string(), value: value.to_string(), domain: domain.to_string() });
            }
        }
    }

    if platform == "netease" && !entries.is_empty() {
        for (name, value) in [("os", "pc"), ("appver", "8.10.35")] {
            if !entries.iter().any(|entry| entry.name == name) {
                entries.push(CookieEntry { name: name.into(), value: value.into(), domain: domain.into() });
            }
        }
    }

    // YouTube 需要额外为 google.com 注入部分 Cookie
    if platform == "youtube" {
        let google_entries: Vec<CookieEntry> = entries.iter()
            .filter(|c| matches!(c.name.as_str(), "SID" | "HSID" | "SSID" | "APISID" | "SAPISID" | "LSID" | "SIDCC"))
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
    use super::*;
    use crate::auth::state::{NeteaseAuth, YouTubeAuth};

    #[test]
    fn raw_cookie_parser_accepts_all_supported_separators() {
        let entries = parse_raw_cookie_text("first=1;second=2\rthird=3\nfourth=4", "netease");
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();

        assert_eq!(&names[..4], ["first", "second", "third", "fourth"]);
    }

    #[test]
    fn raw_cookie_parser_uses_last_nonempty_value_and_rejects_invalid_pairs() {
        let entries = parse_raw_cookie_text(
            "SESSDATA=first; SESSDATA=last=part; empty=; bad key=value; injected=bad\u{7f}",
            "bilibili",
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "SESSDATA");
        assert_eq!(entries[0].value, "last=part");
    }

    #[test]
    fn raw_cookie_header_prefix_is_accepted_for_each_platform() {
        for (platform, name) in [("netease", "MUSIC_U"), ("bilibili", "SESSDATA"), ("youtube", "SAPISID")] {
            let entries = parse_raw_cookie_text(&format!("Cookie: {name}=fixture"), platform);
            assert!(entries.iter().any(|entry| entry.name == name && entry.value == "fixture"), "{platform}");
        }
    }

    #[test]
    fn raw_cookie_parser_applies_netease_defaults_without_overwriting_explicit_values() {
        let entries = parse_raw_cookie_text("MUSIC_U=fixture; os=android", "netease");
        assert!(entries.iter().any(|entry| entry.name == "os" && entry.value == "android"));
        assert!(entries.iter().any(|entry| entry.name == "appver" && entry.value == "8.10.35"));
        assert!(parse_raw_cookie_text("empty=; bad key=value", "netease").is_empty());
    }

    #[test]
    fn youtube_raw_headers_only_import_allowlisted_cookies() {
        let entries = parse_raw_cookie_text(
            "Cookie: SAPISID=fixture; account_secret=private; __Secure-1PAPISID=alias\nAuthorization: SAPISIDHASH unused\nX-Goog-AuthUser: 1",
            "youtube",
        );
        assert!(entries.iter().any(|entry| entry.name == "SAPISID" && entry.domain == ".youtube.com"));
        assert!(entries.iter().any(|entry| entry.name == "__Secure-1PAPISID"));
        assert!(!entries.iter().any(|entry| entry.name == "account_secret"));
        assert!(parse_raw_cookie_text("Authorization: SAPISIDHASH unused", "youtube").is_empty());
    }

    #[test]
    fn rotated_jar_cookies_are_written_back_into_auth_state() {
        let jar = Arc::new(Jar::default());
        let mut auth = AuthState {
            netease: Some(NeteaseAuth {
                cookies: vec![
                    CookieEntry {
                        name: "MUSIC_U".into(),
                        value: "stale".into(),
                        domain: "music.163.com".into(),
                    },
                    CookieEntry {
                        name: "os".into(),
                        value: "pc".into(),
                        domain: "music.163.com".into(),
                    },
                ],
                user_id: None,
                nickname: None,
                avatar_url: None,
            }),
            ..Default::default()
        };
        inject_cookies(&jar, &auth.netease.as_ref().unwrap().cookies);

        // 模拟服务端轮换
        jar.add_cookie_str(
            "MUSIC_U=rotated; Domain=music.163.com; Path=/",
            &"https://music.163.com".parse::<Url>().unwrap(),
        );

        assert!(sync_auth_from_jar(&jar, &mut auth));
        let cookies = &auth.netease.as_ref().unwrap().cookies;
        assert_eq!(
            cookies.iter().find(|c| c.name == "MUSIC_U").unwrap().value,
            "rotated"
        );
        assert_eq!(cookies.iter().find(|c| c.name == "os").unwrap().value, "pc");
        // 无变化时不应报告变更，避免每次心跳都写钥匙串
        assert!(!sync_auth_from_jar(&jar, &mut auth));
    }

    #[test]
    fn allowlisted_new_cookie_is_persisted_without_collecting_unknown_keys() {
        let jar = Arc::new(Jar::default());
        jar.add_cookie_str(
            "__csrf=csrf-value; Domain=music.163.com; Path=/",
            &"https://music.163.com".parse::<Url>().unwrap(),
        );
        jar.add_cookie_str(
            "account_secret=must-not-persist; Domain=music.163.com; Path=/",
            &"https://music.163.com".parse::<Url>().unwrap(),
        );
        let mut auth = AuthState {
            netease: Some(NeteaseAuth {
                cookies: vec![CookieEntry {
                    name: "MUSIC_U".into(),
                    value: "music".into(),
                    domain: "music.163.com".into(),
                }],
                user_id: None,
                nickname: None,
                avatar_url: None,
            }),
            ..Default::default()
        };

        assert!(sync_auth_from_jar(&jar, &mut auth));
        let cookies = &auth.netease.as_ref().unwrap().cookies;
        assert!(cookies.iter().any(|cookie| {
            cookie.name == "__csrf" && cookie.value == "csrf-value"
        }));
        assert!(!cookies.iter().any(|cookie| cookie.name == "account_secret"));
    }

    #[test]
    fn logout_expiry_clears_allowlisted_runtime_cookie() {
        let jar = Arc::new(Jar::default());
        let url = "https://music.163.com".parse::<Url>().unwrap();
        jar.add_cookie_str(
            "__csrf=csrf-value; Domain=music.163.com; Path=/",
            &url,
        );
        let auth = AuthState {
            netease: Some(NeteaseAuth {
                cookies: Vec::new(),
                user_id: None,
                nickname: None,
                avatar_url: None,
            }),
            ..Default::default()
        };

        expire_platform_cookies(&jar, &auth, "netease");

        assert!(!read_jar_cookies(&jar, "music.163.com").contains_key("__csrf"));
    }

    #[test]
    fn rotating_youtube_cookie_is_not_recovered_from_generic_jar() {
        let jar = Arc::new(Jar::default());
        jar.add_cookie_str(
            "__Secure-1PSIDTS=jar-value; Domain=.google.com; Path=/",
            &"https://accounts.google.com".parse::<Url>().unwrap(),
        );
        let mut auth = AuthState {
            youtube: Some(YouTubeAuth {
                cookies: vec![
                    CookieEntry {
                        name: "SAPISID".into(),
                        value: "binding".into(),
                        domain: ".google.com".into(),
                    },
                    CookieEntry {
                        name: "__Secure-1PSIDTS".into(),
                        value: "persisted".into(),
                        domain: ".google.com".into(),
                    },
                ],
                nickname: None,
                avatar_url: None,
                auth_user: None,
            }),
            ..Default::default()
        };

        assert!(!sync_auth_from_jar(&jar, &mut auth));
        assert_eq!(
            auth.youtube
                .as_ref()
                .unwrap()
                .cookies
                .iter()
                .find(|cookie| cookie.name == "__Secure-1PSIDTS")
                .unwrap()
                .value,
            "persisted"
        );
    }
}
