use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};

// 三平台登录/登出命令
use crate::api::netease::client::NeteaseAccountProfile;
use crate::api::youtube::client::YouTubeAccountProfile;
use crate::auth::cookies;
use crate::auth::netease_hydration;
use crate::auth::state::{
    AuthInfo, AuthState, AuthStatusResponse, BiliAuth, CookieEntry, NeteaseAuth, YouTubeAuth,
};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

const BILIBILI_LOGIN_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
static COOKIE_CLEANER_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
    entries
        .iter()
        .any(|entry| entry.name == sentinel_cookie && !entry.value.is_empty())
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

async fn fetch_youtube_profile(
    state: &AppState,
    auth: &YouTubeAuth,
) -> AppResult<YouTubeAccountProfile> {
    let client = crate::api::youtube::client::YouTubeClient::new(&state.http());
    client.get_account_profile(auth).await
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
        account_id: None,
    }
}

pub(crate) fn youtube_auth_matches(left: &YouTubeAuth, right: &YouTubeAuth) -> bool {
    left.get_sapisid()
        .zip(right.get_sapisid())
        .is_some_and(|(left, right)| left == right)
}

async fn validate_netease_account(
    state: &AppState,
    entries: &[CookieEntry],
) -> AppResult<NeteaseAccountProfile> {
    let candidate_jar = Arc::new(reqwest::cookie::Jar::default());
    cookies::inject_cookies(&candidate_jar, entries);
    let http = state.http_with_cookie_jar(candidate_jar)?;
    let client = crate::api::netease::client::NeteaseClient::new(&http);
    let account = client.get_user_account().await?;
    crate::api::netease::client::parse_netease_account_profile(&account)
}

fn netease_music_u(auth: &NeteaseAuth) -> Option<&str> {
    auth.cookies
        .iter()
        .find(|cookie| cookie.name == "MUSIC_U" && !cookie.value.is_empty())
        .map(|cookie| cookie.value.as_str())
}

#[derive(Debug, PartialEq, Eq)]
enum NeteaseHydrationApplyResult {
    Applied,
    AlreadyHydrated,
    SessionChanged,
}

fn apply_hydrated_netease_profile(
    auth_state: &mut AuthState,
    expected_music_u: &str,
    profile: NeteaseAccountProfile,
) -> NeteaseHydrationApplyResult {
    let Some(auth) = auth_state.netease.as_mut() else {
        return NeteaseHydrationApplyResult::SessionChanged;
    };
    if netease_music_u(auth) != Some(expected_music_u) {
        return NeteaseHydrationApplyResult::SessionChanged;
    }
    if auth.user_id.is_some() {
        return NeteaseHydrationApplyResult::AlreadyHydrated;
    }

    auth.user_id = Some(profile.user_id);
    auth.nickname = profile.nickname;
    auth.avatar_url = profile.avatar_url;
    NeteaseHydrationApplyResult::Applied
}

#[cfg(test)]
mod tests {
    use super::{
        apply_hydrated_netease_profile, cookie_domain_matches_urls, has_completed_login,
        NeteaseHydrationApplyResult,
    };
    use crate::api::netease::client::{parse_netease_account_profile, NeteaseAccountProfile};
    use crate::auth::state::{AuthState, CookieEntry, NeteaseAuth};
    use serde_json::json;

    const BILIBILI_URLS: &[&str] = &[
        "https://www.bilibili.com",
        "https://passport.bilibili.com",
        "https://api.bilibili.com",
    ];

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
            domain: "google.com".into(),
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
    fn netease_account_response_requires_success_code_and_profile_user_id() {
        assert!(parse_netease_account_profile(&json!({
            "code": 301,
            "profile": { "userId": 42 },
        }))
        .is_err());

        assert!(parse_netease_account_profile(&json!({
            "code": 200,
            "profile": null,
        }))
        .is_err());

        assert!(parse_netease_account_profile(&json!({
            "code": 200,
            "profile": {},
        }))
        .is_err());

        let profile = parse_netease_account_profile(&json!({
            "code": 200,
            "profile": {
                "userId": 42,
                "nickname": "Neri",
                "avatarUrl": "https://img.example/avatar.png",
            },
        }))
        .expect("valid account profile");
        assert_eq!(profile.user_id, 42);
        assert_eq!(profile.nickname.as_deref(), Some("Neri"));
    }

