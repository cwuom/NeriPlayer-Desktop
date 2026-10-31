use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};

// 三平台登录/登出命令
use crate::api::youtube::client::YouTubeAccountProfile;
use crate::webview_args::MainBrowserArgs;
use crate::auth::cookies;
use crate::auth::coordination::{AuthOperation, AuthOperations, AuthPlatform};
use crate::auth::state::{
    AuthInfo, AuthState, AuthStatusResponse, BiliAuth, CookieEntry, NeteaseAuth, YouTubeAuth,
};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

const BILIBILI_LOGIN_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
const AUTH_PROFILE_REFRESH_TIMEOUT: Duration = Duration::from_secs(12);

struct PendingLogin<'a> {
    app: &'a AppHandle,
    state: &'a AppState,
    operation: AuthOperation,
}

impl<'a> PendingLogin<'a> {
    fn new(app: &'a AppHandle, state: &'a AppState, platform: AuthPlatform) -> Self {
        let operation = state.auth_operations.lock().begin_login(platform);
        Self { app, state, operation }
    }
}

impl Drop for PendingLogin<'_> {
    fn drop(&mut self) {
        self.state.auth_operations.lock().finish_login(self.operation);
        schedule_missing_account_profiles(self.app, self.state);
    }
}

fn login_window_label(operation: AuthOperation) -> String {
    format!("{}-login-{}", operation.platform.as_str(), operation.generation())
}

// 登录检测机制：
// 打开 WebviewWindow 加载平台登录页
// 每 800ms 调用 Tauri 内置 cookies_for_url() 读取 Cookie（含 HttpOnly）
// 检测到 sentinel cookie 后关闭窗口，保存 cookie

fn track_login_window_close<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
) -> Arc<AtomicBool> {
    let close_requested = Arc::new(AtomicBool::new(false));
    let event_close_requested = close_requested.clone();
    window.on_window_event(move |event| {
        match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                event_close_requested.store(true, Ordering::Release);
            }
            WindowEvent::Destroyed => {
                event_close_requested.store(true, Ordering::Release);
            }
            _ => {}
        }
    });
    close_requested
}

fn cookie_domain_matches_urls(cookie_domain: &str, cookie_urls: &[&str]) -> bool {
    let cookie_domain = cookie_domain.trim_start_matches('.').to_ascii_lowercase();
    if cookie_domain.is_empty() {
        return false;
    }

    cookie_urls.iter().any(|url_str| {
        url::Url::parse(url_str)
            .ok()
            .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
            .is_some_and(|host| {
                host == cookie_domain || host.ends_with(&format!(".{cookie_domain}"))
            })
    })
}

fn insert_cookie_entry(
    entries: &mut Vec<CookieEntry>,
    name: String,
    value: String,
    domain: String,
) {
    if entries
        .iter()
        .any(|entry| entry.name == name && entry.domain == domain)
    {
        return;
    }
    entries.push(CookieEntry { name, value, domain });
}

fn read_webview_cookies<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    cookie_urls: &[&str],
) -> Vec<CookieEntry> {
    let mut entries = Vec::new();

    if let Ok(all_cookies) = window.cookies() {
        for cookie in all_cookies {
            let domain = cookie.domain().unwrap_or("").to_string();
            if !cookie_domain_matches_urls(&domain, cookie_urls) {
                continue;
            }
            insert_cookie_entry(
                &mut entries,
                cookie.name().to_string(),
                cookie.value().to_string(),
                domain,
            );
        }
    }

    // URL 查询保留为跨平台兜底，避免全量 Cookie API 在旧版 WebView 上不可用
    for url_str in cookie_urls {
        let Ok(url) = url::Url::parse(url_str) else {
            continue;
        };
        let Ok(cookies) = window.cookies_for_url(url) else {
            continue;
        };
        for cookie in cookies {
            insert_cookie_entry(
                &mut entries,
                cookie.name().to_string(),
                cookie.value().to_string(),
                cookie.domain().unwrap_or("").to_string(),
            );
        }
    }

    entries
}

fn has_login_cookie(entries: &[CookieEntry], sentinel_cookie: &str) -> bool {
    login_sentinel_value(entries, sentinel_cookie).is_some()
}

fn login_sentinel_value(entries: &[CookieEntry], sentinel_cookie: &str) -> Option<String> {
    if sentinel_cookie == "SAPISID" {
        return crate::auth::state::youtube_sapisid(entries).map(String::from);
    }
    entries
        .iter()
        .find(|entry| entry.name == sentinel_cookie && !entry.value.is_empty())
        .map(|entry| entry.value.clone())
}

fn has_completed_login(
    entries: &[CookieEntry],
    sentinel_cookie: &str,
    current_url: Option<&url::Url>,
    required_host: Option<&str>,
) -> bool {
    if !has_login_cookie(entries, sentinel_cookie) {
        return false;
    }

    required_host.is_none_or(|host| {
        current_url
            .and_then(url::Url::host_str)
            .is_some_and(|current_host| current_host.eq_ignore_ascii_case(host))
    })
}

fn has_new_completed_login(
    entries: &[CookieEntry],
    sentinel_cookie: &str,
    initial_sentinel: Option<&str>,
    initial_url: Option<&url::Url>,
    current_url: Option<&url::Url>,
    required_host: Option<&str>,
) -> bool {
    if !has_completed_login(entries, sentinel_cookie, current_url, required_host) {
        return false;
    }

    match (initial_sentinel, login_sentinel_value(entries, sentinel_cookie)) {
        (Some(initial), Some(current)) if initial != current => true,
        // 同账号重新授权时平台可能保留原 sentinel 值。只要窗口已离开初始登录页，
        // 且 Cookie/目标 Host 都满足，就不能继续等一个永远不会变化的值
        (Some(_), Some(_)) => matches!(
            (initial_url, current_url),
            (Some(initial), Some(current)) if initial != current
        ),
        (Some(_), None) => false,
        (None, Some(_)) => true,
        (None, None) => false,
    }
}

async fn fetch_youtube_profile(
    state: &AppState,
    auth: &YouTubeAuth,
) -> AppResult<YouTubeAccountProfile> {
    let jar = isolated_cookie_jar(&auth.cookies);
    let (http, alternate) = state.auth_http_clients(jar)?;
    let client = crate::api::youtube::client::YouTubeClient::with_transport(
        crate::api::transport::FallbackHttp::with_fallback(&http, &alternate, "youtube-auth"),
    );
    client.get_account_profile(auth).await
}

fn isolated_cookie_jar(entries: &[CookieEntry]) -> Arc<reqwest::cookie::Jar> {
    let jar = Arc::new(reqwest::cookie::Jar::default());
    cookies::inject_cookies(&jar, entries);
    jar
}

fn filter_youtube_login_cookies(entries: &mut Vec<CookieEntry>) {
    entries.retain(|entry| crate::api::youtube::session::canonical_cookie_key(&entry.name).is_some());
}

fn netease_auth_client(state: &AppState, jar: &Arc<reqwest::cookie::Jar>) -> AppResult<crate::api::netease::client::NeteaseClient> {
    let (http, alternate) = state.auth_http_clients(jar.clone())?;
    Ok(crate::api::netease::client::NeteaseClient::with_fallback(&http, &alternate)
        .with_cookie_jar(jar.clone()))
}

fn bilibili_auth_client(state: &AppState, jar: &Arc<reqwest::cookie::Jar>) -> AppResult<crate::api::bilibili::client::BiliClient> {
    let (http, alternate) = state.auth_http_clients(jar.clone())?;
    Ok(crate::api::bilibili::client::BiliClient::with_transport(
        crate::api::transport::FallbackHttp::with_fallback(&http, &alternate, "bilibili-auth"),
    ).with_cookie_jar(jar.clone()))
}

fn commit_login(
    app: &AppHandle,
    state: &AppState,
    operation: AuthOperation,
    mut candidate: AuthState,
    jar: &Arc<reqwest::cookie::Jar>,
) -> AppResult<AuthInfo> {
    cookies::sync_auth_from_jar(jar, &mut candidate);
    let operations = state.auth_operations.lock();
    let mut auth = state.auth.lock();
    let info = commit_login_state(&operations, &mut auth, &state.cookie_jar, operation, candidate)?;
    cookies::save_auth(app, &auth);
    Ok(info)
}

