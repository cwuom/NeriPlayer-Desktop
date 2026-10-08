use std::collections::{HashSet, VecDeque};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::api::transport::FallbackHttp;
use crate::auth::state::{CookieEntry, YouTubeAuth};
use crate::error::{AppError, AppResult};

const TTL: Duration = Duration::from_secs(10 * 60);
const SNAPSHOT_TTL_MS: u64 = 12 * 60 * 60 * 1000;
const SNAPSHOT_VERSION: u32 = 1;
const MAX_HTML_BYTES: usize = 8 * 1024 * 1024;
static CACHE: LazyLock<Mutex<VecDeque<Cached>>> = LazyLock::new(|| Mutex::new(VecDeque::new()));
static REFRESHING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct PlaybackBootstrap {
    pub(super) api_key: String,
    pub(super) client_version: String,
    pub(super) visitor_data: String,
    pub(super) player_js_url: String,
    pub(super) signature_timestamp: Option<u64>,
    pub(super) session_index: String,
    pub(super) user_session_id: String,
    pub(super) delegated_session_id: String,
    pub(super) logged_in: bool,
    #[serde(default)]
    pub(super) remote_host: String,
    #[serde(default)]
    pub(super) config_info: serde_json::Value,
    #[serde(default)]
    pub(super) rollout_token: String,
    #[serde(default)]
    pub(super) device_experiment_id: String,
    #[serde(skip)]
    pub(super) cookies: Vec<CookieEntry>,
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    key: String,
    fetched_at_ms: u64,
    value: PlaybackBootstrap,
}

struct Cached {
    key: String,
    value: PlaybackBootstrap,
    fetched_at: Instant,
}

pub(super) fn auth_fingerprint(auth: Option<&YouTubeAuth>) -> String {
    let mut hash = Sha256::new();
    // Music 和 Web/TV 的可见 Cookie 都影响会话，域外登录 Cookie 不参与取流身份
    for host in ["music.youtube.com", "www.youtube.com"] {
        let values = auth
            .map(|auth| super::account::select_cookie_values(&auth.cookies, host))
            .unwrap_or_default();
        hash.update(host.as_bytes());
        hash.update([0]);
        for (name, value) in values {
            hash.update(name.as_bytes());
            hash.update([0]);
            hash.update(value.as_bytes());
            hash.update([0]);
        }
    }
    hex::encode(hash.finalize())
}

