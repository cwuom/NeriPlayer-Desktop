use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};

// 三平台登录/登出命令
use crate::api::youtube::client::YouTubeAccountProfile;
use crate::webview_args::MainBrowserArgs;
use crate::auth::cookies;
use crate::auth::state::{
    AuthInfo, AuthStatusResponse, BiliAuth, CookieEntry, NeteaseAuth, YouTubeAuth,
};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

const BILIBILI_LOGIN_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

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

fn login_sentinel_value(entries: &[CookieEntry], sentinel_cookie: &str) -> Option<String> {
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
    let client = state.youtube();
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
    }
}

fn youtube_auth_matches(left: &YouTubeAuth, right: &YouTubeAuth) -> bool {
    left.get_sapisid()
        .zip(right.get_sapisid())
        .is_some_and(|(left, right)| left == right)
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
    };
    use crate::auth::state::CookieEntry;

    const BILIBILI_URLS: &[&str] = &[
        "https://www.bilibili.com",
        "https://passport.bilibili.com",
        "https://api.bilibili.com",
    ];

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
            domain: "google.com".into(),
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
    let initial_window = app.get_webview_window(window_label);
    let initial_sentinel = initial_window
        .as_ref()
        .map(|window| read_webview_cookies(window, cookie_urls))
        .and_then(|entries| login_sentinel_value(&entries, sentinel_cookie));
    let initial_url = initial_window.and_then(|window| window.url().ok());

    loop {
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
    let label = "netease-login";
    let window = WebviewWindowBuilder::new(
        &app,
        label,
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
        &app, label, "MUSIC_U", None, cookie_urls, 300, &close_requested,
    ).await?;

    // 补全默认 Cookie（与 Android 一致）
    if !entries.iter().any(|c| c.name == "os") {
        entries.push(CookieEntry { name: "os".into(), value: "pc".into(), domain: "music.163.com".into() });
    }
    if !entries.iter().any(|c| c.name == "appver") {
        entries.push(CookieEntry { name: "appver".into(), value: "8.10.35".into(), domain: "music.163.com".into() });
    }

    // 注入 Jar
    cookies::inject_cookies(&state.cookie_jar, &entries);

    // 调用 API 获取用户信息
    let client = state.netease();
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
    {
        let mut auth_state = state.auth.lock();
        auth_state.netease = Some(auth);
        cookies::save_auth(&app, &auth_state);
    }

    Ok(AuthInfo {
        platform: "netease".into(),
        logged_in: true,
        nickname,
        avatar_url,
    })
}

/// B站登录
#[tauri::command]
pub async fn login_bilibili(app: AppHandle, state: State<'_, AppState>) -> AppResult<AuthInfo> {
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
    let client = state.bilibili();
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
    })
}

