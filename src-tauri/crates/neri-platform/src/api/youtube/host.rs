//! 取流时需要宿主提供的能力：系统代理策略与隐藏 WebView 里的 PoToken 铸造。
//! 平台层不依赖 Tauri；桌面端实现见 app 的 `platform_webview` 模块

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use neri_common::error::AppResult;

use crate::auth::state::YouTubeAuth;

pub type PoTokenFuture = Pin<Box<dyn Future<Output = AppResult<String>> + Send + 'static>>;

/// PoToken 铸造请求；fingerprint 已按 `bootstrap::auth_fingerprint` 计算
pub struct PoTokenRequest {
    pub auth: YouTubeAuth,
    pub fingerprint: String,
    pub video_id: String,
    pub visitor_data: String,
    pub remote_host: String,
}

pub trait YouTubeHost: Send + Sync {
    /// 用户关闭了「走系统代理」时，取流客户端直连
    fn bypasses_system_proxy(&self) -> bool;
    /// 只查缓存，不触发铸造
    fn cached_po_token(
        &self,
        fingerprint: &str,
        remote_host: &str,
        video_id: &str,
    ) -> Option<String>;
    /// 后台预热令牌，不等待结果
    fn warm_po_token(&self, request: PoTokenRequest);
    /// 铸造（或复用缓存的）令牌
    fn mint_po_token(&self, request: PoTokenRequest, force_refresh: bool) -> PoTokenFuture;
}

pub type SharedYouTubeHost = Arc<dyn YouTubeHost>;