    #[test]
    fn hydration_applies_profile_to_the_same_pending_session() {
        let mut auth_state = AuthState {
            netease: Some(NeteaseAuth {
                cookies: vec![CookieEntry {
                    name: "MUSIC_U".into(),
                    value: "session-a".into(),
                    domain: "music.163.com".into(),
                }],
                user_id: None,
                nickname: None,
                avatar_url: None,
            }),
            ..Default::default()
        };

        let result = apply_hydrated_netease_profile(
            &mut auth_state,
            "session-a",
            NeteaseAccountProfile {
                user_id: 42,
                nickname: Some("Neri".into()),
                avatar_url: Some("https://img.example/avatar.png".into()),
            },
        );

        assert_eq!(result, NeteaseHydrationApplyResult::Applied);
        let auth = auth_state.netease.unwrap();
        assert_eq!(auth.user_id, Some(42));
        assert_eq!(auth.nickname.as_deref(), Some("Neri"));
    }

    #[test]
    fn hydration_does_not_overwrite_a_changed_cookie_session() {
        let mut auth_state = AuthState {
            netease: Some(NeteaseAuth {
                cookies: vec![CookieEntry {
                    name: "MUSIC_U".into(),
                    value: "session-b".into(),
                    domain: "music.163.com".into(),
                }],
                user_id: None,
                nickname: Some("Current".into()),
                avatar_url: None,
            }),
            ..Default::default()
        };

        let result = apply_hydrated_netease_profile(
            &mut auth_state,
            "session-a",
            NeteaseAccountProfile {
                user_id: 42,
                nickname: Some("Stale".into()),
                avatar_url: None,
            },
        );

        assert_eq!(result, NeteaseHydrationApplyResult::SessionChanged);
        let auth = auth_state.netease.unwrap();
        assert_eq!(auth.user_id, None);
        assert_eq!(auth.nickname.as_deref(), Some("Current"));
        assert_eq!(auth.cookies[0].value, "session-b");
    }
}

