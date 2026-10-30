use std::collections::BTreeMap;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::state::{CookieEntry, YouTubeAuth};
use crate::error::{AppError, AppResult};
use crate::api::transport::FallbackHttp;

use super::client::USER_AGENT;

const MUSIC_ORIGIN: &str = "https://music.youtube.com";
pub(super) const MUSIC_HOST: &str = "music.youtube.com";
const WEB_REMIX_CLIENT_NAME: &str = "67";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct YouTubeAccountProfile {
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
}

impl YouTubeAccountProfile {
    fn has_profile(&self) -> bool {
        self.nickname.is_some() || self.avatar_url.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct YouTubeBootstrap {
    pub(super) api_key: String,
    pub(super) client_version: String,
    pub(super) visitor_data: String,
    pub(super) session_index: String,
    pub(super) user_session_id: String,
    logged_in: bool,
}

impl YouTubeBootstrap {
    pub(super) fn context(&self) -> Value {
        json!({
            "client": {
                "clientName": "WEB_REMIX",
                "clientVersion": self.client_version,
                "hl": super::innertube_locale().0,
                "gl": super::innertube_locale().1,
                "visitorData": self.visitor_data,
                "platform": "DESKTOP",
                "userAgent": USER_AGENT,
                "originalUrl": format!("{MUSIC_ORIGIN}/"),
                "utcOffsetMinutes": 480
            },
            "request": {
                "useSsl": true,
                "internalExperimentFlags": [],
                "consistencyTokenJars": []
            },
            "user": { "lockedSafetyMode": false }
        })
    }
}

/// bootstrap 缓存
///
/// 每次库请求都去抓一遍 music.youtube.com 的 HTML 太贵；但也不能永久缓存，
/// clientVersion / visitorData 会过期，同一 Cookie 下的账号索引也要分开缓存
const BOOTSTRAP_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 60);

struct CachedBootstrap {
    account_key: String,
    value: YouTubeBootstrap,
    fetched_at: std::time::Instant,
}

static BOOTSTRAP_CACHE: std::sync::Mutex<Option<CachedBootstrap>> =
    std::sync::Mutex::new(None);

fn bootstrap_account_key(auth: &YouTubeAuth) -> String {
    format!("{}|{}", auth.get_sapisid().unwrap_or_default(), auth.selected_auth_user().unwrap_or("0"))
}

/// 取一份可用的 bootstrap，命中缓存则直接返回
pub(super) async fn cached_bootstrap(
    http: &FallbackHttp,
    auth: &YouTubeAuth,
) -> AppResult<YouTubeBootstrap> {
    let account_key = bootstrap_account_key(auth);
    if let Ok(guard) = BOOTSTRAP_CACHE.lock() {
        if let Some(cached) = guard.as_ref() {
            if cached.account_key == account_key && cached.fetched_at.elapsed() < BOOTSTRAP_TTL {
                return Ok(cached.value.clone());
            }
        }
    }

    let cookie_values = select_cookie_values(&auth.cookies, MUSIC_HOST);
    if cookie_values.is_empty() {
        return Err(AppError::Api(
            "YouTube login is incomplete, please sign in again".into(),
        ));
    }
    let bootstrap = fetch_bootstrap(http, &build_cookie_header(&cookie_values), auth.selected_auth_user()).await?;
    log::info!(
        target: "youtube",
        "bootstrap ready: clientVersion={}, sessionIndex={}",
        bootstrap.client_version,
        bootstrap.session_index,
    );

    if let Ok(mut guard) = BOOTSTRAP_CACHE.lock() {
        *guard = Some(CachedBootstrap {
            account_key,
            value: bootstrap.clone(),
            fetched_at: std::time::Instant::now(),
        });
    }
    Ok(bootstrap)
}

/// 登录态变化或请求被拒时丢弃缓存，下次重新抓取
pub(super) fn invalidate_bootstrap_cache() {
    if let Ok(mut guard) = BOOTSTRAP_CACHE.lock() {
        *guard = None;
    }
}

pub async fn get_account_profile(
    http: &FallbackHttp,
    auth: &YouTubeAuth,
) -> AppResult<YouTubeAccountProfile> {
    let cookie_values = select_cookie_values(&auth.cookies, MUSIC_HOST);
    if cookie_values.is_empty() {
        return Err(AppError::Api(
            "YouTube login is incomplete, please sign in again".into(),
        ));
    }

    let cookie_header = build_cookie_header(&cookie_values);
    let bootstrap = fetch_bootstrap(http, &cookie_header, auth.selected_auth_user()).await?;
    let response = fetch_account_menu(http, &cookie_values, &cookie_header, &bootstrap).await?;
    parse_account_profile(&response)
        .filter(YouTubeAccountProfile::has_profile)
        .ok_or_else(|| AppError::Api("YouTube account menu did not contain a profile".into()))
}

pub(super) async fn fetch_bootstrap(
    http: &FallbackHttp,
    cookie_header: &str,
    auth_user: Option<&str>,
) -> AppResult<YouTubeBootstrap> {
    let response = http
        .send(|client| build_bootstrap_request(client, cookie_header, auth_user))
        .await?
        .error_for_status()?;
    let html = response.text().await?;
    let mut bootstrap = parse_bootstrap(&html)?;
    if let Some(auth_user) = auth_user.and_then(crate::auth::state::youtube_auth_user) {
        bootstrap.session_index = auth_user.to_string();
    }
    Ok(bootstrap)
}

fn build_bootstrap_request(client: &reqwest::Client, cookie_header: &str, auth_user: Option<&str>) -> reqwest::RequestBuilder {
    let request = client
        .get(format!("{MUSIC_ORIGIN}/"))
        .header("User-Agent", USER_AGENT)
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .header("Cookie", cookie_header);
    apply_auth_user(request, auth_user)
}

pub(super) fn apply_auth_user(request: reqwest::RequestBuilder, auth_user: Option<&str>) -> reqwest::RequestBuilder {
    match auth_user.and_then(crate::auth::state::youtube_auth_user) {
        Some(auth_user) => request.query(&[("authuser", auth_user)]).header("X-Goog-AuthUser", auth_user),
        None => request,
    }
}

async fn fetch_account_menu(
    http: &FallbackHttp,
    cookie_values: &BTreeMap<String, String>,
    cookie_header: &str,
    bootstrap: &YouTubeBootstrap,
) -> AppResult<Value> {
    let authorization = crate::auth::youtube_hash::build_youtube_authorization(
        cookie_values.get("SAPISID").map(String::as_str),
        cookie_values.get("__Secure-1PAPISID").map(String::as_str),
        cookie_values.get("__Secure-3PAPISID").map(String::as_str),
        cookie_values.get("APISID").map(String::as_str),
        MUSIC_ORIGIN,
        &bootstrap.user_session_id,
    )
    .ok_or_else(|| AppError::Api("No SAPISID for YouTube auth".into()))?;
    let url = format!(
        "{MUSIC_ORIGIN}/youtubei/v1/account/account_menu?prettyPrint=false&key={}",
        bootstrap.api_key
    );
    let body = json!({ "context": bootstrap.context() });

    let response = http
        .send(|client| {
            client
                .post(&url)
                .header("User-Agent", USER_AGENT)
                .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
                .header("Content-Type", "application/json")
                .header("Origin", MUSIC_ORIGIN)
                .header("X-Origin", MUSIC_ORIGIN)
                .header("Referer", format!("{MUSIC_ORIGIN}/"))
                .header("X-Goog-AuthUser", &bootstrap.session_index)
                .header("X-Goog-Visitor-Id", &bootstrap.visitor_data)
                .header("X-YouTube-Client-Name", WEB_REMIX_CLIENT_NAME)
                .header("X-YouTube-Client-Version", &bootstrap.client_version)
                .header("X-YouTube-Bootstrap-Logged-In", "true")
                .header("Authorization", &authorization)
                .header("Cookie", cookie_header)
                .json(&body)
        })
        .await?;

    let status = response.status();
    if !status.is_success() {
        return Err(AppError::Api(format!(
            "YouTube account menu request failed: HTTP {status}"
        )));
    }
    Ok(response.json().await?)
}

fn parse_bootstrap(html: &str) -> AppResult<YouTubeBootstrap> {
    let api_key = require_bootstrap_string(html, &["INNERTUBE_API_KEY", "innertubeApiKey"])?;
    let client_version = require_bootstrap_string(
        html,
        &[
            "INNERTUBE_CLIENT_VERSION",
            "INNERTUBE_CONTEXT_CLIENT_VERSION",
            "innertubeContextClientVersion",
        ],
    )?;
    let visitor_data = require_bootstrap_string(html, &["VISITOR_DATA", "visitorData"])?;
    let session_index = first_bootstrap_number(html, &["SESSION_INDEX"])
        .unwrap_or_else(|| "0".into());
    let logged_in = first_bootstrap_bool(html, &["LOGGED_IN"]).unwrap_or(false);
    if !logged_in {
        return Err(AppError::Api(
            "YouTube Music bootstrap returned a signed-out session".into(),
        ));
    }

    let data_sync_id = first_bootstrap_string(html, &["DATASYNC_ID", "datasyncId"])
        .unwrap_or_default();
    let user_session_id = first_bootstrap_string(html, &["USER_SESSION_ID"])
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| parse_user_session_id(&data_sync_id));

