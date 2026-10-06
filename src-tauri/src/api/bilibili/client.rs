// B站 API 客户端
use reqwest::{
    cookie::{CookieStore, Jar},
    Client,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as TokioMutex;

use crate::error::{AppError, AppResult};
use crate::api::transport::{parse_json_response, FallbackHttp};
use super::wbi;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const MIXIN_KEY_TTL: Duration = Duration::from_secs(10 * 60);

type MixinKeyCache = TokioMutex<Option<(String, Instant)>>;
type AnonymousCookieCache = TokioMutex<Option<(BTreeMap<String, String>, Instant)>>;

fn mixin_key_cache() -> &'static MixinKeyCache {
    static CACHE: OnceLock<MixinKeyCache> = OnceLock::new();
    CACHE.get_or_init(|| TokioMutex::new(None))
}

pub struct BiliClient {
    http: FallbackHttp,
    cookie_jar: Option<Arc<Jar>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiliAudioStream {
    pub url: String,
    pub bandwidth: u64,
    pub codecs: String,
    pub quality_id: u32,
    pub mime_type: String,
    pub quality_tag: Option<String>,
    pub candidate_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiliVideoInfo {
    pub bvid: String,
    pub title: String,
    pub owner: String,
    pub cover: String,
    pub cid: u64,
    pub duration: u64,
    pub pages: Vec<BiliVideoPage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiliVideoPage {
    pub cid: u64,
    pub page: u64,
    pub part: String,
    pub duration: u64,
}

impl BiliClient {
    pub fn new(http: &Client) -> Self {
        Self::with_transport(FallbackHttp::new(http, "bilibili"))
    }

    pub fn with_transport(http: FallbackHttp) -> Self {
        Self {
            http,
            cookie_jar: None,
        }
    }

    pub fn with_cookie_jar(mut self, cookie_jar: Arc<Jar>) -> Self {
        self.cookie_jar = Some(cookie_jar);
        self
    }

    async fn ensure_anonymous_session(&self) -> AppResult<()> {
        let Some(jar) = self.cookie_jar.as_ref() else {
            return Ok(());
        };
        if has_bili_identity(jar) {
            return Ok(());
        }
        static CACHE: OnceLock<AnonymousCookieCache> = OnceLock::new();
        let mut cache = CACHE.get_or_init(|| TokioMutex::new(None)).lock().await;
        if cache
            .as_ref()
            .is_none_or(|(_, expires)| *expires <= Instant::now())
        {
            let response = self.http.send(|client| client.get("https://api.bilibili.com/x/frontend/finger/spi")
                .header("User-Agent", "Mozilla/5.0 (iPhone; CPU iPhone OS 13_2_3 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/13.0.3 Mobile/15E148 Safari/604.1 Edg/114.0.0.0")).await?;
            let body: Value = parse_json_response(response, "bilibili fingerprint").await?;
            *cache = Some((
                parse_anonymous_cookies(&body),
                Instant::now() + Duration::from_secs(3600),
            ));
        }
        // 等待指纹请求时用户可能已登录，不能覆盖新的会话
        if !has_bili_identity(jar) {
            if let Some((cookies, expires)) = cache.as_ref() {
                let max_age = expires.saturating_duration_since(Instant::now()).as_secs();
                inject_anonymous_cookies(jar, cookies, max_age);
            }
        }
        Ok(())
    }

    /// 获取或刷新 mixin_key
    async fn ensure_mixin_key(&self) -> AppResult<String> {
        // BiliClient 按命令临时构造，缓存必须放在客户端之外；用异步锁
        // 合并并发冷启动，避免收藏夹分页同时触发多次 nav（AP-02）
        let mut cache = mixin_key_cache().lock().await;
        if let Some((key, expires_at)) = cache.as_ref() {
            if *expires_at > Instant::now() {
                return Ok(key.clone());
            }
        }

        self.ensure_anonymous_session().await?;
        let nav: AppResult<String> = async {
            let response = self
                .http
                .send(|client| {
                    client
                        .get("https://api.bilibili.com/x/web-interface/nav")
                        .header("User-Agent", USER_AGENT)
                        .header("Referer", "https://www.bilibili.com")
                })
                .await?;
            let body: Value = parse_json_response(response, "bilibili nav").await?;
            parse_mixin_key(&body["data"]["wbi_img"], "img_url", "sub_url")
        }
        .await;
        let key = match nav {
            Ok(key) => key,
            Err(_) => self.fetch_ticket_mixin_key().await?,
        };
        *cache = Some((key.clone(), Instant::now() + MIXIN_KEY_TTL));
        Ok(key)
    }

    async fn fetch_ticket_mixin_key(&self) -> AppResult<String> {
        let ts = chrono::Utc::now().timestamp().to_string();
        let mut params = BTreeMap::from([
            ("key_id".to_string(), "ec02".to_string()),
            ("hexsign".to_string(), web_ticket_signature(&ts)),
            ("context[ts]".to_string(), ts),
        ]);
        if let Some(csrf) = self
            .cookie_jar
            .as_ref()
            .and_then(|jar| bili_cookie_value(jar, "bili_jct"))
        {
            params.insert("csrf".into(), csrf);
        }
        let response = self
            .http
            .send(|client| {
                client
                    .post(
                        "https://api.bilibili.com/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket",
                    )
                    .query(&params)
                    .header(
                        "User-Agent",
                        "Mozilla/5.0 (X11; Linux x86_64; rv:109.0) Gecko/20100101 Firefox/115.0",
                    )
                    .body(Vec::<u8>::new())
            })
            .await?;
        let body: Value = parse_json_response(response, "bilibili ticket").await?;
        parse_mixin_key(&body["data"]["nav"], "img", "sub")
    }

    async fn api_get(&self, url: &str, params: &BTreeMap<String, String>) -> AppResult<Value> {
        self.ensure_anonymous_session().await?;
        let response = self
            .http
            .send(|client| {
                client
                    .get(url)
                    .query(params)
                    .header("User-Agent", USER_AGENT)
                    .header("Referer", "https://www.bilibili.com")
            })
            .await?;
        let body: Value = parse_json_response(response, "bilibili api").await?;
        validate_bili_response(body)
    }

    /// 带 Wbi 签名的 GET 请求
    async fn wbi_get(&self, url: &str, mut params: BTreeMap<String, String>) -> AppResult<Value> {
        self.ensure_anonymous_session().await?;
        let mixin_key = self.ensure_mixin_key().await?;
        wbi::sign_params(&mut params, &mixin_key);

        let query: String = params
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        let full_url = format!("{}?{}", url, query);
        let resp: Value = self
            .http
            .send(|client| {
                client
                    .get(&full_url)
                    .header("User-Agent", USER_AGENT)
                    .header("Referer", "https://www.bilibili.com")
            })
            .await
            .map(|response| parse_json_response::<Value>(response, "bilibili wbi"))?
            .await?;

        validate_bili_response(resp)
    }

    pub async fn get_uploader_profile(&self, mid: u64) -> AppResult<Value> {
        self.wbi_get("https://api.bilibili.com/x/space/wbi/acc/info", BTreeMap::from([
            ("mid".into(), mid.to_string()),
            ("platform".into(), "web".into()),
            ("web_location".into(), "1550101".into()),
        ])).await
    }

    pub async fn get_uploader_videos(&self, mid: u64, page: u32) -> AppResult<Value> {
        self.wbi_get("https://api.bilibili.com/x/space/wbi/arc/search", BTreeMap::from([
            ("mid".into(), mid.to_string()),
            ("pn".into(), page.max(1).to_string()),
            ("ps".into(), "30".into()),
            ("order".into(), "pubdate".into()),
        ])).await
    }

    pub async fn get_uploader_contents(&self, mid: u64, page: u32) -> AppResult<Value> {
        self.wbi_get("https://api.bilibili.com/x/polymer/web-space/seasons_series_list", BTreeMap::from([
            ("mid".into(), mid.to_string()),
            ("page_num".into(), page.max(1).to_string()),
            ("page_size".into(), "20".into()),
            ("web_location".into(), "333.999".into()),
        ])).await
    }

    pub async fn get_uploader_collection(&self, mid: u64, content_id: u64, kind: &str, page: u32) -> AppResult<Value> {
        match kind {
            "collection" => self.wbi_get("https://api.bilibili.com/x/polymer/web-space/seasons_archives_list", BTreeMap::from([
                ("mid".into(), mid.to_string()), ("season_id".into(), content_id.to_string()),
                ("page_num".into(), page.max(1).to_string()), ("page_size".into(), "30".into()),
                ("sort_reverse".into(), "false".into()), ("web_location".into(), "333.999".into()),
            ])).await,
            "series" => self.api_get("https://api.bilibili.com/x/series/archives", &BTreeMap::from([
                ("mid".into(), mid.to_string()), ("series_id".into(), content_id.to_string()),
                ("pn".into(), page.max(1).to_string()), ("ps".into(), "30".into()),
                ("only_normal".into(), "true".into()), ("sort".into(), "desc".into()),
            ])).await,
            _ => Err(AppError::Other("Unsupported uploader content type".into())),
        }
    }

    /// 获取视频信息
    pub async fn get_video_info(&self, bvid: &str) -> AppResult<BiliVideoInfo> {
        let mut params = BTreeMap::new();
        params.insert("bvid".into(), bvid.into());

        let resp = self.wbi_get("https://api.bilibili.com/x/web-interface/wbi/view", params).await?;
        let data = &resp["data"];

        Ok(BiliVideoInfo {
            bvid: bvid.to_string(),
            title: data["title"].as_str().unwrap_or("").to_string(),
            owner: data["owner"]["name"].as_str().unwrap_or("").to_string(),
            cover: data["pic"].as_str().unwrap_or("").to_string(),
            cid: data["cid"].as_u64().unwrap_or(0),
            duration: data["duration"].as_u64().unwrap_or(0),
            pages: parse_video_pages(data),
        })
    }

    /// 获取音频流 URL（DASH 模式）
    pub async fn get_audio_url(&self, bvid: &str, cid: u64) -> AppResult<Vec<BiliAudioStream>> {
        fetch_audio_streams(|html5| async move {
            let params = playback_params(bvid, cid, html5);
            self.wbi_get("https://api.bilibili.com/x/player/wbi/playurl", params).await
        }).await
    }

    /// 搜索视频
    pub async fn search(&self, keyword: &str) -> AppResult<Value> {
        self.search_with_duration(keyword, 0).await
    }

    pub async fn search_with_duration(&self, keyword: &str, duration: u8) -> AppResult<Value> {
        let mut params = BTreeMap::new();
        params.insert("search_type".into(), "video".into());
        params.insert("keyword".into(), keyword.into());
        params.insert("page".into(), "1".into());
        params.insert("duration".into(), duration.to_string());

        self.wbi_get("https://api.bilibili.com/x/web-interface/wbi/search/type", params).await
    }

    // 需要登录的 API
    /// 获取登录用户信息（也用于 Wbi key 刷新）
    pub async fn get_user_info(&self) -> AppResult<Value> {
        self.ensure_anonymous_session().await?;
        let resp = self
            .http
            .send(|client| {
                client
                    .get("https://api.bilibili.com/x/web-interface/nav")
                    .header("User-Agent", USER_AGENT)
                    .header("Referer", "https://www.bilibili.com")
            })
            .await?;
        parse_json_response(resp, "bilibili nav").await
    }

    /// 获取用户创建的收藏夹列表（分页版，包含封面）
    pub async fn get_user_favorites(&self, mid: u64) -> AppResult<Value> {
        let mut params = BTreeMap::new();
        params.insert("up_mid".into(), mid.to_string());
        params.insert("pn".into(), "1".into());
        params.insert("ps".into(), "50".into());
        self.api_get("https://api.bilibili.com/x/v3/fav/folder/created/list", &params).await
    }

    /// 获取收藏夹内容
    pub async fn get_favorite_items(&self, media_id: u64, page: u32) -> AppResult<Value> {
        let mut params = BTreeMap::new();
        params.insert("media_id".into(), media_id.to_string());
        params.insert("pn".into(), page.to_string());
        params.insert("ps".into(), "20".into());
        params.insert("platform".into(), "web".into());
        self.api_get("https://api.bilibili.com/x/v3/fav/resource/list", &params).await
    }

    /// 验证登录会话是否有效
    pub async fn validate_session(&self) -> AppResult<bool> {
        let resp = self.get_user_info().await?;
        // code == 0 且 isLogin == true 表示会话有效
        let is_login = resp["data"]["isLogin"].as_bool().unwrap_or(false);
        Ok(resp["code"].as_i64() == Some(0) && is_login && resp["data"]["mid"].as_u64().is_some_and(|mid| mid > 0))
    }

    /// 获取单个收藏夹信息
    pub async fn get_fav_folder_info(&self, media_id: u64) -> AppResult<Value> {
        let mut params = BTreeMap::new();
        params.insert("media_id".into(), media_id.to_string());
        self.api_get("https://api.bilibili.com/x/v3/fav/folder/info", &params).await
    }

    /// 按 avid 获取视频信息
    pub async fn get_video_info_by_avid(&self, avid: u64) -> AppResult<BiliVideoInfo> {
        let mut params = BTreeMap::new();
        params.insert("aid".into(), avid.to_string());

        let resp = self.wbi_get("https://api.bilibili.com/x/web-interface/wbi/view", params).await?;
        let data = &resp["data"];

        Ok(BiliVideoInfo {
            bvid: data["bvid"].as_str().unwrap_or("").to_string(),
            title: data["title"].as_str().unwrap_or("").to_string(),
            owner: data["owner"]["name"].as_str().unwrap_or("").to_string(),
            cover: data["pic"].as_str().unwrap_or("").to_string(),
            cid: data["cid"].as_u64().unwrap_or(0),
            duration: data["duration"].as_u64().unwrap_or(0),
            pages: parse_video_pages(data),
        })
    }

    /// 获取视频分 P 列表
    pub async fn get_video_pages(&self, bvid: &str) -> AppResult<Value> {
        let mut params = BTreeMap::new();
        params.insert("bvid".into(), bvid.into());
        self.api_get("https://api.bilibili.com/x/player/pagelist", &params).await
    }

    /// 按 Android 旧式分 P 编号查找对应 cid
    pub async fn get_video_page_cid(&self, bvid: &str, page: u64) -> AppResult<Option<u64>> {
        let response = self.get_video_pages(bvid).await?;
        Ok(find_video_page_cid(&response, page))
    }
}

fn validate_bili_response(body: Value) -> AppResult<Value> {
    if json_u64(&body["code"]) != Some(0) {
        return Err(AppError::Api(format!(
            "Bili API error: code={}, message={}",
            body["code"], body["message"]
        )));
    }
    Ok(body)
}

fn json_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
}

fn clean_string(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("null"))
        .map(str::to_string)
}

fn parse_video_pages(data: &Value) -> Vec<BiliVideoPage> {
    data["pages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|page| {
            let cid = json_u64(&page["cid"]).filter(|cid| *cid > 0)?;
            Some(BiliVideoPage {
                cid,
                page: json_u64(&page["page"]).unwrap_or(0),
                part: clean_string(&page["part"]).unwrap_or_default(),
                duration: json_u64(&page["duration"]).unwrap_or(0),
            })
        })
        .collect()
}

