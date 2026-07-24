use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;
use tokio::task::JoinHandle;

use crate::sync::models::SyncSong;

use crate::audio::player::PlayerEngine;
use crate::audio::queue::PlayQueue;
use crate::auth::state::AuthState;
use crate::listen_together::session::LtSession;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

struct HttpClientSnapshot {
    client: reqwest::Client,
    bypass_proxy: bool,
}

pub struct DownloadTaskControl {
    pub cancel_flag: Arc<AtomicBool>,
    pub handle: JoinHandle<()>,
}

/// 全局应用状态，通过 tauri::State 注入
pub struct AppState {
    pub player: Arc<Mutex<PlayerEngine>>,
    pub playback_generation: Arc<AtomicU64>,
    pub queue: Mutex<PlayQueue>,
    http: parking_lot::RwLock<HttpClientSnapshot>,
    /// 共享 Cookie Jar：允许外部注入持久化登录 Cookie
    pub cookie_jar: Arc<reqwest::cookie::Jar>,
    /// 三平台登录状态
    pub auth: Mutex<AuthState>,
    /// Shared WebView and HTTP cookie mutations must not overlap across auth commands.
    pub auth_cookie_gate: tokio::sync::Mutex<()>,
    /// 一起听会话
    pub lt_session: Mutex<LtSession>,
    /// 后台下载任务
    pub download_tasks: Mutex<HashMap<String, DownloadTaskControl>>,
    /// YouTube 会话保鲜闸门(冷却 + 熔断), 对齐 Android 自动刷新
    pub youtube_refresh: Mutex<crate::api::youtube::refresh::YouTubeRefreshGate>,
}

impl AppState {
    pub fn new() -> Self {
        let jar = Arc::new(reqwest::cookie::Jar::default());
        let http = reqwest::Client::builder()
            .cookie_provider(jar.clone())
            .user_agent(USER_AGENT)
            .no_proxy()
            .build()
            .expect("Failed to create HTTP client");

        let playback_generation = Arc::new(AtomicU64::new(0));
        Self {
            player: Arc::new(Mutex::new(PlayerEngine::with_playback_generation(
                playback_generation.clone(),
            ))),
            playback_generation,
            queue: Mutex::new(PlayQueue::new()),
            http: parking_lot::RwLock::new(HttpClientSnapshot {
                client: http,
                bypass_proxy: true,
            }),
            cookie_jar: jar,
            auth: Mutex::new(AuthState::default()),
            auth_cookie_gate: tokio::sync::Mutex::new(()),
            lt_session: Mutex::new(LtSession::new()),
            download_tasks: Mutex::new(HashMap::new()),
            youtube_refresh: Mutex::new(crate::api::youtube::refresh::YouTubeRefreshGate::default()),
        }
    }

    /// 重建 HTTP Client，切换代理模式
    pub fn rebuild_http(&self, bypass_proxy: bool) {
        let mut builder = reqwest::Client::builder()
            .cookie_provider(self.cookie_jar.clone())
            .user_agent(USER_AGENT);
        if bypass_proxy {
            builder = builder.no_proxy();
        }
        if let Ok(client) = builder.build() {
            *self.http.write() = HttpClientSnapshot {
                client,
                bypass_proxy,
            };
        }
    }

    /// Build a short-lived client with an isolated cookie jar while retaining the
    /// application's active proxy policy and request identity.
    pub fn http_with_cookie_jar(
        &self,
        cookie_jar: Arc<reqwest::cookie::Jar>,
    ) -> Result<reqwest::Client, reqwest::Error> {
        let bypass_proxy = self.http.read().bypass_proxy;
        let mut builder = reqwest::Client::builder()
            .cookie_provider(cookie_jar)
            .user_agent(USER_AGENT);
        if bypass_proxy {
            builder = builder.no_proxy();
        }
        builder.build()
    }

    /// 获取当前 HTTP Client 的克隆（O(1)，reqwest::Client 内部是 Arc）
    pub fn http(&self) -> reqwest::Client {
        self.http.read().client.clone()
    }
}

/// 曲目信息（前后端共享）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackInfo {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub source: TrackSource,
    /// 本地文件路径或远程 URL
    pub url: String,
    pub cover_url: Option<String>,
    #[serde(default, alias = "addedAt")]
    pub added_at: i64,
    #[serde(default, alias = "syncPayload", skip_serializing_if = "Option::is_none")]
    pub sync_payload: Option<SyncSong>,
    #[serde(default, alias = "playlistKey", skip_serializing_if = "Option::is_none")]
    pub playlist_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TrackSource {
    Local,
    Netease,
    Qq,
    Bilibili,
    Youtube,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RepeatMode {
    Off,
    All,
    One,
}