fn commit_login_state(
    operations: &AuthOperations,
    auth: &mut AuthState,
    shared_jar: &Arc<reqwest::cookie::Jar>,
    operation: AuthOperation,
    candidate: AuthState,
) -> AppResult<AuthInfo> {
    if !operations.is_current(operation) {
        return Err(AppError::Other("Authentication superseded by a newer operation".into()));
    }
    cookies::expire_platform_cookies(shared_jar, auth, operation.platform.as_str());
    cookies::inject_all(shared_jar, &candidate);
    match operation.platform {
        AuthPlatform::Netease => auth.netease = candidate.netease,
        AuthPlatform::Bilibili => auth.bilibili = candidate.bilibili,
        AuthPlatform::Youtube => auth.youtube = candidate.youtube,
    }
    let status = auth.to_status_response();
    Ok(match operation.platform {
        AuthPlatform::Netease => status.netease,
        AuthPlatform::Bilibili => status.bilibili,
        AuthPlatform::Youtube => status.youtube,
    })
}

fn apply_youtube_profile(auth: &mut YouTubeAuth, profile: YouTubeAccountProfile) {
    if profile.nickname.is_some() {
        auth.nickname = profile.nickname;
    }
    if profile.avatar_url.is_some() {
        auth.avatar_url = profile.avatar_url;
    }
}

async fn apply_youtube_profile_best_effort(state: &AppState, auth: &mut YouTubeAuth) {
    match fetch_youtube_profile(state, auth).await {
        Ok(profile) => apply_youtube_profile(auth, profile),
        Err(e) => log::warn!(target: "auth", "YouTube 账号资料获取失败: {}", e),
    }
}

fn youtube_auth_info(auth: &YouTubeAuth) -> AuthInfo {
    AuthInfo {
        platform: "youtube".into(),
        logged_in: auth.has_login(),
        nickname: auth.nickname.clone(),
        avatar_url: auth.avatar_url.clone(),
    }
}

fn youtube_auth_matches(left: &YouTubeAuth, right: &YouTubeAuth) -> bool {
    left.get_sapisid()
        .zip(right.get_sapisid())
        .is_some_and(|(left, right)| left == right)
        && left.selected_auth_user().unwrap_or("0") == right.selected_auth_user().unwrap_or("0")
}

fn with_current_youtube_session<T>(
    operations: &parking_lot::Mutex<AuthOperations>,
    auth: &parking_lot::Mutex<AuthState>,
    operation: AuthOperation,
    previous: &YouTubeAuth,
    update: impl FnOnce() -> T,
) -> Option<T> {
    let operations = operations.lock();
    if !operations.is_current(operation) { return None; }
    let auth = auth.lock();
    let saved = auth.youtube.as_ref().filter(|saved| saved.has_login())?;
    if !youtube_auth_matches(saved, previous) || !youtube_rotating_cookies_match(saved, previous) { return None; }
    Some(update())
}

fn netease_verified_account_id(account: &serde_json::Value) -> Option<u64> {
    if account["code"].as_i64() != Some(200) {
        return None;
    }
    [&account["profile"]["userId"], &account["account"]["id"]]
        .into_iter()
        .find_map(|value| {
            value.as_u64()
                .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
                .filter(|id| *id > 0)
        })
}

fn bili_verified_nav_mid(info: &serde_json::Value, entries: &[CookieEntry]) -> Option<u64> {
    if info["code"].as_i64() != Some(0) || info["data"]["isLogin"].as_bool() != Some(true) {
        return None;
    }
    // nav 属于服务端验证的当前会话，不能让旧 DedeUserID 覆盖它
    info["data"]["mid"].as_u64()
        .or_else(|| info["data"]["mid"].as_str().and_then(|value| value.trim().parse().ok()))
        .filter(|mid| *mid > 0)
        .or_else(|| entries.iter().find(|entry| entry.name == "DedeUserID")
            .and_then(|entry| entry.value.trim().parse().ok()).filter(|mid| *mid > 0))
}

#[cfg(test)]
mod tests {
    use super::{
        bili_verified_nav_mid, cookie_domain_matches_urls, has_completed_login, has_new_completed_login,
        netease_verified_account_id,
    };
    use crate::auth::state::CookieEntry;

    #[test]
    fn new_web_login_has_its_own_window_while_the_previous_poll_is_still_running() {
        let mut operations = crate::auth::coordination::AuthOperations::default();
        for platform in crate::auth::coordination::AuthPlatform::ALL {
            let old = operations.begin_login(platform);
            let current = operations.begin_login(platform);
            let old_label = super::login_window_label(old);
            let current_label = super::login_window_label(current);
            assert_ne!(old_label, current_label);
            assert!(current_label.starts_with(&format!("{}-login-", platform.as_str())));
            assert!(!operations.is_current(old));
        }
    }

    #[test]
    fn rejected_candidate_cookies_cannot_replace_a_new_login_or_restore_a_logout() {
        use reqwest::cookie::CookieStore;
        let entry = |value: &str| CookieEntry { name: "MUSIC_U".into(), value: value.into(), domain: "music.163.com".into() };
        let url = "https://music.163.com".parse().unwrap();
        for current in [Some("new-login"), None] {
            let shared = super::isolated_cookie_jar(&current.map(|value| vec![entry(value)]).unwrap_or_default());
            let candidate = super::isolated_cookie_jar(&[entry("candidate")]);
            super::cookies::inject_cookies(&candidate, &[entry("candidate-server-response")]);
            drop(candidate);
            let saved = shared.cookies(&url).map(|value| value.to_str().unwrap().to_string());
            match current {
                Some(_) => assert!(saved.unwrap().contains("MUSIC_U=new-login")),
                None => assert!(saved.is_none()),
            }
        }
    }

    #[test]
    fn superseded_login_commit_keeps_auth_state_and_shared_cookies_unchanged() {
        use crate::auth::coordination::{AuthOperations, AuthPlatform};
        use reqwest::cookie::CookieStore;
        for platform in AuthPlatform::ALL {
            for logged_out in [false, true] {
                let mut operations = AuthOperations::default();
                let stale = operations.begin_login(platform);
                operations.begin(platform);
                let fixture = serde_json::json!({
                    "netease": {"cookies": [{"name": "MUSIC_U", "value": "current", "domain": "music.163.com"}]},
                    "bilibili": {"cookies": [{"name": "SESSDATA", "value": "current", "domain": ".bilibili.com"}]},
                    "youtube": {"cookies": [{"name": "SAPISID", "value": "current", "domain": ".youtube.com"}]},
                });
                let mut auth: super::AuthState = serde_json::from_value(fixture.clone()).unwrap();
                if logged_out {
                    match platform {
                        AuthPlatform::Netease => auth.netease = None,
                        AuthPlatform::Bilibili => auth.bilibili = None,
                        AuthPlatform::Youtube => auth.youtube = None,
                    }
                }
                let shared = super::isolated_cookie_jar(&[]);
                super::cookies::inject_all(&shared, &auth);
                let urls = ["https://music.163.com", "https://api.bilibili.com", "https://music.youtube.com"];
                let before_cookies: Vec<_> = urls.iter().map(|url| shared.cookies(&url.parse().unwrap())).collect();
                let before_auth = serde_json::to_value(&auth).unwrap();
                let mut candidate: super::AuthState = serde_json::from_value(fixture).unwrap();
                candidate.netease.as_mut().unwrap().cookies[0].value = "stale".into();
                candidate.bilibili.as_mut().unwrap().cookies[0].value = "stale".into();
                candidate.youtube.as_mut().unwrap().cookies[0].value = "stale".into();
                let result = super::commit_login_state(&operations, &mut auth, &shared, stale, candidate);
                assert!(result.unwrap_err().to_string().contains("Authentication superseded"));
                assert_eq!(serde_json::to_value(&auth).unwrap(), before_auth);
                let after_cookies: Vec<_> = urls.iter().map(|url| shared.cookies(&url.parse().unwrap())).collect();
                assert_eq!(after_cookies, before_cookies, "{platform:?}, logged out: {logged_out}");
            }
        }
    }

