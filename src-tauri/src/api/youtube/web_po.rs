use std::collections::VecDeque;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use tauri::{
    webview::PageLoadEvent, AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

use crate::auth::state::YouTubeAuth;
use crate::error::{AppError, AppResult};

const TOKEN_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const BRIDGE_HOST: &str = "neriplayer-youtube-bridge.invalid";
static ACCESS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static TOKENS: LazyLock<Mutex<VecDeque<Token>>> = LazyLock::new(|| Mutex::new(VecDeque::new()));

struct Token {
    fingerprint: String,
    video_id: Option<String>,
    value: String,
    minted_at: Instant,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MintResult {
    token: String,
    video_bound: bool,
}

trait CloseWindow {
    fn close_window(&self);
}

impl CloseWindow for WebviewWindow {
    fn close_window(&self) {
        let _ = self.close();
    }
}

struct WindowGuard<W: CloseWindow = WebviewWindow>(W);
impl<W: CloseWindow> Drop for WindowGuard<W> {
    fn drop(&mut self) {
        self.0.close_window();
    }
}

fn allowed_navigation(url: &url::Url) -> bool {
    url.as_str() == "about:blank"
        || (url.scheme() == "https"
            && url.port_or_known_default() == Some(443)
            && matches!(
                url.host_str(),
                Some("www.youtube.com" | "music.youtube.com")
            )
            && url.username().is_empty()
            && url.password().is_none())
}

fn parse_bridge_result(url: &url::Url, nonce: &str) -> Option<MintResult> {
    if url.scheme() != "https"
        || url.host_str() != Some(BRIDGE_HOST)
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != format!("/{nonce}")
    {
        return None;
    }
    let mut pairs = url.query_pairs().filter(|(key, _)| key == "payload");
    let payload = pairs.next()?.1;
    if pairs.next().is_some() || url.fragment().is_some() || payload.len() > 16384 {
        return None;
    }
    let result: MintResult = serde_json::from_str(&payload).ok()?;
    if result.token.is_empty()
        || result.token.len() > 8192
        || result.token.chars().any(char::is_control)
    {
        return None;
    }
    Some(result)
}

fn mint_script(video_id: &str, visitor_data: &str, authenticated: bool, nonce: &str) -> String {
    let config = serde_json::json!({
        "videoId": video_id, "visitorData": visitor_data, "authenticated": authenticated,
        "callback": format!("https://{BRIDGE_HOST}/{nonce}")
    });
    // 与 Android WebPoTokenProvider 使用相同 factory、内容绑定与 SDF:notready 退避
    format!(
        r#"(async()=>{{
      if(window.__neriMintStarted)return;window.__neriMintStarted=true;
      const cfg={config}, deadline=Date.now()+20000;
      const wait=ms=>new Promise(resolve=>setTimeout(resolve,ms));
      const get=k=>{{try{{return window.ytcfg?.get?.(k)??window.ytcfg?.data_?.[k]}}catch(_){{return null}}}};
      const find=()=>{{try{{const w=window.top;for(const key of Object.getOwnPropertyNames(w)){{const f=w[key]?.bevasrs?.wpc;if(typeof f==='function')return f;}}}}catch(_){{}}return null;}};
      while(Date.now()<deadline){{
        const factory=find();
        if(!factory){{await wait(500);continue;}}
        const contexts=get('WEB_PLAYER_CONTEXT_CONFIGS')||{{}};
        const videoBound=Object.values(contexts).some(c=>String(c?.serializedExperimentFlags||'').includes('html5_generate_content_po_token=true'));
        const binding=videoBound?cfg.videoId:cfg.authenticated?String(get('DATASYNC_ID')||get('datasyncId')||''):String(get('VISITOR_DATA')||get('INNERTUBE_CONTEXT')?.client?.visitorData||cfg.visitorData);
        if(!binding)return;
        try{{
          const client=await factory();const token=String(await client.mws({{c:binding,mc:false,me:false}})||'');
          if(token&&Date.now()<deadline){{location.href=cfg.callback+'?payload='+encodeURIComponent(JSON.stringify({{token,videoBound}}));return;}}
        }}catch(error){{if(!String(error).includes('SDF:notready'))return;}}
        await wait(1000);
      }}
    }})().catch(()=>{{}});"#
    )
}

