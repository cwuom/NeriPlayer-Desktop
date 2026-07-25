// B站 API 客户端
use reqwest::cookie::{CookieStore, Jar};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

use super::wbi;
use crate::error::{AppError, AppResult};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const FINGERPRINT_USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 13_2_3 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/13.0.3 Mobile/15E148 Safari/604.1 Edg/114.0.0.0";
const FINGERPRINT_URL: &str = "https://api.bilibili.com/x/frontend/finger/spi";
const BILIBILI_API_URL: &str = "https://api.bilibili.com/";

pub struct BiliClient {
    http: Client,
    cookie_jar: Arc<Jar>,
    mixin_key: parking_lot::Mutex<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiliAudioStream {
    pub url: String,
    pub bandwidth: u64,
    pub codecs: String,
    pub quality_id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiliVideoInfo {
    pub bvid: String,
    pub title: String,
    pub owner: String,
    pub cover: String,
    pub cid: u64,
    pub duration: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BiliVideoPage {
    pub cid: u64,
    pub title: String,
    pub duration_seconds: u64,
}

impl BiliClient {
    pub fn new(http: &Client, cookie_jar: Arc<Jar>) -> Self {
        Self {
            http: http.clone(),
            cookie_jar,
            mixin_key: parking_lot::Mutex::new(None),
        }
    }

    async fn ensure_effective_cookies(&self) -> AppResult<()> {
        let api_url = Url::parse(BILIBILI_API_URL)
            .map_err(|error| AppError::Api(format!("Invalid Bilibili API URL: {error}")))?;
        if has_effective_bilibili_cookie(self.cookie_jar.cookies(&api_url).as_ref()) {
            return Ok(());
        }

        let response = self
            .http
            .get(FINGERPRINT_URL)
            .header("User-Agent", FINGERPRINT_USER_AGENT)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(AppError::Api(format!(
                "Bilibili fingerprint request failed with HTTP {status}"
            )));
        }
        let body: Value = response.json().await?;
        if body["code"].as_i64().unwrap_or(-1) != 0 {
            return Err(AppError::Api(format!(
                "Bilibili fingerprint API error: {}",
                body["message"].as_str().unwrap_or("unknown error")
            )));
        }
        let cookies = parse_fingerprint_cookies(&body);
        if cookies.is_empty() {
            return Err(AppError::Api(
                "Bilibili fingerprint response contained no cookies".into(),
            ));
        }
        inject_fingerprint_cookies(&self.cookie_jar, &api_url, &cookies);
        Ok(())
    }

    /// 获取或刷新 mixin_key
    async fn ensure_mixin_key(&self) -> AppResult<String> {
        if let Some(ref key) = *self.mixin_key.lock() {
            return Ok(key.clone());
        }

        self.ensure_effective_cookies().await?;

        let resp: Value = self
            .http
            .get("https://api.bilibili.com/x/web-interface/nav")
            .header("User-Agent", USER_AGENT)
            .header("Referer", "https://www.bilibili.com")
            .send()
            .await?
            .json()
            .await?;

        let img_url = resp["data"]["wbi_img"]["img_url"]
            .as_str()
            .ok_or_else(|| AppError::Api("No wbi img_url".into()))?;
        let sub_url = resp["data"]["wbi_img"]["sub_url"]
            .as_str()
            .ok_or_else(|| AppError::Api("No wbi sub_url".into()))?;

        // 提取文件名（去掉路径和扩展名）
        let img_key = img_url
            .rsplit('/')
            .next()
            .unwrap_or("")
            .split('.')
            .next()
            .unwrap_or("");
        let sub_key = sub_url
            .rsplit('/')
            .next()
            .unwrap_or("")
            .split('.')
            .next()
            .unwrap_or("");

        let key = wbi::get_mixin_key(img_key, sub_key);
        *self.mixin_key.lock() = Some(key.clone());
        Ok(key)
    }

    /// 带 Wbi 签名的 GET 请求
    async fn wbi_get(&self, url: &str, mut params: BTreeMap<String, String>) -> AppResult<Value> {
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
            .get(&full_url)
            .header("User-Agent", USER_AGENT)
            .header("Referer", "https://www.bilibili.com")
            .send()
            .await?
            .json()
            .await?;

        if resp["code"].as_i64() != Some(0) {
            return Err(AppError::Api(format!(
                "Bili API error: {}",
                resp["message"]
            )));
        }
        Ok(resp)
    }

    /// 获取视频信息
    pub async fn get_video_info(&self, bvid: &str) -> AppResult<BiliVideoInfo> {
        let mut params = BTreeMap::new();
        params.insert("bvid".into(), bvid.into());

        let resp = self
            .wbi_get("https://api.bilibili.com/x/web-interface/wbi/view", params)
            .await?;
        let data = &resp["data"];

        Ok(BiliVideoInfo {
            bvid: bvid.to_string(),
            title: data["title"].as_str().unwrap_or("").to_string(),
            owner: data["owner"]["name"].as_str().unwrap_or("").to_string(),
            cover: data["pic"].as_str().unwrap_or("").to_string(),
            cid: data["cid"].as_u64().unwrap_or(0),
            duration: data["duration"].as_u64().unwrap_or(0),
        })
    }

    /// 获取音频流 URL（DASH 模式）
    pub async fn get_audio_url(&self, bvid: &str, cid: u64) -> AppResult<Vec<BiliAudioStream>> {
        let mut params = BTreeMap::new();
        params.insert("bvid".into(), bvid.into());
        params.insert("cid".into(), cid.to_string());
        params.insert("fnval".into(), "272".into()); // DASH + Dolby
        params.insert("fnver".into(), "0".into());
        params.insert("fourk".into(), "0".into());
        params.insert("otype".into(), "json".into());
        params.insert("platform".into(), "pc".into());

        let resp = self
            .wbi_get("https://api.bilibili.com/x/player/wbi/playurl", params)
            .await?;
        let dash = &resp["data"]["dash"];

        let mut streams = Vec::new();

        // 普通音频流
        if let Some(audios) = dash["audio"].as_array() {
            for a in audios {
                if let Some(url) = a["baseUrl"].as_str().or(a["base_url"].as_str()) {
                    streams.push(BiliAudioStream {
                        url: url.to_string(),
                        bandwidth: a["bandwidth"].as_u64().unwrap_or(0),
                        codecs: a["codecs"].as_str().unwrap_or("").to_string(),
                        quality_id: a["id"].as_u64().unwrap_or(0) as u32,
                    });
                }
            }
        }

        // FLAC 无损
        if let Some(flac) = dash["flac"]["audio"].as_object() {
            if let Some(url) = flac
                .get("baseUrl")
                .or(flac.get("base_url"))
                .and_then(|v| v.as_str())
            {
                streams.push(BiliAudioStream {
                    url: url.to_string(),
                    bandwidth: flac.get("bandwidth").and_then(|v| v.as_u64()).unwrap_or(0),
                    codecs: "flac".to_string(),
                    quality_id: 30251,
                });
            }
        }

        // Dolby
        if let Some(dolby_audios) = dash["dolby"]["audio"].as_array() {
            for a in dolby_audios {
                if let Some(url) = a["baseUrl"].as_str().or(a["base_url"].as_str()) {
                    streams.push(BiliAudioStream {
                        url: url.to_string(),
                        bandwidth: a["bandwidth"].as_u64().unwrap_or(0),
                        codecs: "ec-3".to_string(),
                        quality_id: 30250,
                    });
                }
            }
        }

        // 按码率降序
        streams.sort_by(|a, b| b.bandwidth.cmp(&a.bandwidth));
        Ok(streams)
    }

    /// 搜索视频
    pub async fn search(&self, keyword: &str, duration: u8) -> AppResult<Value> {
        let mut params = BTreeMap::new();
        params.insert("search_type".into(), "video".into());
        params.insert("keyword".into(), keyword.into());
        params.insert("order".into(), "totalrank".into());
        params.insert("duration".into(), duration.min(4).to_string());
        params.insert("tids".into(), "0".into());
        params.insert("page".into(), "1".into());

        self.wbi_get(
            "https://api.bilibili.com/x/web-interface/wbi/search/type",
            params,
        )
        .await
    }

    // 需要登录的 API
    /// 获取登录用户信息（也用于 Wbi key 刷新）
    pub async fn get_user_info(&self) -> AppResult<Value> {
        let resp = self
            .http
            .get("https://api.bilibili.com/x/web-interface/nav")
            .header("User-Agent", USER_AGENT)
            .header("Referer", "https://www.bilibili.com")
            .send()
            .await?;
        let body: Value = resp.json().await?;
        Ok(body)
    }

    /// 获取用户创建的收藏夹列表（分页版，包含封面）
    pub async fn get_user_favorites(&self, mid: u64) -> AppResult<Value> {
        let mut params = BTreeMap::new();
        params.insert("up_mid".into(), mid.to_string());
        params.insert("pn".into(), "1".into());
        params.insert("ps".into(), "50".into());
        self.wbi_get(
            "https://api.bilibili.com/x/v3/fav/folder/created/list",
            params,
        )
        .await
    }

    /// 获取收藏夹内容
    pub async fn get_favorite_items(&self, media_id: u64, page: u32) -> AppResult<Value> {
        let mut params = BTreeMap::new();
        params.insert("media_id".into(), media_id.to_string());
        params.insert("pn".into(), page.to_string());
        params.insert("ps".into(), "20".into());
        params.insert("platform".into(), "web".into());
        self.wbi_get("https://api.bilibili.com/x/v3/fav/resource/list", params)
            .await
    }

    /// 验证登录会话是否有效
    pub async fn validate_session(&self) -> AppResult<bool> {
        let resp = self.get_user_info().await?;
        // code == 0 且 isLogin == true 表示会话有效
        let is_login = resp["data"]["isLogin"].as_bool().unwrap_or(false);
        Ok(is_login)
    }

    /// 获取单个收藏夹信息
    pub async fn get_fav_folder_info(&self, media_id: u64) -> AppResult<Value> {
        let mut params = BTreeMap::new();
        params.insert("media_id".into(), media_id.to_string());
        self.wbi_get("https://api.bilibili.com/x/v3/fav/folder/info", params)
            .await
    }

    /// 按 avid 获取视频信息
    pub async fn get_video_info_by_avid(&self, avid: u64) -> AppResult<BiliVideoInfo> {
        let mut params = BTreeMap::new();
        params.insert("aid".into(), avid.to_string());

        let resp = self
            .wbi_get("https://api.bilibili.com/x/web-interface/wbi/view", params)
            .await?;
        let data = &resp["data"];

        Ok(BiliVideoInfo {
            bvid: data["bvid"].as_str().unwrap_or("").to_string(),
            title: data["title"].as_str().unwrap_or("").to_string(),
            owner: data["owner"]["name"].as_str().unwrap_or("").to_string(),
            cover: data["pic"].as_str().unwrap_or("").to_string(),
            cid: data["cid"].as_u64().unwrap_or(0),
            duration: data["duration"].as_u64().unwrap_or(0),
        })
    }

    /// 获取视频分 P 列表
    pub async fn get_video_pages(&self, bvid: &str) -> AppResult<Value> {
        let mut params = BTreeMap::new();
        params.insert("bvid".into(), bvid.into());
        self.wbi_get("https://api.bilibili.com/x/player/wbi/pagelist", params)
            .await
    }

    pub async fn get_video_page_details(&self, bvid: &str) -> AppResult<Vec<BiliVideoPage>> {
        let response = self.get_video_pages(bvid).await?;
        parse_video_pages(&response)
    }

    /// 按 Android 旧式分 P 编号查找对应 cid
    pub async fn get_video_page_cid(&self, bvid: &str, page: u64) -> AppResult<Option<u64>> {
        let response = self.get_video_pages(bvid).await?;
        Ok(find_video_page_cid(&response, page))
    }
}

fn has_effective_bilibili_cookie(header: Option<&reqwest::header::HeaderValue>) -> bool {
    let Some(header) = header.and_then(|value| value.to_str().ok()) else {
        return false;
    };
    header.split(';').any(|part| {
        part.trim().split_once('=').is_some_and(|(name, value)| {
            matches!(name.trim(), "SESSDATA" | "buvid3" | "buvid4") && !value.trim().is_empty()
        })
    })
}

fn parse_fingerprint_cookies(response: &Value) -> BTreeMap<String, String> {
    let data = &response["data"];
    [
        (
            "buvid3",
            data["b_3"].as_str().or_else(|| data["buvid3"].as_str()),
        ),
        (
            "buvid4",
            data["b_4"].as_str().or_else(|| data["buvid4"].as_str()),
        ),
        ("buvid_fp", data["buvid_fp"].as_str()),
        ("buvid_fp_plain", data["buvid_fp_plain"].as_str()),
        ("b_lsid", data["b_lsid"].as_str()),
    ]
    .into_iter()
    .filter_map(|(name, value)| {
        let value = value?.trim();
        (!value.is_empty()).then(|| (name.to_string(), value.to_string()))
    })
    .collect()
}

fn inject_fingerprint_cookies(jar: &Jar, url: &Url, cookies: &BTreeMap<String, String>) {
    for (name, value) in cookies {
        jar.add_cookie_str(
            &format!("{name}={value}; Domain=.bilibili.com; Path=/"),
            url,
        );
    }
}

fn parse_video_pages(response: &Value) -> AppResult<Vec<BiliVideoPage>> {
    let pages = response["data"]
        .as_array()
        .ok_or_else(|| AppError::Api("Invalid Bilibili page list response".into()))?;

    Ok(pages
        .iter()
        .filter_map(|page| {
            let cid = page["cid"].as_u64()?;
            (cid > 0).then(|| BiliVideoPage {
                cid,
                title: page["part"].as_str().unwrap_or("").trim().to_string(),
                duration_seconds: page["duration"].as_u64().unwrap_or(0),
            })
        })
        .collect())
}

fn find_video_page_cid(response: &Value, page: u64) -> Option<u64> {
    response["data"]
        .as_array()?
        .iter()
        .find(|item| item["page"].as_u64() == Some(page))
        .and_then(|item| item["cid"].as_u64())
}

#[cfg(test)]
mod tests {
    use super::{
        find_video_page_cid, has_effective_bilibili_cookie, inject_fingerprint_cookies,
        parse_fingerprint_cookies, parse_video_pages,
    };
    use reqwest::cookie::{CookieStore, Jar};
    use reqwest::Url;
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
    fn parses_structured_video_pages_for_auto_source_selection() {
        let response = json!({
            "data": [
                { "page": 1, "cid": 101, "part": "opening", "duration": 42 },
                { "page": 2, "cid": 202, "part": "target song", "duration": 180 },
                { "page": 3, "cid": 0, "part": "invalid", "duration": 999 }
            ]
        });

        let pages = parse_video_pages(&response).expect("valid Bilibili page list");

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].cid, 101);
        assert_eq!(pages[0].title, "opening");
        assert_eq!(pages[0].duration_seconds, 42);
        assert_eq!(pages[1].cid, 202);
        assert_eq!(pages[1].title, "target song");
        assert_eq!(pages[1].duration_seconds, 180);
    }

