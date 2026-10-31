// YouTube 登录会话主动保鲜: 轻量 InnerTube + Cookie 方案
// 通过加载 music/www 首页回收轮换 Cookie(SIDCC / __Secure-*SIDTS 等), 避免长期空闲掉登录;
// 不模拟完整浏览器, 因此不会与移动端同账号互相挤掉登录(对齐 Android YouTubeAuthAutoRefreshManager)
use std::time::{SystemTime, UNIX_EPOCH};


use crate::auth::state::{CookieEntry, YouTubeAuth};
use crate::error::{AppError, AppResult};
use crate::api::transport::FallbackHttp;

use super::client::USER_AGENT;
use super::session;

// 常规保鲜冷却: 15 分钟
pub const REFRESH_COOLDOWN_MS: u64 = 15 * 60 * 1000;
// 强制保鲜退避: 90 秒, 用于 401/403 等恢复路径避免请求风暴
pub const FORCE_REFRESH_BACKOFF_MS: u64 = 90_000;
// 连续失败阈值, 达到即触发熔断
pub const MAX_CONSECUTIVE_FAILURES: u32 = 2;
// 熔断持续时间: 30 分钟
pub const CIRCUIT_BREAK_MS: u64 = 30 * 60 * 1000;

const MUSIC_HOME_URL: &str = "https://music.youtube.com/";
const WWW_REFRESH_URL: &str = "https://www.youtube.com/?themeRefresh=1";
const ROTATE_COOKIES_URL: &str = "https://accounts.google.com/RotateCookies";
const ROTATE_COOKIES_ORIGIN: &str = "https://accounts.google.com";
const ROTATE_COOKIES_PAYLOAD: &str = "[000,\"-0000000000000000000\"]";
const ROTATED_COOKIE_KEYS: &[&str] = &["__Secure-1PSIDTS", "__Secure-3PSIDTS"];
const ROTATION_REQUEST_COOKIE_KEYS: &[&str] = &[
    "SID",
    "HSID",
    "SSID",
    "APISID",
    "SAPISID",
    "LSID",
    "OSID",
    "__Secure-1PSID",
    "__Secure-3PSID",
    "__Secure-1PAPISID",
    "__Secure-3PAPISID",
    "__Secure-1PSIDTS",
    "__Secure-3PSIDTS",
    "SIDCC",
    "__Secure-1PSIDCC",
    "__Secure-3PSIDCC",
];

pub const ROTATION_MIN_INTERVAL_MS: u64 = 60_000;
pub const ROTATION_DEFAULT_INTERVAL_MS: u64 = 600_000;
const ROTATION_REJECTIONS_BEFORE_BACKOFF: u32 = 3;
const ROTATION_MAX_BACKOFF_MS: u64 = 6 * 60 * 60 * 1000;
const ROTATION_MAX_BACKOFF_EXPONENT: u32 = 16;

#[derive(Debug, Clone, Default)]
pub struct YouTubeCookieRotationResult {
    pub updated_auth: Option<YouTubeAuth>,
    pub next_interval_ms: u64,
}

#[derive(Debug)]
pub struct YouTubeCookieRotationGate {
    last_attempt_ms: Option<u64>,
    next_interval_ms: u64,
    consecutive_rejections: u32,
}

impl Default for YouTubeCookieRotationGate {
    fn default() -> Self {
        Self {
            last_attempt_ms: None,
            next_interval_ms: ROTATION_DEFAULT_INTERVAL_MS,
            consecutive_rejections: 0,
        }
    }
}

impl YouTubeCookieRotationGate {
    pub fn should_attempt(&self, now_ms: u64, force: bool, has_prerequisites: bool) -> bool {
        if !has_prerequisites {
            return false;
        }
        let retry_interval = self.retry_interval_ms(force);
        self.last_attempt_ms
            .map(|last| now_ms.saturating_sub(last) >= retry_interval)
            .unwrap_or(true)
    }

    pub fn record_attempt(&mut self, now_ms: u64) {
        self.last_attempt_ms = Some(now_ms);
    }

    pub fn record_success(&mut self, interval_ms: u64) {
        self.next_interval_ms = interval_ms.max(ROTATION_MIN_INTERVAL_MS);
        self.consecutive_rejections = 0;
    }

    pub fn record_failure(&mut self) {
        self.consecutive_rejections = self.consecutive_rejections.saturating_add(1);
    }

