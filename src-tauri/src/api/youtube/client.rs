// YouTube Music InnerTube API 客户端
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use parking_lot::Mutex;

use crate::error::{AppError, AppResult};

pub use super::account::YouTubeAccountProfile;

const INNERTUBE_URL: &str = "https://music.youtube.com/youtubei/v1";
pub(super) const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36";

// 默认 API key（可能随时变化，需要从页面 bootstrap 获取）
const DEFAULT_API_KEY: &str = "AIzaSyC9XL3ZjWddXya6X74dJoCTL-WEYFDNX30";
const DEFAULT_CLIENT_VERSION: &str = "1.20250415.01.00";

pub struct YouTubeClient {
    http: Client,
    api_key: Mutex<String>,
    client_version: Mutex<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YtSearchResult {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub thumbnail_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YtAudioStream {
    pub url: String,
    pub bitrate: u64,
    pub mime_type: String,
    pub content_length: u64,
}

impl YouTubeClient {
    pub fn new(http: &Client) -> Self {
        Self {
            http: http.clone(),
            api_key: Mutex::new(DEFAULT_API_KEY.to_string()),
            client_version: Mutex::new(DEFAULT_CLIENT_VERSION.to_string()),
        }
    }

    /// 构建 InnerTube context
    fn build_context(&self) -> Value {
        let version = self.client_version.lock().clone();
        json!({
            "client": {
                "clientName": "WEB_REMIX",
                "clientVersion": version,
                "hl": "zh-CN",
                "gl": "JP",
                "platform": "DESKTOP",
                "userAgent": USER_AGENT,
                "utcOffsetMinutes": 480
            },
            "user": { "lockedSafetyMode": false }
        })
    }

    /// InnerTube POST 请求
    async fn innertube_post(&self, endpoint: &str, body: &Value) -> AppResult<Value> {
        let api_key = self.api_key.lock().clone();
        let url = format!("{}/{}?prettyPrint=false&key={}", INNERTUBE_URL, endpoint, api_key);

        let resp = self.http
            .post(&url)
            .header("User-Agent", USER_AGENT)
            .header("Content-Type", "application/json")
            .header("Origin", "https://music.youtube.com")
            .header("Referer", "https://music.youtube.com/")
            .header("X-YouTube-Client-Name", "67")
            .json(body)
            .send().await?;

        let data: Value = resp.json().await?;
        Ok(data)
    }

    /// 搜索音乐
    pub async fn search(&self, query: &str) -> AppResult<Value> {
        let body = json!({
            "context": self.build_context(),
            "query": query,
            "params": "EgWKAQIIAWoMEA4QChADEAQQCRAF"  // 搜索歌曲过滤器
        });

        self.innertube_post("search", &body).await
    }

    /// 获取音频流 (兼容入口: 委托 playback; 无 auth 时仅 guest 路径)
    /// 正式播放请走 commands 注入 YouTubeAuth, 以便 Premium 生效
    pub async fn get_streams(&self, video_id: &str) -> AppResult<Vec<YtAudioStream>> {
        let _ = self;
        super::playback::resolve_audio_streams(video_id, None).await
    }

    /// 获取歌词（通过 next endpoint）
    pub async fn get_lyrics(&self, video_id: &str) -> AppResult<Option<String>> {
        let body = json!({
            "context": self.build_context(),
            "videoId": video_id,
            "isAudioOnly": true
        });

        let resp = self.innertube_post("next", &body).await?;

        // 歌词在 tabs 中
        let tabs = resp["contents"]["singleColumnMusicWatchNextResultsRenderer"]
            ["tabbedRenderer"]["watchNextTabbedResultsRenderer"]["tabs"]
            .as_array();

        if let Some(tabs) = tabs {
            for tab in tabs {
                let endpoint = &tab["tabRenderer"]["endpoint"];
                if let Some(browse_id) = endpoint["browseEndpoint"]["browseId"].as_str() {
                    if browse_id.starts_with("MPLYt_") {
                        // 获取歌词内容
                        let lyrics_body = json!({
                            "context": self.build_context(),
                            "browseId": browse_id
                        });
                        let lyrics_resp = self.innertube_post("browse", &lyrics_body).await?;
                        let text = lyrics_resp["contents"]["sectionListRenderer"]["contents"]
                            [0]["musicDescriptionShelfRenderer"]["description"]["runs"]
                            [0]["text"].as_str();
                        return Ok(text.map(String::from));
                    }
                }
            }
        }

        Ok(None)
    }

    // 需要登录的 API
    /// 认证版 InnerTube POST: 使用完整 SAPISID*HASH (对齐 Android buildYouTubeInnertubeRequestHeaders)
    async fn innertube_post_auth(
        &self,
        endpoint: &str,
        body: &Value,
        auth: &crate::auth::state::YouTubeAuth,
    ) -> AppResult<Value> {
        let (data, _) = self.innertube_post_auth_with_session(endpoint, body, auth).await?;
        Ok(data)
    }

    /// 构建 Cookie 头字符串
    fn build_cookie_header(cookies: &[crate::auth::state::CookieEntry]) -> String {
        cookies
            .iter()
            .filter(|c| !c.name.is_empty() && !c.value.is_empty())
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn cookie_map(auth: &crate::auth::state::YouTubeAuth) -> std::collections::BTreeMap<String, String> {
        let mut map = std::collections::BTreeMap::new();
        for cookie in &auth.cookies {
            if cookie.name.is_empty() || cookie.value.is_empty() {
                continue;
            }
            map.insert(cookie.name.clone(), cookie.value.clone());
        }
        map
    }

    fn authorization_header(auth: &crate::auth::state::YouTubeAuth) -> AppResult<String> {
        let cookies = Self::cookie_map(auth);
        crate::auth::youtube_hash::build_youtube_authorization(
            cookies.get("SAPISID").map(String::as_str),
            cookies.get("__Secure-1PAPISID").map(String::as_str),
            cookies.get("__Secure-3PAPISID").map(String::as_str),
            "https://music.youtube.com",
            "",
        )
        .ok_or_else(|| AppError::Api("No SAPISID for YouTube auth".into()))
    }

    /// 获取当前 YouTube Music 账号资料
    pub async fn get_account_profile(
        &self,
        auth: &crate::auth::state::YouTubeAuth,
    ) -> AppResult<YouTubeAccountProfile> {
        super::account::get_account_profile(&self.http, auth).await
    }

    /// YouTube Music 首页信息流（需登录）
    pub async fn get_home_feed(
        &self,
        auth: &crate::auth::state::YouTubeAuth,
    ) -> AppResult<Value> {
        let body = json!({
            "context": self.build_context(),
            "browseId": "FEmusic_home"
        });
        self.innertube_post_auth("browse", &body, auth).await
    }

    /// YouTube Music 用户音乐库歌单（需登录）
    pub async fn get_library_playlists(
        &self,
        auth: &crate::auth::state::YouTubeAuth,
    ) -> AppResult<Value> {
        let body = json!({
            "context": self.build_context(),
            "browseId": "FEmusic_liked_playlists"
        });
        self.innertube_post_auth("browse", &body, auth).await
    }

    /// YouTube Music 歌单详情（需登录）
    pub async fn get_playlist_detail(
        &self,
        browse_id: &str,
        auth: &crate::auth::state::YouTubeAuth,
    ) -> AppResult<Value> {
        let body = json!({
            "context": self.build_context(),
            "browseId": browse_id
        });
        self.innertube_post_auth("browse", &body, auth).await
    }

    /// 认证版 InnerTube POST, 并回收 Set-Cookie 用于会话保鲜
    /// 采用轻量 InnerTube + Cookie 方案, 不模拟完整浏览器环境, 避免与移动端会话互相挤掉登录
    async fn innertube_post_auth_with_session(
        &self,
        endpoint: &str,
        body: &Value,
        auth: &crate::auth::state::YouTubeAuth,
    ) -> AppResult<(Value, Option<crate::auth::state::YouTubeAuth>)> {
        if !auth.has_login() {
            return Err(AppError::Api("YouTube not logged in".into()));
        }
        let cookie_header = Self::build_cookie_header(&auth.cookies);
        let api_key = self.api_key.lock().clone();
        let url = format!("{}/{}?prettyPrint=false&key={}", INNERTUBE_URL, endpoint, api_key);
        let auth_header = Self::authorization_header(auth)?;

        let resp = self.http
            .post(&url)
            .header("User-Agent", USER_AGENT)
            .header("Content-Type", "application/json")
            .header("Origin", "https://music.youtube.com")
            .header("X-Origin", "https://music.youtube.com")
            .header("Referer", "https://music.youtube.com/")
            .header("X-YouTube-Client-Name", "67")
            .header("Authorization", auth_header)
            .header("X-Goog-AuthUser", "0")
            .header("Cookie", cookie_header)
            .json(body)
            .send()
            .await?
            .error_for_status()?;

        let set_cookie: Vec<String> = resp
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok().map(str::to_string))
            .collect();

        let data: Value = resp.json().await?;

        // 若响应携带新的身份 cookie, 合并回会话(保留旧身份, 避免被短暂响应冲掉)
        let updated = if set_cookie.is_empty() {
            None
        } else {
            let observed = super::session::parse_set_cookie_headers(&set_cookie);
            if observed.is_empty() {
                None
            } else {
                let merged = super::session::merge_youtube_auth_cookies(auth, &observed);
                if super::session::youtube_auth_changed(auth, &merged) {
                    Some(merged)
                } else {
                    None
                }
            }
        };

        Ok((data, updated))
    }

    /// 歌单详情(携带会话刷新)
    pub async fn get_playlist_detail_with_session(
        &self,
        browse_id: &str,
        auth: &crate::auth::state::YouTubeAuth,
    ) -> AppResult<(Value, Option<crate::auth::state::YouTubeAuth>)> {
        let body = json!({
            "context": self.build_context(),
            "browseId": browse_id
        });
        self.innertube_post_auth_with_session("browse", &body, auth).await
    }

    /// 歌单分页 continuation(携带会话刷新)
    pub async fn continue_playlist_with_session(
        &self,
        continuation: &str,
        auth: &crate::auth::state::YouTubeAuth,
    ) -> AppResult<(Value, Option<crate::auth::state::YouTubeAuth>)> {
        let body = json!({
            "context": self.build_context(),
            "continuation": continuation
        });
        self.innertube_post_auth_with_session("browse", &body, auth).await
    }
}
