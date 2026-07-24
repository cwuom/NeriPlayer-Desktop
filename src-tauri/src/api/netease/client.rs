// 网易云音乐 API 客户端
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use super::crypto;

const BASE_URL: &str = "https://music.163.com";

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

pub struct NeteaseClient {
    http: Client,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NeteaseAccountProfile {
    pub(crate) user_id: u64,
    pub(crate) nickname: Option<String>,
    pub(crate) avatar_url: Option<String>,
}

pub(crate) fn parse_netease_account_profile(body: &Value) -> AppResult<NeteaseAccountProfile> {
    let code = body["code"]
        .as_i64()
        .or_else(|| body["code"].as_str().and_then(|value| value.parse().ok()))
        .unwrap_or(-1);
    if code != 200 {
        return Err(AppError::Api(format!(
            "Netease account API returned code {code}"
        )));
    }

    let profile = body["profile"]
        .as_object()
        .ok_or_else(|| AppError::Api("Netease account response has no profile".into()))?;
    let user_id = profile
        .get("userId")
        .map(json_u64)
        .filter(|user_id| *user_id > 0)
        .ok_or_else(|| {
            AppError::Api("Netease account response has no valid profile.userId".into())
        })?;

    Ok(NeteaseAccountProfile {
        user_id,
        nickname: profile
            .get("nickname")
            .and_then(Value::as_str)
            .map(String::from),
        avatar_url: profile
            .get("avatarUrl")
            .and_then(Value::as_str)
            .map(String::from),
    })
}

impl NeteaseClient {
    pub fn new(http: &Client) -> Self {
        Self { http: http.clone() }
    }

    /// WEAPI POST 请求
    async fn weapi_post(&self, url: &str, params: &Value) -> AppResult<Value> {
        let json_str = serde_json::to_string(params)?;
        let (encrypted_params, enc_sec_key) = crypto::weapi_encrypt(&json_str);

        let resp = self.http
            .post(url)
            .header("User-Agent", USER_AGENT)
            .header("Referer", "https://music.163.com")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[("params", &encrypted_params), ("encSecKey", &enc_sec_key)])
            .send()
            .await?;

        let body: Value = resp.json().await?;
        Ok(body)
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