fn bili_cookie_value(jar: &Jar, name: &str) -> Option<String> {
    let url = "https://api.bilibili.com/"
        .parse()
        .expect("valid bilibili URL");
    let header = jar.cookies(&url)?;
    header
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, value)| *key == name && !value.trim().is_empty())
        .map(|(_, value)| value.to_string())
}

fn has_bili_identity(jar: &Jar) -> bool {
    bili_cookie_value(jar, "SESSDATA").is_some() || bili_cookie_value(jar, "buvid3").is_some()
}

fn parse_anonymous_cookies(body: &Value) -> BTreeMap<String, String> {
    let data = &body["data"];
    [
        ("buvid3", "b_3"),
        ("buvid4", "b_4"),
        ("buvid_fp", "buvid_fp"),
        ("buvid_fp_plain", "buvid_fp_plain"),
        ("b_lsid", "b_lsid"),
    ]
    .iter()
    .filter_map(|(name, source)| {
        clean_string(&data[*source])
            .or_else(|| clean_string(&data[*name]))
            .map(|value| ((*name).to_string(), value))
    })
    .collect()
}

fn inject_anonymous_cookies(jar: &Jar, cookies: &BTreeMap<String, String>, max_age: u64) {
    if has_bili_identity(jar) {
        return;
    }
    let url = "https://api.bilibili.com/"
        .parse()
        .expect("valid bilibili URL");
    for (name, value) in cookies {
        jar.add_cookie_str(
            &format!("{name}={value}; Domain=bilibili.com; Path=/; Max-Age={max_age}"),
            &url,
        );
    }
}

