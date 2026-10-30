// YouTube 登录会话维护：合并 Set-Cookie，避免会话漂移
use std::collections::{HashMap, HashSet};

use crate::auth::state::{CookieEntry, YouTubeAuth};

const IDENTITY_COOKIE_KEYS: &[&str] = &[
    "SID",
    "HSID",
    "SSID",
    "LSID",
    "LOGIN_INFO",
    "__Secure-1PSID",
    "__Secure-3PSID",
];

const PERSISTED_COOKIE_KEYS: &[&str] = &[
    "SID",
    "HSID",
    "SSID",
    "APISID",
    "SAPISID",
    "LOGIN_INFO",
    "SIDCC",
    "__Secure-1PSID",
    "__Secure-3PSID",
    "__Secure-1PAPISID",
    "__Secure-3PAPISID",
    "__Secure-1PSIDTS",
    "__Secure-3PSIDTS",
    "__Secure-1PSIDCC",
    "__Secure-3PSIDCC",
    "PREF",
    "VISITOR_INFO1_LIVE",
    "YSC",
];

/// 带轮换语义的会话令牌：Google 每次签发新值都会作废旧值
///
/// 桌面与移动端若共享同一 SID 家族，两端各自收编轮换值就会互相让对方失效，
/// 表现为「另一台设备登录后这边掉登录」。这里一旦本地已有值就不再收编，
/// 保持登录时拿到的那一份，把轮换主导权让给用户的主力设备。
const ROTATING_SESSION_COOKIE_KEYS: &[&str] = &["__Secure-1PSIDTS", "__Secure-3PSIDTS"];

fn is_rotating_session_cookie(name: &str) -> bool {
    ROTATING_SESSION_COOKIE_KEYS
        .iter()
        .any(|key| key.eq_ignore_ascii_case(name))
}

pub(crate) fn canonical_cookie_key(name: &str) -> Option<&'static str> {
    PERSISTED_COOKIE_KEYS
        .iter()
        .copied()
        .find(|key| key.eq_ignore_ascii_case(name))
}

pub(crate) fn cookie_key_allowed(name: &str) -> bool {
    canonical_cookie_key(name).is_some()
        || name.starts_with("__Secure-")
        || name.starts_with("__Host-")
}

/// Set-Cookie 没带 Domain 时的兜底域
///
/// __Secure-/__Host- 前缀的会话 cookie 归 Google 账号域，其余归 YouTube 域。
fn domain_for_name(name: &str) -> String {
    if name.starts_with("__Secure-") || name.starts_with("__Host-") {
        ".google.com".into()
    } else {
        ".youtube.com".into()
    }
}

/// 解析响应 Set-Cookie 头为 CookieEntry
pub fn parse_set_cookie_headers(headers: &[String]) -> Vec<CookieEntry> {
    let mut out = Vec::new();
    for header in headers {
        if let Some(entry) = parse_single_set_cookie(header) {
            out.push(entry);
        }
    }
    out
}

fn parse_single_set_cookie(header: &str) -> Option<CookieEntry> {
    let first = header.split(';').next()?.trim();
    let (name, value) = first.split_once('=')?;
    let name = name.trim();
    let value = value.trim();
    if name.is_empty() || value.is_empty() || !cookie_key_allowed(name) {
        return None;
    }

    let mut domain = domain_for_name(name);
    for part in header.split(';').skip(1) {
        let part = part.trim();
        if let Some(raw) = part
            .strip_prefix("Domain=")
            .or_else(|| part.strip_prefix("domain="))
        {
            let d = raw.trim();
            if !d.is_empty() {
                domain = if d.starts_with('.') {
                    d.to_string()
                } else {
                    format!(".{d}")
                };
            }
        }
    }

    Some(CookieEntry {
        name: name.to_string(),
        value: value.to_string(),
        domain,
    })
}

fn cookies_to_map(entries: &[CookieEntry]) -> HashMap<String, CookieEntry> {
    let mut map = HashMap::new();
    for entry in entries {
        if entry.name.is_empty() || entry.value.is_empty() {
            continue;
        }
        map.insert(entry.name.clone(), entry.clone());
    }
    map
}