    #[test]
    fn current_login_commit_replaces_only_its_platform_state_and_cookies() {
        use crate::auth::coordination::{AuthOperations, AuthPlatform};
        use reqwest::cookie::CookieStore;
        let cases = [
            (AuthPlatform::Netease, "MUSIC_U", "music.163.com", "https://music.163.com", "__csrf"),
            (AuthPlatform::Bilibili, "SESSDATA", ".bilibili.com", "https://api.bilibili.com", "bili_jct"),
            (AuthPlatform::Youtube, "SAPISID", ".youtube.com", "https://music.youtube.com", "SIDCC"),
        ];
        for (platform, sentinel, domain, url, jar_only_name) in cases {
            let mut operations = AuthOperations::default();
            let operation = operations.begin_login(platform);
            let mut auth: super::AuthState = serde_json::from_value(serde_json::json!({
                "netease": {"cookies": [{"name": "MUSIC_U", "value": "old", "domain": "music.163.com"}]},
                "bilibili": {"cookies": [{"name": "SESSDATA", "value": "old", "domain": ".bilibili.com"}]},
                "youtube": {"cookies": [{"name": "SAPISID", "value": "old", "domain": ".youtube.com"}]},
            })).unwrap();
            let candidate: super::AuthState = serde_json::from_value(serde_json::json!({
                (platform.as_str()): {"cookies": [{"name": sentinel, "value": "new", "domain": domain}], "nickname": "new account"},
            })).unwrap();
            let shared = super::isolated_cookie_jar(&[]);
            super::cookies::inject_all(&shared, &auth);
            super::cookies::inject_cookies(&shared, &[CookieEntry {
                name: jar_only_name.into(), value: "jar-only-old".into(), domain: domain.into(),
            }]);
            if platform == AuthPlatform::Youtube {
                super::cookies::inject_cookies(&shared, &[
                    CookieEntry { name: "__Secure-1PSIDTS".into(), value: "first-jar-only".into(), domain: "music.youtube.com".into() },
                    CookieEntry { name: "__Secure-3PSIDTS".into(), value: "third-jar-only".into(), domain: "music.youtube.com".into() },
                ]);
            }
            let current_url = url.parse().unwrap();
            assert!(shared.cookies(&current_url).unwrap().to_str().unwrap().contains(&format!("{jar_only_name}=jar-only-old")));
            let info = super::commit_login_state(&operations, &mut auth, &shared, operation, candidate).unwrap();
            assert!(info.logged_in);
            assert_eq!(info.nickname.as_deref(), Some("new account"));
            let saved = serde_json::to_value(&auth).unwrap();
            for (other, other_sentinel, _, other_url, _) in cases {
                let expected = if other == platform { "new" } else { "old" };
                assert_eq!(saved[other.as_str()]["cookies"][0]["value"], expected);
                assert!(shared.cookies(&other_url.parse().unwrap()).unwrap().to_str().unwrap().contains(&format!("{other_sentinel}={expected}")));
            }
            let current_cookies = shared.cookies(&current_url).unwrap();
            let current_cookies = current_cookies.to_str().unwrap();
            assert!(!current_cookies.contains(&format!("{jar_only_name}=")), "{platform:?}");
            if platform == AuthPlatform::Youtube {
                assert!(current_cookies.contains("__Secure-1PSIDTS=first-jar-only"));
                assert!(current_cookies.contains("__Secure-3PSIDTS=third-jar-only"));
            }
        }
    }

    fn youtube_refresh_fixture() -> super::YouTubeAuth {
        serde_json::from_value(serde_json::json!({
            "cookies": [
                {"name": "SAPISID", "value": "identity", "domain": ".youtube.com"},
                {"name": "SID", "value": "sid", "domain": ".youtube.com"},
                {"name": "__Secure-1PSIDTS", "value": "first-old", "domain": ".google.com"},
                {"name": "__Secure-3PSIDTS", "value": "third-old", "domain": ".google.com"},
            ],
            "nickname": "saved account", "avatar_url": "saved.png", "auth_user": "2",
        })).unwrap()
    }

    #[test]
    fn stale_youtube_page_refresh_cannot_overwrite_rotated_session() {
        use crate::api::youtube::session::{merge_rotated_youtube_cookies, merge_youtube_auth_cookies};
        for name in ["__Secure-1PSIDTS", "__Secure-3PSIDTS"] {
            for previously_present in [false, true] {
                let mut snapshot = youtube_refresh_fixture();
                if !previously_present { snapshot.cookies.retain(|cookie| cookie.name != name); }
                let page_result = merge_youtube_auth_cookies(&snapshot, &[CookieEntry {
                    name: "SIDCC".into(), value: "page-update".into(), domain: ".youtube.com".into(),
                }]);
                let mut saved = merge_rotated_youtube_cookies(&snapshot, &[CookieEntry {
                    name: name.into(), value: "rotation-update".into(), domain: ".google.com".into(),
                }]);
                let before = serde_json::to_value(&saved).unwrap();
                assert!(!super::merge_youtube_session_update_state(
                    &mut saved, &snapshot, page_result,
                ), "{name}, previous token present: {previously_present}");
                assert_eq!(serde_json::to_value(&saved).unwrap(), before);
            }
        }
    }

    #[test]
    fn youtube_page_refresh_updates_unrotated_session() {
        let snapshot = youtube_refresh_fixture();
        let mut saved = snapshot.clone();
        saved.cookies.reverse();
        for cookie in &mut saved.cookies {
            if cookie.name.ends_with("PSIDTS") { cookie.domain = "GOOGLE.COM".into(); }
        }
        let mut page_result = crate::api::youtube::session::merge_youtube_auth_cookies(&snapshot, &[CookieEntry {
            name: "SIDCC".into(), value: "page-update".into(), domain: ".youtube.com".into(),
        }]);
        page_result.nickname = None;
        page_result.avatar_url = None;
        assert!(super::merge_youtube_session_update_state(
            &mut saved, &snapshot, page_result,
        ));
        assert!(saved.cookies.iter().any(|cookie| cookie.name == "SIDCC" && cookie.value == "page-update"));
        assert_eq!(saved.nickname.as_deref(), Some("saved account"));
        assert_eq!(saved.avatar_url.as_deref(), Some("saved.png"));
        assert_eq!(saved.selected_auth_user(), Some("2"));
    }

    #[test]
    fn youtube_page_refresh_rejects_a_changed_rotating_cookie_domain() {
        let snapshot = youtube_refresh_fixture();
        let mut saved = snapshot.clone();
        saved.cookies.iter_mut().find(|cookie| cookie.name == "__Secure-1PSIDTS").unwrap().domain = "accounts.google.com".into();
        let before = serde_json::to_value(&saved).unwrap();
        assert!(!super::merge_youtube_session_update_state(
            &mut saved, &snapshot, snapshot.clone(),
        ));
        assert_eq!(serde_json::to_value(&saved).unwrap(), before);
    }

    #[test]
    fn youtube_cookie_rotation_can_update_rotating_session_tokens() {
        let snapshot = youtube_refresh_fixture();
        let mut saved = snapshot.clone();
        let updated = crate::api::youtube::session::merge_rotated_youtube_cookies(&snapshot, &[
            CookieEntry { name: "__Secure-1PSIDTS".into(), value: "first-new".into(), domain: ".google.com".into() },
            CookieEntry { name: "__Secure-3PSIDTS".into(), value: "third-new".into(), domain: ".google.com".into() },
        ]);
        assert!(super::merge_youtube_session_update_state(
            &mut saved, &snapshot, updated,
        ));
        for (name, value) in [("__Secure-1PSIDTS", "first-new"), ("__Secure-3PSIDTS", "third-new")] {
            assert!(saved.cookies.iter().any(|cookie| cookie.name == name && cookie.value == value));
        }
    }

    #[test]
    fn overlapping_youtube_cookie_rotations_cannot_restore_an_older_snapshot() {
        use crate::api::youtube::session::merge_rotated_youtube_cookies;
        for name in ["__Secure-1PSIDTS", "__Secure-3PSIDTS"] {
            let snapshot = youtube_refresh_fixture();
            let first_result = merge_rotated_youtube_cookies(&snapshot, &[CookieEntry {
                name: name.into(), value: "committed-rotation".into(), domain: ".google.com".into(),
            }]);
            let delayed_result = merge_rotated_youtube_cookies(&snapshot, &[CookieEntry {
                name: name.into(), value: "late-rotation".into(), domain: ".google.com".into(),
            }]);
            let mut saved = snapshot.clone();
            assert!(super::merge_youtube_session_update_state(
                &mut saved, &snapshot, first_result,
            ));
            let committed = serde_json::to_value(&saved).unwrap();
            assert!(!super::merge_youtube_session_update_state(
                &mut saved, &snapshot, delayed_result,
            ), "{name}");
            assert_eq!(serde_json::to_value(&saved).unwrap(), committed);
        }
    }

    #[test]
    fn stale_youtube_rotating_snapshot_cannot_change_gate_success_or_failure() {
        use crate::api::youtube::refresh::{YouTubeCookieRotationGate, ROTATION_DEFAULT_INTERVAL_MS};
        use crate::auth::coordination::{AuthOperations, AuthPlatform};
        for success in [false, true] {
            let operations = parking_lot::Mutex::new(AuthOperations::default());
            let operation = operations.lock().begin(AuthPlatform::Youtube);
            let snapshot = youtube_refresh_fixture();
            let current = crate::api::youtube::session::merge_rotated_youtube_cookies(&snapshot, &[CookieEntry {
                name: "__Secure-1PSIDTS".into(), value: "committed-rotation".into(), domain: ".google.com".into(),
            }]);
            let auth = parking_lot::Mutex::new(super::AuthState { youtube: Some(current), ..Default::default() });
            let mut gate = YouTubeCookieRotationGate::default();
            gate.record_attempt(0);
            let applied = super::with_current_youtube_session(&operations, &auth, operation, &snapshot, || {
                if success { gate.record_success(60_000); }
                else { for _ in 0..3 { gate.record_failure(); } }
            });
            assert!(applied.is_none(), "success: {success}");
            assert!(!gate.should_attempt(60_000, false, true));
            assert!(gate.should_attempt(ROTATION_DEFAULT_INTERVAL_MS, false, true));
        }
    }

