use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

use super::ws_client::LtWsClient;

/// 一起听会话状态
pub struct LtSession {
    pub base_url: Option<String>,
    pub room_id: Option<String>,
    pub token: Option<String>,
    pub ws_url: Option<String>,
    pub user_uuid: String,
    pub nickname: String,
    pub ws_client: Arc<TokioMutex<Option<LtWsClient>>>,
    pub ws_revision: Arc<AtomicU64>,
    pub ws_session_id: Arc<AtomicU64>,
    pub ws_generation: Arc<AtomicU64>,
}

impl LtSession {
    pub fn new() -> Self {
        Self {
            base_url: None,
            room_id: None,
            token: None,
            ws_url: None,
            user_uuid: String::new(),
            nickname: String::new(),
            ws_client: Arc::new(TokioMutex::new(None)),
            ws_revision: Arc::new(AtomicU64::new(0)),
            ws_session_id: Arc::new(AtomicU64::new(0)),
            ws_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn reset(&mut self) {
        self.base_url = None;
        self.room_id = None;
        self.token = None;
        self.ws_url = None;
    }
}