fn parse_mixin_key(data: &Value, img_field: &str, sub_field: &str) -> AppResult<String> {
    let img = clean_string(&data[img_field])
        .ok_or_else(|| AppError::Api("Missing WBI image key".into()))?;
    let sub = clean_string(&data[sub_field])
        .ok_or_else(|| AppError::Api("Missing WBI sub key".into()))?;
    let key_part = |url: &str| {
        url.rsplit('/')
            .next()
            .unwrap_or("")
            .split('.')
            .next()
            .unwrap_or("")
            .to_string()
    };
    let key = wbi::get_mixin_key(&key_part(&img), &key_part(&sub));
    if key.len() != 32 {
        return Err(AppError::Api("Invalid WBI image keys".into()));
    }
    Ok(key)
}

fn web_ticket_signature(ts: &str) -> String {
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"XgwSnGZ1p").expect("valid HMAC key");
    mac.update(format!("ts{ts}").as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn playback_params(bvid: &str, cid: u64, html5: bool) -> BTreeMap<String, String> {
    let mut params = BTreeMap::from([
        ("bvid".into(), bvid.into()),
        ("cid".into(), cid.to_string()),
        ("fnval".into(), if html5 { "0" } else { "272" }.into()),
        ("fnver".into(), "0".into()),
        ("fourk".into(), "0".into()),
        ("otype".into(), "json".into()),
        ("platform".into(), if html5 { "html5" } else { "pc" }.into()),
    ]);
    if html5 {
        params.insert("high_quality".into(), "1".into());
    }
    params
}

async fn fetch_audio_streams<F, Fut>(mut fetch: F) -> AppResult<Vec<BiliAudioStream>>
where
    F: FnMut(bool) -> Fut,
    Fut: std::future::Future<Output = AppResult<Value>>,
{
    let mut last_response = Value::Null;
    for attempt in 0..3 {
        let response = fetch(false).await?;
        let streams = parse_dash_audio_streams(&response);
        if !streams.is_empty() {
            return Ok(streams);
        }
        let has_video = response["data"]["dash"]["video"]
            .as_array()
            .is_some_and(|video| !video.is_empty());
        let has_progressive = response["data"]["durl"]
            .as_array()
            .is_some_and(|durl| !durl.is_empty());
        if !has_video && !has_progressive {
            return Ok(parse_progressive_audio_streams(&response));
        }
        last_response = response;
        if attempt < 2 {
            tokio::time::sleep(Duration::from_millis(250 * (attempt + 1))).await;
        }
    }
    let html5_response = fetch(true).await?;
    let streams = parse_progressive_audio_streams(&html5_response);
    Ok(if streams.is_empty() {
        parse_progressive_audio_streams(&last_response)
    } else {
        streams
    })
}

fn stream_urls(primary: String, backups: &Value) -> Vec<String> {
    let mut candidates = vec![primary];
    candidates.extend(
        backups
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(clean_string),
    );
    let mut deduped = Vec::new();
    for url in candidates {
        let url = url.trim();
        let url = if url.starts_with("//") {
            format!("https:{url}")
        } else {
            url.to_string()
        };
        if !url.is_empty() && !deduped.contains(&url) {
            deduped.push(url);
        }
    }
    deduped.sort_by_key(|url| {
        let host = url::Url::parse(url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_lowercase))
            .unwrap_or_default();
        std::cmp::Reverse(
            if host.starts_with("upos-") && host.contains("bilivideo.") {
                3
            } else if host.contains("bilivideo.") {
                2
            } else if host.ends_with(".mountaintoys.cn") {
                1
            } else {
                0
            },
        )
    });
    deduped
}