/// YouTube Music 登录
#[tauri::command]
pub async fn login_youtube(app: AppHandle, state: State<'_, AppState>) -> AppResult<AuthInfo> {
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
        &app, label, "SAPISID", Some("music.youtube.com"), cookie_urls, 300, &close_requested,
    ).await?;

    // 入库前按白名单过滤: google 账号域会带 LSID 等超出播放所需的整套会话 cookie,
    // 只保留 session 持久化白名单键, 对齐 Android sanitizePersistedCookies（AU-13）
    let entries: Vec<CookieEntry> = entries
        .into_iter()
        .filter(|c| crate::api::youtube::session::cookie_key_allowed(&c.name))
        .collect();

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
    // 解析用户粘贴的 Cookie 文本
    let mut entries = cookies::parse_raw_cookie_text(&raw_cookies, &platform);

    if entries.is_empty() {
        return Err(AppError::Other("No valid cookies found".into()));
    }

    if platform == "youtube" {
        entries.retain(|entry| crate::api::youtube::session::cookie_key_allowed(&entry.name));
    }

    // 先做格式校验（必需 cookie 是否存在），通过后再注入共享 Jar
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

    let previous_entries = {
        let auth = state.auth.lock();
        match platform.as_str() {
            "netease" => auth.netease.as_ref().map(|value| value.cookies.clone()),
            "bilibili" => auth.bilibili.as_ref().map(|value| value.cookies.clone()),
            "youtube" => auth.youtube.as_ref().map(|value| value.cookies.clone()),
            _ => None,
        }
    };

    // 注入 Jar
    cookies::inject_cookies(&state.cookie_jar, &entries);

    let result = match platform.as_str() {
        "netease" => {
            let client = state.netease();
            let (user_id, nickname, avatar_url) = match client.get_user_account().await {
                Ok(account) => {
                    let profile = &account["profile"];
                    (
                        profile["userId"].as_u64(),
                        profile["nickname"].as_str().map(String::from),
                        profile["avatarUrl"].as_str().map(String::from),
                    )
                }
                Err(e) => {
                    return rollback_cookie_login(
                        &state.cookie_jar,
                        &entries,
                        previous_entries.as_deref(),
                        AppError::Other(format!("Cookie validation failed: {}", e)),
                    )
                }
            };

            let auth = NeteaseAuth { cookies: entries, user_id, nickname: nickname.clone(), avatar_url: avatar_url.clone() };
            {
                let mut auth_state = state.auth.lock();
                auth_state.netease = Some(auth);
                cookies::save_auth(&app, &auth_state);
            }

            Ok(AuthInfo { platform: "netease".into(), logged_in: true, nickname, avatar_url })
        }
        "bilibili" => {
            let client = state.bilibili();
            let (mid, nickname, avatar_url) = match client.get_user_info().await {
                Ok(info) => {
                    let data = &info["data"];
                    if let Some(mid) = bili_verified_nav_mid(&info, &entries) {
                        (Some(mid), data["uname"].as_str().map(String::from), data["face"].as_str().map(String::from))
                    } else {
                        return rollback_cookie_login(
                            &state.cookie_jar,
                            &entries,
                            previous_entries.as_deref(),
                            AppError::Other("Cookie 验证失败：B站未返回有效登录身份".into()),
                        );
                    }
                }
                Err(e) => {
                    return rollback_cookie_login(
                        &state.cookie_jar,
                        &entries,
                        previous_entries.as_deref(),
                        AppError::Other(format!("Cookie validation failed: {}", e)),
                    )
                }
            };

            let auth = BiliAuth { cookies: entries, mid, nickname: nickname.clone(), avatar_url: avatar_url.clone() };
            {
                let mut auth_state = state.auth.lock();
                auth_state.bilibili = Some(auth);
                cookies::save_auth(&app, &auth_state);
            }

            Ok(AuthInfo { platform: "bilibili".into(), logged_in: true, nickname, avatar_url })
        }
        "youtube" => {
            if !entries.iter().any(|c| c.name == "SAPISID" && !c.value.is_empty()) {
                return rollback_cookie_login(
                    &state.cookie_jar,
                    &entries,
                    previous_entries.as_deref(),
                    AppError::Other("Missing required cookie: SAPISID".into()),
                );
            }

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
    };

    result
}

