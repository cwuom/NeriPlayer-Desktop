// 三平台登录状态数据结构
use serde::{Deserialize, Serialize};

/// 全局登录状态
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthState {
    pub netease: Option<NeteaseAuth>,
    pub bilibili: Option<BiliAuth>,
    pub youtube: Option<YouTubeAuth>,
}

/// 网易云登录凭证
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeteaseAuth {
    pub cookies: Vec<CookieEntry>,
    pub user_id: Option<u64>,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
}

/// B站登录凭证
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiliAuth {
    pub cookies: Vec<CookieEntry>,
    pub mid: Option<u64>,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
}

/// YouTube Music 登录凭证
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YouTubeAuth {
    pub cookies: Vec<CookieEntry>,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub auth_user: Option<String>,
}

/// 持久化的单条 Cookie
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookieEntry {
    pub name: String,
    pub value: String,
    pub domain: String,
}

impl NeteaseAuth {
    /// 检查是否有有效的 MUSIC_U Cookie
    pub fn has_login(&self) -> bool {
        self.cookies.iter().any(|c| c.name == "MUSIC_U" && !c.value.is_empty())
    }
}

impl BiliAuth {
    /// 检查是否有有效的 SESSDATA Cookie
    pub fn has_login(&self) -> bool {
        self.cookies.iter().any(|c| c.name == "SESSDATA" && !c.value.is_empty())
    }
}

impl YouTubeAuth {
    pub fn selected_auth_user(&self) -> Option<&str> {
        self.auth_user.as_deref().and_then(youtube_auth_user)
    }

    /// Google 账号域的 Cookie 不能单独代表 YouTube 登录
    pub fn has_login(&self) -> bool {
        self.get_sapisid().is_some()
    }

    /// 优先 SAPISID，再使用 Android 支持的两个安全别名
    pub fn get_sapisid(&self) -> Option<&str> {
        youtube_sapisid(&self.cookies)
    }
}

pub(crate) fn youtube_sapisid(cookies: &[CookieEntry]) -> Option<&str> {
    ["SAPISID", "__Secure-1PAPISID", "__Secure-3PAPISID"]
        .into_iter()
        .find_map(|name| {
            cookies.iter()
                .filter(|cookie| {
                    let domain = cookie.domain.trim_start_matches('.').to_ascii_lowercase();
                    cookie.name == name && !cookie.value.is_empty()
                        && (domain == "youtube.com" || domain.ends_with(".youtube.com"))
                })
                .max_by_key(|cookie| match cookie.domain.trim_start_matches('.').to_ascii_lowercase().as_str() {
                    "music.youtube.com" => 3,
                    "youtube.com" => 2,
                    _ => 1,
                })
                .map(|cookie| cookie.value.as_str())
        })
}

pub fn youtube_auth_user(raw: &str) -> Option<&str> {
    let value = raw.trim();
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) && value.parse::<u32>().is_ok())
        .then_some(value)
}

/// 前端友好的登录状态摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthInfo {
    pub platform: String,
    pub logged_in: bool,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
}

/// 三平台登录状态聚合响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStatusResponse {
    pub netease: AuthInfo,
    pub bilibili: AuthInfo,
    pub youtube: AuthInfo,
}

impl AuthState {
    /// 生成前端状态摘要
    pub fn to_status_response(&self) -> AuthStatusResponse {
        AuthStatusResponse {
            netease: match &self.netease {
                Some(a) => AuthInfo {
                    platform: "netease".into(),
                    logged_in: a.has_login(),
                    nickname: a.nickname.clone(),
                    avatar_url: a.avatar_url.clone(),
                },
                None => AuthInfo {
                    platform: "netease".into(),
                    logged_in: false,
                    nickname: None,
                    avatar_url: None,
                },
            },
            bilibili: match &self.bilibili {
                Some(a) => AuthInfo {
                    platform: "bilibili".into(),
                    logged_in: a.has_login(),
                    nickname: a.nickname.clone(),
                    avatar_url: a.avatar_url.clone(),
                },
                None => AuthInfo {
                    platform: "bilibili".into(),
                    logged_in: false,
                    nickname: None,
                    avatar_url: None,
                },
            },
            youtube: match &self.youtube {
                Some(a) => AuthInfo {
                    platform: "youtube".into(),
                    logged_in: a.has_login(),
                    nickname: a.nickname.clone(),
                    avatar_url: a.avatar_url.clone(),
                },
                None => AuthInfo {
                    platform: "youtube".into(),
                    logged_in: false,
                    nickname: None,
                    avatar_url: None,
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CookieEntry, YouTubeAuth};

    #[test]
    fn youtube_account_index_is_optional_and_rejects_header_injection() {
        for value in ["", "-1", "account", "1\r\nOther: value", "4294967296"] {
            assert_eq!(super::youtube_auth_user(value), None);
        }
        assert_eq!(super::youtube_auth_user(" 2 "), Some("2"));
        let legacy: YouTubeAuth = serde_json::from_str(r#"{"cookies":[]}"#).unwrap();
        assert_eq!(legacy.selected_auth_user(), None);
    }

    #[test]
    fn youtube_web_host_cookie_is_a_valid_identity_for_web_playback() {
        let auth = YouTubeAuth {
            cookies: vec![CookieEntry { name: "SAPISID".into(), value: "web".into(), domain: "www.youtube.com".into() }],
            nickname: None,
            avatar_url: None,
            auth_user: None,
        };
        assert!(auth.has_login());
        assert_eq!(auth.get_sapisid(), Some("web"));
    }

    #[test]
    fn youtube_login_accepts_nonempty_music_scope_sapisid_aliases() {
        for name in ["SAPISID", "__Secure-1PAPISID", "__Secure-3PAPISID"] {
            for domain in [".youtube.com", "music.youtube.com", "www.youtube.com"] {
                let auth = YouTubeAuth {
                    cookies: vec![CookieEntry { name: name.into(), value: "fixture".into(), domain: domain.into() }],
                    nickname: None,
                    avatar_url: None,
                    auth_user: None,
                };
                assert!(auth.has_login(), "{name}: {domain}");
                assert_eq!(auth.get_sapisid(), Some("fixture"));
            }
            for domain in [".google.com", "accounts.google.com", "youtube.com.example", "evilyoutube.com"] {
                let cookies = [CookieEntry { name: name.into(), value: "fixture".into(), domain: domain.into() }];
                assert_eq!(super::youtube_sapisid(&cookies), None, "{name}: {domain}");
            }
        }
    }

    #[test]
    fn youtube_empty_or_unrelated_identity_does_not_hide_valid_alias() {
        let mut auth = YouTubeAuth {
            cookies: vec![
                CookieEntry { name: "SAPISID".into(), value: String::new(), domain: ".youtube.com".into() },
                CookieEntry { name: "SAPISID".into(), value: "google-only".into(), domain: ".google.com".into() },
                CookieEntry { name: "__Secure-3PAPISID".into(), value: "music".into(), domain: "music.youtube.com".into() },
            ],
            nickname: None,
            avatar_url: None,
            auth_user: None,
        };
        assert_eq!(auth.get_sapisid(), Some("music"));
        auth.cookies.pop();
        assert!(!auth.has_login());
        assert_eq!(auth.get_sapisid(), None);
    }
}
