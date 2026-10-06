// 网易云音乐 API 客户端
use reqwest::{
    cookie::{CookieStore, Jar},
    Client,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::error::{AppError, AppResult};
use super::crypto;
use crate::api::transport::{parse_json_response, FallbackHttp};

const BASE_URL: &str = "https://music.163.com";
const SONG_URL_PATH: &str = "/eapi/song/enhance/player/url/v1";
const EAPI_BASE_URL: &str = "https://interface.music.163.com";

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const EAPI_USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 10; NeriPlayer) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Mobile Safari/537.36";

pub struct NeteaseClient {
    http: FallbackHttp,
    /// 无共享 Jar 时的 csrf_token 回退值，歌词管理器等独立调用仍可使用
    csrf: String,
    /// AppState 的共享 Cookie Jar，首页预热后必须从这里重新读取 __csrf
    cookie_jar: Option<Arc<Jar>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeteaseSearchResult {
    pub id: u64,
    pub name: String,
    pub artists: Vec<String>,
    pub album: String,
    pub duration_ms: u64,
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeteaseSongUrl {
    pub url: Option<String>,
    pub br: u64,
    pub size: u64,
    pub r#type: String,
    pub is_preview: bool,
    pub unavailable_reason: Option<NeteasePlaybackUnavailableReason>,
    pub level: Option<String>,
    pub content_md5: Option<String>,
    pub duration_ms: Option<u64>,
    pub song_id: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NeteasePlaybackUnavailableReason {
    RequiresLogin,
    NoPermission,
    NoPlayUrl,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeteaseLyrics {
    pub lrc: Option<String>,
    pub tlyric: Option<String>,
    pub yrc: Option<String>,
    pub ytlrc: Option<String>,
    pub romalrc: Option<String>,
}

impl NeteaseClient {
    pub fn with_transport(http: crate::api::transport::FallbackHttp) -> Self {
        Self {
            http,
            csrf: String::new(),
            cookie_jar: None,
        }
    }

    pub fn new(http: &Client) -> Self {
        Self::with_transport(FallbackHttp::new(http, "netease"))
    }

    pub fn with_fallback(http: &Client, fallback: &Client) -> Self {
        Self::with_transport(FallbackHttp::with_fallback(http, fallback, "netease"))
    }

    /// 注入 WEAPI 使用的 csrf_token(取自 __csrf Cookie), 对齐 Android 行为
    pub fn with_csrf(mut self, csrf: String) -> Self {
        self.csrf = csrf;
        self
    }

    pub fn with_cookie_jar(mut self, cookie_jar: Arc<Jar>) -> Self {
        let url = BASE_URL.parse().expect("valid netease URL");
        let existing = cookie_jar.cookies(&url);
        let existing = existing
            .as_ref()
            .and_then(|header| header.to_str().ok())
            .unwrap_or("");
        let context = [
            ("os", "pc".to_string()),
            ("appver", "8.10.35".to_string()),
            ("__remember_me", "true".to_string()),
            ("NMTID", random_session_cookie()),
            ("_ntes_nuid", random_session_cookie()),
        ];
        for (name, value) in context {
            if !cookie_header_has_value(existing, name) {
                cookie_jar.add_cookie_str(
                    &format!("{name}={value}; Domain=music.163.com; Path=/"),
                    &url,
                );
            }
        }
        self.cookie_jar = Some(cookie_jar);
        self
    }

    fn has_login(&self) -> bool {
        let Some(jar) = self.cookie_jar.as_ref() else {
            return false;
        };
        let url = BASE_URL.parse().expect("valid netease URL");
        jar.cookies(&url)
            .and_then(|header| {
                header
                    .to_str()
                    .ok()
                    .map(|value| cookie_header_has_value(value, "MUSIC_U"))
            })
            .unwrap_or(false)
    }

    fn csrf_token(&self) -> String {
        match self.cookie_jar.as_ref() {
            Some(cookie_jar) => crate::auth::cookies::read_netease_csrf(cookie_jar),
            None => self.csrf.clone(),
        }
    }

    async fn send_with_fallback(
        &self,
        build: impl Fn(&Client) -> reqwest::RequestBuilder,
    ) -> AppResult<reqwest::Response> {
        Ok(self.http.send(build).await?)
    }

    /// 非幂等写请求专用：只在连接失败时兜底，避免超时重发造成重复提交
    async fn send_once_with_fallback(
        &self,
        build: impl Fn(&Client) -> reqwest::RequestBuilder,
    ) -> AppResult<reqwest::Response> {
        Ok(self.http.send_once(build).await?)
    }

    /// 访问一次站点首页，通常会下发 __csrf 等 Cookie，用于取流 code==301 后预热重试
    /// 对齐 Android NeteaseClient.ensureWeapiSession
    async fn ensure_weapi_session(&self) {
        let _ = self
            .send_with_fallback(|client| {
                client
                    .get(BASE_URL)
                    .header("User-Agent", USER_AGENT)
                    .header("Referer", "https://music.163.com")
            })
            .await;
    }

    /// WEAPI POST 请求（幂等读接口）
    async fn weapi_post(&self, url: &str, params: &Value) -> AppResult<Value> {
        self.weapi_request(url, params, true).await
    }

    /// WEAPI POST 写接口（非幂等）：只在连接失败时兜底，避免超时重发造成重复提交
    async fn weapi_post_write(&self, url: &str, params: &Value) -> AppResult<Value> {
        self.weapi_request(url, params, false).await
    }

    /// WEAPI POST 公共实现；`idempotent` 决定传输层是否允许超时重试
    async fn weapi_request(&self, url: &str, params: &Value, idempotent: bool) -> AppResult<Value> {
        if self.has_login() && self.csrf_token().is_empty() {
            self.ensure_weapi_session().await;
            if self.csrf_token().is_empty() {
                return Err(AppError::Api(
                    "NetEase session preheat did not provide __csrf".into(),
                ));
            }
        }
        let json_str = serde_json::to_string(params)?;
        let (encrypted_params, enc_sec_key) = crypto::weapi_encrypt(&json_str);
        let csrf = self.csrf_token();

        let build = |client: &Client| {
            client
                .post(url)
                .query(&[("csrf_token", csrf.as_str())])
                .header("User-Agent", USER_AGENT)
                .header("Referer", "https://music.163.com")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .form(&[("params", &encrypted_params), ("encSecKey", &enc_sec_key)])
        };

        let resp = if idempotent {
            self.send_with_fallback(build).await?
        } else {
            self.send_once_with_fallback(build).await?
        };

        parse_json_response(resp, "netease weapi").await
    }

    /// 搜索歌曲
    pub async fn search(&self, keyword: &str, limit: u32, offset: u32) -> AppResult<Vec<NeteaseSearchResult>> {
        let params = json!({
            "s": keyword,
            "type": "1",
            "limit": limit.to_string(),
            "offset": offset.to_string(),
            "total": "true"
        });

        let body = self.weapi_post(
            &format!("{}/weapi/cloudsearch/get/web", BASE_URL),
            &params,
        ).await?;

        let songs = body["result"]["songs"].as_array()
            .ok_or_else(|| AppError::Api("No search results".into()))?;

        let results = songs.iter().filter_map(|s| {
            Some(NeteaseSearchResult {
                id: s["id"].as_u64()?,
                name: s["name"].as_str()?.to_string(),
                artists: s["ar"].as_array()?
                    .iter()
                    .filter_map(|a| a["name"].as_str().map(String::from))
                    .collect(),
                album: s["al"]["name"].as_str().unwrap_or("").to_string(),
                duration_ms: s["dt"].as_u64().unwrap_or(0),
                cover_url: s["al"]["picUrl"].as_str().map(String::from),
            })
        }).collect();

        Ok(results)
    }

    /// 与 Android 使用同一 EAPI 取流入口
    pub async fn get_song_url(&self, song_id: u64, quality: &str) -> AppResult<NeteaseSongUrl> {
        let level = match quality {
            "standard" => "standard",
            "high" | "higher" => "higher",
            "exhigh" => "exhigh",
            "lossless" => "lossless",
            "hires" => "hires",
            "jyeffect" => "jyeffect",
            "sky" => "sky",
            "jymaster" => "jymaster",
            _ => "exhigh",
        };

        let params = json!({
            "ids": format!("[{}]", song_id),
            "level": level,
            "encodeType": "flac"
        });

        log::debug!(target: "netease", "get_song_url: id={}, level={}", song_id, level);

        let mut body = self.song_url_eapi_post(&params).await?;

        log::debug!(target: "netease", "song url response code: {:?}", body["code"]);

        // code==301 多见于登录态 __csrf 会话失效；预热首页后重试一次，对齐 Android getSongDownloadUrl
        if json_i64(&body["code"]) == Some(301) && self.has_login() {
            log::debug!(target: "netease", "song url code 301, warming session then retrying once");
            self.ensure_weapi_session().await;
            body = self.song_url_eapi_post(&params).await?;
            log::debug!(target: "netease", "song url retry response code: {:?}", body["code"]);
        }

        let result = parse_song_url_response(&body);
        log::debug!(target: "netease", "song url result: has_url={}, br={}", result.url.is_some(), result.br);
        Ok(result)
    }

    fn build_song_url_eapi_request(client: &Client, encrypted: &str) -> reqwest::RequestBuilder {
        client
            .post(format!("{EAPI_BASE_URL}{SONG_URL_PATH}"))
            .header("User-Agent", EAPI_USER_AGENT)
            .header("Referer", BASE_URL)
            .form(&[("params", encrypted)])
    }

    async fn song_url_eapi_post(&self, params: &Value) -> AppResult<Value> {
        let body = serde_json::to_string(params)?;
        let encrypted = crypto::eapi_encrypt(SONG_URL_PATH, &body);
        let response = self
            .send_with_fallback(|client| Self::build_song_url_eapi_request(client, &encrypted))
            .await?;
        parse_json_response(response, "netease eapi playback").await
    }

    /// 获取歌词（plain API，无需加密，最可靠）
    pub async fn get_lyrics(&self, song_id: u64) -> AppResult<NeteaseLyrics> {
        // 使用 v1 端点获取逐字歌词支持
        // tv/yv/ytv 显式请求翻译、逐字与翻译逐字字段；网易云在省略或传 0
        // 时可能省略 romalrc/ytlrc，导致音译链路只能偶发命中
        let url = format!("{}/api/song/lyric/v1?id={}&cp=false&lv=0&tv=1&rv=0&kv=0&yv=1&ytv=1&yrv=0",
            BASE_URL, song_id);

        log::debug!(target: "netease", "get_lyrics: id={}", song_id);

        let resp = self
            .send_with_fallback(|client| {
                client
                    .get(&url)
                    .header("User-Agent", USER_AGENT)
                    .header("Referer", "https://music.163.com")
            })
            .await?;

        let mut body: Value = parse_json_response(resp, "netease lyrics")
            .await
            .inspect_err(|error| {
                log::error!(target: "netease", "lyrics response invalid: {}", error);
            })?;

        // 登录态失效时接口会返回 301；预热首页让 __csrf/会话恢复后只重试一次，
        // 避免把临时鉴权过期误报成无歌词
        if body["code"].as_i64() == Some(301) {
            self.ensure_weapi_session().await;
            let retry_resp = self
                .send_with_fallback(|client| {
                    client
                        .get(&url)
                        .header("User-Agent", USER_AGENT)
                        .header("Referer", "https://music.163.com")
                })
                .await?;
            body = parse_json_response(retry_resp, "netease lyrics retry").await?;
        }

        let code = body["code"].as_i64().unwrap_or(-1);
        log::debug!(target: "netease", "lyrics response code={}, has_lrc={}, has_tlyric={}, has_yrc={}, has_romalrc={}",
            code,
            body["lrc"]["lyric"].is_string(),
            body["tlyric"]["lyric"].is_string(),
            body["yrc"]["lyric"].is_string(),
            body["romalrc"]["lyric"].is_string(),
        );

        if code != 200 {
            return Err(AppError::Api(format!("Lyrics API code: {}", code)));
        }

        Ok(NeteaseLyrics {
            lrc: body["lrc"]["lyric"].as_str().map(String::from),
            tlyric: body["tlyric"]["lyric"].as_str().map(String::from),
            yrc: body["yrc"]["lyric"].as_str().map(String::from),
            ytlrc: body["ytlrc"]["lyric"].as_str().map(String::from),
            romalrc: body["romalrc"]["lyric"].as_str().map(String::from),
        })
    }

    /// 获取歌曲详情
    pub async fn get_song_detail(&self, song_ids: &[u64]) -> AppResult<Value> {
        let c: Vec<Value> = song_ids.iter()
            .map(|id| json!({"id": id}))
            .collect();
        let params = json!({
            "c": serde_json::to_string(&c).unwrap_or_default(),
            "ids": serde_json::to_string(&song_ids).unwrap_or_default()
        });

        self.weapi_post(&format!("{}/weapi/v3/song/detail", BASE_URL), &params).await
    }

    /// 获取歌单详情
    pub async fn get_playlist(&self, playlist_id: u64) -> AppResult<Value> {
        let params = json!({
            "id": playlist_id.to_string(),
            "n": 100000,
            "s": 8
        });

        self.weapi_post(
            &format!("{}/weapi/v3/playlist/detail", BASE_URL),
            &params,
        ).await
    }

    // 需要登录的 API
    /// 获取当前登录用户信息
    pub async fn get_user_account(&self) -> AppResult<Value> {
        self.weapi_post(
            &format!("{}/weapi/w/nuser/account/get", BASE_URL),
            &json!({}),
        ).await
    }

    /// 获取用户歌单列表
    pub async fn get_user_playlists(&self, uid: u64, limit: u32, offset: u32) -> AppResult<Value> {
        self.weapi_post(
            &format!("{}/weapi/user/playlist", BASE_URL),
            &json!({
                "uid": uid.to_string(),
                "offset": offset.to_string(),
                "limit": limit.to_string(),
                "includeVideo": "true"
            }),
        ).await
    }

    /// 个性化推荐歌单（需登录）
    pub async fn get_recommended_playlists(&self, limit: u32) -> AppResult<Value> {
        self.weapi_post(
            &format!("{}/weapi/personalized/playlist", BASE_URL),
            &json!({ "limit": limit.to_string() }),
        ).await
    }

    /// 每日推荐歌曲（需登录）
    pub async fn get_recommended_songs(&self) -> AppResult<Value> {
        self.weapi_post(
            &format!("{}/weapi/v3/discovery/recommend/songs", BASE_URL),
            &json!({}),
        ).await
    }

    /// 精品歌单（按分类）
    pub async fn get_high_quality_playlists(&self, cat: &str, limit: u32) -> AppResult<Value> {
        self.weapi_post(
            &format!("{}/weapi/playlist/highquality/list", BASE_URL),
            &json!({
                "cat": cat,
                "limit": limit,
                "lasttime": 0,
                "total": true
            }),
        ).await
    }

    /// 用户喜欢的歌曲 ID 列表
    pub async fn get_liked_song_ids(&self, uid: u64) -> AppResult<Value> {
        self.weapi_post(
            &format!("{}/weapi/song/like/get", BASE_URL),
            &json!({ "uid": uid.to_string() }),
        ).await
    }

    /// 喜欢/取消喜欢歌曲
    pub async fn like_song(&self, song_id: u64, like: bool) -> AppResult<Value> {
        self.weapi_post_write(
            &format!("{}/weapi/song/like", BASE_URL),
            &json!({
                "trackId": song_id.to_string(),
                "like": like.to_string(),
                "alg": "itembased",
                "time": "3"
            }),
        ).await
    }

    /// 下载和在线播放使用同一个 EAPI 入口
    pub async fn get_song_download_url(&self, song_id: u64, quality: &str) -> AppResult<NeteaseSongUrl> {
        self.get_song_url(song_id, quality).await
    }

    /// 关注歌手列表使用 Android 相同的 WEAPI 分页入口
    pub async fn get_followed_artists(&self, offset: u32, limit: u32) -> AppResult<Value> {
        self.weapi_post(
            &format!("{}/weapi/artist/sublist", BASE_URL),
            &json!({ "offset": offset, "limit": limit.clamp(1, 100), "total": true }),
        ).await
    }

    /// 歌手头部信息 (对齐 Android getArtistDetail: /api/artist/head/info/get)
    pub async fn get_artist_detail(&self, artist_id: u64) -> AppResult<Value> {
        let resp = self
            .send_with_fallback(|client| {
                client
                    .post(format!("{}/api/artist/head/info/get", BASE_URL))
                    .header("User-Agent", USER_AGENT)
                    .header("Referer", "https://music.163.com")
                    .form(&[("id", artist_id.to_string())])
            })
            .await?;
        parse_json_response(resp, "netease artist detail").await
    }

    /// 歌手专辑列表 (对齐 Android getArtistAlbums: /api/artist/albums/{id})
    pub async fn get_artist_albums(&self, artist_id: u64, offset: u32, limit: u32) -> AppResult<Value> {
        let resp = self
            .send_with_fallback(|client| {
                client
                    .post(format!("{}/api/artist/albums/{}", BASE_URL, artist_id))
                    .header("User-Agent", USER_AGENT)
                    .header("Referer", "https://music.163.com")
                    .form(&[
                        ("limit", limit.to_string()),
                        ("offset", offset.to_string()),
                        ("total", "true".into()),
                    ])
            })
            .await?;
        parse_json_response(resp, "netease artist albums").await
    }

    /// 歌手全部歌曲 (对齐 Android getArtistSongs: /api/v1/artist/songs)
    pub async fn get_artist_songs(
        &self,
        artist_id: u64,
        order: &str,
        offset: u32,
        limit: u32,
    ) -> AppResult<Value> {
        let resp = self
            .send_with_fallback(|client| {
                client
                    .post(format!("{}/api/v1/artist/songs", BASE_URL))
                    .header("User-Agent", USER_AGENT)
                    .header("Referer", "https://music.163.com")
                    .form(&[
                        ("id", artist_id.to_string()),
                        ("private_cloud", "true".into()),
                        ("work_type", "1".into()),
                        ("order", order.to_string()),
                        ("offset", offset.to_string()),
                        ("limit", limit.to_string()),
                    ])
            })
            .await?;
        parse_json_response(resp, "netease artist songs").await
    }

    /// 获取专辑详情
    pub async fn get_album_detail(&self, album_id: u64) -> AppResult<Value> {
        let resp = self
            .send_with_fallback(|client| {
                client
                    .get(format!("{}/api/v1/album/{}", BASE_URL, album_id))
                    .header("User-Agent", USER_AGENT)
                    .header("Referer", "https://music.163.com")
            })
            .await?;
        parse_json_response(resp, "netease album detail").await
    }

    /// 获取精品歌单分类标签
    pub async fn get_high_quality_tags(&self) -> AppResult<Value> {
        self.weapi_post(
            &format!("{}/weapi/playlist/highquality/tags", BASE_URL),
            &json!({}),
        ).await
    }

    /// 获取用户收藏的专辑列表
    pub async fn get_user_stared_albums(&self, offset: u32, limit: u32) -> AppResult<Value> {
        let params = json!({
            "offset": offset.to_string(),
            "limit": limit.to_string(),
            "total": "true",
            "csrf_token": ""
        });
        self.weapi_post(
            &format!("{}/weapi/album/sublist", BASE_URL),
            &params,
        ).await
    }
}

fn parse_song_url_response(body: &Value) -> NeteaseSongUrl {
    let root_code = json_i64(&body["code"]).unwrap_or(-1);
    if root_code == 301 {
        return unavailable_song_url(NeteasePlaybackUnavailableReason::RequiresLogin);
    }
    if root_code != 200 {
        return unavailable_song_url(NeteasePlaybackUnavailableReason::Unknown);
    }

    let data = match &body["data"] {
        Value::Array(values) => values.first(),
        Value::Object(_) => Some(&body["data"]),
        _ => None,
    };
    let Some(data) = data else {
        return unavailable_song_url(NeteasePlaybackUnavailableReason::NoPlayUrl);
    };

    let url = clean_json_string(&data["url"]);
    let unavailable_reason = if url.is_none() {
        let data_code = json_i64(&data["code"]).unwrap_or(-1);
        let cannot_listen_reason = data["freeTrialPrivilege"]["cannotListenReason"]
            .as_i64()
            .or_else(|| {
                data["freeTrialPrivilege"]["cannotListenReason"]
                    .as_str()
                    .and_then(|value| value.parse::<i64>().ok())
            });
        let fee = json_i64(&data["fee"]).unwrap_or(0);
        Some(
            if data_code == 404 || cannot_listen_reason == Some(1) || fee > 0 {
                NeteasePlaybackUnavailableReason::NoPermission
            } else {
                NeteasePlaybackUnavailableReason::NoPlayUrl
            },
        )
    } else {
        None
    };

    NeteaseSongUrl {
        url,
        br: ["br", "bitrate", "bitrateKbps"]
            .iter()
            .map(|field| json_u64(&data[*field]))
            .find(|value| *value > 0)
            .map(|value| if value < 10_000 { value * 1_000 } else { value })
            .unwrap_or(0),
        size: json_u64(&data["size"]),
        r#type: clean_json_string(&data["type"]).unwrap_or_else(|| "mp3".into()),
        is_preview: data
            .get("freeTrialInfo")
            .is_some_and(|value| !value.is_null()),
        unavailable_reason,
        level: clean_json_string(&data["level"]),
        content_md5: clean_json_string(&data["md5"])
            .filter(|value| value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .map(|value| value.to_lowercase()),
        duration_ms: positive_json_u64(&data["time"]),
        song_id: positive_json_u64(&data["id"]),
    }
}

fn unavailable_song_url(reason: NeteasePlaybackUnavailableReason) -> NeteaseSongUrl {
    NeteaseSongUrl {
        url: None,
        br: 0,
        size: 0,
        r#type: "mp3".into(),
        is_preview: false,
        unavailable_reason: Some(reason),
        level: None,
        content_md5: None,
        duration_ms: None,
        song_id: None,
    }
}

fn clean_json_string(value: &Value) -> Option<String> {
    let value = value.as_str()?.trim();
    (!value.is_empty() && !value.eq_ignore_ascii_case("null")).then(|| value.to_string())
}

fn json_u64(value: &Value) -> u64 {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.parse::<u64>().ok()))
        .unwrap_or(0)
}

fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
}

fn positive_json_u64(value: &Value) -> Option<u64> {
    let value = json_u64(value);
    (value > 0).then_some(value)
}

fn random_session_cookie() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn cookie_header_has_value(header: &str, name: &str) -> bool {
    header
        .split(';')
        .filter_map(|item| item.trim().split_once('='))
        .any(|(key, value)| key == name && !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::{parse_song_url_response, NeteaseClient, NeteasePlaybackUnavailableReason};
    use crate::api::transport::FallbackHttp;
    use reqwest::cookie::Jar;
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn weapi_csrf_uses_latest_value_from_shared_cookie_jar() {
        let jar = Arc::new(Jar::default());
        let http = reqwest::Client::builder()
            .cookie_provider(jar.clone())
            .build()
            .unwrap();
        let url = "https://music.163.com/".parse().unwrap();
        jar.add_cookie_str("__csrf=before; Domain=music.163.com; Path=/", &url);
        let client = NeteaseClient::with_transport(FallbackHttp::new(&http, "test"))
            .with_csrf("fallback".into())
            .with_cookie_jar(jar.clone());

        assert_eq!(client.csrf_token(), "before");
        jar.add_cookie_str("__csrf=after; Domain=music.163.com; Path=/", &url);
        assert_eq!(client.csrf_token(), "after");
    }

    #[test]
    fn playback_response_marks_explicit_free_trial_as_preview() {
        let result = parse_song_url_response(&json!({
            "code": 200,
            "data": [{
                "url": "https://m801.music.126.net/demo.mp3",
                "br": 320000,
                "size": "1234567",
                "type": "mp3",
                "freeTrialInfo": { "fragmentType": 6 }
            }]
        }));

        assert!(result.is_preview);
        assert_eq!(result.size, 1_234_567);
        assert_eq!(result.unavailable_reason, None);
    }

    #[test]
    fn playback_response_does_not_infer_preview_without_free_trial_info() {
        let result = parse_song_url_response(&json!({
            "code": 200,
            "data": [{
                "url": "https://m801.music.126.net/full.flac",
                "freeTrialInfo": null
            }]
        }));

        assert!(!result.is_preview);
    }

    #[test]
    fn playback_response_classifies_null_url_permission_failure() {
        let result = parse_song_url_response(&json!({
            "code": 200,
            "data": [{
                "url": null,
                "code": 404,
                "fee": 0,
                "freeTrialPrivilege": { "cannotListenReason": 1 }
            }]
        }));

        assert_eq!(result.url, None);
        assert_eq!(
            result.unavailable_reason,
            Some(NeteasePlaybackUnavailableReason::NoPermission)
        );
    }

    #[test]
    fn playback_response_classifies_login_requirement() {
        let result = parse_song_url_response(&json!({ "code": 301 }));

        assert_eq!(
            result.unavailable_reason,
            Some(NeteasePlaybackUnavailableReason::RequiresLogin)
        );
    }

    #[test]
    fn playback_response_rejects_literal_null_url() {
        let result = parse_song_url_response(&json!({
            "code": 200,
            "data": [{ "url": "null" }]
        }));

        assert_eq!(result.url, None);
        assert_eq!(
            result.unavailable_reason,
            Some(NeteasePlaybackUnavailableReason::NoPlayUrl)
        );
    }

    #[test]
    fn android_alignment_playback_accepts_numeric_strings_and_bitrate_aliases() {
        let result = parse_song_url_response(&json!({
            "code": "200",
            "data": { "url": "https://m801.music.126.net/full.flac", "bitrateKbps": "192" }
        }));
        assert_eq!(
            result.url.as_deref(),
            Some("https://m801.music.126.net/full.flac")
        );
        assert_eq!(result.br, 192_000);
    }

    #[test]
    fn android_alignment_playback_classifies_numeric_string_fee() {
        let result = parse_song_url_response(&json!({
            "code": 200,
            "data": { "url": null, "fee": "1" }
        }));
        assert_eq!(
            result.unavailable_reason,
            Some(NeteasePlaybackUnavailableReason::NoPermission)
        );
    }

    #[test]
    fn android_alignment_eapi_request_matches_playback_endpoint_and_encryption() {
        let params = json!({ "ids": "[42]", "level": "hires", "encodeType": "flac" });
        let encrypted = super::crypto::eapi_encrypt(super::SONG_URL_PATH, &params.to_string());
        let request =
            NeteaseClient::build_song_url_eapi_request(&reqwest::Client::new(), &encrypted)
                .build()
                .unwrap();
        assert_eq!(
            request.url().as_str(),
            "https://interface.music.163.com/eapi/song/enhance/player/url/v1"
        );
        assert_eq!(request.method(), reqwest::Method::POST);
        let form = request.body().unwrap().as_bytes().unwrap();
        let values: std::collections::HashMap<_, _> =
            url::form_urlencoded::parse(form).into_owned().collect();
        assert_eq!(values.len(), 1);
        let decrypted =
            super::crypto::eapi_decrypt(&hex::decode(&values["params"]).unwrap()).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&decrypted).unwrap(),
            json!({ "ids": "[42]", "level": "hires", "encodeType": "flac" })
        );
    }

    #[test]
    fn android_alignment_response_preserves_actual_quality_and_cache_integrity() {
        let result = parse_song_url_response(&json!({
            "code": 200,
            "data": [{ "url": "https://m801.music.126.net/full.flac", "level": "lossless", "br": "1411000", "size": "7000000", "id": "42", "time": "180000", "md5": "0123456789ABCDEF0123456789ABCDEF" }]
        }));
        assert_eq!(result.level.as_deref(), Some("lossless"));
        assert_eq!(result.br, 1_411_000);
        assert_eq!(result.song_id, Some(42));
        assert_eq!(result.duration_ms, Some(180_000));
        assert_eq!(
            result.content_md5.as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        let invalid = parse_song_url_response(
            &json!({ "code": 200, "data": [{ "url": "https://m801.music.126.net/full.flac", "md5": "invalid", "id": 0, "time": -1 }] }),
        );
        assert_eq!(invalid.content_md5, None);
        assert_eq!(invalid.duration_ms, None);
        assert_eq!(invalid.song_id, None);
    }

    #[test]
    fn android_alignment_cookie_context_preserves_login_and_is_stable() {
        use reqwest::cookie::CookieStore;
        let jar = Arc::new(Jar::default());
        let url = "https://music.163.com/".parse().unwrap();
        jar.add_cookie_str("MUSIC_U=fixture-login; Domain=music.163.com; Path=/", &url);
        jar.add_cookie_str("appver=fixture-version; Domain=music.163.com; Path=/", &url);
        let http = reqwest::Client::builder()
            .cookie_provider(jar.clone())
            .build()
            .unwrap();
        let client = NeteaseClient::new(&http).with_cookie_jar(jar.clone());
        assert!(client.has_login());
        let before = jar.cookies(&url).unwrap().to_str().unwrap().to_string();
        NeteaseClient::new(&http).with_cookie_jar(jar.clone());
        let after = jar.cookies(&url).unwrap().to_str().unwrap().to_string();
        assert_eq!(before, after);
        assert!(after.contains("MUSIC_U=fixture-login"));
        assert!(after.contains("appver=fixture-version"));
        assert!(after.contains("os=pc"));
        assert!(after.contains("NMTID="));
        let eapi_url = "https://interface.music.163.com/".parse().unwrap();
        assert!(jar
            .cookies(&eapi_url)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("MUSIC_U=fixture-login"));
    }
}