    Ok(YouTubeBootstrap {
        api_key,
        client_version,
        visitor_data,
        session_index,
        user_session_id,
        logged_in,
    })
}

fn require_bootstrap_string(html: &str, keys: &[&str]) -> AppResult<String> {
    first_bootstrap_string(html, keys).ok_or_else(|| {
        AppError::Api(format!(
            "YouTube Music bootstrap missing {}",
            keys.first().copied().unwrap_or("field")
        ))
    })
}

fn first_bootstrap_string(html: &str, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let pattern = format!(r#""{}"\s*:\s*"([^"]*)""#, regex::escape(key));
        let captures = Regex::new(&pattern).ok()?.captures(html)?;
        decode_json_string(captures.get(1)?.as_str())
    })
}

fn first_bootstrap_number(html: &str, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let pattern = format!(r#""{}"\s*:\s*"?([0-9]+)"?"#, regex::escape(key));
        Regex::new(&pattern)
            .ok()?
            .captures(html)?
            .get(1)
            .map(|value| value.as_str().to_string())
    })
}

fn first_bootstrap_bool(html: &str, keys: &[&str]) -> Option<bool> {
    keys.iter().find_map(|key| {
        let pattern = format!(r#""{}"\s*:\s*(true|false)"#, regex::escape(key));
        Regex::new(&pattern)
            .ok()?
            .captures(html)?
            .get(1)
            .map(|value| value.as_str() == "true")
    })
}

fn decode_json_string(raw: &str) -> Option<String> {
    serde_json::from_str::<String>(&format!("\"{raw}\""))
        .ok()
        .or_else(|| Some(raw.replace("\\x3d", "=").replace("\\u003d", "=")))
}

fn parse_user_session_id(data_sync_id: &str) -> String {
    match data_sync_id.split_once("||") {
        Some((_, session_id)) if !session_id.is_empty() => session_id.to_string(),
        _ => data_sync_id.to_string(),
    }
}

pub(super) fn select_cookie_values(cookies: &[CookieEntry], host: &str) -> BTreeMap<String, String> {
    cookies
        .iter()
        .filter(|cookie| cookie_domain_matches_host(&cookie.domain, host))
        .map(|cookie| (cookie.name.clone(), cookie.value.clone()))
        .collect()
}

fn cookie_domain_matches_host(domain: &str, host: &str) -> bool {
    let domain = domain.trim_start_matches('.').to_ascii_lowercase();
    let host = host.to_ascii_lowercase();
    !domain.is_empty() && (host == domain || host.ends_with(&format!(".{domain}")))
}

pub(super) fn build_cookie_header(cookie_values: &BTreeMap<String, String>) -> String {
    let mut values = cookie_values.clone();
    values.entry("SOCS".into()).or_insert_with(|| "CAI".into());
    values
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn parse_account_profile(value: &Value) -> Option<YouTubeAccountProfile> {
    find_profile_renderer(value, &["activeAccountHeaderRenderer"])
        .or_else(|| find_profile_renderer(value, &["accountItemRenderer", "accountItem"]))
}

fn find_profile_renderer(value: &Value, renderer_keys: &[&str]) -> Option<YouTubeAccountProfile> {
    match value {
        Value::Object(map) => {
            for key in renderer_keys {
                if let Some(renderer) = map.get(*key) {
                    let profile = profile_from_renderer(renderer);
                    if profile.has_profile() {
                        return Some(profile);
                    }
                }
            }
            map.values()
                .find_map(|child| find_profile_renderer(child, renderer_keys))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_profile_renderer(child, renderer_keys)),
        _ => None,
    }
}

fn profile_from_renderer(value: &Value) -> YouTubeAccountProfile {
    YouTubeAccountProfile {
        nickname: first_text_field(
            value,
            &["accountName", "displayName", "channelName", "title"],
        ),
        avatar_url: first_thumbnail_field(
            value,
            &["accountPhoto", "avatar", "thumbnail"],
        ),
    }
}

fn first_text_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| value.get(*key))
        .find_map(extract_text)
}