fn config_string(html: &str, keys: &[&str]) -> Option<String> {
    for key in keys {
        let pattern = format!(r#""{}"\s*:\s*("(?:[^"\\]|\\.)*")"#, regex::escape(key));
        if let Some(value) = Regex::new(&pattern)
            .ok()?
            .captures(html)
            .and_then(|capture| capture.get(1))
        {
            if let Ok(value) = serde_json::from_str::<String>(value.as_str()) {
                if !value.trim().is_empty() {
                    return Some(value);
                }
            }
        }
    }
    None
}

fn config_number(html: &str, key: &str) -> Option<u64> {
    let pattern = format!(r#""{}"\s*:\s*"?(\d+)"?"#, regex::escape(key));
    Regex::new(&pattern)
        .ok()?
        .captures(html)?
        .get(1)?
        .as_str()
        .parse()
        .ok()
}

fn parse(html: &str) -> AppResult<PlaybackBootstrap> {
    let logged_in = Regex::new(r#""LOGGED_IN"\s*:\s*true"#)
        .expect("static regex")
        .is_match(html);
    let data_sync_id = config_string(html, &["DATASYNC_ID", "datasyncId"]).unwrap_or_default();
    let mut data_sync = data_sync_id.split("||");
    let first_session = data_sync.next().unwrap_or_default();
    let second_session = data_sync.next().unwrap_or_default();
    let mut config_info = serde_json::json!({});
    for (name, keys) in [
        ("appInstallData", &["appInstallData"][..]),
        ("coldConfigData", &["coldConfigData"][..]),
        (
            "coldHashData",
            &["coldHashData", "SERIALIZED_COLD_HASH_DATA"][..],
        ),
        (
            "hotHashData",
            &["hotHashData", "SERIALIZED_HOT_HASH_DATA"][..],
        ),
    ] {
        if let Some(value) = config_string(html, keys) {
            config_info[name] = serde_json::json!(value);
        }
    }
    let player_js_url = config_string(html, &["PLAYER_JS_URL", "jsUrl", "js"])
        .and_then(|url| super::challenge::trusted_player_script_url(&url))
        .or_else(|| {
            Regex::new(r#"(?i)<script[^>]+src\s*=\s*["']([^"']+/s/player/[^"']+\.js|/s/player/[^"']+\.js)["']"#)
                .ok()?
                .captures_iter(html)
                .find_map(|capture| super::challenge::trusted_player_script_url(capture.get(1)?.as_str()))
        })
        .unwrap_or_default();
    Ok(PlaybackBootstrap {
        api_key: config_string(html, &["INNERTUBE_API_KEY", "innertubeApiKey"])
            .ok_or_else(|| AppError::Api("YouTube playback bootstrap has no API key".into()))?,
        client_version: config_string(
            html,
            &[
                "INNERTUBE_CLIENT_VERSION",
                "INNERTUBE_CONTEXT_CLIENT_VERSION",
                "innertubeContextClientVersion",
            ],
        )
        .unwrap_or_else(|| "1.20260403.09.00".into()),
        visitor_data: config_string(html, &["VISITOR_DATA", "visitorData"]).unwrap_or_default(),
        player_js_url,
        signature_timestamp: config_number(html, "STS")
            .or_else(|| config_number(html, "signatureTimestamp")),
        session_index: config_number(html, "SESSION_INDEX")
            .unwrap_or(0)
            .to_string(),
        user_session_id: config_string(html, &["USER_SESSION_ID"]).unwrap_or_else(|| {
            if second_session.is_empty() {
                first_session
            } else {
                second_session
            }
            .into()
        }),
        delegated_session_id: config_string(html, &["DELEGATED_SESSION_ID"]).unwrap_or_else(|| {
            if second_session.is_empty() {
                ""
            } else {
                first_session
            }
            .into()
        }),
        logged_in,
        remote_host: config_string(html, &["remoteHost"]).unwrap_or_default(),
        config_info,
        rollout_token: config_string(html, &["rolloutToken"]).unwrap_or_default(),
        device_experiment_id: config_string(html, &["deviceExperimentId"]).unwrap_or_default(),
        cookies: Vec::new(),
    })
}

fn cache_key(auth: Option<&YouTubeAuth>, music_context: bool) -> String {
    let locale = super::innertube_locale();
    format!(
        "{}|{music_context}|{}|{}",
        auth_fingerprint(auth),
        locale.0,
        locale.1
    )
}

fn fresh_in_memory(key: &str) -> Option<PlaybackBootstrap> {
    let cache = CACHE.lock().ok()?;
    cache
        .iter()
        .find(|entry| entry.key == key && entry.fetched_at.elapsed() < TTL)
        .map(|entry| entry.value.clone())
}

/// 不发请求，只取内存或磁盘快照。快照过了新鲜期但仍在保留期内时照样先拿来起播，
/// 同时后台刷新——这与在线获取失败时回退到快照是同一个取舍
pub(super) fn cached(
    http: &FallbackHttp,
    auth: Option<&YouTubeAuth>,
    music_context: bool,
) -> Option<PlaybackBootstrap> {
    let key = cache_key(auth, music_context);
    if let Some(value) = fresh_in_memory(&key) {
        return Some(value);
    }
    let (value, age) =
        snapshot_path(music_context).and_then(|path| load_snapshot(&path, &key, auth, now_ms()))?;
    if age >= TTL.as_millis() as u64 {
        refresh_in_background(http, auth, music_context);
    }
    Some(value)
}

pub(super) fn refresh_in_background(
    http: &FallbackHttp,
    auth: Option<&YouTubeAuth>,
    music_context: bool,
) {
    let key = cache_key(auth, music_context);
    if !REFRESHING
        .lock()
        .map(|mut keys| keys.insert(key.clone()))
        .unwrap_or(false)
    {
        return;
    }
    let http = http.clone();
    let auth = auth.cloned();
    tokio::spawn(async move {
        tokio::time::sleep(super::BACKGROUND_WARMUP_DELAY).await;
        let _ = fetch(&http, auth.as_ref(), music_context, false).await;
        if let Ok(mut keys) = REFRESHING.lock() {
            keys.remove(&key);
        }
    });
}

pub(super) async fn fetch(
    http: &FallbackHttp,
    auth: Option<&YouTubeAuth>,
    music_context: bool,
    force_refresh: bool,
) -> AppResult<PlaybackBootstrap> {
    let key = cache_key(auth, music_context);
    if !force_refresh {
        if let Some(value) = fresh_in_memory(&key) {
            return Ok(value);
        }
    }
    let archived = if force_refresh {
        None
    } else {
        snapshot_path(music_context).and_then(|path| load_snapshot(&path, &key, auth, now_ms()))
    };
    if let Some((value, age)) = &archived {
        if *age < TTL.as_millis() as u64 {
            return Ok(value.clone());
        }
    }
    let started = Instant::now();
    let fetched = fetch_live(http, auth, music_context).await;
    log::info!(
        target: "youtube-playback",
        "bootstrap fetched music={music_context}, ok={}, archived={}, elapsed_ms={}",
        fetched.is_ok(),
        archived.is_some(),
        started.elapsed().as_millis(),
    );
    let bootstrap = match fetched {
        Ok(value) => value,
        Err(error) => return archived.map(|(value, _)| value).ok_or(error),
    };
    if let Some(path) = snapshot_path(music_context) {
        let _ = save_snapshot(&path, &key, &bootstrap, now_ms());
    }
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|entry| entry.key != key);
        cache.push_back(Cached {
            key,
            value: bootstrap.clone(),
            fetched_at: Instant::now(),
        });
        while cache.len() > 2 {
            cache.pop_front();
        }
    }
    Ok(bootstrap)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn snapshot_path(music_context: bool) -> Option<std::path::PathBuf> {
    dirs_next::cache_dir().map(|root| {
        root.join("NeriPlayer")
            .join("youtube-playback")
            .join(if music_context {
                "bootstrap-music.json"
            } else {
                "bootstrap-web.json"
            })
    })
}

fn load_snapshot(
    path: &std::path::Path,
    key: &str,
    auth: Option<&YouTubeAuth>,
    now: u64,
) -> Option<(PlaybackBootstrap, u64)> {
    if std::fs::metadata(path).ok()?.len() > 256 * 1024 {
        return None;
    }
    let snapshot: Snapshot = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let age = now.checked_sub(snapshot.fetched_at_ms)?;
    if snapshot.version != SNAPSHOT_VERSION
        || snapshot.key != key
        || age >= SNAPSHOT_TTL_MS
        || snapshot.value.api_key.is_empty()
        || super::challenge::trusted_player_script_url(&snapshot.value.player_js_url).is_none()
        || (auth.is_some_and(YouTubeAuth::has_login) && !snapshot.value.logged_in)
    {
        return None;
    }
    let mut value = snapshot.value;
    value.cookies = auth.map(|auth| auth.cookies.clone()).unwrap_or_default();
    Some((value, age))
}

fn save_snapshot(
    path: &std::path::Path,
    key: &str,
    value: &PlaybackBootstrap,
    now: u64,
) -> std::io::Result<()> {
    if value.api_key.is_empty()
        || super::challenge::trusted_player_script_url(&value.player_js_url).is_none()
    {
        return Ok(());
    }
    let bytes = serde_json::to_vec(&Snapshot {
        version: SNAPSHOT_VERSION,
        key: key.into(),
        fetched_at_ms: now,
        value: value.clone(),
    })?;
    if bytes.len() > 256 * 1024 {
        return Ok(());
    }
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Missing bootstrap cache directory",
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".bootstrap-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

async fn fetch_live(
    http: &FallbackHttp,
    auth: Option<&YouTubeAuth>,
    music_context: bool,
) -> AppResult<PlaybackBootstrap> {
    let authenticated = auth.is_some_and(YouTubeAuth::has_login);
    let cookie_values = auth
        .map(|auth| super::account::select_cookie_values(&auth.cookies, "music.youtube.com"))
        .unwrap_or_default();
    let cookie_header = cookie_values
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("; ");
    let page_url = if music_context {
        "https://music.youtube.com/"
    } else {
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ&bpctr=9999999999&has_verified=1"
    };
    let mut response = http
        .send(|client| {
            let request = client
                .get(page_url)
                .header("User-Agent", super::client::USER_AGENT)
                .header(
                    "Accept-Language",
                    format!("{},en;q=0.8", super::innertube_locale().0),
                );
            if cookie_header.is_empty() {
                request
            } else {
                request.header("Cookie", &cookie_header)
            }
        })
        .await?
        .error_for_status()?;
    if response.url().scheme() != "https"
        || response.url().port_or_known_default() != Some(443)
        || !matches!(
            response.url().host_str(),
            Some("music.youtube.com" | "www.youtube.com")
        )
    {
        return Err(AppError::Api(
            "YouTube playback bootstrap redirected to login or an untrusted host".into(),
        ));
    }
    let observed = response
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok().map(String::from))
        .collect::<Vec<_>>();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > MAX_HTML_BYTES {
            return Err(AppError::Api("YouTube bootstrap exceeds size limit".into()));
        }
        bytes.extend_from_slice(&chunk);
    }
    let html = String::from_utf8(bytes)
        .map_err(|_| AppError::Api("YouTube bootstrap is not UTF-8".into()))?;
    let mut bootstrap = parse(&html)?;
    if authenticated && !bootstrap.logged_in {
        return Err(AppError::Api(
            "YouTube playback bootstrap rejected login session".into(),
        ));
    }
    bootstrap.cookies = auth.map(|auth| auth.cookies.clone()).unwrap_or_default();
    for cookie in super::session::parse_set_cookie_headers(&observed) {
        bootstrap
            .cookies
            .retain(|saved| saved.name != cookie.name || saved.domain != cookie.domain);
        bootstrap.cookies.push(cookie);
    }
    Ok(bootstrap)
}

pub(super) fn timestamp_from_script(script: &str) -> Option<u64> {
    config_number(script, "signatureTimestamp").or_else(|| {
        Regex::new(r"\bsignatureTimestamp\s*:\s*(\d+)")
            .ok()?
            .captures(script)?
            .get(1)?
            .as_str()
            .parse()
            .ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dynamic_bootstrap_with_json_escapes_and_account_context() {
        let value = parse(r#"{"INNERTUBE_API_KEY":"key","VISITOR_DATA":"visit\u006fr","INNERTUBE_CLIENT_VERSION":"1.20260403.09.00","PLAYER_JS_URL":"\/s\/player\/abc\/base.js","STS":1234,"SESSION_INDEX":2,"LOGGED_IN":true,"DATASYNC_ID":"page||session"}"#).unwrap();
        assert_eq!(value.visitor_data, "visitor");
        assert_eq!(
            value.player_js_url,
            "https://www.youtube.com/s/player/abc/base.js"
        );
        assert_eq!(value.signature_timestamp, Some(1234));
        assert_eq!(value.session_index, "2");
        assert_eq!(value.user_session_id, "session");
        assert_eq!(value.delegated_session_id, "page");
        assert!(value.logged_in);
    }

    #[test]
    fn auth_fingerprint_is_stable_and_changes_on_cookie_rotation() {
        let mut auth = YouTubeAuth {
            cookies: vec![CookieEntry {
                name: "SID".into(),
                value: "test-session".into(),
                domain: ".youtube.com".into(),
            }],
            nickname: None,
            avatar_url: None,
        };
        let first = auth_fingerprint(Some(&auth));
        assert!(!first.contains("test-session"));
        auth.cookies[0].value = "rotated".into();
        assert_ne!(first, auth_fingerprint(Some(&auth)));
    }

    #[test]
    fn fingerprint_covers_both_player_cookie_views_and_ignores_unrelated_domains() {
        let mut auth = YouTubeAuth {
            cookies: vec![
                CookieEntry {
                    name: "SAPISID".into(),
                    value: "music-session".into(),
                    domain: "music.youtube.com".into(),
                },
                CookieEntry {
                    name: "SAPISID".into(),
                    value: "web-session".into(),
                    domain: "www.youtube.com".into(),
                },
                CookieEntry {
                    name: "SAPISID".into(),
                    value: "foreign-session".into(),
                    domain: "accounts.google.com".into(),
                },
            ],
            nickname: None,
            avatar_url: None,
        };
        let initial = auth_fingerprint(Some(&auth));
        auth.cookies[1].value = "web-rotated".into();
        let web_rotated = auth_fingerprint(Some(&auth));
        assert_ne!(initial, web_rotated);
        auth.cookies[0].value = "music-rotated".into();
        let both_rotated = auth_fingerprint(Some(&auth));
        assert_ne!(web_rotated, both_rotated);
        auth.cookies[2].value = "foreign-rotated".into();
        assert_eq!(both_rotated, auth_fingerprint(Some(&auth)));
        auth.cookies.reverse();
        assert_eq!(both_rotated, auth_fingerprint(Some(&auth)));
    }

    #[test]
    fn reads_signature_timestamp_from_player_script() {
        assert_eq!(
            timestamp_from_script("var config={signatureTimestamp:12345};"),
            Some(12345)
        );
    }

    #[test]
    fn dynamic_web_context_and_script_tag_fallback_match_android() {
        let value = parse(r#"<script>ytcfg.set({"INNERTUBE_API_KEY":"key","innertubeContextClientVersion":"dynamic","DATASYNC_ID":"derived-page||derived-user","USER_SESSION_ID":"explicit-user","DELEGATED_SESSION_ID":"explicit-page","remoteHost":"remote","appInstallData":"install","coldConfigData":"config","SERIALIZED_COLD_HASH_DATA":"cold","hotHashData":"hot","rolloutToken":"rollout","deviceExperimentId":"device"});</script><script src="/s/player/abc/base.js"></script>"#).unwrap();
        assert_eq!(value.client_version, "dynamic");
        assert_eq!(
            value.player_js_url,
            "https://www.youtube.com/s/player/abc/base.js"
        );
        assert_eq!(value.user_session_id, "explicit-user");
        assert_eq!(value.delegated_session_id, "explicit-page");
        assert_eq!(value.config_info["coldHashData"], "cold");
        assert_eq!(value.config_info["hotHashData"], "hot");
        assert_eq!(value.remote_host, "remote");
        assert_eq!(value.rollout_token, "rollout");
        assert_eq!(value.device_experiment_id, "device");
    }

    #[test]
    fn persisted_bootstrap_never_archives_cookies_and_checks_identity_and_age() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bootstrap.json");
        let mut value = parse(r#"{"INNERTUBE_API_KEY":"key","PLAYER_JS_URL":"/s/player/abc/base.js","LOGGED_IN":true}"#).unwrap();
        value.cookies.push(CookieEntry {
            name: "SAPISID".into(),
            value: "not-on-disk".into(),
            domain: ".youtube.com".into(),
        });
        save_snapshot(&path, "account-one", &value, 100).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(!saved.contains("not-on-disk"));
        assert!(!saved.contains("cookies"));
        assert!(load_snapshot(&path, "account-one", None, 101)
            .unwrap()
            .0
            .cookies
            .is_empty());
        assert!(load_snapshot(&path, "account-two", None, 101).is_none());
        assert!(load_snapshot(&path, "account-one", None, 99).is_none());
        assert!(load_snapshot(&path, "account-one", None, 100 + SNAPSHOT_TTL_MS).is_none());
        assert!(load_snapshot(&path, "account-one", None, 100 + SNAPSHOT_TTL_MS - 1).is_some());
        let auth = YouTubeAuth {
            cookies: value.cookies.clone(),
            nickname: None,
            avatar_url: None,
        };
        assert_eq!(
            load_snapshot(&path, "account-one", Some(&auth), 101)
                .unwrap()
                .0
                .cookies[0]
                .value,
            "not-on-disk"
        );
        value.logged_in = false;
        save_snapshot(&path, "account-one", &value, 100).unwrap();
        assert!(load_snapshot(&path, "account-one", Some(&auth), 101).is_none());
        let mut snapshot: Snapshot =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        snapshot.version += 1;
        std::fs::write(&path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
        assert!(load_snapshot(&path, "account-one", None, 101).is_none());
    }
}