pub(super) async fn mint(
    app: &AppHandle,
    auth: Option<&YouTubeAuth>,
    fingerprint: &str,
    video_id: &str,
    visitor_data: &str,
    remote_host: &str,
    force_refresh: bool,
) -> AppResult<String> {
    let _access = ACCESS.lock().await;
    if app.get_webview_window("youtube-login").is_some() {
        return Err(AppError::Api(
            "YouTube playback token waits for foreground login".into(),
        ));
    }
    let fingerprint = format!("{remote_host}|{fingerprint}");
    if let Ok(mut tokens) = TOKENS.lock() {
        tokens.retain(|token| {
            token.minted_at.elapsed() < TOKEN_TTL && token.fingerprint == fingerprint
        });
        if !force_refresh {
            if let Some(token) = tokens
                .iter()
                .find(|token| token.video_id.as_deref().is_none_or(|id| id == video_id))
            {
                return Ok(token.value.clone());
            }
        }
    }
    let mut result = None;
    // 前台按 Android 顺序回退到 Music 首页，每次尝试均独立持有窗口 guard
    for page_url in [
        "https://www.youtube.com/?themeRefresh=1",
        "https://music.youtube.com/",
    ] {
        if let Ok(value) = mint_on_page(app, auth, video_id, visitor_data, page_url).await {
            result = Some(value);
            break;
        }
    }
    let result =
        result.ok_or_else(|| AppError::Api("YouTube playback token pages unavailable".into()))?;
    if let Ok(mut tokens) = TOKENS.lock() {
        tokens.push_back(Token {
            fingerprint,
            video_id: result.video_bound.then(|| video_id.into()),
            value: result.token.clone(),
            minted_at: Instant::now(),
        });
        while tokens.len() > 16 {
            tokens.pop_front();
        }
    }
    Ok(result.token)
}