    #[test]
    fn stale_youtube_rotation_responses_cannot_change_the_current_gate() {
        use crate::api::youtube::refresh::{YouTubeCookieRotationGate, ROTATION_DEFAULT_INTERVAL_MS};
        use crate::auth::coordination::{AuthOperations, AuthPlatform};
        for changed_generation in [false, true] {
            let operations = parking_lot::Mutex::new(AuthOperations::default());
            let operation = operations.lock().begin(AuthPlatform::Youtube);
            let previous: super::YouTubeAuth = serde_json::from_value(serde_json::json!({
                "cookies": [{"name": "SAPISID", "value": "old", "domain": ".youtube.com"}],
            })).unwrap();
            let mut current = previous.clone();
            if !changed_generation { current.cookies[0].value = "new".into(); }
            let auth = parking_lot::Mutex::new(super::AuthState { youtube: Some(current), ..Default::default() });
            if changed_generation { operations.lock().begin(AuthPlatform::Youtube); }
            let mut gate = YouTubeCookieRotationGate::default();
            gate.record_attempt(0);
            let applied = super::with_current_youtube_session(&operations, &auth, operation, &previous, || gate.record_success(60_000));
            assert!(applied.is_none(), "generation changed: {changed_generation}");
            assert!(!gate.should_attempt(60_000, false, true));
            assert!(gate.should_attempt(ROTATION_DEFAULT_INTERVAL_MS, false, true));
        }
    }

    #[test]
    fn current_youtube_rotation_response_updates_the_gate() {
        use crate::api::youtube::refresh::YouTubeCookieRotationGate;
        use crate::auth::coordination::{AuthOperations, AuthPlatform};
        let operations = parking_lot::Mutex::new(AuthOperations::default());
        let operation = operations.lock().begin(AuthPlatform::Youtube);
        let current: super::YouTubeAuth = serde_json::from_value(serde_json::json!({
            "cookies": [{"name": "SAPISID", "value": "current", "domain": ".youtube.com"}],
            "auth_user": "2",
        })).unwrap();
        let auth = parking_lot::Mutex::new(super::AuthState { youtube: Some(current.clone()), ..Default::default() });
        let mut gate = YouTubeCookieRotationGate::default();
        gate.record_attempt(0);
        assert!(super::with_current_youtube_session(&operations, &auth, operation, &current, || gate.record_success(60_000)).is_some());
        assert!(gate.should_attempt(60_000, false, true));
    }

    #[test]
    fn youtube_login_persists_known_session_cookies_without_unrelated_prefixed_keys() {
        let required = ["SAPISID", "__Secure-1PSID", "__Secure-3PSID", "__Secure-1PAPISID", "__Secure-3PAPISID", "__Secure-1PSIDTS", "__Secure-3PSIDTS", "__Secure-1PSIDCC", "__Secure-3PSIDCC"];
        let mut entries: Vec<_> = required.into_iter().chain(["__Secure-unrelated", "__Host-unrelated"]).map(|name| CookieEntry {
            name: name.into(), value: "fixture".into(), domain: ".google.com".into(),
        }).collect();
        super::filter_youtube_login_cookies(&mut entries);
        assert_eq!(entries.iter().map(|entry| entry.name.as_str()).collect::<Vec<_>>(), required);
    }

    #[test]
    fn youtube_login_accepts_secure_sapisid_aliases_after_music_landing() {
        let initial_url = url::Url::parse("https://accounts.google.com/ServiceLogin").unwrap();
        let music_url = url::Url::parse("https://music.youtube.com/").unwrap();
        for name in ["__Secure-1PAPISID", "__Secure-3PAPISID"] {
            let entries = [CookieEntry { name: name.into(), value: "alias-session".into(), domain: ".youtube.com".into() }];
            assert_eq!(super::login_sentinel_value(&entries, "SAPISID").as_deref(), Some("alias-session"));
            assert!(!has_completed_login(&entries, "SAPISID", Some(&initial_url), Some("music.youtube.com")));
            assert!(has_completed_login(&entries, "SAPISID", Some(&music_url), Some("music.youtube.com")));
            assert!(!has_new_completed_login(&entries, "SAPISID", Some("alias-session"), Some(&music_url), Some(&music_url), Some("music.youtube.com")));
            assert!(has_new_completed_login(&entries, "SAPISID", Some("alias-session"), Some(&initial_url), Some(&music_url), Some("music.youtube.com")));
            assert!(has_new_completed_login(&entries, "SAPISID", Some("older-session"), Some(&music_url), Some(&music_url), Some("music.youtube.com")));
        }
    }

    #[test]
    fn youtube_login_rejects_google_only_sapisid_even_after_music_landing() {
        let music_url = url::Url::parse("https://music.youtube.com/").unwrap();
        for name in ["SAPISID", "__Secure-1PAPISID", "__Secure-3PAPISID"] {
            let entries = [CookieEntry { name: name.into(), value: "google-only".into(), domain: ".google.com".into() }];
            assert!(super::login_sentinel_value(&entries, "SAPISID").is_none());
            assert!(!has_completed_login(&entries, "SAPISID", Some(&music_url), Some("music.youtube.com")));
        }
    }

    const BILIBILI_URLS: &[&str] = &[
        "https://www.bilibili.com",
        "https://passport.bilibili.com",
        "https://api.bilibili.com",
    ];

    #[test]
    fn netease_cookie_validation_requires_success_and_a_positive_server_identity() {
        for account in [
            serde_json::json!({"code": 200, "profile": {"userId": 123}}),
            serde_json::json!({"code": 200, "profile": {"userId": "123"}}),
            serde_json::json!({"code": 200, "account": {"id": 123}}),
        ] {
            assert_eq!(netease_verified_account_id(&account), Some(123));
        }
        for account in [
            serde_json::json!({"code": 200, "profile": null}),
            serde_json::json!({"code": 200, "profile": {"userId": 0}}),
            serde_json::json!({"code": 301, "profile": {"userId": 123}}),
            serde_json::json!({"code": 200, "profile": {"userId": -1}}),
        ] {
            assert_eq!(netease_verified_account_id(&account), None);
        }
    }

    #[test]
    fn bilibili_imported_session_resolves_verified_nav_identity_without_dedeuserid_cookie() {
        let nav = serde_json::json!({ "code": 0, "data": { "isLogin": true, "mid": 123 } });
        assert_eq!(bili_verified_nav_mid(&nav, &[]), Some(123));
        let nav = serde_json::json!({ "code": 0, "data": { "isLogin": true, "mid": "123" } });
        assert_eq!(bili_verified_nav_mid(&nav, &[]), Some(123));
    }

    #[test]
    fn bilibili_verified_nav_identity_wins_over_stale_cookie_and_fallback_is_positive() {
        let entries = [CookieEntry { name: "DedeUserID".into(), value: "999".into(), domain: ".bilibili.com".into() }];
        let nav = serde_json::json!({ "code": 0, "data": { "isLogin": true, "mid": 123 } });
        assert_eq!(bili_verified_nav_mid(&nav, &entries), Some(123));
        let nav = serde_json::json!({ "code": 0, "data": { "isLogin": true } });
        assert_eq!(bili_verified_nav_mid(&nav, &entries), Some(999));
        let invalid = [CookieEntry { value: "0".into(), ..entries[0].clone() }];
        assert_eq!(bili_verified_nav_mid(&nav, &invalid), None);
        let nav = serde_json::json!({ "code": -101, "data": { "isLogin": false, "mid": 123 } });
        assert_eq!(bili_verified_nav_mid(&nav, &entries), None);
    }

    #[test]
    fn parent_cookie_domain_matches_bilibili_subdomains() {
        assert!(cookie_domain_matches_urls(".bilibili.com", BILIBILI_URLS));
    }

    #[test]
    fn host_cookie_domain_matches_exact_url() {
        assert!(cookie_domain_matches_urls(
            "passport.bilibili.com",
            BILIBILI_URLS
        ));
    }

    #[test]
    fn unrelated_cookie_domain_is_rejected() {
        assert!(!cookie_domain_matches_urls("evilbilibili.com", BILIBILI_URLS));
    }