    #[test]
    fn parses_and_attaches_android_anonymous_fingerprint_cookies() {
        let response = json!({
            "code": 0,
            "data": {
                "b_3": "anon-buvid-3",
                "b_4": "anon-buvid-4",
                "buvid_fp": "fingerprint",
                "buvid_fp_plain": "fingerprint-plain",
                "b_lsid": "browser-session"
            }
        });
        let cookies = parse_fingerprint_cookies(&response);
        assert_eq!(
            cookies.get("buvid3").map(String::as_str),
            Some("anon-buvid-3")
        );
        assert_eq!(
            cookies.get("buvid4").map(String::as_str),
            Some("anon-buvid-4")
        );

        let jar = Jar::default();
        let search_url = Url::parse("https://api.bilibili.com/x/web-interface/wbi/search/type")
            .expect("valid Bilibili search URL");
        inject_fingerprint_cookies(&jar, &search_url, &cookies);

        let header = jar.cookies(&search_url).expect("fingerprint cookie header");
        let header_text = header.to_str().expect("ASCII cookie header");
        assert!(header_text.contains("buvid3=anon-buvid-3"));
        assert!(header_text.contains("buvid4=anon-buvid-4"));
        assert!(header_text.contains("buvid_fp=fingerprint"));
        assert!(header_text.contains("b_lsid=browser-session"));
        assert!(has_effective_bilibili_cookie(Some(&header)));
    }

    #[test]
    fn distinguishes_effective_bilibili_cookies_from_unrelated_values() {
        let unrelated = reqwest::header::HeaderValue::from_static("bili_jct=csrf; sid=short");
        let logged_in = reqwest::header::HeaderValue::from_static("SESSDATA=session");
        let empty = reqwest::header::HeaderValue::from_static("SESSDATA=; buvid3= ");

        assert!(!has_effective_bilibili_cookie(Some(&unrelated)));
        assert!(has_effective_bilibili_cookie(Some(&logged_in)));
        assert!(!has_effective_bilibili_cookie(Some(&empty)));
        assert!(!has_effective_bilibili_cookie(None));
    }
}
