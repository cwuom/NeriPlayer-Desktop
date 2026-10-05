use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::sync::Mutex as TokioMutex;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use super::protocol::LtSocketEnvelope;

const WS_WRITE_TIMEOUT: Duration = Duration::from_secs(15);
const WS_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const WS_CLOSE_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LtSocketMessage<'a> {
    connection_id: &'a str,
    #[serde(flatten)]
    envelope: &'a LtSocketEnvelope,
}

/// WebSocket 客户端：管理与一起听服务器的连接
pub struct LtWsClient {
    tx: mpsc::UnboundedSender<String>,
    shutdown: watch::Sender<bool>,
    // 读写循环句柄都必须在 disconnect/Drop 时终止，否则 split 的任一半仍会
    // 持有底层 TcpStream，半开连接会把 socket FD 与 tokio task 留到进程退出（MK-01）
    read_handle: tokio::task::JoinHandle<()>,
    write_handle: tokio::task::JoinHandle<()>,
}

impl LtWsClient {
    /// 建立 WebSocket 连接并启动读写循环
    pub async fn connect(ws_url: &str, app_handle: AppHandle) -> Result<Self, String> {
        let (ws_stream, _) =
            tokio::time::timeout(WS_CONNECT_TIMEOUT, tokio_tungstenite::connect_async(ws_url))
                .await
                .map_err(|_| "WebSocket connect timed out".to_string())?
                .map_err(|e| format!("WebSocket connect failed: {e}"))?;

        let (mut ws_write, mut ws_read) = ws_stream.split();

        // 写通道：前端 -> WS
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        // 关闭信号
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        // 每条连接使用独立 ID，旧连接的异步收尾事件不能影响新连接
        let connection_id = Uuid::new_v4().to_string();

        let _ = app_handle.emit(
            "lt:connected",
            serde_json::json!({ "connectionId": connection_id }),
        );

        // 写循环
        let handle_w = app_handle.clone();
        let writer_connection_id = connection_id.clone();
        let mut writer_shutdown_rx = shutdown_rx.clone();
        let writer_shutdown = shutdown_tx.clone();
        let write_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    msg = rx.recv() => {
                        match msg {
                            Some(text) => {
                                match tokio::time::timeout(
                                    WS_WRITE_TIMEOUT,
                                    ws_write.send(Message::Text(text)),
                                )
                                .await
                                {
                                    Ok(Ok(())) => {}
                                    Ok(Err(e)) => {
                                        log::error!(target: "lt-ws", "write error: {e}");
                                        break;
                                    }
                                    Err(_) => {
                                        log::error!(
                                            target: "lt-ws",
                                            "write timed out after {}s",
                                            WS_WRITE_TIMEOUT.as_secs(),
                                        );
                                        break;
                                    }
                                }
                            }
                            None => break,
                        }
                    }
                    changed = writer_shutdown_rx.changed() => {
                        if changed.is_err() || *writer_shutdown_rx.borrow() {
                            let _ = tokio::time::timeout(WS_CLOSE_TIMEOUT, ws_write.close()).await;
                        }
                        break;
                    }
                }
            }
            let _ = writer_shutdown.send(true);
            let _ = handle_w.emit(
                "lt:disconnected",
                serde_json::json!({
                    "connectionId": writer_connection_id,
                    "code": 1000,
                    "reason": "client_closed"
                }),
            );
        });

        // 读循环
        let handle_r = app_handle.clone();
        let reader_connection_id = connection_id;
        let mut reader_shutdown_rx = shutdown_rx;
        let reader_shutdown = shutdown_tx.clone();
        let read_handle = tokio::spawn(async move {
            loop {
                let result = tokio::select! {
                    result = ws_read.next() => result,
                    changed = reader_shutdown_rx.changed() => {
                        if changed.is_err() || *reader_shutdown_rx.borrow() {
                            break;
                        }
                        continue;
                    }
                };
                let Some(result) = result else { break };
                match result {
                    Ok(Message::Text(text)) => {
                        // 尝试解析为 envelope 并转发给前端
                        match serde_json::from_str::<LtSocketEnvelope>(&text) {
                            Ok(envelope) => {
                                let _ = handle_r.emit(
                                    "lt:message",
                                    LtSocketMessage {
                                        connection_id: &reader_connection_id,
                                        envelope: &envelope,
                                    },
                                );
                            }
                            Err(e) => {
                                log::warn!(target: "lt-ws", "parse error: {e}");
                            }
                        }
                    }
                    Ok(Message::Close(frame)) => {
                        let (code, reason) = frame
                            .map(|f| (f.code.into(), f.reason.to_string()))
                            .unwrap_or((1000u16, "closed".to_string()));
                        let _ = handle_r.emit(
                            "lt:disconnected",
                            serde_json::json!({
                                "connectionId": reader_connection_id,
                                "code": code,
                                "reason": reason
                            }),
                        );
                        break;
                    }
                    Ok(Message::Ping(data)) => {
                        // tungstenite 自动回 pong，忽略
                        let _ = data;
                    }
                    Err(e) => {
                        let _ = handle_r.emit(
                            "lt:disconnected",
                            serde_json::json!({
                                "connectionId": reader_connection_id,
                                "code": 1006,
                                "reason": format!("read error: {e}")
                            }),
                        );
                        break;
                    }
                    _ => {}
                }
            }
            let _ = reader_shutdown.send(true);
        });

        Ok(Self {
            tx,
            shutdown: shutdown_tx,
            read_handle,
            write_handle,
        })
    }

    /// 发送事件到 WebSocket
    pub fn send(&self, json: &str) -> Result<(), String> {
        self.tx
            .send(json.to_string())
            .map_err(|e| format!("send failed: {e}"))
    }

    /// 发送 ping
    pub fn send_ping(&self, client_time_ms: Option<i64>) -> Result<(), String> {
        let timestamp = client_time_ms.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64
        });
        let payload = format!(r#"{{"type":"np_ping","t":{timestamp}}}"#);
        self.send(&payload)
    }

    /// 断开连接
    pub async fn disconnect(&self) {
        // 先广播关闭信号，再终止两个循环，避免任一 half 在半开连接上悬挂
        let _ = self.shutdown.send(true);
        self.read_handle.abort();
        self.write_handle.abort();
    }
}

impl Drop for LtWsClient {
    fn drop(&mut self) {
        // 兜底：即便未显式 disconnect，销毁时也终止读写循环，防止 FD/任务泄漏
        let _ = self.shutdown.send(true);
        self.read_handle.abort();
        self.write_handle.abort();
    }
}

/// 全局 WS 客户端引用（存在 AppState 中）
pub type SharedWsClient = Arc<TokioMutex<Option<LtWsClient>>>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn connection_identity_is_added_only_to_local_socket_messages() {
        let envelope: LtSocketEnvelope = serde_json::from_value(json!({
            "type": "np_pong",
            "t": 123,
            "nowMs": 456
        }))
        .unwrap();
        let payload = serde_json::to_value(LtSocketMessage {
            connection_id: "connection-1",
            envelope: &envelope,
        })
        .unwrap();

        assert_eq!(payload["connectionId"], "connection-1");
        assert_eq!(payload["type"], "np_pong");
        assert_eq!(payload["t"], 123);
        assert!(serde_json::to_value(envelope)
            .unwrap()
            .get("connectionId")
            .is_none());
    }
}