fn extract_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => clean_text(text),
        Value::Object(map) => {
            if let Some(text) = map.get("simpleText").and_then(Value::as_str) {
                return clean_text(text);
            }
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                return clean_text(text);
            }
            let joined = map
                .get("runs")
                .and_then(Value::as_array)?
                .iter()
                .filter_map(|run| run.get("text").and_then(Value::as_str))
                .collect::<String>();
            clean_text(&joined)
        }
        _ => None,
    }
}

fn clean_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn first_thumbnail_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| value.get(*key))
        .find_map(extract_thumbnail_url)
}

fn extract_thumbnail_url(value: &Value) -> Option<String> {
    if let Some(url) = value.as_str() {
        return normalize_thumbnail_url(url);
    }
    if let Some(thumbnails) = value.get("thumbnails").and_then(Value::as_array) {
        return thumbnails
            .iter()
            .rev()
            .filter_map(|item| item.get("url").and_then(Value::as_str))
            .find_map(normalize_thumbnail_url);
    }
    value.get("thumbnail").and_then(extract_thumbnail_url)
}

fn normalize_thumbnail_url(url: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        None
    } else if trimmed.starts_with("//") {
        Some(format!("https:{trimmed}"))
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_request_preserves_explicit_account_selection() {
        let client = reqwest::Client::new();
        let request = build_bootstrap_request(&client, "SAPISID=fixture", Some("2")).build().unwrap();
        assert_eq!(request.headers()["X-Goog-AuthUser"], "2");
        assert!(request.url().query_pairs().any(|(key, value)| key == "authuser" && value == "2"));
        let request = build_bootstrap_request(&client, "SAPISID=fixture", None).build().unwrap();
        assert!(!request.headers().contains_key("X-Goog-AuthUser"));
        assert!(request.url().query().is_none());
    }

    #[test]
    fn account_bootstrap_cache_distinguishes_accounts_sharing_cookies() {
        let mut auth: YouTubeAuth = serde_json::from_value(json!({
            "cookies": [{"name":"SAPISID", "value":"fixture", "domain":".youtube.com"}],
        })).unwrap();
        let default = bootstrap_account_key(&auth);
        auth.auth_user = Some("2".into());
        assert_ne!(default, bootstrap_account_key(&auth));
    }

    #[test]
    fn parses_current_music_bootstrap_fields() {
        let html = r#"<script>ytcfg.set({"INNERTUBE_API_KEY":"key","INNERTUBE_CLIENT_VERSION":"1.20260712.05.00","VISITOR_DATA":"visitor\u003d","SESSION_INDEX":"0","DATASYNC_ID":"channel||session","LOGGED_IN":true});</script>"#;

        assert_eq!(
            parse_bootstrap(html).unwrap(),
            YouTubeBootstrap {
                api_key: "key".into(),
                client_version: "1.20260712.05.00".into(),
                visitor_data: "visitor=".into(),
                session_index: "0".into(),
                user_session_id: "session".into(),
                logged_in: true,
            }
        );
    }

    #[test]
    fn filters_cookie_header_to_youtube_domain() {
        let cookies = vec![
            CookieEntry {
                name: "SAPISID".into(),
                value: "google".into(),
                domain: "google.com".into(),
            },
            CookieEntry {
                name: "SAPISID".into(),
                value: "youtube".into(),
                domain: "youtube.com".into(),
            },
            CookieEntry {
                name: "__Host-GAPS".into(),
                value: "accounts".into(),
                domain: "accounts.google.com".into(),
            },
        ];

        let selected = select_cookie_values(&cookies, MUSIC_HOST);
        assert_eq!(selected.get("SAPISID").map(String::as_str), Some("youtube"));
        assert!(!selected.contains_key("__Host-GAPS"));
        assert_eq!(build_cookie_header(&selected), "SAPISID=youtube; SOCS=CAI");
    }

    #[test]
    fn parses_active_account_header_profile() {
        let value = json!({
            "actions": [{
                "openPopupAction": {
                    "popup": {
                        "multiPageMenuRenderer": {
                            "header": {
                                "activeAccountHeaderRenderer": {
                                    "accountName": { "runs": [{ "text": "Neri User" }] },
                                    "accountPhoto": {
                                        "thumbnails": [
                                            { "url": "//yt3.ggpht.com/small=s32" },
                                            { "url": "//yt3.ggpht.com/large=s88" }
                                        ]
                                    }
                                }
                            }
                        }
                    }
                }
            }]
        });

        assert_eq!(
            parse_account_profile(&value),
            Some(YouTubeAccountProfile {
                nickname: Some("Neri User".into()),
                avatar_url: Some("https://yt3.ggpht.com/large=s88".into()),
            })
        );
    }

    #[test]
    fn skips_empty_account_item_before_profile() {
        let value = json!({
            "contents": [
                { "accountItemRenderer": { "accountName": { "simpleText": "" } } },
                {
                    "accountItemRenderer": {
                        "accountName": { "simpleText": "Music Account" },
                        "thumbnail": {
                            "thumbnails": [{ "url": "https://yt3.ggpht.com/avatar=s64" }]
                        }
                    }
                }
            ]
        });

        assert_eq!(
            parse_account_profile(&value),
            Some(YouTubeAccountProfile {
                nickname: Some("Music Account".into()),
                avatar_url: Some("https://yt3.ggpht.com/avatar=s64".into()),
            })
        );
    }
}