    #[test]
    fn youtube_google_sapisid_does_not_finish_login_before_music_landing() {
        let entries = vec![CookieEntry {
            name: "SAPISID".into(),
            value: "google-session".into(),
            domain: "google.com".into(),
        }];
        let current_url = url::Url::parse("https://accounts.google.com/ServiceLogin").unwrap();

        assert!(!has_completed_login(
            &entries,
            "SAPISID",
            Some(&current_url),
            Some("music.youtube.com"),
        ));
    }

    #[test]
    fn youtube_login_finishes_after_music_landing() {
        let entries = vec![CookieEntry {
            name: "SAPISID".into(),
            value: "google-session".into(),
            domain: ".youtube.com".into(),
        }];
        let current_url = url::Url::parse("https://music.youtube.com/").unwrap();

        assert!(has_completed_login(
            &entries,
            "SAPISID",
            Some(&current_url),
            Some("music.youtube.com"),
        ));
    }

    #[test]
    fn unchanged_sentinel_does_not_finish_before_navigation() {
        let entries = vec![CookieEntry {
            name: "MUSIC_U".into(),
            value: "old-session".into(),
            domain: "music.163.com".into(),
        }];
        let initial_url = url::Url::parse("https://music.163.com/#/login").unwrap();
        let current_url = url::Url::parse("https://music.163.com/#/login").unwrap();

        assert!(!has_new_completed_login(
            &entries,
            "MUSIC_U",
            Some("old-session"),
            Some(&initial_url),
            Some(&current_url),
            None,
        ));
    }

    #[test]
    fn unchanged_sentinel_finishes_after_leaving_login_page() {
        let entries = vec![CookieEntry {
            name: "MUSIC_U".into(),
            value: "old-session".into(),
            domain: "music.163.com".into(),
        }];
        let initial_url = url::Url::parse("https://music.163.com/#/login").unwrap();
        let current_url = url::Url::parse("https://music.163.com/#/discover/recommend").unwrap();

        assert!(has_new_completed_login(
            &entries,
            "MUSIC_U",
            Some("old-session"),
            Some(&initial_url),
            Some(&current_url),
            None,
        ));
    }

    #[test]
    fn changed_sentinel_finishes_after_required_host_is_reached() {
        let entries = vec![CookieEntry {
            name: "SAPISID".into(),
            value: "new-session".into(),
            domain: ".youtube.com".into(),
        }];
        let current_url = url::Url::parse("https://music.youtube.com/").unwrap();

        assert!(has_new_completed_login(
            &entries,
            "SAPISID",
            Some("old-session"),
            None,
            Some(&current_url),
            Some("music.youtube.com"),
        ));
    }
}

/// 从 WebView 窗口轮询提取 Cookie（使用 Tauri 内置 API 读取 HttpOnly）
async fn poll_webview_cookies(
    pending: &PendingLogin<'_>,
    window_label: &str,
    sentinel_cookie: &str,
    required_host: Option<&str>,
    cookie_urls: &[&str],
    timeout_secs: u64,
    close_requested: &AtomicBool,
) -> AppResult<Vec<CookieEntry>> {
    let app = pending.app;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    let poll_interval = Duration::from_millis(800);
    let initial_window = app.get_webview_window(window_label);
    let initial_sentinel = initial_window
        .as_ref()
        .map(|window| read_webview_cookies(window, cookie_urls))
        .and_then(|entries| login_sentinel_value(&entries, sentinel_cookie));
    let initial_url = initial_window.and_then(|window| window.url().ok());

    loop {
        if !pending.state.auth_operations.lock().is_current(pending.operation) {
            if let Some(window) = app.get_webview_window(window_label) {
                let _ = window.destroy();
            }
            return Err(AppError::Other("Authentication superseded by a newer operation".into()));
        }
        if close_requested.load(Ordering::Acquire) {
            if let Some(window) = app.get_webview_window(window_label) {
                let entries = read_webview_cookies(&window, cookie_urls);
                // 用户主动关闭代表流程已结束，此处只验证 Cookie 和目标 Host。
                // 不要求 sentinel 变化，否则同账号重新授权会被错误判为取消
                let current_url = window.url().ok();
                let login_succeeded = has_completed_login(
                    &entries,
                    sentinel_cookie,
                    current_url.as_ref(),
                    required_host,
                );
                let _ = window.destroy();
                if login_succeeded {
                    return Ok(entries);
                }
            }
            return Err(AppError::Other("Login cancelled".into()));
        }

        if tokio::time::Instant::now() > deadline {
            if let Some(w) = app.get_webview_window(window_label) {
                let _ = w.destroy();
            }
            return Err(AppError::Other("Login timeout".into()));
        }

        // 检测窗口是否仍然存在
        let window = match app.get_webview_window(window_label) {
            Some(w) => w,
            None => return Err(AppError::Other("Login cancelled".into())),
        };

        let all_entries = read_webview_cookies(&window, cookie_urls);
        let current_url = window.url().ok();

        if has_new_completed_login(
            &all_entries,
            sentinel_cookie,
            initial_sentinel.as_deref(),
            initial_url.as_ref(),
            current_url.as_ref(),
            required_host,
        ) {
            let _ = window.destroy();
            return Ok(all_entries);
        }

        tokio::time::sleep(poll_interval).await;
    }
}

/// 网易云登录
#[tauri::command]
pub async fn login_netease(app: AppHandle, state: State<'_, AppState>) -> AppResult<AuthInfo> {
    let pending = PendingLogin::new(&app, &state, AuthPlatform::Netease);
    let operation = pending.operation;
    let label = login_window_label(operation);
    let window = WebviewWindowBuilder::new(
        &app,
        &label,
        WebviewUrl::External("https://music.163.com/#/login".parse().unwrap()),
    )
    .title("NeriPlayer - 网易云音乐登录")
    .inner_size(420.0, 600.0)
    .center()
    .main_browser_args(&app)
    .build()
    .map_err(|e| AppError::Other(format!("Failed to create login window: {}", e)))?;
    let close_requested = track_login_window_close(&window);
    drop(window);

    let cookie_urls = &[
        "https://music.163.com",
        "https://interface.music.163.com",
        "https://interface3.music.163.com",
    ];
    let mut entries = poll_webview_cookies(
        &pending, &label, "MUSIC_U", None, cookie_urls, 300, &close_requested,
    ).await?;

    // 补全默认 Cookie（与 Android 一致）
    if !entries.iter().any(|c| c.name == "os") {
        entries.push(CookieEntry { name: "os".into(), value: "pc".into(), domain: "music.163.com".into() });
    }
    if !entries.iter().any(|c| c.name == "appver") {
        entries.push(CookieEntry { name: "appver".into(), value: "8.10.35".into(), domain: "music.163.com".into() });
    }

    let jar = isolated_cookie_jar(&entries);

    // 调用 API 获取用户信息
    let client = netease_auth_client(&state, &jar)?;
    let (user_id, nickname, avatar_url) = match client.get_user_account().await {
        Ok(account) => {
            let profile = &account["profile"];
            (
                profile["userId"].as_u64(),
                profile["nickname"].as_str().map(String::from),
                profile["avatarUrl"].as_str().map(String::from),
            )
        }
        Err(_) => (None, None, None),
    };

    let auth = NeteaseAuth { cookies: entries, user_id, nickname: nickname.clone(), avatar_url: avatar_url.clone() };
    commit_login(&app, &state, operation, AuthState { netease: Some(auth), ..AuthState::default() }, &jar)
}