fn identity_matches(previous: &HashMap<String, CookieEntry>, current: &HashMap<String, CookieEntry>) -> bool {
    IDENTITY_COOKIE_KEYS.iter().any(|key| {
        match (previous.get(*key), current.get(*key)) {
            (Some(left), Some(right)) => !left.value.is_empty() && left.value == right.value,
            _ => false,
        }
    })
}

fn has_conflicting_identity(
    previous: &HashMap<String, CookieEntry>,
    current: &HashMap<String, CookieEntry>,
) -> bool {
    IDENTITY_COOKIE_KEYS.iter().any(|key| {
        match (previous.get(*key), current.get(*key)) {
            (Some(left), Some(right)) => {
                !left.value.is_empty() && !right.value.is_empty() && left.value != right.value
            }
            _ => false,
        }
    })
}

/// 合并观察值到现有会话；同身份时保留旧 cookie，避免短暂响应冲掉完整登录态
pub fn merge_youtube_auth_cookies(
    base: &YouTubeAuth,
    observed: &[CookieEntry],
) -> YouTubeAuth {
    if observed.is_empty() {
        return base.clone();
    }

    let previous = cookies_to_map(&base.cookies);
    let mut merged = previous.clone();
    for entry in observed {
        if entry.name.is_empty() || entry.value.is_empty() || !cookie_key_allowed(&entry.name) {
            continue;
        }
        // 已持有轮换令牌时保留旧值，不参与与其它设备的轮换竞争
        if is_rotating_session_cookie(&entry.name)
            && previous
                .get(&entry.name)
                .is_some_and(|existing| !existing.value.is_empty())
        {
            continue;
        }
        merged.insert(entry.name.clone(), entry.clone());
    }

    // 观察值与旧会话同身份时，补回观察值缺失的关键 cookie
    if identity_matches(&previous, &merged) && !has_conflicting_identity(&previous, &merged) {
        for (key, value) in &previous {
            merged.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }

    let mut cookies: Vec<CookieEntry> = merged.into_values().collect();
    cookies.sort_by(|a, b| a.name.cmp(&b.name));

    YouTubeAuth {
        cookies,
        nickname: base.nickname.clone(),
        avatar_url: base.avatar_url.clone(),
        auth_user: base.auth_user.clone(),
    }
}

/// 应用 Google RotateCookies 明确签发的新 SIDTS
///
/// 普通页面响应仍走 `merge_youtube_auth_cookies` 的保守策略，避免共享 Jar 的
/// 轮换值覆盖登录态；RotateCookies 是唯一允许替换已有 SIDTS 的受控入口
pub fn merge_rotated_youtube_cookies(
    base: &YouTubeAuth,
    rotated: &[CookieEntry],
) -> YouTubeAuth {
    let mut cookies = base.cookies.clone();
    for observed in rotated {
        if !is_rotating_session_cookie(&observed.name) || observed.value.is_empty() {
            continue;
        }
        if let Some(existing) = cookies.iter_mut().find(|cookie| cookie.name == observed.name) {
            *existing = observed.clone();
        } else {
            cookies.push(observed.clone());
        }
    }
    cookies.sort_by(|left, right| left.name.cmp(&right.name));
    YouTubeAuth {
        cookies,
        nickname: base.nickname.clone(),
        avatar_url: base.avatar_url.clone(),
        auth_user: base.auth_user.clone(),
    }
}

pub fn youtube_auth_changed(previous: &YouTubeAuth, current: &YouTubeAuth) -> bool {
    let left: HashSet<_> = previous
        .cookies
        .iter()
        .map(|c| (c.name.as_str(), c.value.as_str(), c.domain.as_str()))
        .collect();
    let right: HashSet<_> = current
        .cookies
        .iter()
        .map(|c| (c.name.as_str(), c.value.as_str(), c.domain.as_str()))
        .collect();
    left != right || previous.selected_auth_user() != current.selected_auth_user()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_set_cookie_keeps_sapisid() {
        let headers = vec![
            "SAPISID=abc123; Path=/; Domain=.youtube.com; Secure".into(),
            "random=1; Path=/".into(),
        ];
        let parsed = parse_set_cookie_headers(&headers);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "SAPISID");
        assert_eq!(parsed[0].value, "abc123");
    }

    #[test]
    fn merge_preserves_identity_cookies() {
        let base = YouTubeAuth {
            cookies: vec![
                CookieEntry {
                    name: "SID".into(),
                    value: "sid-old".into(),
                    domain: ".youtube.com".into(),
                },
                CookieEntry {
                    name: "SAPISID".into(),
                    value: "sap-old".into(),
                    domain: ".youtube.com".into(),
                },
            ],
            nickname: Some("n".into()),
            avatar_url: None,
            auth_user: Some("2".into()),
        };
        let observed = vec![CookieEntry {
            name: "SAPISID".into(),
            value: "sap-new".into(),
            domain: ".youtube.com".into(),
        }];
        let merged = merge_youtube_auth_cookies(&base, &observed);
        assert_eq!(merged.selected_auth_user(), Some("2"));
        assert!(merged.cookies.iter().any(|c| c.name == "SID" && c.value == "sid-old"));
        assert!(merged
            .cookies
            .iter()
            .any(|c| c.name == "SAPISID" && c.value == "sap-new"));
    }

    #[test]
    fn rotating_session_tokens_are_not_adopted_once_present() {
        let base = YouTubeAuth {
            cookies: vec![
                CookieEntry {
                    name: "SID".into(),
                    value: "sid".into(),
                    domain: ".youtube.com".into(),
                },
                CookieEntry {
                    name: "__Secure-3PSIDTS".into(),
                    value: "sidts-login".into(),
                    domain: ".google.com".into(),
                },
            ],
            nickname: None,
            avatar_url: None,
            auth_user: None,
        };
        let observed = vec![
            CookieEntry {
                name: "__Secure-3PSIDTS".into(),
                value: "sidts-rotated".into(),
                domain: ".google.com".into(),
            },
            CookieEntry {
                name: "SIDCC".into(),
                value: "sidcc-new".into(),
                domain: ".youtube.com".into(),
            },
        ];

        let merged = merge_youtube_auth_cookies(&base, &observed);

        assert!(merged
            .cookies
            .iter()
            .any(|c| c.name == "__Secure-3PSIDTS" && c.value == "sidts-login"));
        // 非轮换令牌照常收编
        assert!(merged
            .cookies
            .iter()
            .any(|c| c.name == "SIDCC" && c.value == "sidcc-new"));
    }

    #[test]
    fn rotating_session_tokens_are_adopted_when_absent() {
        let base = YouTubeAuth {
            cookies: vec![CookieEntry {
                name: "SID".into(),
                value: "sid".into(),
                domain: ".youtube.com".into(),
            }],
            nickname: None,
            avatar_url: None,
            auth_user: None,
        };
        let observed = vec![CookieEntry {
            name: "__Secure-1PSIDTS".into(),
            value: "sidts-first".into(),
            domain: ".google.com".into(),
        }];

        let merged = merge_youtube_auth_cookies(&base, &observed);

        assert!(merged
            .cookies
            .iter()
            .any(|c| c.name == "__Secure-1PSIDTS" && c.value == "sidts-first"));
    }

    #[test]
    fn explicit_rotation_replaces_existing_session_tokens() {
        let base = YouTubeAuth {
            cookies: vec![CookieEntry {
                name: "__Secure-3PSIDTS".into(),
                value: "sidts-old".into(),
                domain: ".google.com".into(),
            }],
            nickname: Some("name".into()),
            avatar_url: None,
            auth_user: None,
        };
        let rotated = vec![CookieEntry {
            name: "__Secure-3PSIDTS".into(),
            value: "sidts-new".into(),
            domain: ".google.com".into(),
        }];

        let merged = merge_rotated_youtube_cookies(&base, &rotated);

        assert_eq!(
            merged
                .cookies
                .iter()
                .find(|cookie| cookie.name == "__Secure-3PSIDTS")
                .map(|cookie| cookie.value.as_str()),
            Some("sidts-new")
        );
        assert_eq!(merged.nickname.as_deref(), Some("name"));
    }
}