fn rollback_cookie_login(
    jar: &std::sync::Arc<reqwest::cookie::Jar>,
    candidate_entries: &[CookieEntry],
    previous_entries: Option<&[CookieEntry]>,
    error: AppError,
) -> AppResult<AuthInfo> {
    // 验证失败时移除本次候选值并恢复旧会话，避免粘贴坏 Cookie 后整个平台请求都被污染
    cookies::expire_cookie_entries(jar, candidate_entries);
    if let Some(previous) = previous_entries {
        cookies::inject_cookies(jar, previous);
    }
    Err(error)
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
fn apply_youtube_session_update(
    app: &AppHandle,
    state: &AppState,
    current: &YouTubeAuth,
    updated: YouTubeAuth,
    reason: &str,
) -> bool {
    let mut auth_state = state.auth.lock();
    let Some(saved) = auth_state.youtube.as_mut() else {
        return false;
    };
    // 仅当仍是同一账号且未登出时才落盘, 避免刷新期间账号切换被旧 cookie 覆盖
    if !saved.has_login() || saved.get_sapisid() != current.get_sapisid() {
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
    // 先克隆再落盘, 避免 saved 的可变借用跨越 save_auth 的不可变借用
    let refreshed_cookies = saved.cookies.clone();
    cookies::save_auth(app, &auth_state);
    crate::auth::cookies::inject_cookies(&state.cookie_jar, &refreshed_cookies);
    log::info!(target: "youtube-refresh", "YouTube session cookies updated: {reason}");
    true
}

pub async fn maybe_refresh_youtube_session(app: &AppHandle, state: &AppState, force: bool) {
    use crate::api::youtube::refresh;

    let now = refresh::now_ms();
    let current = {
        let auth = state.auth.lock();
        auth.youtube.clone().filter(YouTubeAuth::has_login)
    };
    let Some(current) = current else { return };

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
        if gate.should_attempt(
            now,
            force,
            refresh::has_rotation_prerequisites(&current),
        ) {
            gate.record_attempt(now);
            true
        } else {
            false
        }
    };
    if !should_refresh && !should_rotate {
        return;
    }

    let http = state.transport("youtube-refresh");
    if should_rotate {
        match refresh::rotate_youtube_session(&http, &current).await {
            Ok(result) => {
                state
                    .youtube_cookie_rotation
                    .lock()
                    .record_success(result.next_interval_ms);
                if let Some(updated) = result.updated_auth {
                    if apply_youtube_session_update(app, state, &current, updated, "RotateCookies")
                    {
                        return;
                    }
                }
            }
            Err(error) => {
                state.youtube_cookie_rotation.lock().record_failure();
                log::warn!(target: "youtube-refresh", "RotateCookies failed: {error}");
            }
        }
    }

    if !should_refresh {
        return;
    }

    match refresh::refresh_youtube_session(&http, &current).await {
        Ok(updated) => {
            state.youtube_refresh.lock().record_success(refresh::now_ms());
            if let Some(new_auth) = updated {
                apply_youtube_session_update(app, state, &current, new_auth, "page refresh");
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
pub async fn check_auth_status(state: State<'_, AppState>) -> AppResult<AuthStatusResponse> {
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

        clear_and_reinject_webview_cookies(
            &app,
            &state.cookie_jar,
            &crate::auth::state::AuthState::default(),
            None,
        )
        .await
    }
}

/// 登出指定平台
#[tauri::command]
pub async fn logout(platform: String, app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
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
    // 清除后重新注入剩余已登录平台的 cookie
    let remaining_auth = auth.clone();
    drop(auth);

    // 在后台清除该平台的 WebView cookie
    let app_clone = app.clone();
    let jar = state.cookie_jar.clone();
    tokio::task::spawn(async move {
        if let Err(e) =
            clear_and_reinject_webview_cookies(&app_clone, &jar, &remaining_auth, Some(&platform))
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

/// 清除指定平台的 WebView cookie 并重新注入剩余平台的 cookie
///
/// `platform` 为 None 时清空全部（仅 Debug 的清库入口使用）。
/// 三个平台共用同一个 WebView cookie store，所以绝不能用
/// `clear_all_browsing_data()` —— 登出一个平台会把另外两个的浏览器登录态一起抹掉。
async fn clear_and_reinject_webview_cookies(
    app: &AppHandle,
    jar: &std::sync::Arc<reqwest::cookie::Jar>,
    remaining_auth: &crate::auth::state::AuthState,
    platform: Option<&str>,
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

    // 重新注入剩余已登录平台的 cookie 到 reqwest Jar
    cookies::inject_all(jar, remaining_auth);

    Ok(())
}
