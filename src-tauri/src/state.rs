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

/// TCP 建连超时：SYN 黑洞网络下 OS 默认 TCP 超时（macOS 约 75s）太长，
/// 会拖垮代理兜底通道的切换速度
const HTTP_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// 单次读超时：这是「两次读到数据之间」的间隔上限，不是总时长上限——
/// 流式播放/大文件下载只要在稳定传输就不会触发；只有连接假死
/// （半开 TCP、服务器停止发数据）才会在 30s 后报错，让上层可重试。
/// 刻意不设总超时（`.timeout()`），否则会杀掉正常的长流式请求
const HTTP_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub struct DownloadTaskControl {
    pub cancel_flag: Arc<AtomicBool>,
    pub handle: JoinHandle<()>,
}

/// 全局应用状态，通过 tauri::State 注入
/// 媒体会话元数据镜像（前端切歌时推送，供 ticker 更新 SMTC/MPRIS，PB-01）
#[derive(Clone, Default)]
pub struct MediaMetadataMirror {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub cover_url: Option<String>,
    pub duration_ms: u64,
}

pub struct AppState {
    pub player: Arc<Mutex<PlayerEngine>>,
    pub playback_generation: Arc<AtomicU64>,
    pub queue: Mutex<PlayQueue>,
    pub http: parking_lot::RwLock<reqwest::Client>,
    /// 共享 Cookie Jar：允许外部注入持久化登录 Cookie
    pub cookie_jar: Arc<reqwest::cookie::Jar>,
    /// 三平台登录状态
    pub auth: Mutex<AuthState>,
    /// 一起听会话
    pub lt_session: Mutex<LtSession>,
    /// 后台下载任务
    pub download_tasks: Mutex<HashMap<String, DownloadTaskControl>>,
    /// YouTube 会话保鲜闸门(冷却 + 熔断), 对齐 Android 自动刷新
    pub youtube_refresh: Mutex<crate::api::youtube::refresh::YouTubeRefreshGate>,
    /// YouTube SIDTS 主动轮换闸门，避免 RotateCookies 请求风暴
    pub youtube_cookie_rotation:
        Mutex<crate::api::youtube::refresh::YouTubeCookieRotationGate>,
    /// 播放统计, 启动时从磁盘恢复
    pub stats: Mutex<crate::stats::StatsStore>,
    /// 前端切歌时镜像的媒体会话元数据（PlayQueue 从不被前端填充, 不能作为元数据源, PB-01）
    pub media_metadata: Mutex<Option<MediaMetadataMirror>>,
    /// 与 http 代理设置相反的备用客户端
    ///
    /// 系统代理配错时直连能通，需要代理才能出网时直连不通 —— 两种情况都存在，
    /// 单一设置无法覆盖。平台请求在网络层失败时用它重试一次，两个方向都能自愈。
    alt_http: parking_lot::RwLock<reqwest::Client>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        let jar = Arc::new(reqwest::cookie::Jar::default());
        let http = reqwest::Client::builder()
            .cookie_provider(jar.clone())
            .user_agent(USER_AGENT)
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .read_timeout(HTTP_READ_TIMEOUT)
            .no_proxy()
            .build()
            .expect("Failed to create HTTP client");

        // 初始客户端直连，备用客户端走系统代理
        let alt = reqwest::Client::builder()
            .cookie_provider(jar.clone())
            .user_agent(USER_AGENT)
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .read_timeout(HTTP_READ_TIMEOUT)
            .build()
            .expect("Failed to create fallback HTTP client");

        let playback_generation = Arc::new(AtomicU64::new(0));
        Self {
            player: Arc::new(Mutex::new(PlayerEngine::with_playback_generation(
                playback_generation.clone(),
            ))),
            playback_generation,
            queue: Mutex::new(PlayQueue::new()),
            http: parking_lot::RwLock::new(http),
            cookie_jar: jar,
            auth: Mutex::new(AuthState::default()),
            lt_session: Mutex::new(LtSession::new()),
            download_tasks: Mutex::new(HashMap::new()),
            youtube_refresh: Mutex::new(crate::api::youtube::refresh::YouTubeRefreshGate::default()),
            youtube_cookie_rotation: Mutex::new(
                crate::api::youtube::refresh::YouTubeCookieRotationGate::default(),
            ),
            stats: Mutex::new(crate::stats::load()),
            media_metadata: Mutex::new(None),
            alt_http: parking_lot::RwLock::new(alt),
        }
    }

    /// 重建 HTTP Client，切换代理模式
    pub fn rebuild_http(&self, bypass_proxy: bool) {
        let build = |no_proxy: bool| {
            let mut builder = reqwest::Client::builder()
                .cookie_provider(self.cookie_jar.clone())
                .user_agent(USER_AGENT)
                .connect_timeout(HTTP_CONNECT_TIMEOUT)
                .read_timeout(HTTP_READ_TIMEOUT);
            if no_proxy {
                builder = builder.no_proxy();
            }
            builder.build()
        };
        if let Ok(client) = build(bypass_proxy) {
            *self.http.write() = client;
        }
        // 备用客户端始终取相反设置
        if let Ok(client) = build(!bypass_proxy) {
            *self.alt_http.write() = client;
        }
    }

    /// 代理设置与 http() 相反的备用客户端
    pub fn alt_http(&self) -> reqwest::Client {
        self.alt_http.read().clone()
    }

    /// 带代理兜底的传输通道
    ///
    /// 平台客户端一律经由它构造，代理配错或必须走代理两种情况都能自愈。
    pub fn transport(&self, target: &'static str) -> crate::api::transport::FallbackHttp {
        crate::api::transport::FallbackHttp::with_fallback(
            &self.http(),
            &self.alt_http(),
            target,
        )
    }

    pub fn netease(&self) -> crate::api::netease::client::NeteaseClient {
        let csrf = crate::auth::cookies::read_netease_csrf(&self.cookie_jar);
        crate::api::netease::client::NeteaseClient::with_fallback(&self.http(), &self.alt_http())
            .with_csrf(csrf)
            .with_cookie_jar(self.cookie_jar.clone())
    }

    pub fn youtube(&self) -> crate::api::youtube::client::YouTubeClient {
        crate::api::youtube::client::YouTubeClient::with_transport(self.transport("youtube"))
    }

    pub fn bilibili(&self) -> crate::api::bilibili::client::BiliClient {
        crate::api::bilibili::client::BiliClient::with_transport(self.transport("bilibili"))
            .with_cookie_jar(self.cookie_jar.clone())
    }

    pub fn qq(&self) -> crate::api::qq::client::QqMusicClient {
        crate::api::qq::client::QqMusicClient::with_transport(self.transport("qq"))
    }

    pub fn lrclib(&self) -> crate::api::lrclib::LrcLibClient {
        crate::api::lrclib::LrcLibClient::with_transport(self.transport("lrclib"))
    }

    /// 获取当前 HTTP Client 的克隆（O(1)，reqwest::Client 内部是 Arc）
    pub fn http(&self) -> reqwest::Client {
        self.http.read().clone()
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