/// 从 WebView 窗口轮询提取 Cookie（使用 Tauri 内置 API 读取 HttpOnly）
async fn poll_webview_cookies(
    app: &AppHandle,
    window_label: &str,
    sentinel_cookie: &str,
    required_host: Option<&str>,
    cookie_urls: &[&str],
    timeout_secs: u64,
    close_requested: &AtomicBool,
) -> AppResult<Vec<CookieEntry>> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    let poll_interval = Duration::from_millis(800);

    loop {
        if close_requested.load(Ordering::Acquire) {
            if let Some(window) = app.get_webview_window(window_label) {
                let entries = read_webview_cookies(&window, cookie_urls);
                let login_succeeded = has_login_cookie(&entries, sentinel_cookie);
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

        if has_completed_login(
            &all_entries,
            sentinel_cookie,
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
    let _cookie_guard = state.auth_cookie_gate.lock().await;
    let label = "netease-login";
    let window = WebviewWindowBuilder::new(
        &app,
        label,
        WebviewUrl::External("https://music.163.com/#/login".parse().unwrap()),
    )
    .title("NeriPlayer - 网易云音乐登录")
    .inner_size(420.0, 600.0)
    .center()
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
        &app, label, "MUSIC_U", None, cookie_urls, 300, &close_requested,
    ).await?;

    // 补全默认 Cookie（与 Android 一致）
    if !entries.iter().any(|c| c.name == "os") {
        entries.push(CookieEntry { name: "os".into(), value: "pc".into(), domain: "music.163.com".into() });
    }
    if !entries.iter().any(|c| c.name == "appver") {
        entries.push(CookieEntry { name: "appver".into(), value: "8.10.35".into(), domain: "music.163.com".into() });
    }

    // Validate against the candidate domains without touching the shared response cookie store.
    let profile = match validate_netease_account(&state, &entries).await {
        Ok(profile) => profile,
        Err(validation_error) => {
            let previous_auth = {
                let mut auth_state = state.auth.lock();
                let previous_auth = auth_state.clone();
                auth_state.netease = None;
                cookies::save_auth(&app, &auth_state);
                previous_auth
            };
            cookies::expire_platform_cookies(&state.cookie_jar, &previous_auth, "netease");

            if let Err(clear_error) = clear_and_reinject_webview_cookies(&app, &state).await {
                return Err(AppError::Other(format!(
                    "{validation_error}; failed to clear stale NetEase login cookies: {clear_error}"
                )));
            }
            return Err(validation_error);
        }
    };
    cookies::inject_cookies(&state.cookie_jar, &entries);
    let account_id = Some(profile.user_id.to_string());
    let nickname = profile.nickname;
    let avatar_url = profile.avatar_url;
    let auth = NeteaseAuth {
        cookies: entries,
        user_id: Some(profile.user_id),
        nickname: nickname.clone(),
        avatar_url: avatar_url.clone(),
    };
    let logged_in = auth.has_login();
    {
        let mut auth_state = state.auth.lock();
        auth_state.netease = Some(auth);
        cookies::save_auth(&app, &auth_state);
    }

    Ok(AuthInfo {
        platform: "netease".into(),
        logged_in,
        nickname,
        avatar_url,
        account_id,
    })
}

/// B站登录
#[tauri::command]
pub async fn login_bilibili(app: AppHandle, state: State<'_, AppState>) -> AppResult<AuthInfo> {
    let _cookie_guard = state.auth_cookie_gate.lock().await;
    let label = "bilibili-login";
    let window = WebviewWindowBuilder::new(
        &app,
        label,
        WebviewUrl::External("https://passport.bilibili.com/login".parse().unwrap()),
    )
    .title("NeriPlayer - 哔哩哔哩登录")
    .inner_size(420.0, 600.0)
    .center()
    .user_agent(BILIBILI_LOGIN_USER_AGENT)
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
        &app, label, "SESSDATA", None, cookie_urls, 300, &close_requested,
    ).await?;

    // B 站登录 cookie 必须关联到 .bilibili.com 域，确保 api.bilibili.com 子域名也能发送
    let bili_core_cookies = ["SESSDATA", "DedeUserID", "DedeUserID__ckMd5", "bili_jct", "sid"];
    for entry in &mut entries {
        if bili_core_cookies.contains(&entry.name.as_str()) && !entry.domain.starts_with('.') {
            entry.domain = ".bilibili.com".to_string();
        }
    }

    // 注入 Jar（含 Domain 属性，确保子域名 API 生效）
    cookies::inject_cookies(&state.cookie_jar, &entries);

    // 从 Cookie 提取 DedeUserID
    let mid = entries.iter()
        .find(|c| c.name == "DedeUserID")
        .and_then(|c| c.value.parse::<u64>().ok());

    // 调用 B站 nav API 获取用户信息
    let client = crate::api::bilibili::client::BiliClient::new(&state.http());
    let (nickname, avatar_url) = match client.get_user_info().await {
        Ok(info) => {
            let data = &info["data"];
            // 必须检查 isLogin，未登录时 data 中无有效用户信息
            let is_login = data["isLogin"].as_bool().unwrap_or(false);
            if is_login {
                (
                    data["uname"].as_str().map(String::from),
                    data["face"].as_str().map(String::from),
                )
            } else {
                log::warn!(target: "auth", "Bilibili nav API 返回 isLogin=false，cookie 可能未生效");
                (None, None)
            }
        }
        Err(e) => {
            log::warn!(target: "auth", "Bilibili get_user_info 失败: {}", e);
            (None, None)
        }
    };

    let auth = BiliAuth { cookies: entries, mid, nickname: nickname.clone(), avatar_url: avatar_url.clone() };
    {
        let mut auth_state = state.auth.lock();
        auth_state.bilibili = Some(auth);
        cookies::save_auth(&app, &auth_state);
    }

    Ok(AuthInfo {
        platform: "bilibili".into(),
        logged_in: true,
        nickname,
        avatar_url,
        account_id: mid.map(|id| id.to_string()),
    })
}

/// YouTube Music 登录
#[tauri::command]
pub async fn login_youtube(app: AppHandle, state: State<'_, AppState>) -> AppResult<AuthInfo> {
    let _cookie_guard = state.auth_cookie_gate.lock().await;
    let login_url = "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F";
    let label = "youtube-login";

    let window = WebviewWindowBuilder::new(
        &app,
        label,
        WebviewUrl::External(login_url.parse().unwrap()),
    )
    .title("NeriPlayer - YouTube Music Login")
    .inner_size(480.0, 680.0)
    .center()
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
        &app, label, "SAPISID", Some("music.youtube.com"), cookie_urls, 300, &close_requested,
    ).await?;

    // 注入 Jar
    cookies::inject_cookies(&state.cookie_jar, &entries);

    let mut auth = YouTubeAuth {
        cookies: entries,
        nickname: None,
        avatar_url: None,
    };
    apply_youtube_profile_best_effort(&state, &mut auth).await;
    {
        let mut auth_state = state.auth.lock();
        auth_state.youtube = Some(auth.clone());
        cookies::save_auth(&app, &auth_state);
    }

    Ok(youtube_auth_info(&auth))
}