/// B站登录
#[tauri::command]
pub async fn login_bilibili(app: AppHandle, state: State<'_, AppState>) -> AppResult<AuthInfo> {
    let pending = PendingLogin::new(&app, &state, AuthPlatform::Bilibili);
    let operation = pending.operation;
    let label = login_window_label(operation);
    let window = WebviewWindowBuilder::new(
        &app,
        &label,
        WebviewUrl::External("https://passport.bilibili.com/login".parse().unwrap()),
    )
    .title("NeriPlayer - 哔哩哔哩登录")
    .inner_size(420.0, 600.0)
    .center()
    .user_agent(BILIBILI_LOGIN_USER_AGENT)
    .main_browser_args(&app)
    .build()
    .map_err(|e| AppError::Other(format!("Failed to create login window: {}", e)))?;
    let close_requested = track_login_window_close(&window);
    drop(window);

    let cookie_urls = &[
        "https://www.bilibili.com",
        "https://passport.bilibili.com",
        "https://api.bilibili.com",
    ];
    let mut entries = poll_webview_cookies(
        &pending, &label, "SESSDATA", None, cookie_urls, 300, &close_requested,
    ).await?;

    // B 站登录 cookie 必须关联到 .bilibili.com 域，确保 api.bilibili.com 子域名也能发送
    let bili_core_cookies = ["SESSDATA", "DedeUserID", "DedeUserID__ckMd5", "bili_jct", "sid"];
    for entry in &mut entries {
        if bili_core_cookies.contains(&entry.name.as_str()) && !entry.domain.starts_with('.') {
            entry.domain = ".bilibili.com".to_string();
        }
    }

    let jar = isolated_cookie_jar(&entries);

    // 从 Cookie 提取 DedeUserID
    let mid = entries.iter()
        .find(|c| c.name == "DedeUserID")
        .and_then(|c| c.value.parse::<u64>().ok());

    // 调用 B站 nav API 获取用户信息
    let client = bilibili_auth_client(&state, &jar)?;
    let (mid, nickname, avatar_url) = match client.get_user_info().await {
        Ok(info) => {
            let data = &info["data"];
            // 必须检查 isLogin，未登录时 data 中无有效用户信息
            let is_login = data["isLogin"].as_bool().unwrap_or(false);
            if is_login {
                (
                    bili_verified_nav_mid(&info, &entries),
                    data["uname"].as_str().map(String::from),
                    data["face"].as_str().map(String::from),
                )
            } else {
                log::warn!(target: "auth", "Bilibili nav API 返回 isLogin=false，cookie 可能未生效");
                (mid, None, None)
            }
        }
        Err(e) => {
            log::warn!(target: "auth", "Bilibili get_user_info 失败: {}", e);
            (mid, None, None)
        }
    };

    let auth = BiliAuth { cookies: entries, mid, nickname: nickname.clone(), avatar_url: avatar_url.clone() };
    commit_login(&app, &state, operation, AuthState { bilibili: Some(auth), ..AuthState::default() }, &jar)
}

/// YouTube Music 登录
#[tauri::command]
pub async fn login_youtube(app: AppHandle, state: State<'_, AppState>) -> AppResult<AuthInfo> {
    let pending = PendingLogin::new(&app, &state, AuthPlatform::Youtube);
    let operation = pending.operation;
    let login_url = "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F";
    let label = login_window_label(operation);

    let window = WebviewWindowBuilder::new(
        &app,
        &label,
        WebviewUrl::External(login_url.parse().unwrap()),
    )
    .title("NeriPlayer - YouTube Music Login")
    .inner_size(480.0, 680.0)
    .center()
    .main_browser_args(&app)
    .build()
    .map_err(|e| AppError::Other(format!("Failed to create login window: {}", e)))?;
    let close_requested = track_login_window_close(&window);
    drop(window);

    // YouTube cookie 分布在多个域
    let cookie_urls = &[
        "https://music.youtube.com",
        "https://www.youtube.com",
        "https://youtube.com",
        "https://accounts.google.com",
        "https://www.google.com",
        "https://google.com",
        "https://m.youtube.com",
    ];
    let entries = poll_webview_cookies(
        &pending, &label, "SAPISID", Some("music.youtube.com"), cookie_urls, 300, &close_requested,
    ).await?;

    // 入库前按白名单过滤: google 账号域会带 LSID 等超出播放所需的整套会话 cookie,
    // 只保留 session 持久化白名单键, 对齐 Android sanitizePersistedCookies（AU-13）
    let mut entries = entries;
    filter_youtube_login_cookies(&mut entries);

    let jar = isolated_cookie_jar(&entries);

    let mut auth = YouTubeAuth {
        cookies: entries,
        nickname: None,
        avatar_url: None,
        auth_user: None,
    };
    apply_youtube_profile_best_effort(&state, &mut auth).await;
    commit_login(&app, &state, operation, AuthState { youtube: Some(auth), ..AuthState::default() }, &jar)
}

/// Cookie 粘贴登录（对齐 Android 端）
#[tauri::command]
pub async fn login_with_cookies(
    platform: String,
    raw_cookies: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<AuthInfo> {
    let auth_platform = AuthPlatform::from_name(&platform)
        .ok_or_else(|| AppError::Other(format!("Unknown platform: {platform}")))?;
    let pending = PendingLogin::new(&app, &state, auth_platform);
    let operation = pending.operation;
    let mut entries = cookies::parse_raw_cookie_text(&raw_cookies, &platform);

    if entries.is_empty() {
        return Err(AppError::Other("No valid cookies found".into()));
    }

    if platform == "youtube" {
        filter_youtube_login_cookies(&mut entries);
    }

    // 候选凭据只进入隔离 Jar，验证中的响应不能污染当前登录
    match platform.as_str() {
        "netease" if !entries.iter().any(|c| c.name == "MUSIC_U" && !c.value.is_empty()) => {
            return Err(AppError::Other("Missing required cookie: MUSIC_U".into()));
        }
        "bilibili" if !entries.iter().any(|c| c.name == "SESSDATA" && !c.value.is_empty()) => {
            return Err(AppError::Other("Missing required cookie: SESSDATA".into()));
        }
        "netease" | "bilibili" | "youtube" => {}
        _ => return Err(AppError::Other(format!("Unknown platform: {}", platform))),
    }

    let jar = isolated_cookie_jar(&entries);

    let result = match platform.as_str() {
        "netease" => {
            let client = netease_auth_client(&state, &jar)?;
            let (user_id, nickname, avatar_url) = match client.get_user_account().await {
                Ok(account) => {
                    let profile = &account["profile"];
                    let Some(user_id) = netease_verified_account_id(&account) else {
                        return Err(AppError::Other("Cookie 验证失败：网易云未返回有效登录身份".into()));
                    };
                    (
                        Some(user_id),
                        profile["nickname"].as_str().map(String::from),
                        profile["avatarUrl"].as_str().map(String::from),
                    )
                }
                Err(e) => {
                    return Err(AppError::Other(format!("Cookie validation failed: {e}")));
                }
            };

            let auth = NeteaseAuth { cookies: entries, user_id, nickname: nickname.clone(), avatar_url: avatar_url.clone() };
            commit_login(&app, &state, operation, AuthState { netease: Some(auth), ..AuthState::default() }, &jar)
        }
        "bilibili" => {
            let client = bilibili_auth_client(&state, &jar)?;
            let (mid, nickname, avatar_url) = match client.get_user_info().await {
                Ok(info) => {
                    let data = &info["data"];
                    if let Some(mid) = bili_verified_nav_mid(&info, &entries) {
                        (Some(mid), data["uname"].as_str().map(String::from), data["face"].as_str().map(String::from))
                    } else {
                        return Err(AppError::Other("Cookie 验证失败：B站未返回有效登录身份".into()));
                    }
                }
                Err(e) => {
                    return Err(AppError::Other(format!("Cookie validation failed: {e}")));
                }
            };

            let auth = BiliAuth { cookies: entries, mid, nickname: nickname.clone(), avatar_url: avatar_url.clone() };
            commit_login(&app, &state, operation, AuthState { bilibili: Some(auth), ..AuthState::default() }, &jar)
        }
        "youtube" => {
            let auth_user = raw_cookies.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if !name.trim().eq_ignore_ascii_case("x-goog-authuser") {
                    return None;
                }
                crate::auth::state::youtube_auth_user(value).map(String::from)
            });
            let mut auth = YouTubeAuth {
                cookies: entries,
                nickname: None,
                avatar_url: None,
                auth_user,
            };
            if !auth.has_login() {
                return Err(AppError::Other("Missing required cookie: SAPISID / __Secure-1PAPISID / __Secure-3PAPISID".into()));
            }

            apply_youtube_profile_best_effort(&state, &mut auth).await;
            commit_login(&app, &state, operation, AuthState { youtube: Some(auth), ..AuthState::default() }, &jar)
        }
        _ => Err(AppError::Other(format!("Unknown platform: {}", platform))),
    };

    result
}

