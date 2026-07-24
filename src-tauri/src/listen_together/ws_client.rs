use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::Notify;
use tokio::task::AbortHandle;
use tokio_tungstenite::tungstenite::Message;

use super::protocol::LtSocketEnvelope;

const CONNECTION_PENDING: u8 = 0;
const CONNECTION_ACTIVE: u8 = 1;
const CONNECTION_CLOSED: u8 = 2;

struct ConnectionLifecycle {
    session_id: u64,
    state: AtomicU8,
    ready: Notify,
    event_gate: Mutex<()>,
}

#[derive(Clone)]
pub struct LtWsActivation {
    lifecycle: Arc<ConnectionLifecycle>,
}

impl LtWsActivation {
    pub fn activate(&self, app_handle: &AppHandle) {
        self.lifecycle.activate(app_handle);
    }
}

impl ConnectionLifecycle {
    fn new(session_id: u64) -> Self {
        Self {
            session_id,
            state: AtomicU8::new(CONNECTION_PENDING),
            ready: Notify::new(),
            event_gate: Mutex::new(()),
        }
    }

    fn is_active(&self) -> bool {
        self.state.load(Ordering::Acquire) == CONNECTION_ACTIVE
    }

    async fn wait_until_ready(&self) -> bool {
        loop {
            let notified = self.ready.notified();
            match self.state.load(Ordering::Acquire) {
                CONNECTION_ACTIVE => return true,
                CONNECTION_CLOSED => return false,
                _ => notified.await,
            }
        }
    }

    fn activate(&self, app_handle: &AppHandle) {
        let _event_guard = self.event_gate.lock().unwrap_or_else(|e| e.into_inner());
        if self
            .state
            .compare_exchange(
                CONNECTION_PENDING,
                CONNECTION_ACTIVE,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            self.ready.notify_waiters();
            let _ = app_handle.emit(
                "lt:connected",
                serde_json::json!({ "sessionId": self.session_id }),
            );
        }
    }

    fn deactivate(&self) {
        let _event_guard = self.event_gate.lock().unwrap_or_else(|e| e.into_inner());
        self.state.store(CONNECTION_CLOSED, Ordering::Release);
        self.ready.notify_waiters();
    }

    fn emit_message(&self, app_handle: &AppHandle, envelope: &LtSocketEnvelope) {
        let _event_guard = self.event_gate.lock().unwrap_or_else(|e| e.into_inner());
        if self.is_active() {
            let _ = app_handle.emit(
                "lt:message",
                serde_json::json!({
                    "sessionId": self.session_id,
                    "envelope": envelope,
                }),
            );
        }
    }

    fn emit_disconnected(&self, app_handle: &AppHandle, code: u16, reason: String) {
        let _event_guard = self.event_gate.lock().unwrap_or_else(|e| e.into_inner());
        if self.is_active() {
            self.state.store(CONNECTION_CLOSED, Ordering::Release);
            self.ready.notify_waiters();
            let _ = app_handle.emit(
                "lt:disconnected",
                serde_json::json!({
                    "sessionId": self.session_id,
                    "code": code,
                    "reason": reason,
                }),
            );
        }
    }
}

/// WebSocket client for a Listen Together session.
pub struct LtWsClient {
    session_id: u64,
    tx: mpsc::UnboundedSender<String>,
    shutdown: mpsc::Sender<()>,
    lifecycle: Arc<ConnectionLifecycle>,
    task_abort: AbortHandle,
}

impl LtWsClient {
    /// Establishes a pending socket. Activate it only after installing the
    /// client in shared application state.
    pub async fn connect(
        ws_url: &str,
        session_id: u64,
        app_handle: AppHandle,
    ) -> Result<Self, String> {
        let (mut ws_stream, _) = tokio_tungstenite::connect_async(ws_url)
            .await
            .map_err(|e| format!("WebSocket connect failed: {e}"))?;

        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        let lifecycle = Arc::new(ConnectionLifecycle::new(session_id));
        let task_lifecycle = lifecycle.clone();

        let task = tokio::spawn(async move {
            if !task_lifecycle.wait_until_ready().await {
                let _ = ws_stream.close(None).await;
                return;
            }

            loop {
                tokio::select! {
                    outbound = rx.recv() => {
                        match outbound {
                            Some(text) => {
                                if let Err(error) = ws_stream.send(Message::Text(text.into())).await {
                                    log::error!(target: "lt-ws", "write error: {error}");
                                    task_lifecycle.emit_disconnected(
                                        &app_handle,
                                        1006,
                                        format!("write error: {error}"),
                                    );
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                    inbound = ws_stream.next() => {
                        match inbound {
                            Some(Ok(Message::Text(text))) => {
                                match serde_json::from_str::<LtSocketEnvelope>(&text) {
                                    Ok(envelope) => task_lifecycle.emit_message(&app_handle, &envelope),
                                    Err(error) => {
                                        log::warn!(
                                            target: "lt-ws",
                                            "parse error: {error}, raw: {text}"
                                        );
                                    }
                                }
                            }
                            Some(Ok(Message::Close(frame))) => {
                                let (code, reason) = frame
                                    .map(|value| (value.code.into(), value.reason.to_string()))
                                    .unwrap_or((1000, "closed".to_string()));
                                task_lifecycle.emit_disconnected(&app_handle, code, reason);
                                break;
                            }
                            Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
                            Some(Ok(_)) => {}
                            Some(Err(error)) => {
                                task_lifecycle.emit_disconnected(
                                    &app_handle,
                                    1006,
                                    format!("read error: {error}"),
                                );
                                break;
                            }
                            None => {
                                task_lifecycle.emit_disconnected(
                                    &app_handle,
                                    1006,
                                    "connection ended".to_string(),
                                );
                                break;
                            }
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        let _ = ws_stream.close(None).await;
                        break;
                    }
                }
            }
        });

        Ok(Self {
            session_id,
            tx,
            shutdown: shutdown_tx,
            lifecycle,
            task_abort: task.abort_handle(),
        })
    }

    pub fn session_id(&self) -> u64 {
        self.session_id
    }

    pub fn activation_handle(&self) -> LtWsActivation {
        LtWsActivation {
            lifecycle: self.lifecycle.clone(),
        }
    }

    pub fn send(&self, json: &str) -> Result<(), String> {
        let _event_guard = self
            .lifecycle
            .event_gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !self.lifecycle.is_active() {
            return Err("WebSocket is not connected".to_string());
        }
        self.tx
            .send(json.to_string())
            .map_err(|e| format!("send failed: {e}"))
    }

    pub fn send_ping(&self) -> Result<(), String> {
        self.send(r#"{"type":"ping"}"#)
    }

    pub async fn disconnect(&self) {
        self.lifecycle.deactivate();
        let _ = self.shutdown.try_send(());
        self.task_abort.abort();
    }
}

pub type SharedWsClient = Arc<TokioMutex<Option<LtWsClient>>>;