fn parse_dash_audio_streams(body: &Value) -> Vec<BiliAudioStream> {
    let dash = &body["data"]["dash"];
    let mut streams = Vec::new();
    let mut append =
        |audio: &Value, tag: Option<&str>, fallback_codec: &str, fallback_mime: &str| {
            let Some(primary) =
                clean_string(&audio["baseUrl"]).or_else(|| clean_string(&audio["base_url"]))
            else {
                return;
            };
            let backups = audio
                .get("backupUrl")
                .filter(|value| value.is_array())
                .unwrap_or(&audio["backup_url"]);
            let candidates = stream_urls(primary, backups);
            let Some(url) = candidates.first().cloned() else {
                return;
            };
            streams.push(BiliAudioStream {
                url,
                bandwidth: json_u64(&audio["bandwidth"]).unwrap_or(0),
                codecs: clean_string(&audio["codecs"]).unwrap_or_else(|| fallback_codec.into()),
                quality_id: json_u64(&audio["id"])
                    .and_then(|id| u32::try_from(id).ok())
                    .unwrap_or(0),
                mime_type: clean_string(&audio["mimeType"])
                    .or_else(|| clean_string(&audio["mime_type"]))
                    .unwrap_or_else(|| fallback_mime.into()),
                quality_tag: tag.map(str::to_string),
                candidate_urls: candidates,
            });
        };
    for audio in dash["audio"].as_array().into_iter().flatten() {
        append(audio, None, "mp4a.40.2", "audio/mp4");
    }
    for audio in dash["dolby"]["audio"].as_array().into_iter().flatten() {
        append(audio, Some("dolby"), "ec-3", "audio/eac3");
    }
    if dash["flac"]["audio"].is_object() {
        append(&dash["flac"]["audio"], Some("hires"), "flac", "audio/flac");
    }
    streams.sort_by_key(|stream| std::cmp::Reverse(stream.bandwidth));
    streams
}

