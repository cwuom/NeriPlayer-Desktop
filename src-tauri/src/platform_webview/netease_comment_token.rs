//! 网易云评论写操作（发评论 / 回复 / 点赞）需要的易盾 checkToken
//! （对齐 Android NeteaseYdDeviceTokenProvider.getCommentToken）：
//! 在隐藏 WebView 里打开网易云首页，加载网页版同款 watchman 脚本取令牌，
//! 再通过导航到保留域名的回调地址把令牌交回来，不给远程页面开放任何 IPC
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{webview::PageLoadEvent, AppHandle, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::webview_args::MainBrowserArgs;
use neri_common::error::{AppError, AppResult};

const PAGE_URL: &str = "https://music.163.com/";
const BRIDGE_HOST: &str = "neriplayer-netease-bridge.invalid";
const WATCHMAN_SCRIPT: &str = "https://acstatic-dun.126.net/tool.min.js";
const WATCHMAN_PRODUCT: &str = "YD00000558929251";
const COMMENT_BUSINESS_ID: &str = "bd5d2f973ef74cd2a61325a412ae54d9";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36 Edg/149.0.0.0";
const MINT_TIMEOUT: Duration = Duration::from_secs(30);
/// 同一时刻只开一个取令牌窗口，连续点赞时排队复用同一条通道
static ACCESS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct WindowGuard(WebviewWindow);
impl Drop for WindowGuard {
    fn drop(&mut self) {
        let _ = self.0.close();
    }
}

fn allowed_navigation(url: &url::Url) -> bool {
    url.as_str() == "about:blank"
        || (url.scheme() == "https"
            && url.port_or_known_default() == Some(443)
            && url.host_str() == Some("music.163.com")
            && url.username().is_empty()
            && url.password().is_none())
}

/// 回调地址必须带本次的 nonce；令牌为空表示网页侧取令牌失败
fn parse_bridge_token(url: &url::Url, nonce: &str) -> Option<String> {
    if url.scheme() != "https"
        || url.host_str() != Some(BRIDGE_HOST)
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.path() != format!("/{nonce}")
    {
        return None;
    }
    let mut pairs = url.query_pairs().filter(|(key, _)| key == "token");
    let token = pairs.next()?.1.into_owned();
    if pairs.next().is_some() || token.len() > 8192 || token.chars().any(char::is_control) {
        return None;
    }
    Some(token)
}

fn token_script(nonce: &str) -> String {
    let callback =
        serde_json::to_string(&format!("https://{BRIDGE_HOST}/{nonce}")).expect("static string");
    format!(
        r#"(function(){{
  if(window.__neriYdStarted)return;window.__neriYdStarted=true;
  var done=false;
  function send(token){{if(done)return;done=true;location.href={callback}+'?token='+encodeURIComponent(token||'');}}
  function run(){{try{{window.WM.getToken('{COMMENT_BUSINESS_ID}',function(token){{send(token);}});}}catch(_){{send('');}}}}
  if(window.WM&&typeof window.WM.getToken==='function'){{run();return;}}
  var script=document.createElement('script');
  script.src='{WATCHMAN_SCRIPT}';
  script.onload=function(){{try{{initWatchman({{productNumber:'{WATCHMAN_PRODUCT}',onload:function(instance){{window.WM=instance;run();}}}});}}catch(_){{send('');}}}};
  script.onerror=function(){{send('');}};
  document.head.appendChild(script);
}})();"#
    )
}

/// 每次写操作现取一次令牌（易盾令牌短时有效，Android 同样不缓存）
pub async fn mint_comment_token(app: &AppHandle) -> AppResult<String> {
    let _access = ACCESS.lock().await;
    let nonce = uuid::Uuid::new_v4().to_string();
    let label = format!("netease-comment-token-{nonce}");
    let script = token_script(&nonce);
    let (result_tx, result_rx) = tokio::sync::oneshot::channel::<String>();
    let sender = Arc::new(Mutex::new(Some(result_tx)));
    let builder_app = app.clone();
    let (created_tx, created_rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let window = WebviewWindowBuilder::new(
            &builder_app,
            &label,
            WebviewUrl::External(PAGE_URL.parse().expect("static URL")),
        )
        .visible(false)
        .skip_taskbar(true)
        .user_agent(USER_AGENT)
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .on_navigation(move |url| {
            if let Some(token) = parse_bridge_token(url, &nonce) {
                if let Some(sender) = sender.lock().ok().and_then(|mut sender| sender.take()) {
                    let _ = sender.send(token);
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
        .main_browser_args(&builder_app)
        .build()
        .map(WindowGuard);
        let _ = created_tx.send(window);
    })
    .map_err(|_| AppError::Api("NetEase comment token window could not be scheduled".into()))?;
    let _guard = tokio::time::timeout(Duration::from_secs(5), created_rx)
        .await
        .map_err(|_| AppError::Api("NetEase comment token window creation timed out".into()))?
        .map_err(|_| AppError::Api("NetEase comment token window creation cancelled".into()))?
        .map_err(|_| AppError::Api("NetEase comment token window unavailable".into()))?;
    let token = tokio::time::timeout(MINT_TIMEOUT, result_rx)
        .await
        .map_err(|_| AppError::Api("NetEase comment verification timed out".into()))?
        .map_err(|_| AppError::Api("NetEase comment verification cancelled".into()))?;
    if token.is_empty() {
        return Err(AppError::Api(
            "NetEase comment verification unavailable".into(),
        ));
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_is_limited_to_netease_home() {
        for url in [
            "https://music.163.com/",
            "https://music.163.com/#/discover",
            "about:blank",
        ] {
            assert!(allowed_navigation(&url.parse().unwrap()));
        }
        for url in [
            "http://music.163.com/",
            "https://music.163.com.evil.test/",
            "https://user@music.163.com/",
            "https://music.163.com:8443/",
            "https://acstatic-dun.126.net/",
        ] {
            assert!(!allowed_navigation(&url.parse().unwrap()));
        }
    }

    #[test]
    fn bridge_token_requires_matching_nonce() {
        let url: url::Url = format!("https://{BRIDGE_HOST}/abc?token=tok%2B1")
            .parse()
            .unwrap();
        assert_eq!(parse_bridge_token(&url, "abc").as_deref(), Some("tok+1"));
        assert!(parse_bridge_token(&url, "other").is_none());
        let empty: url::Url = format!("https://{BRIDGE_HOST}/abc?token=").parse().unwrap();
        assert_eq!(parse_bridge_token(&empty, "abc").as_deref(), Some(""));
        for invalid in [
            format!("https://{BRIDGE_HOST}/abc?token=a&token=b"),
            format!("https://{BRIDGE_HOST}/abc?token=a#x"),
            format!("http://{BRIDGE_HOST}/abc?token=a"),
            "https://music.163.com/abc?token=a".to_string(),
        ] {
            assert!(parse_bridge_token(&invalid.parse().unwrap(), "abc").is_none());
        }
    }
}