/// 刷新已保存的 YouTube Music 账号资料
#[tauri::command]
pub async fn refresh_youtube_profile(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<AuthInfo> {
    let (operation, current_auth) = {
        let operations = state.auth_operations.lock();
        let auth_state = state.auth.lock();
        let auth = auth_state
            .youtube
            .clone()
            .filter(YouTubeAuth::has_login)
            .ok_or_else(|| AppError::Other("YouTube not logged in".into()))?;
        (operations.current(AuthPlatform::Youtube), auth)
    };

    let profile = fetch_youtube_profile(&state, &current_auth).await?;
    let mut updated_auth = current_auth;
    apply_youtube_profile(&mut updated_auth, profile);

    {
        let operations = state.auth_operations.lock();
        if !operations.is_current(operation) {
            return Err(AppError::Other("Authentication superseded by a newer operation".into()));
        }
        let mut auth_state = state.auth.lock();
        let Some(saved_auth) = auth_state.youtube.as_mut() else {
            return Err(AppError::Other("YouTube not logged in".into()));
        };
        if !saved_auth.has_login() {
            return Err(AppError::Other("YouTube not logged in".into()));
        }
        if !youtube_auth_matches(saved_auth, &updated_auth) {
            return Ok(youtube_auth_info(saved_auth));
        }

        updated_auth.cookies = saved_auth.cookies.clone();
        *saved_auth = updated_auth.clone();
        cookies::save_auth(&app, &auth_state);
    }

    Ok(youtube_auth_info(&updated_auth))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum YouTubeSessionUpdateSource {
    PageRefresh,
    CookieRotation,
}

/// 机会式/强制保鲜 YouTube 登录会话(受冷却 + 熔断闸门约束)
/// 成功回收轮换 cookie 后持久化并注入共享 Jar; 失败仅计数熔断, 绝不清除本地登录
pub(crate) fn apply_youtube_session_update(
    app: &AppHandle,
    state: &AppState,
    operation: AuthOperation,
    current: &YouTubeAuth,
    updated: YouTubeAuth,
    source: YouTubeSessionUpdateSource,
) -> bool {
    let operations = state.auth_operations.lock();
    if !operations.is_current(operation) { return false; }
    let mut auth_state = state.auth.lock();
    let Some(saved) = auth_state.youtube.as_mut() else {
        return false;
    };
    if !merge_youtube_session_update_state(saved, current, updated) { return false; }
    let refreshed_cookies = saved.cookies.clone();
    cookies::save_auth(app, &auth_state);
    crate::auth::cookies::inject_cookies(&state.cookie_jar, &refreshed_cookies);
    let reason = match source {
        YouTubeSessionUpdateSource::PageRefresh => "page refresh",
        YouTubeSessionUpdateSource::CookieRotation => "RotateCookies",
    };
    log::info!(target: "youtube-refresh", "YouTube session cookies updated: {reason}");
    true
}

fn youtube_rotating_cookies_match(left: &YouTubeAuth, right: &YouTubeAuth) -> bool {
    ["__Secure-1PSIDTS", "__Secure-3PSIDTS"].into_iter().all(|name| {
        let left_values: std::collections::BTreeSet<_> = left.cookies.iter()
            .filter(|cookie| cookie.name.eq_ignore_ascii_case(name))
            .map(|cookie| (cookie.domain.trim_start_matches('.').to_ascii_lowercase(), cookie.value.as_str()))
            .collect();
        let right_values: std::collections::BTreeSet<_> = right.cookies.iter()
            .filter(|cookie| cookie.name.eq_ignore_ascii_case(name))
            .map(|cookie| (cookie.domain.trim_start_matches('.').to_ascii_lowercase(), cookie.value.as_str()))
            .collect();
        left_values == right_values
    })
}

fn merge_youtube_session_update_state(
    saved: &mut YouTubeAuth,
    current: &YouTubeAuth,
    updated: YouTubeAuth,
) -> bool {
    // 仅当仍是同一账号且未登出时才落盘, 避免刷新期间账号切换被旧 cookie 覆盖
    if !saved.has_login() || !youtube_auth_matches(saved, current) {
        return false;
    }
    // 会话已更新时丢弃旧请求结果，避免并发响应回退轮换令牌
    if !youtube_rotating_cookies_match(saved, current) {
        return false;
    }

    let mut merged = updated;
    if merged.nickname.is_none() {
        merged.nickname = saved.nickname.clone();
    }
    if merged.avatar_url.is_none() {
        merged.avatar_url = saved.avatar_url.clone();
    }
    *saved = merged;
    true
}

pub async fn maybe_refresh_youtube_session(app: &AppHandle, state: &AppState, force: bool) {
    use crate::api::youtube::refresh;

    let now = refresh::now_ms();
    let (operation, current) = {
        let operations = state.auth_operations.lock();
        let auth = state.auth.lock();
        (operations.current(AuthPlatform::Youtube), auth.youtube.clone().filter(YouTubeAuth::has_login))
    };
    let Some(current) = current else { return };

    let Some((should_refresh, should_rotate)) = with_current_youtube_session(
        &state.auth_operations, &state.auth, operation, &current, || {
            let should_refresh = {
                let mut gate = state.youtube_refresh.lock();
                if gate.should_attempt(now, force, true) {
                    gate.record_attempt(now);
                    true
                } else {
                    false
                }
            };
            let should_rotate = {
                let mut gate = state.youtube_cookie_rotation.lock();
                if gate.should_attempt(now, force, refresh::has_rotation_prerequisites(&current)) {
                    gate.record_attempt(now);
                    true
                } else {
                    false
                }
            };
            (should_refresh, should_rotate)
        },
    ) else { return };
    if !should_refresh && !should_rotate {
        return;
    }

    let jar = isolated_cookie_jar(&current.cookies);
    let Ok((primary, alternate)) = state.auth_http_clients(jar) else { return };
    let http = crate::api::transport::FallbackHttp::with_fallback(&primary, &alternate, "youtube-refresh");
    if should_rotate {
        match refresh::rotate_youtube_session(&http, &current).await {
            Ok(result) => {
                if with_current_youtube_session(&state.auth_operations, &state.auth, operation, &current, || {
                    state.youtube_cookie_rotation.lock().record_success(result.next_interval_ms);
                }).is_none() { return; }
                if let Some(updated) = result.updated_auth {
                    if apply_youtube_session_update(app, state, operation, &current, updated, YouTubeSessionUpdateSource::CookieRotation)
                    {
                        return;
                    }
                }
            }
            Err(error) => {
                if with_current_youtube_session(&state.auth_operations, &state.auth, operation, &current, || {
                    state.youtube_cookie_rotation.lock().record_failure();
                }).is_none() { return; }
                log::warn!(target: "youtube-refresh", "RotateCookies failed: {error}");
            }
        }
    }

    if !should_refresh {
        return;
    }

    match refresh::refresh_youtube_session(&http, &current).await {
        Ok(updated) => {
            if with_current_youtube_session(&state.auth_operations, &state.auth, operation, &current, || {
                state.youtube_refresh.lock().record_success(refresh::now_ms());
            }).is_none() { return; }
            if let Some(new_auth) = updated {
                apply_youtube_session_update(app, state, operation, &current, new_auth, YouTubeSessionUpdateSource::PageRefresh);
            }
        }
        Err(e) => {
            if with_current_youtube_session(&state.auth_operations, &state.auth, operation, &current, || {
                state.youtube_refresh.lock().record_failure(refresh::now_ms());
            }).is_none() { return; }
            log::warn!(target: "youtube-refresh", "refresh failed: {e}");
        }
    }
}

fn schedule_missing_account_profiles(app: &AppHandle, state: &AppState) {
    let (netease, bilibili) = {
        let mut operations = state.auth_operations.lock();
        let auth = state.auth.lock();
        (
            auth.netease.as_ref().filter(|auth| auth.has_login() && auth.user_id.is_none())
                .and_then(|auth| operations.claim_profile_refresh(AuthPlatform::Netease, std::time::Instant::now())
                    .map(|operation| (operation, AuthState { netease: Some(auth.clone()), ..AuthState::default() }))),
            auth.bilibili.as_ref().filter(|auth| auth.has_login() && auth.mid.is_none())
                .and_then(|auth| operations.claim_profile_refresh(AuthPlatform::Bilibili, std::time::Instant::now())
                    .map(|operation| (operation, AuthState { bilibili: Some(auth.clone()), ..AuthState::default() }))),
        )
    };
    for (operation, previous) in [netease, bilibili].into_iter().flatten() {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            let result = refresh_missing_account_profile(&app, &state, operation, previous).await;
            state.auth_operations.lock().finish_profile_refresh(operation, result.is_ok(), std::time::Instant::now());
            if result.is_err() {
                log::warn!(target: "auth", "{} 登录资料补齐失败，保留已保存凭据", operation.platform.as_str());
            }
        });
    }
}