    fn retry_interval_ms(&self, force: bool) -> u64 {
        let base = if force {
            ROTATION_MIN_INTERVAL_MS
        } else {
            self.next_interval_ms.max(ROTATION_MIN_INTERVAL_MS)
        };
        if self.consecutive_rejections < ROTATION_REJECTIONS_BEFORE_BACKOFF {
            return base;
        }
        let exponent = (self.consecutive_rejections - ROTATION_REJECTIONS_BEFORE_BACKOFF + 1)
            .min(ROTATION_MAX_BACKOFF_EXPONENT);
        base.saturating_mul(1_u64 << exponent).min(ROTATION_MAX_BACKOFF_MS)
    }
}

/// 会话保鲜闸门: 冷却窗口 + 熔断状态机(纯逻辑, 便于单测与并发去重)
#[derive(Debug, Default)]
pub struct YouTubeRefreshGate {
    last_attempt_ms: Option<u64>,
    last_success_ms: Option<u64>,
    consecutive_failures: u32,
    circuit_open_until_ms: Option<u64>,
}

impl YouTubeRefreshGate {
    /// 是否允许发起一次保鲜: 未登录不刷; 熔断期一律不刷(强制也不例外); 其余按冷却窗口
    pub fn should_attempt(&self, now_ms: u64, force: bool, has_login: bool) -> bool {
        if !has_login {
            return false;
        }
        if let Some(until) = self.circuit_open_until_ms {
            if now_ms < until {
                return false;
            }
        }
        match self.last_attempt_ms {
            None => true,
            Some(last) => {
                let cooldown = if force {
                    FORCE_REFRESH_BACKOFF_MS
                } else {
                    REFRESH_COOLDOWN_MS
                };
                now_ms.saturating_sub(last) >= cooldown
            }
        }
    }

    /// 决定发起后立即记录, 使冷却窗口对并发调用生效
    pub fn record_attempt(&mut self, now_ms: u64) {
        self.last_attempt_ms = Some(now_ms);
    }

    /// 成功后清零失败计数并解除熔断
    pub fn record_success(&mut self, now_ms: u64) {
        self.last_success_ms = Some(now_ms);
        self.consecutive_failures = 0;
        self.circuit_open_until_ms = None;
    }

    /// 失败累计, 达到阈值则打开熔断窗口
    pub fn record_failure(&mut self, now_ms: u64) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        if self.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
            self.circuit_open_until_ms = Some(now_ms.saturating_add(CIRCUIT_BREAK_MS));
        }
    }

    pub fn last_success_ms(&self) -> Option<u64> {
        self.last_success_ms
    }
}

/// 当前 Unix 毫秒时间
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn build_cookie_header(auth: &YouTubeAuth) -> String {
    auth.cookies
        .iter()
        .filter(|cookie| !cookie.name.is_empty() && !cookie.value.is_empty())
        .map(|cookie| format!("{}={}", cookie.name, cookie.value))
        .collect::<Vec<_>>()
        .join("; ")
}