async fn mint_on_page(
    app: &AppHandle,
    auth: Option<&YouTubeAuth>,
    video_id: &str,
    visitor_data: &str,
    page_url: &str,
) -> AppResult<MintResult> {
    let nonce = uuid::Uuid::new_v4().to_string();
    let label = format!("youtube-token-{nonce}");
    let script = mint_script(
        video_id,
        visitor_data,
        auth.is_some_and(YouTubeAuth::has_login),
        &nonce,
    );
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(result_tx)));
    let builder_app = app.clone();
    let (created_tx, created_rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let window = WebviewWindowBuilder::new(
            &builder_app,
            &label,
            WebviewUrl::External("about:blank".parse().expect("static URL")),
        )
        .visible(false)
        .skip_taskbar(true)
        .incognito(true)
        .user_agent(super::client::USER_AGENT)
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .on_navigation(move |url| {
            if let Some(result) = parse_bridge_result(url, &nonce) {
                if let Ok(mut sender) = sender.lock() {
                    if let Some(sender) = sender.take() {
                        let _ = sender.send(result);
                    }
                }
                return false;
            }
            allowed_navigation(url)
        })
        .on_page_load(move |window, payload| {
            if payload.event() == PageLoadEvent::Finished
                && allowed_navigation(payload.url())
                && payload.url().scheme() == "https"
            {
                let _ = window.eval(&script);
            }
        })
        .build()
        .map(WindowGuard);
        // receiver 超时或取消时仍由 guard 关闭刚创建的窗口
        let _ = created_tx.send(window);
    })
    .map_err(|_| AppError::Api("YouTube token window could not be scheduled".into()))?;
    let guard = tokio::time::timeout(Duration::from_secs(5), created_rx)
        .await
        .map_err(|_| AppError::Api("YouTube token window creation timed out".into()))?
        .map_err(|_| AppError::Api("YouTube token window creation cancelled".into()))?
        .map_err(|_| AppError::Api("YouTube token window unavailable".into()))?;
    if let Some(auth) = auth {
        for cookie in &auth.cookies {
            let domain = cookie.domain.trim_start_matches('.');
            if !(domain == "youtube.com" || domain.ends_with(".youtube.com"))
                || cookie.name.is_empty()
            {
                continue;
            }
            let cookie = tauri::webview::Cookie::build((cookie.name.clone(), cookie.value.clone()))
                .domain(cookie.domain.clone())
                .path("/")
                .secure(true)
                .http_only(true)
                .build();
            guard.0.set_cookie(cookie).map_err(|_| {
                AppError::Api("YouTube token session cookies could not be initialized".into())
            })?;
        }
    }
    guard
        .0
        .navigate(page_url.parse().expect("static URL"))
        .map_err(|_| AppError::Api("YouTube token page unavailable".into()))?;
    let result = tokio::time::timeout(Duration::from_secs(25), result_rx)
        .await
        .map_err(|_| AppError::Api("YouTube playback token timed out".into()))?
        .map_err(|_| AppError::Api("YouTube playback token cancelled".into()))?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_is_limited_and_callback_requires_nonce() {
        for url in [
            "https://www.youtube.com/",
            "https://music.youtube.com/",
            "about:blank",
        ] {
            assert!(allowed_navigation(&url.parse().unwrap()));
        }
        for url in [
            "https://accounts.google.com/",
            "https://youtube.com.evil.test/",
            "http://music.youtube.com/",
            "https://www.youtube.com@evil.test/",
            "https://www.youtube.com:8443/",
        ] {
            assert!(!allowed_navigation(&url.parse().unwrap()));
        }
        let mut url = url::Url::parse("https://neriplayer-youtube-bridge.invalid/nonce").unwrap();
        url.query_pairs_mut()
            .append_pair("payload", r#"{"token":"test","videoBound":true}"#);
        assert!(parse_bridge_result(&url, "nonce").unwrap().video_bound);
        assert!(parse_bridge_result(&url, "wrong").is_none());
        for invalid in [
            "https://user@neriplayer-youtube-bridge.invalid/nonce?payload=%7B%22token%22:%22test%22,%22videoBound%22:true%7D",
            "https://neriplayer-youtube-bridge.invalid:8443/nonce?payload=%7B%22token%22:%22test%22,%22videoBound%22:true%7D",
            "https://neriplayer-youtube-bridge.invalid/nonce?payload=%7B%22token%22:%22test%22,%22videoBound%22:true%7D&payload=%7B%7D",
        ] {
            assert!(parse_bridge_result(&invalid.parse().unwrap(), "nonce").is_none());
        }
    }

    #[tokio::test]
    async fn cancelled_window_creation_closes_before_and_after_channel_send() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct FakeWindow(Arc<AtomicUsize>);
        impl CloseWindow for FakeWindow {
            fn close_window(&self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        for before_send in [true, false] {
            let closed = Arc::new(AtomicUsize::new(0));
            let (sender, receiver) = tokio::sync::oneshot::channel();
            let guard = WindowGuard(FakeWindow(closed.clone()));
            if before_send {
                drop(receiver);
                drop(sender.send(Ok::<_, ()>(guard)));
            } else {
                sender.send(Ok::<_, ()>(guard)).ok().unwrap();
                drop(receiver);
            }
            assert_eq!(closed.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn token_script_uses_android_content_binding_and_escapes_external_input() {
        use rquickjs::{Context, Runtime};
        for (authenticated, video_bound, expected) in [
            (false, false, "page-visitor"),
            (true, false, "page||account"),
            (true, true, "video');globalThis.injected=true;//"),
        ] {
            let runtime = Runtime::new().unwrap();
            let context = Context::full(&runtime).unwrap();
            context.with(|ctx| {
                let config = serde_json::json!({
                    "VISITOR_DATA": "page-visitor",
                    "DATASYNC_ID": "page||account",
                    "WEB_PLAYER_CONTEXT_CONFIGS": {"main": {"serializedExperimentFlags": if video_bound {"html5_generate_content_po_token=true"} else {""}}}
                });
                ctx.eval::<(), _>(format!("globalThis.location={{}};globalThis.window={{}};window.top=window;window.ytcfg={{get:key=>({config})[key]}};window.factory={{bevasrs:{{wpc:async()=>({{mws:async options=>{{globalThis.binding=options.c;return 'local-fixture-token'}}}})}}}};")).unwrap();
                ctx.eval::<(), _>(mint_script("video');globalThis.injected=true;//", "fallback-visitor", authenticated, "nonce")).unwrap();
            });
            while runtime.execute_pending_job().unwrap() {}
            context.with(|ctx| {
                assert_eq!(
                    ctx.eval::<String, _>("globalThis.binding").unwrap(),
                    expected
                );
                assert_eq!(
                    ctx.eval::<String, _>("typeof globalThis.injected").unwrap(),
                    "undefined"
                );
                let callback = ctx.eval::<String, _>("location.href").unwrap();
                assert_eq!(
                    parse_bridge_result(&callback.parse().unwrap(), "nonce")
                        .unwrap()
                        .token,
                    "local-fixture-token"
                );
            });
        }
    }
}