/// Cookie 粘贴登录（对齐 Android 端）
#[tauri::command]
pub async fn login_with_cookies(
    platform: String,
    raw_cookies: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<AuthInfo> {
    let _cookie_guard = state.auth_cookie_gate.lock().await;
    // 解析用户粘贴的 Cookie 文本
    let entries = cookies::parse_raw_cookie_text(&raw_cookies, &platform);

    if entries.is_empty() {
        return Err(AppError::Other("No valid cookies found".into()));
    }

    match platform.as_str() {
        "netease" => {
            // 验证 MUSIC_U 存在
            if !entries.iter().any(|c| c.name == "MUSIC_U" && !c.value.is_empty()) {
                return Err(AppError::Other("Missing required cookie: MUSIC_U".into()));
            }

            let profile = validate_netease_account(&state, &entries)
                .await
                .map_err(|e| AppError::Other(format!("Cookie validation failed: {e}")))?;
            cookies::inject_cookies(&state.cookie_jar, &entries);
            let account_id = Some(profile.user_id.to_string());
            let nickname = profile.nickname;
            let avatar_url = profile.avatar_url;
            let auth = NeteaseAuth {
                cookies: entries,
                user_id: Some(profile.user_id),
                nickname: nickname.clone(),
                avatar_url: avatar_url.clone(),
            };
            let logged_in = auth.has_login();
            {
                let mut auth_state = state.auth.lock();
                auth_state.netease = Some(auth);
                cookies::save_auth(&app, &auth_state);
            }

            Ok(AuthInfo { platform: "netease".into(), logged_in, nickname, avatar_url, account_id })
        }
        "bilibili" => {
            if !entries.iter().any(|c| c.name == "SESSDATA" && !c.value.is_empty()) {
                return Err(AppError::Other("Missing required cookie: SESSDATA".into()));
            }

            cookies::inject_cookies(&state.cookie_jar, &entries);

            let mid = entries.iter()
                .find(|c| c.name == "DedeUserID")
                .and_then(|c| c.value.parse::<u64>().ok());

            let client = crate::api::bilibili::client::BiliClient::new(&state.http());
            let (nickname, avatar_url) = match client.get_user_info().await {
                Ok(info) => {
                    let data = &info["data"];
                    let is_login = data["isLogin"].as_bool().unwrap_or(false);
                    if is_login {
                        (data["uname"].as_str().map(String::from), data["face"].as_str().map(String::from))
                    } else {
                        return Err(AppError::Other("Cookie 验证失败：B站返回未登录状态".into()));
                    }
                }
                Err(e) => return Err(AppError::Other(format!("Cookie validation failed: {}", e))),
            };

            let auth = BiliAuth { cookies: entries, mid, nickname: nickname.clone(), avatar_url: avatar_url.clone() };
            {
                let mut auth_state = state.auth.lock();
                auth_state.bilibili = Some(auth);
                cookies::save_auth(&app, &auth_state);
            }

            Ok(AuthInfo {
                platform: "bilibili".into(),
                logged_in: true,
                nickname,
                avatar_url,
                account_id: mid.map(|id| id.to_string()),
            })
        }
        "youtube" => {
            if !entries.iter().any(|c| c.name == "SAPISID" && !c.value.is_empty()) {
                return Err(AppError::Other("Missing required cookie: SAPISID".into()));
            }

            cookies::inject_cookies(&state.cookie_jar, &entries);

            let mut auth = YouTubeAuth {
                cookies: entries,
                nickname: None,
                avatar_url: None,
            };
            apply_youtube_profile_best_effort(&state, &mut auth).await;
            {
                let mut auth_state = state.auth.lock();
                auth_state.youtube = Some(auth.clone());
                cookies::save_auth(&app, &auth_state);
            }

            Ok(youtube_auth_info(&auth))
        }
        _ => Err(AppError::Other(format!("Unknown platform: {}", platform))),
    }
}

/// 刷新已保存的 YouTube Music 账号资料
#[tauri::command]
pub async fn refresh_youtube_profile(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<AuthInfo> {
    let current_auth = {
        let auth_state = state.auth.lock();
        auth_state
            .youtube
            .clone()
            .filter(YouTubeAuth::has_login)
            .ok_or_else(|| AppError::Other("YouTube not logged in".into()))?
    };

    let profile = fetch_youtube_profile(&state, &current_auth).await?;
    let mut updated_auth = current_auth;
    apply_youtube_profile(&mut updated_auth, profile);

    {
        let _cookie_guard = state.auth_cookie_gate.lock().await;
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

        *saved_auth = updated_auth.clone();
        cookies::save_auth(&app, &auth_state);
    }

    Ok(youtube_auth_info(&updated_auth))
}

/// 机会式/强制保鲜 YouTube 登录会话(受冷却 + 熔断闸门约束)
/// 成功回收轮换 cookie 后持久化并注入共享 Jar; 失败仅计数熔断, 绝不清除本地登录
pub async fn maybe_refresh_youtube_session(app: &AppHandle, state: &AppState, force: bool) {
    use crate::api::youtube::refresh;

    let now = refresh::now_ms();
    let current = {
        let mut gate = state.youtube_refresh.lock();
        let auth = state.auth.lock();
        let has_login = auth
            .youtube
            .as_ref()
            .map(YouTubeAuth::has_login)
            .unwrap_or(false);
        if !gate.should_attempt(now, force, has_login) {
            return;
        }
        gate.record_attempt(now);
        match auth.youtube.clone() {
            Some(a) => a,
            None => return,
        }
    };

    let http = state.http();
    match refresh::refresh_youtube_session(&http, &current).await {
        Ok(updated) => {
            state.youtube_refresh.lock().record_success(refresh::now_ms());
            if let Some(new_auth) = updated {
                let _cookie_guard = state.auth_cookie_gate.lock().await;
                let mut auth_state = state.auth.lock();
                if let Some(saved) = auth_state.youtube.as_mut() {
                    // 仅当仍是同一账号且未登出时才落盘, 避免刷新期间账号切换被旧 cookie 覆盖
                    let same_account = saved.get_sapisid() == current.get_sapisid();
                    if saved.has_login() && same_account {
                        let mut merged = new_auth;
                        if merged.nickname.is_none() {
                            merged.nickname = saved.nickname.clone();
                        }
                        if merged.avatar_url.is_none() {
                            merged.avatar_url = saved.avatar_url.clone();
                        }
                        *saved = merged;
                        // 先克隆再落盘, 避免 saved 的可变借用跨越 save_auth 的不可变借用
                        let refreshed_cookies = saved.cookies.clone();
                        cookies::save_auth(app, &auth_state);
                        crate::auth::cookies::inject_cookies(&state.cookie_jar, &refreshed_cookies);
                        log::info!(target: "youtube-refresh", "session cookies refreshed");
                    }
                }
            }
        }
        Err(e) => {
            state.youtube_refresh.lock().record_failure(refresh::now_ms());
            log::warn!(target: "youtube-refresh", "refresh failed: {e}");
        }
    }
}

/// 查询所有平台登录状态
#[tauri::command]
pub async fn check_auth_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<AuthStatusResponse> {
    let hydration_candidate = {
        let mut hydration_gate = state.netease_hydration.lock();
        let auth_state = state.auth.lock();
        auth_state.netease.as_ref().and_then(|auth| {
            if auth.user_id.is_some() {
                return None;
            }
            let music_u = netease_music_u(auth)?.to_string();
            hydration_gate
                .try_begin(&music_u, netease_hydration::now_ms())
                .then(|| (music_u, auth.cookies.clone()))
        })
    };

    if let Some((music_u, entries)) = hydration_candidate {
        let result = tokio::time::timeout(
            netease_hydration::REQUEST_TIMEOUT,
            validate_netease_account(&state, &entries),
        )
        .await;

        match result {
            Ok(Ok(profile)) => {
                let apply_result = {
                    let _cookie_guard = state.auth_cookie_gate.lock().await;
                    let mut auth_state = state.auth.lock();
                    let apply_result =
                        apply_hydrated_netease_profile(&mut auth_state, &music_u, profile);
                    if apply_result == NeteaseHydrationApplyResult::Applied {
                        cookies::save_auth(&app, &auth_state);
                    }
                    apply_result
                };

                match apply_result {
                    NeteaseHydrationApplyResult::Applied
                    | NeteaseHydrationApplyResult::AlreadyHydrated => {
                        state.netease_hydration.lock().record_success(&music_u);
                        log::info!(target: "auth", "NetEase imported session profile restored");
                    }
                    NeteaseHydrationApplyResult::SessionChanged => {
                        state.netease_hydration.lock().record_abandoned(&music_u);
                    }
                }
            }
            Ok(Err(error)) => {
                state
                    .netease_hydration
                    .lock()
                    .record_failure(&music_u, netease_hydration::now_ms());
                log::warn!(target: "auth", "NetEase imported session recovery failed: {error}");
            }
            Err(_) => {
                state
                    .netease_hydration
                    .lock()
                    .record_failure(&music_u, netease_hydration::now_ms());
                log::warn!(
                    target: "auth",
                    "NetEase imported session recovery timed out after {} seconds",
                    netease_hydration::REQUEST_TIMEOUT.as_secs()
                );
            }
        }
    }

    let auth = state.auth.lock();
    Ok(auth.to_status_response())
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
        let _cookie_guard = state.auth_cookie_gate.lock().await;

        if !cookies::delete_persisted_auth(&app) {
            return Err(AppError::Other(
                "Failed to delete debug Cookie storage".into(),
            ));
        }

        let previous_auth = {
            let mut auth = state.auth.lock();
            std::mem::take(&mut *auth)
        };
        for platform in ["netease", "bilibili", "youtube"] {
            cookies::expire_platform_cookies(&state.cookie_jar, &previous_auth, platform);
        }

        clear_and_reinject_webview_cookies(&app, &state).await
    }
}

/// 登出指定平台
#[tauri::command]
pub async fn logout(platform: String, app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let _cookie_guard = state.auth_cookie_gate.lock().await;
    {
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
    }

    clear_and_reinject_webview_cookies(&app, &state).await?;

    Ok(())
}

/// 清除 WebView2 cookie 并重新注入剩余平台的 cookie
pub(crate) async fn clear_and_reinject_webview_cookies(
    app: &AppHandle,
    state: &AppState,
) -> AppResult<()> {
    // 创建一个不可见的临时窗口来操作 WebView2 cookie
    let label = format!(
        "cookie-cleaner-{}",
        COOKIE_CLEANER_SEQUENCE.fetch_add(1, Ordering::Relaxed),
    );
    let window = WebviewWindowBuilder::new(
        app, label.as_str(),
        WebviewUrl::External("about:blank".parse().unwrap()),
    )
    .visible(false)
    .build()
    .map_err(|e| AppError::Other(format!("Failed to create cleaner window: {}", e)))?;

    // 短暂等待窗口初始化
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 清除所有浏览数据（含所有 cookie）
    let clear_result = window.clear_all_browsing_data()
        .map_err(|e| AppError::Other(format!("Failed to clear WebView browsing data: {e}")));

    // 关闭临时窗口
    let close_result = window.close()
        .map_err(|e| AppError::Other(format!("Failed to close cookie cleaner window: {e}")));

    // Re-read after the destructive clear so no logout snapshot can revive a later login.
    let current_auth = state.auth.lock().clone();
    cookies::inject_all(&state.cookie_jar, &current_auth);

    match (clear_result, close_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(clear_error), Ok(())) => Err(clear_error),
        (Ok(()), Err(close_error)) => Err(close_error),
        (Err(clear_error), Err(close_error)) => Err(AppError::Other(format!(
            "{clear_error}; {close_error}"
        ))),
    }
}