fn build_rotation_cookie_header(auth: &YouTubeAuth) -> String {
    ROTATION_REQUEST_COOKIE_KEYS
        .iter()
        .filter_map(|key| {
            auth.cookies
                .iter()
                .find(|cookie| cookie.name == *key && !cookie.value.is_empty())
                .map(|cookie| format!("{}={}", cookie.name, cookie.value))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn has_rotation_prerequisites(auth: &YouTubeAuth) -> bool {
    let has_session = auth.cookies.iter().any(|cookie| {
        (cookie.name == "__Secure-1PSID" || cookie.name == "__Secure-3PSID")
            && !cookie.value.is_empty()
    });
    let has_binding = auth.cookies.iter().any(|cookie| {
        (cookie.name == "SAPISID" || cookie.name == "APISID") && !cookie.value.is_empty()
    });
    has_session && has_binding
}

fn parse_rotation_interval_ms(body: &str) -> u64 {
    let seconds = regex::Regex::new(r#"\"identity\.hfcr\"\s*,\s*(\d+)"#)
        .ok()
        .and_then(|pattern| pattern.captures(body))
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse::<u64>().ok());
    seconds
        .filter(|seconds| *seconds > 0)
        .map(|seconds| seconds.saturating_mul(1000))
        .unwrap_or(ROTATION_DEFAULT_INTERVAL_MS)
}

fn collect_rotated_cookies(headers: &[String]) -> Vec<CookieEntry> {
    let mut rotated = Vec::new();
    for cookie in session::parse_set_cookie_headers(headers) {
        if !ROTATED_COOKIE_KEYS
            .iter()
            .any(|key| *key == cookie.name)
        {
            continue;
        }
        if let Some(existing) = rotated.iter_mut().find(|entry: &&mut CookieEntry| {
            entry.name == cookie.name
        }) {
            *existing = cookie;
        } else {
            rotated.push(cookie);
        }
    }
    rotated
}

async fn request_rotated_cookies(
    http: &FallbackHttp,
    auth: &YouTubeAuth,
) -> AppResult<(Vec<CookieEntry>, u64)> {
    request_rotated_cookies_at(http, auth, ROTATE_COOKIES_URL).await
}

async fn request_rotated_cookies_at(
    http: &FallbackHttp,
    auth: &YouTubeAuth,
    endpoint: &str,
) -> AppResult<(Vec<CookieEntry>, u64)> {
    let cookie_header = build_rotation_cookie_header(auth);
    let response = http
        .send_once(|client| {
            client
                .post(endpoint)
                .header("Origin", ROTATE_COOKIES_ORIGIN)
                .header("Referer", ROTATE_COOKIES_ORIGIN)
                .header("User-Agent", USER_AGENT)
                .header("Content-Type", "application/json")
                .header("Cookie", &cookie_header)
                .body(ROTATE_COOKIES_PAYLOAD)
        })
        .await
        .map_err(|error| AppError::Api(format!("YouTube RotateCookies network: {error}")))?;

    let status = response.status();
    let headers: Vec<String> = response
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_string))
        .collect();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(AppError::Api(format!(
            "YouTube RotateCookies failed: HTTP {status}"
        )));
    }

    Ok((collect_rotated_cookies(&headers), parse_rotation_interval_ms(&body)))
}

/// 主动向 Google 申请 SIDTS 轮换，返回只由该端点签发的新会话
///
/// 页面刷新路径仍然拒收已有 SIDTS，只有这条明确的 RotateCookies 响应允许替换它们
pub async fn rotate_youtube_session(
    http: &FallbackHttp,
    auth: &YouTubeAuth,
) -> AppResult<YouTubeCookieRotationResult> {
    if !has_rotation_prerequisites(auth) {
        return Ok(YouTubeCookieRotationResult {
            updated_auth: None,
            next_interval_ms: ROTATION_DEFAULT_INTERVAL_MS,
        });
    }

    let (rotated, next_interval_ms) = request_rotated_cookies(http, auth).await?;
    if rotated.is_empty() {
        return Ok(YouTubeCookieRotationResult {
            updated_auth: None,
            next_interval_ms,
        });
    }

    let updated = session::merge_rotated_youtube_cookies(auth, &rotated);
    Ok(YouTubeCookieRotationResult {
        updated_auth: session::youtube_auth_changed(auth, &updated).then_some(updated),
        next_interval_ms,
    })
}

/// 解析页面登录信号: Some(true)=已登录, Some(false)=游客, None=无法判断
pub fn html_session_logged_in(html: &str) -> Option<bool> {
    if html.contains("\"LOGGED_IN\":true") || html.contains("\"LOGGED_IN\": true") {
        return Some(true);
    }
    // 存在有效会话 id 同样视为已登录
    if html.contains("\"DELEGATED_SESSION_ID\"") || html.contains("\"USER_SESSION_ID\":\"") {
        return Some(true);
    }
    if html.contains("\"LOGGED_IN\":false") || html.contains("\"LOGGED_IN\": false") {
        return Some(false);
    }
    None
}

async fn fetch_with_cookies(
    http: &FallbackHttp,
    url: &str,
    cookie_header: &str,
) -> AppResult<(Vec<String>, String)> {
    let resp = http
        .send(|client| {
            client
                .get(url)
                .header("User-Agent", USER_AGENT)
                .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
                .header("Cookie", cookie_header)
        })
        .await
        .map_err(|e| AppError::Api(format!("youtube refresh network: {e}")))?;

    let set_cookie: Vec<String> = resp
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_string))
        .collect();
    let body = resp.text().await.unwrap_or_default();
    Ok((set_cookie, body))
}