fn parse_progressive_audio_streams(body: &Value) -> Vec<BiliAudioStream> {
    let Some(durl) = body["data"]["durl"]
        .as_array()
        .filter(|durl| durl.len() == 1)
    else {
        return Vec::new();
    };
    let item = &durl[0];
    let Some(primary) = clean_string(&item["url"]) else {
        return Vec::new();
    };
    let backups = item
        .get("backup_url")
        .filter(|value| value.is_array())
        .unwrap_or(&item["backupUrl"]);
    let candidates = stream_urls(primary, backups);
    let duration_ms = json_u64(&item["length"]).unwrap_or(0);
    let size = json_u64(&item["size"]).unwrap_or(0);
    let bandwidth = size
        .saturating_mul(8)
        .saturating_mul(1000)
        .checked_div(duration_ms)
        .unwrap_or(0);
    vec![BiliAudioStream {
        url: candidates[0].clone(),
        candidate_urls: candidates,
        bandwidth,
        codecs: "mp4a.40.2".into(),
        quality_id: 0,
        mime_type: "video/mp4".into(),
        quality_tag: None,
    }]
}

pub fn bili_quality_key(stream: &BiliAudioStream) -> &'static str {
    match stream.quality_tag.as_deref() {
        Some("dolby") => "dolby",
        Some("hires") => "hires",
        Some("lossless") => "lossless",
        _ if is_lossless_stream(stream) => "lossless",
        _ if stream.bandwidth >= 1_000_000 => "hires",
        _ if stream.bandwidth >= 500_000 => "lossless",
        _ if stream.bandwidth >= 180_000 => "high",
        _ if stream.bandwidth >= 120_000 => "medium",
        _ => "low",
    }
}