async fn refresh_missing_account_profile(app: &AppHandle, state: &AppState, operation: AuthOperation, previous: AuthState) -> AppResult<()> {
    let (id, nickname, avatar_url) = match operation.platform {
        AuthPlatform::Netease => {
            let previous = previous.netease.ok_or_else(|| AppError::Other("Netease session missing".into()))?;
            let jar = isolated_cookie_jar(&previous.cookies);
            let client = netease_auth_client(state, &jar)?;
            let account = tokio::time::timeout(AUTH_PROFILE_REFRESH_TIMEOUT, client.get_user_account()).await
                .map_err(|_| AppError::Other("Account profile request timed out".into()))??;
            let id = netease_verified_account_id(&account).ok_or_else(|| AppError::Other("Netease account identity missing".into()))?;
            (id, account["profile"]["nickname"].as_str().map(String::from), account["profile"]["avatarUrl"].as_str().map(String::from))
        }
        AuthPlatform::Bilibili => {
            let previous = previous.bilibili.ok_or_else(|| AppError::Other("Bilibili session missing".into()))?;
            let jar = isolated_cookie_jar(&previous.cookies);
            let client = bilibili_auth_client(state, &jar)?;
            let info = tokio::time::timeout(AUTH_PROFILE_REFRESH_TIMEOUT, client.get_user_info()).await
                .map_err(|_| AppError::Other("Account profile request timed out".into()))??;
            let id = bili_verified_nav_mid(&info, &previous.cookies).ok_or_else(|| AppError::Other("Bilibili account identity missing".into()))?;
            (id, info["data"]["uname"].as_str().map(String::from), info["data"]["face"].as_str().map(String::from))
        }
        AuthPlatform::Youtube => return Ok(()),
    };
    {
        let operations = state.auth_operations.lock();
        if !operations.is_current(operation) { return Ok(()); }
        let mut auth = state.auth.lock();
        match operation.platform {
            AuthPlatform::Netease => {
                let Some(saved) = auth.netease.as_mut().filter(|auth| auth.has_login()) else { return Ok(()); };
                saved.user_id = Some(id);
                saved.nickname = nickname.or_else(|| saved.nickname.clone());
                saved.avatar_url = avatar_url.or_else(|| saved.avatar_url.clone());
            }
            AuthPlatform::Bilibili => {
                let Some(saved) = auth.bilibili.as_mut().filter(|auth| auth.has_login()) else { return Ok(()); };
                saved.mid = Some(id);
                saved.nickname = nickname.or_else(|| saved.nickname.clone());
                saved.avatar_url = avatar_url.or_else(|| saved.avatar_url.clone());
            }
            AuthPlatform::Youtube => return Ok(()),
        }
        cookies::save_auth(app, &auth);
    }
    let _ = app.emit("auth-profile-updated", operation.platform.as_str());
    Ok(())
}

/// 立即返回保存状态，缺失的账号 ID 在后台按会话去重补齐
#[tauri::command]
pub async fn check_auth_status(app: AppHandle, state: State<'_, AppState>) -> AppResult<AuthStatusResponse> {
    let status = state.auth.lock().to_status_response();
    schedule_missing_account_profiles(&app, &state);
    Ok(status)
}

#[derive(Serialize)]
pub struct DebugCookieStorageStatus {
    available: bool,
    stored: bool,
}

/// 查询 Debug Cookie 存储状态，不读取 Cookie 内容
#[tauri::command]
pub fn get_debug_cookie_storage_status() -> DebugCookieStorageStatus {
    DebugCookieStorageStatus {
        available: cfg!(debug_assertions),
        stored: crate::security::debug_secret_exists(crate::security::AUTH_STATE_KEY),
    }
}

/// Debug 构建中删除持久化、内存、请求 Jar 和 WebView 中的全部登录 Cookie
#[tauri::command]
pub async fn clear_debug_cookie_storage(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    #[cfg(not(debug_assertions))]
    {
        let _ = app;
        let _ = state;
        return Err(AppError::Other(
            "Debug Cookie storage is unavailable in release builds".into(),
        ));
    }

    #[cfg(debug_assertions)]
    {
        let operations = {
            let mut operations = state.auth_operations.lock();
            let mut auth = state.auth.lock();
            if !cookies::delete_persisted_auth(&app) {
                return Err(AppError::Other("Failed to delete debug Cookie storage".into()));
            }
            let invalidated = operations.invalidate_all();
            let previous_auth = std::mem::take(&mut *auth);
            for platform in ["netease", "bilibili", "youtube"] {
                cookies::expire_platform_cookies(&state.cookie_jar, &previous_auth, platform);
            }
            invalidated
        };
        clear_webview_cookies(&app, None, &operations).await
    }
}

/// 登出指定平台
#[tauri::command]
pub async fn logout(platform: String, app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let auth_platform = AuthPlatform::from_name(&platform)
        .ok_or_else(|| AppError::Other(format!("Unknown platform: {platform}")))?;
    let mut operations = state.auth_operations.lock();
    let operation = operations.begin(auth_platform);
    let mut auth = state.auth.lock();

    // 过期 reqwest Jar 中的 Cookie
    cookies::expire_platform_cookies(&state.cookie_jar, &auth, &platform);

    // 清除内存状态
    match platform.as_str() {
        "netease" => auth.netease = None,
        "bilibili" => auth.bilibili = None,
        "youtube" => auth.youtube = None,
        _ => return Err(AppError::Other(format!("Unknown platform: {}", platform))),
    }

    // 持久化
    cookies::save_auth(&app, &auth);

    // 清除 WebView2 浏览器 cookie（所有平台共享一个 cookie store）
    drop(auth);
    drop(operations);

    // 在后台清除该平台的 WebView cookie
    let app_clone = app.clone();
    tokio::task::spawn(async move {
        if let Err(e) =
            clear_webview_cookies(&app_clone, Some(&platform), &[operation])
                .await
        {
            log::warn!(target: "auth", "清除 WebView cookie 失败: {}", e);
        }
    });

    Ok(())
}

/// 把共享 Jar 中被服务端轮换过的 Cookie 回写到持久层
///
/// 只在真的发生变更时落盘，避免心跳频繁写钥匙串。
pub fn persist_rotated_cookies(app: &AppHandle, state: &AppState) {
    let mut auth = state.auth.lock();
    if cookies::sync_auth_from_jar(&state.cookie_jar, &mut auth) {
        cookies::save_auth(app, &auth);
        log::info!(target: "auth", "已回收服务端轮换的 Cookie 并落盘");
    }
}

/// 各平台在 WebView cookie store 中占用的域
fn platform_cookie_domains(platform: &str) -> &'static [&'static str] {
    match platform {
        "netease" => &["163.com"],
        "bilibili" => &["bilibili.com", "bilivideo.com", "biliapi.net"],
        "youtube" => &["youtube.com", "google.com", "googleapis.com", "ytimg.com"],
        _ => &[],
    }
}

fn cookie_domain_belongs_to(cookie_domain: &str, domains: &[&str]) -> bool {
    let cookie_domain = cookie_domain.trim_start_matches('.').to_ascii_lowercase();
    if cookie_domain.is_empty() {
        return false;
    }
    domains.iter().any(|domain| {
        cookie_domain == *domain || cookie_domain.ends_with(&format!(".{domain}"))
    })
}

/// 仅在对应会话仍未改变时清除 WebView Cookie
///
/// `platform` 为 None 时清空全部（仅 Debug 的清库入口使用）。
/// 三个平台共用同一个 WebView cookie store，所以绝不能用
/// `clear_all_browsing_data()` —— 登出一个平台会把另外两个的浏览器登录态一起抹掉。
async fn clear_webview_cookies(
    app: &AppHandle,
    platform: Option<&str>,
    expected: &[AuthOperation],
) -> AppResult<()> {
    // 创建一个不可见的临时窗口来操作 WebView cookie
    let label = "cookie-cleaner";
    let window = WebviewWindowBuilder::new(
        app, label,
        WebviewUrl::External("about:blank".parse().unwrap()),
    )
    .visible(false)
    .main_browser_args(app)
    .build()
    .map_err(|e| AppError::Other(format!("Failed to create cleaner window: {}", e)))?;

    // 短暂等待窗口初始化
    tokio::time::sleep(Duration::from_millis(200)).await;

    let state = app.state::<AppState>();
    let operations = state.auth_operations.lock();
    if !expected.iter().all(|operation| operations.is_current(*operation)) {
        let _ = window.close();
        return Ok(());
    }

    match platform {
        None => {
            let _ = window.clear_all_browsing_data();
        }
        Some(platform) => {
            let domains = platform_cookie_domains(platform);
            match window.cookies() {
                Ok(all_cookies) => {
                    for cookie in all_cookies {
                        let domain = cookie.domain().unwrap_or_default().to_string();
                        if !cookie_domain_belongs_to(&domain, domains) {
                            continue;
                        }
                        if let Err(e) = window.delete_cookie(cookie) {
                            log::warn!(target: "auth", "删除 WebView cookie 失败: {}", e);
                        }
                    }
                }
                Err(e) => {
                    log::warn!(
                        target: "auth",
                        "无法枚举 WebView cookie，跳过按域清理: {}",
                        e,
                    );
                }
            }
        }
    }

    // 关闭临时窗口
    let _ = window.close();

    Ok(())
}
