//! 依赖隐藏 WebView 的平台能力：网易云评论令牌与 YouTube PoToken。
//! 平台 crate 只声明需要的能力（`YouTubeHost`），这里用 Tauri 窗口实现

pub mod netease_comment_token;
pub mod youtube_web_po;

use std::sync::Arc;

use neri_platform::api::youtube::host::{
    PoTokenFuture, PoTokenRequest, SharedYouTubeHost, YouTubeHost,
};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

struct TauriYouTubeHost(AppHandle);

impl YouTubeHost for TauriYouTubeHost {
    fn bypasses_system_proxy(&self) -> bool {
        self.0
            .try_state::<AppState>()
            .map(|state| state.bypasses_system_proxy())
            .unwrap_or(true)
    }

    fn cached_po_token(
        &self,
        fingerprint: &str,
        remote_host: &str,
        video_id: &str,
    ) -> Option<String> {
        youtube_web_po::cached_token(fingerprint, remote_host, video_id)
    }

    fn warm_po_token(&self, request: PoTokenRequest) {
        youtube_web_po::warm(
            self.0.clone(),
            request.auth,
            request.video_id,
            request.visitor_data,
            request.remote_host,
        );
    }

    fn mint_po_token(&self, request: PoTokenRequest, force_refresh: bool) -> PoTokenFuture {
        let app = self.0.clone();
        Box::pin(async move {
            youtube_web_po::mint(
                &app,
                Some(&request.auth),
                &request.fingerprint,
                &request.video_id,
                &request.visitor_data,
                &request.remote_host,
                force_refresh,
            )
            .await
        })
    }
}

pub fn youtube_host(app: &AppHandle) -> SharedYouTubeHost {
    Arc::new(TauriYouTubeHost(app.clone()))
}