    /// 获取歌曲播放 URL（WEAPI，稳定可靠）
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
            "encodeType": "flac",
            "csrf_token": ""
        });

        log::debug!(target: "netease", "get_song_url: id={}, level={}", song_id, level);

        let body = self.weapi_post(
            &format!("{}/weapi/song/enhance/player/url/v1", BASE_URL),
            &params,
        ).await?;

        log::debug!(target: "netease", "song url response code: {:?}", body["code"]);

        let result = parse_song_url_response(&body);
        log::debug!(target: "netease", "song url result: url={}, br={}",
            result.url.as_deref().unwrap_or("null"), result.br);
        Ok(result)
    }

    /// 获取歌词（plain API，无需加密，最可靠）
    pub async fn get_lyrics(&self, song_id: u64) -> AppResult<NeteaseLyrics> {
        // 使用 v1 端点获取逐字歌词支持
        let url = format!("{}/api/song/lyric/v1?id={}&cp=false&lv=0&tv=0&rv=0&kv=0&yv=0&ytv=0&yrv=0",
            BASE_URL, song_id);

        log::debug!(target: "netease", "get_lyrics: id={}", song_id);

        let resp = self.http
            .get(&url)
            .header("User-Agent", USER_AGENT)
            .header("Referer", "https://music.163.com")
            .send()
            .await?;

        let body: Value = resp.json().await
            .map_err(|e| {
                log::error!(target: "netease", "lyrics JSON parse failed: {}", e);
                AppError::Api(format!("Lyrics parse error: {}", e))
            })?;

        let code = body["code"].as_i64().unwrap_or(-1);
        log::debug!(target: "netease", "lyrics response code={}, has_lrc={}, has_tlyric={}, has_yrc={}",
            code,
            body["lrc"]["lyric"].is_string(),
            body["tlyric"]["lyric"].is_string(),
            body["yrc"]["lyric"].is_string(),
        );

        if code != 200 {
            return Err(AppError::Api(format!("Lyrics API code: {}", code)));
        }

        Ok(NeteaseLyrics {
            lrc: body["lrc"]["lyric"].as_str().map(String::from),
            tlyric: body["tlyric"]["lyric"].as_str().map(String::from),
            yrc: body["yrc"]["lyric"].as_str().map(String::from),
            ytlrc: body["ytlrc"]["lyric"].as_str().map(String::from),
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
        let body = self.weapi_post(
            &format!("{}/weapi/w/nuser/account/get", BASE_URL),
            &json!({}),
        ).await?;
        parse_netease_account_profile(&body)?;
        Ok(body)
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
        self.weapi_post(
            &format!("{}/weapi/radio/like", BASE_URL),
            &json!({
                "trackId": song_id.to_string(),
                "like": like.to_string(),
                "alg": "itembased",
                "time": "3"
            }),
        ).await
    }

    /// 获取歌曲下载 URL（WEAPI）
    pub async fn get_song_download_url(&self, song_id: u64, quality: &str) -> AppResult<NeteaseSongUrl> {
        let br = match quality {
            "standard" => 128000,
            "high" | "higher" => 192000,
            "exhigh" => 320000,
            "lossless" => 999000,
            "hires" => 1999000,
            _ => 320000,
        };

        let params = json!({
            "id": song_id.to_string(),
            "br": br.to_string(),
            "csrf_token": ""
        });

        let body = self.weapi_post(
            &format!("{}/weapi/song/enhance/download/url", BASE_URL),
            &params,
        ).await?;
        parse_download_url_response(&body)
    }

    /// 获取专辑详情
    pub async fn get_album_detail(&self, album_id: u64) -> AppResult<Value> {
        let resp = self.http
            .get(format!("{}/api/v1/album/{}", BASE_URL, album_id))
            .header("User-Agent", USER_AGENT)
            .header("Referer", "https://music.163.com")
            .send()
            .await?;
        let body: Value = resp.json().await?;
        Ok(body)
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

fn parse_download_url_response(body: &Value) -> AppResult<NeteaseSongUrl> {
    let root_code = json_i64(&body["code"]).unwrap_or(-1);
    if root_code != 200 {
        let reason = if root_code == 301 {
            NeteasePlaybackUnavailableReason::RequiresLogin
        } else {
            NeteasePlaybackUnavailableReason::Unknown
        };
        return Err(AppError::Api(format!(
            "Netease download API returned code {root_code} ({reason:?})"
        )));
    }

    let data = match &body["data"] {
        Value::Array(values) => values.first(),
        Value::Object(_) => Some(&body["data"]),
        _ => None,
    }
    .ok_or_else(|| AppError::Api("Netease download response has no data".into()))?;

    if let Some(data_code) = data.get("code").and_then(json_i64) {
        if data_code != 200 {
            let reason = if data_code == 404 {
                NeteasePlaybackUnavailableReason::NoPermission
            } else {
                NeteasePlaybackUnavailableReason::Unknown
            };
            return Err(AppError::Api(format!(
                "Netease download data returned code {data_code} ({reason:?})"
            )));
        }
    }

    let is_preview = data
        .get("freeTrialInfo")
        .is_some_and(|value| !value.is_null());
    let url = clean_json_string(&data["url"]);
    let br = json_u64(&data["br"]);
    let size = json_u64(&data["size"]);
    let r#type = clean_json_string(&data["type"]).unwrap_or_else(|| "mp3".into());

    if is_preview {
        return Ok(NeteaseSongUrl {
            url: None,
            br,
            size,
            r#type,
            is_preview: true,
            unavailable_reason: Some(NeteasePlaybackUnavailableReason::NoPermission),
        });
    }

    Ok(NeteaseSongUrl {
        unavailable_reason: url
            .is_none()
            .then_some(NeteasePlaybackUnavailableReason::NoPlayUrl),
        url,
        br,
        size,
        r#type,
        is_preview: false,
    })
}

fn parse_song_url_response(body: &Value) -> NeteaseSongUrl {
    let root_code = body["code"].as_i64().unwrap_or(-1);
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
        let data_code = data["code"].as_i64().unwrap_or(-1);
        let cannot_listen_reason = data["freeTrialPrivilege"]["cannotListenReason"]
            .as_i64()
            .or_else(|| {
                data["freeTrialPrivilege"]["cannotListenReason"]
                    .as_str()
                    .and_then(|value| value.parse::<i64>().ok())
            });
        let fee = data["fee"].as_i64().unwrap_or(0);
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
        br: json_u64(&data["br"]),
        size: json_u64(&data["size"]),
        r#type: clean_json_string(&data["type"]).unwrap_or_else(|| "mp3".into()),
        is_preview: data
            .get("freeTrialInfo")
            .is_some_and(|value| !value.is_null()),
        unavailable_reason,
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
        .or_else(|| value.as_str().and_then(|value| value.parse::<i64>().ok()))
}

#[cfg(test)]
mod tests {
    use super::{
        parse_download_url_response, parse_song_url_response,
        NeteasePlaybackUnavailableReason,
    };
    use serde_json::json;

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
    fn download_response_rejects_root_business_failures() {
        for code in [301, 500] {
            let error = parse_download_url_response(&json!({ "code": code }))
                .expect_err("business failures must not become download URLs");
            assert!(error.to_string().contains(&code.to_string()));
        }
    }

    #[test]
    fn download_response_rejects_data_permission_failure() {
        let error = parse_download_url_response(&json!({
            "code": 200,
            "data": {
                "code": 404,
                "url": null,
            },
        }))
        .expect_err("data code 404 must not become a download URL");

        assert!(error.to_string().contains("404"));
    }

    #[test]
    fn download_response_never_exposes_free_trial_url() {
        let result = parse_download_url_response(&json!({
            "code": 200,
            "data": {
                "url": "https://m801.music.126.net/preview.mp3",
                "br": 128000,
                "size": "1234",
                "type": "mp3",
                "freeTrialInfo": { "fragmentType": 6 },
            },
        }))
        .expect("preview response should be represented, not exposed");

        assert_eq!(result.url, None);
        assert!(result.is_preview);
        assert_eq!(result.unavailable_reason, Some(NeteasePlaybackUnavailableReason::NoPermission));
    }
}