/// 主动保鲜一次: 加载 music/www 首页, 回收 Set-Cookie 并按身份安全合并
/// 返回 Some(新会话)=cookie 有更新且身份一致; None=无变化;
/// Err=网络失败或明确游客态(交由调用方计入失败, 触发熔断; 但绝不清除本地登录)
pub async fn refresh_youtube_session(
    http: &FallbackHttp,
    auth: &YouTubeAuth,
) -> AppResult<Option<YouTubeAuth>> {
    if !auth.has_login() {
        return Ok(None);
    }
    let cookie_header = build_cookie_header(auth);

    let mut observed = Vec::new();
    let mut saw_logged_in = false;
    let mut saw_guest = false;
    let mut network_ok = false;

    for url in [MUSIC_HOME_URL, WWW_REFRESH_URL] {
        match fetch_with_cookies(http, url, &cookie_header).await {
            Ok((set_cookie, body)) => {
                network_ok = true;
                observed.extend(session::parse_set_cookie_headers(&set_cookie));
                match html_session_logged_in(&body) {
                    Some(true) => saw_logged_in = true,
                    Some(false) => saw_guest = true,
                    None => {}
                }
            }
            Err(e) => {
                log::warn!(target: "youtube-refresh", "load {url} failed: {e}");
            }
        }
    }

    if !network_ok {
        return Err(AppError::Api("youtube refresh network unreachable".into()));
    }

    // 明确游客态且无任何已登录信号: 拒绝合并, 避免用游客 cookie 冲掉完整登录
    if saw_guest && !saw_logged_in {
        return Err(AppError::Api("youtube refresh saw a signed-out session".into()));
    }

    if observed.is_empty() {
        return Ok(None);
    }

    let merged = session::merge_youtube_auth_cookies(auth, &observed);
    if session::youtube_auth_changed(auth, &merged) {
        Ok(Some(merged))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const MIN: u64 = 60 * 1000;

    #[test]
    fn skips_when_not_logged_in() {
        let gate = YouTubeRefreshGate::default();
        assert!(!gate.should_attempt(0, false, false));
        assert!(!gate.should_attempt(0, true, false));
    }

    #[test]
    fn first_attempt_allowed_then_cooldown() {
        let mut gate = YouTubeRefreshGate::default();
        assert!(gate.should_attempt(0, false, true));
        gate.record_attempt(0);
        // 冷却窗口内被拒
        assert!(!gate.should_attempt(MIN, false, true));
        assert!(!gate.should_attempt(REFRESH_COOLDOWN_MS - 1, false, true));
        // 冷却到期后放行
        assert!(gate.should_attempt(REFRESH_COOLDOWN_MS, false, true));
    }

    #[test]
    fn force_uses_shorter_backoff() {
        let mut gate = YouTubeRefreshGate::default();
        gate.record_attempt(0);
        // 常规冷却仍在, 但强制退避已过
        assert!(!gate.should_attempt(FORCE_REFRESH_BACKOFF_MS, false, true));
        assert!(gate.should_attempt(FORCE_REFRESH_BACKOFF_MS, true, true));
        assert!(!gate.should_attempt(FORCE_REFRESH_BACKOFF_MS - 1, true, true));
    }

    #[test]
    fn circuit_breaker_opens_after_threshold_failures() {
        let mut gate = YouTubeRefreshGate::default();
        gate.record_attempt(0);
        gate.record_failure(0);
        // 未达阈值仍可按冷却重试
        assert!(gate.should_attempt(REFRESH_COOLDOWN_MS, false, true));
        gate.record_attempt(REFRESH_COOLDOWN_MS);
        gate.record_failure(REFRESH_COOLDOWN_MS);
        // 达阈值后熔断: 即便强制也不放行
        let after = REFRESH_COOLDOWN_MS + 1;
        assert!(!gate.should_attempt(after, false, true));
        assert!(!gate.should_attempt(after, true, true));
        // 熔断到期后恢复
        let recovered = REFRESH_COOLDOWN_MS + CIRCUIT_BREAK_MS;
        assert!(gate.should_attempt(recovered, true, true));
    }

    #[test]
    fn success_resets_failures_and_circuit() {
        let mut gate = YouTubeRefreshGate::default();
        gate.record_failure(0);
        gate.record_failure(0);
        assert!(!gate.should_attempt(1, true, true));
        gate.record_success(CIRCUIT_BREAK_MS + 1);
        // 成功清零后, 只受冷却约束
        assert!(gate.should_attempt(CIRCUIT_BREAK_MS + 1 + REFRESH_COOLDOWN_MS, false, true));
    }

    #[test]
    fn logged_in_signal_detection() {
        assert_eq!(html_session_logged_in(r#"ytcfg.set({"LOGGED_IN":true});"#), Some(true));
        assert_eq!(html_session_logged_in(r#"{"LOGGED_IN": true}"#), Some(true));
        assert_eq!(html_session_logged_in(r#"{"LOGGED_IN":false}"#), Some(false));
        assert_eq!(
            html_session_logged_in(r#"{"DELEGATED_SESSION_ID":"abc"}"#),
            Some(true)
        );
        assert_eq!(html_session_logged_in("no signal here"), None);
    }

    #[test]
    fn rotation_gate_uses_server_interval_and_force_minimum() {
        let mut gate = YouTubeCookieRotationGate::default();
        assert!(gate.should_attempt(0, false, true));
        gate.record_attempt(0);
        assert!(!gate.should_attempt(ROTATION_DEFAULT_INTERVAL_MS - 1, false, true));
        assert!(gate.should_attempt(ROTATION_DEFAULT_INTERVAL_MS, false, true));

        gate.record_attempt(ROTATION_DEFAULT_INTERVAL_MS);
        assert!(!gate.should_attempt(
            ROTATION_DEFAULT_INTERVAL_MS + ROTATION_MIN_INTERVAL_MS - 1,
            true,
            true,
        ));
        assert!(gate.should_attempt(
            ROTATION_DEFAULT_INTERVAL_MS + ROTATION_MIN_INTERVAL_MS,
            true,
            true,
        ));
    }

    #[test]
    fn rotation_parser_keeps_only_fresh_sidts_values() {
        let headers = vec![
            "__Secure-1PSIDTS=fresh-1; Domain=.google.com; Path=/".into(),
            "__Secure-3PSIDTS=; Max-Age=0; Domain=.google.com; Path=/".into(),
            "SIDCC=ignored; Domain=.google.com; Path=/".into(),
        ];
        let rotated = collect_rotated_cookies(&headers);

        assert_eq!(rotated.len(), 1);
        assert_eq!(rotated[0].name, "__Secure-1PSIDTS");
        assert_eq!(rotated[0].value, "fresh-1");
        assert_eq!(
            parse_rotation_interval_ms(r#")]'[["identity.hfcr",600]]"#),
            ROTATION_DEFAULT_INTERVAL_MS
        );
        assert_eq!(
            parse_rotation_interval_ms(r#")]'[["identity.hfcr",900]]"#),
            900_000
        );
    }

    #[test]
    fn rotation_requires_primary_and_binding_cookies() {
        let auth = YouTubeAuth {
            cookies: vec![CookieEntry {
                name: "SAPISID".into(),
                value: "binding".into(),
                domain: ".google.com".into(),
            }],
            nickname: None,
            avatar_url: None,
            auth_user: None,
        };
        assert!(!has_rotation_prerequisites(&auth));
    }

    #[tokio::test]
    async fn rotation_request_parses_response_without_using_real_google() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/RotateCookies", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let size = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..size]);
            let request_lower = request.to_ascii_lowercase();
            assert!(request_lower.contains("cookie: sapisid=binding"));
            assert!(request.contains(ROTATE_COOKIES_PAYLOAD));
            let body = ")]'[[\"identity.hfcr\",900]]";
            let response = format!(
                "HTTP/1.1 200 OK\r\nSet-Cookie: __Secure-1PSIDTS=fresh; Domain=.google.com; Path=/\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body,
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        });

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let transport = FallbackHttp::new(&client, "youtube-test");
        let auth = YouTubeAuth {
            cookies: vec![
                CookieEntry {
                    name: "SAPISID".into(),
                    value: "binding".into(),
                    domain: ".google.com".into(),
                },
                CookieEntry {
                    name: "__Secure-1PSID".into(),
                    value: "primary".into(),
                    domain: ".google.com".into(),
                },
            ],
            nickname: None,
            avatar_url: None,
            auth_user: None,
        };

        let (rotated, interval_ms) = request_rotated_cookies_at(&transport, &auth, &endpoint)
            .await
            .unwrap();

        assert_eq!(interval_ms, 900_000);
        assert_eq!(rotated.len(), 1);
        assert_eq!(rotated[0].value, "fresh");
        server.await.unwrap();
    }
}
