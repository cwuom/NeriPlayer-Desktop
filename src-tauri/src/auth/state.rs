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
}

/// 持久化的单条 Cookie
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieEntry {
    pub name: String,
    pub value: String,
    pub domain: String,
}

impl NeteaseAuth {
    /// 检查是否有已验证账号和有效的 MUSIC_U Cookie
    pub fn has_login(&self) -> bool {
        self.user_id.is_some_and(|user_id| user_id > 0)
            && self
                .cookies
                .iter()
                .any(|c| c.name == "MUSIC_U" && !c.value.is_empty())
    }
}

impl BiliAuth {
    /// 检查是否有有效的 SESSDATA Cookie
    pub fn has_login(&self) -> bool {
        self.cookies.iter().any(|c| c.name == "SESSDATA" && !c.value.is_empty())
    }
}

impl YouTubeAuth {
    /// 检查是否有 SAPISID 或 __Secure-3PAPISID
    pub fn has_login(&self) -> bool {
        self.cookies.iter().any(|c| {
            (c.name == "SAPISID" || c.name == "__Secure-3PAPISID") && !c.value.is_empty()
        })
    }

    /// 获取 SAPISID 值（优先 SAPISID，fallback __Secure-3PAPISID）
    pub fn get_sapisid(&self) -> Option<&str> {
        self.cookies.iter()
            .find(|c| c.name == "SAPISID")
            .or_else(|| self.cookies.iter().find(|c| c.name == "__Secure-3PAPISID"))
            .map(|c| c.value.as_str())
    }
}

/// 前端友好的登录状态摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthInfo {
    pub platform: String,
    pub logged_in: bool,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub account_id: Option<String>,
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
                    account_id: a.user_id.map(|id| id.to_string()),
                },
                None => AuthInfo {
                    platform: "netease".into(),
                    logged_in: false,
                    nickname: None,
                    avatar_url: None,
                    account_id: None,
                },
            },
            bilibili: match &self.bilibili {
                Some(a) => AuthInfo {
                    platform: "bilibili".into(),
                    logged_in: a.has_login(),
                    nickname: a.nickname.clone(),
                    avatar_url: a.avatar_url.clone(),
                    account_id: a.mid.map(|id| id.to_string()),
                },
                None => AuthInfo {
                    platform: "bilibili".into(),
                    logged_in: false,
                    nickname: None,
                    avatar_url: None,
                    account_id: None,
                },
            },
            youtube: match &self.youtube {
                Some(a) => AuthInfo {
                    platform: "youtube".into(),
                    logged_in: a.has_login(),
                    nickname: a.nickname.clone(),
                    avatar_url: a.avatar_url.clone(),
                    account_id: None,
                },
                None => AuthInfo {
                    platform: "youtube".into(),
                    logged_in: false,
                    nickname: None,
                    avatar_url: None,
                    account_id: None,
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CookieEntry, NeteaseAuth};

    fn music_cookie() -> CookieEntry {
        CookieEntry {
            name: "MUSIC_U".into(),
            value: "session".into(),
            domain: "music.163.com".into(),
        }
    }

    #[test]
    fn netease_auth_without_user_id_is_not_logged_in() {
        let auth = NeteaseAuth {
            cookies: vec![music_cookie()],
            user_id: None,
            nickname: None,
            avatar_url: None,
        };

        assert!(!auth.has_login());
    }

    #[test]
    fn netease_auth_with_zero_user_id_is_not_logged_in() {
        let auth = NeteaseAuth {
            cookies: vec![music_cookie()],
            user_id: Some(0),
            nickname: None,
            avatar_url: None,
        };

        assert!(!auth.has_login());
    }

    #[test]
    fn netease_auth_with_cookie_and_positive_user_id_is_logged_in() {
        let auth = NeteaseAuth {
            cookies: vec![music_cookie()],
            user_id: Some(42),
            nickname: None,
            avatar_url: None,
        };

        assert!(auth.has_login());
    }
}