pub fn is_lossless_stream(stream: &BiliAudioStream) -> bool {
    matches!(stream.quality_tag.as_deref(), Some("hires" | "lossless"))
        || matches!(
            stream
                .mime_type
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "audio/flac" | "audio/x-flac"
        )
        || stream.codecs.eq_ignore_ascii_case("flac")
}

fn find_video_page_cid(response: &Value, page: u64) -> Option<u64> {
    response["data"]
        .as_array()?
        .iter()
        .find(|item| json_u64(&item["page"]) == Some(page))
        .and_then(|item| json_u64(&item["cid"]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_cid_for_legacy_page_number() {
        let response = json!({
            "data": [
                { "page": 1, "cid": 101 },
                { "page": 7, "cid": 707 }
            ]
        });

        assert_eq!(find_video_page_cid(&response, 7), Some(707));
        assert_eq!(find_video_page_cid(&response, 2), None);
    }

    #[test]
    fn android_alignment_dash_keeps_backup_cdns_and_audio_metadata() {
        let streams = parse_dash_audio_streams(&json!({ "data": { "dash": {
            "audio": [{ "id": 30280, "base_url": "https://xy.example.bilivideo.cn/a.m4s", "backup_url": ["https://upos-a.bilivideo.com/a.m4s", "https://upos-a.bilivideo.com/a.m4s", "https://other.mountaintoys.cn/a.m4s"], "bandwidth": "192000", "codecs": "mp4a.40.2", "mime_type": "audio/mp4" }],
            "flac": { "audio": { "id": 30251, "baseUrl": "https://upos-a.bilivideo.com/lossless.m4s", "bandwidth": 1411000, "codecs": "fLaC", "mimeType": "audio/flac" } },
            "dolby": { "audio": [{ "id": 30250, "baseUrl": "https://upos-a.bilivideo.com/dolby.m4s", "bandwidth": 448000, "codecs": "ec-3", "mimeType": "audio/eac3" }] }
        } } }));
        let regular = streams
            .iter()
            .find(|stream| stream.quality_id == 30280)
            .unwrap();
        assert_eq!(regular.url, "https://upos-a.bilivideo.com/a.m4s");
        assert_eq!(
            regular.candidate_urls,
            vec![
                "https://upos-a.bilivideo.com/a.m4s",
                "https://xy.example.bilivideo.cn/a.m4s",
                "https://other.mountaintoys.cn/a.m4s"
            ]
        );
        assert_eq!(regular.bandwidth, 192_000);
        assert_eq!(bili_quality_key(regular), "high");
        assert_eq!(bili_quality_key(&streams[0]), "hires");
        let dolby = streams
            .iter()
            .find(|stream| stream.quality_id == 30250)
            .unwrap();
        assert_eq!(bili_quality_key(dolby), "dolby");
        assert_eq!(dolby.mime_type, "audio/eac3");
    }

    #[test]
    fn android_alignment_progressive_fallback_requires_one_complete_segment() {
        let response = json!({ "data": { "durl": [{ "url": "https://xy.example.bilivideo.cn/video.mp4", "backup_url": ["https://upos-a.bilivideo.com/video.mp4"], "size": 1000000, "length": 10000 }] } });
        let streams = parse_progressive_audio_streams(&response);
        assert_eq!(streams[0].url, "https://upos-a.bilivideo.com/video.mp4");
        assert_eq!(streams[0].bandwidth, 800_000);
        assert_eq!(streams[0].mime_type, "video/mp4");
        let split = json!({ "data": { "durl": [{ "url": "https://a/part1.mp4" }, { "url": "https://a/part2.mp4" }] } });
        assert!(parse_progressive_audio_streams(&split).is_empty());
    }

    #[tokio::test]
    async fn android_alignment_empty_dash_retries_then_falls_back_to_html5_mp4() {
        let mut calls = Vec::new();
        let streams = fetch_audio_streams(|html5| {
            calls.push(html5);
            std::future::ready(Ok(if html5 {
                json!({ "data": { "durl": [{ "url": "https://upos-a.bilivideo.com/full.mp4", "size": 1000000, "length": 10000 }] } })
            } else { json!({ "data": { "dash": { "video": [{}], "audio": [] } } }) }))
        }).await.unwrap();
        assert_eq!(streams[0].url, "https://upos-a.bilivideo.com/full.mp4");
        assert_eq!(calls, vec![false, false, false, true]);
        assert_eq!(playback_params("BVfixture", 7, true)["fnval"], "0");
        assert_eq!(playback_params("BVfixture", 7, true)["platform"], "html5");
        assert_eq!(playback_params("BVfixture", 7, true)["high_quality"], "1");
    }

    #[tokio::test]
    async fn android_alignment_empty_dash_recovery_stops_before_html5() {
        let mut calls = Vec::new();
        let streams = fetch_audio_streams(|html5| {
            calls.push(html5);
            std::future::ready(Ok(if calls.len() == 1 {
                json!({ "data": { "dash": { "video": [{}] } } })
            } else {
                json!({ "data": { "dash": { "audio": [{ "baseUrl": "https://upos-a.bilivideo.com/full.m4s", "bandwidth": 192000 }] } } })
            }))
        }).await.unwrap();
        assert_eq!(streams[0].url, "https://upos-a.bilivideo.com/full.m4s");
        assert_eq!(calls, vec![false, false]);
    }

    #[tokio::test]
    async fn android_alignment_empty_unavailable_response_is_not_amplified() {
        let mut calls = 0;
        let streams = fetch_audio_streams(|_| {
            calls += 1;
            std::future::ready(Ok(json!({ "data": {} })))
        })
        .await
        .unwrap();
        assert!(streams.is_empty());
        assert_eq!(calls, 1);
        let mut calls = 0;
        let error = fetch_audio_streams(|_| {
            calls += 1;
            std::future::ready(Err(AppError::Api("restricted fixture".into())))
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("restricted fixture"));
        assert_eq!(calls, 1);
    }

    #[test]
    fn android_alignment_anonymous_cookies_are_allowlisted_and_never_replace_login() {
        let cookies = parse_anonymous_cookies(
            &json!({ "data": { "b_3": "fixture-buvid", "b_4": "fixture-buvid4", "buvid_fp": "fixture-fingerprint", "SESSDATA": "must-not-import" } }),
        );
        assert!(!cookies.contains_key("SESSDATA"));
        let jar = Jar::default();
        let url = "https://api.bilibili.com/".parse().unwrap();
        inject_anonymous_cookies(&jar, &cookies, 3600);
        assert_eq!(
            bili_cookie_value(&jar, "buvid3").as_deref(),
            Some("fixture-buvid")
        );
        jar.add_cookie_str("SESSDATA=fixture-login; Domain=bilibili.com; Path=/", &url);
        let changed = BTreeMap::from([("buvid3".into(), "different-fingerprint".into())]);
        inject_anonymous_cookies(&jar, &changed, 3600);
        assert_eq!(
            bili_cookie_value(&jar, "SESSDATA").as_deref(),
            Some("fixture-login")
        );
        assert_eq!(
            bili_cookie_value(&jar, "buvid3").as_deref(),
            Some("fixture-buvid")
        );
    }

    #[test]
    fn android_alignment_ticket_signature_and_mixin_parsing_match() {
        assert_eq!(
            web_ticket_signature("1700000000"),
            "bb79f0d980ffbb51597aa1a3e8b55603025cc1322ac766f4c1a98852e6182514"
        );
        let nav = json!({ "img_url": "https://i.example/7cd084941338484aae1ad9425b84077c.png", "sub_url": "https://i.example/4932caff0ff746eab6f01bf08b70ac45.png" });
        assert_eq!(
            parse_mixin_key(&nav, "img_url", "sub_url").unwrap(),
            "ea1db124af3c7062474693fa704f4ff8"
        );
        let ticket = json!({ "img": nav["img_url"], "sub": nav["sub_url"] });
        assert_eq!(
            parse_mixin_key(&ticket, "img", "sub").unwrap(),
            "ea1db124af3c7062474693fa704f4ff8"
        );
        assert!(parse_mixin_key(
            &json!({ "img_url": "x", "sub_url": "y" }),
            "img_url",
            "sub_url"
        )
        .is_err());
    }

    #[test]
    fn android_alignment_video_pages_preserve_explicit_cid() {
        let pages = parse_video_pages(
            &json!({ "pages": [{ "page": 1, "cid": 101, "part": "first" }, { "page": "7", "cid": "707", "part": "requested", "duration": "180" }] }),
        );
        assert_eq!(pages[1].cid, 707);
        assert_eq!(pages[1].duration, 180);
        assert!(!pages.iter().any(|page| page.cid == 999));
    }
}
