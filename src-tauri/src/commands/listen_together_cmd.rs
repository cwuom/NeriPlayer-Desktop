use crate::listen_together::protocol::*;
use crate::listen_together::session::{LtSessionHttpCredentials, LtSessionUpdate};
use crate::listen_together::ws_client::LtWsClient;
use crate::state::AppState;
use std::time::Duration;
use tauri::{AppHandle, State};

const LT_CONTROL_TIMEOUT: Duration = Duration::from_secs(15);
const LT_LEAVE_TIMEOUT: Duration = Duration::from_secs(3);

#[tauri::command]
pub async fn lt_create_room(
    base_url: String,
    user_uuid: String,
    nickname: String,
    initial_snapshot: LtInitialSnapshot,
    state: State<'_, AppState>,
) -> Result<LtRoomResponse, String> {
    let generation = state.lt_session.lock().generation();
    let http = state.transport("listen-together");
    let url = format!("{}/api/rooms", base_url.trim_end_matches('/'));

    let body = LtCreateRoomRequest {
        user_uuid: user_uuid.clone(),
        nickname: nickname.clone(),
        initial_snapshot,
    };

    let resp = http
        // 建房是非幂等写操作：读超时后重发可能创建两个房间，只有连接尚未建立
        // 时才允许换代理重试
        .send_once(|client| client.post(&url).json(&body))
        .await
        .map_err(|e| format!("HTTP error: {e}"))?;

    // 必须先判状态码：房间服务返回 4xx/5xx 时错误体同样可能是 JSON，
    // 直接解析会得到 ok=false 却没有原因，用户只看到一句 Parse error
    let room_resp: LtRoomResponse =
        crate::api::transport::parse_json_response(resp, "listen-together create room")
            .await
            .map_err(|e| e.to_string())?;

    if room_resp.ok {
        let accepted = state.lt_session.lock().update_room_if_current(
            generation,
            LtSessionUpdate {
                base_url: base_url.clone(),
                room_id: room_resp.room_id.clone(),
                user_uuid,
                nickname,
                token: room_resp.token.clone(),
                ws_url: room_resp.ws_url.clone(),
                member_secret: room_resp.member_secret.clone(),
                join_secret: room_resp.join_secret.clone(),
            },
        );
        if !accepted {
            cleanup_abandoned_room(&http, &base_url, &room_resp).await;
            return Err("Listen-together create room was cancelled".to_string());
        }
    }

    Ok(room_resp)
}

#[tauri::command]
pub async fn lt_join_room(
    base_url: String,
    room_id: String,
    user_uuid: String,
    nickname: String,
    member_secret: Option<String>,
    join_secret: Option<String>,
    state: State<'_, AppState>,
) -> Result<LtRoomResponse, String> {
    let http = state.transport("listen-together");
    let url = format!(
        "{}/api/rooms/{}/join",
        base_url.trim_end_matches('/'),
        room_id
    );

    let (generation, credentials) = {
        let session = state.lt_session.lock();
        (
            session.generation(),
            session.credentials_for_join(&base_url, &room_id, &user_uuid),
        )
    };
    let body = LtJoinRoomRequest {
        user_uuid: user_uuid.clone(),
        nickname: nickname.clone(),
        member_secret: member_secret.or(credentials.member_secret.clone()),
        join_secret: join_secret.or(credentials.join_secret.clone()),
    };

    let resp = http
        // 加入请求同样可能产生服务端成员记录，避免请求体已发出后重复提交
        .send_once(|client| {
            let request = client.post(&url).json(&body);
            match credentials.token.as_deref() {
                Some(token) => request.bearer_auth(token),
                None => request,
            }
        })
        .await
        .map_err(|e| format!("HTTP error: {e}"))?;

    let room_resp: LtRoomResponse =
        crate::api::transport::parse_json_response(resp, "listen-together join room")
            .await
            .map_err(|e| e.to_string())?;

    if room_resp.ok {
        let accepted = state.lt_session.lock().update_room_if_current(
            generation,
            LtSessionUpdate {
                base_url,
                room_id: room_resp.room_id.clone().or(Some(room_id)),
                user_uuid,
                nickname,
                token: room_resp.token.clone(),
                ws_url: room_resp.ws_url.clone(),
                member_secret: room_resp.member_secret.clone(),
                join_secret: room_resp.join_secret.clone(),
            },
        );
        if !accepted {
            return Err("Listen-together join room was cancelled".to_string());
        }
    }

    Ok(room_resp)
}

#[tauri::command]
pub async fn lt_get_room_state(
    base_url: String,
    room_id: String,
    state: State<'_, AppState>,
) -> Result<LtStateResponse, String> {
    let http = state.transport("listen-together");
    let token = state
        .lt_session
        .lock()
        .token_for_room_state(&base_url, &room_id);
    let url = format!(
        "{}/api/rooms/{}/state",
        base_url.trim_end_matches('/'),
        room_id
    );

    let resp = http
        .send(|client| {
            let request = client.get(&url);
            match token.as_deref() {
                Some(token) => request.bearer_auth(token),
                None => request,
            }
        })
        .await
        .map_err(|e| format!("HTTP error: {e}"))?;

    crate::api::transport::parse_json_response(resp, "listen-together room state")
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn lt_send_control(
    event: LtEvent,
    state: State<'_, AppState>,
) -> Result<LtControlResponse, String> {
    let credentials = state
        .lt_session
        .lock()
        .http_credentials()
        .ok_or_else(|| "Listen-together session is unavailable".to_string())?;
    post_room_operation(
        &state.transport("listen-together"),
        &credentials,
        "control",
        &event,
        LT_CONTROL_TIMEOUT,
    )
    .await
}

#[tauri::command]
pub async fn lt_leave_room(
    state: State<'_, AppState>,
) -> Result<Option<LtControlResponse>, String> {
    let Some(credentials) = state.lt_session.lock().begin_leave() else {
        return Ok(None);
    };
    post_room_operation(
        &state.transport("listen-together"),
        &credentials,
        "leave",
        &serde_json::json!({}),
        LT_LEAVE_TIMEOUT,
    )
    .await
    .map(Some)
}

async fn cleanup_abandoned_room(
    http: &crate::api::transport::FallbackHttp,
    base_url: &str,
    response: &LtRoomResponse,
) {
    let Some(room_id) = response
        .room_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let Some(token) = response
        .token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let credentials = LtSessionHttpCredentials {
        base_url: base_url.trim().trim_end_matches('/').to_string(),
        room_id: room_id.to_string(),
        token: token.to_string(),
    };
    match post_room_operation(
        http,
        &credentials,
        "leave",
        &serde_json::json!({}),
        LT_LEAVE_TIMEOUT,
    )
    .await
    {
        Ok(result) if !result.ok => {
            log::warn!(target: "listen-together", "abandoned room cleanup was rejected")
        }
        Err(_) => log::warn!(target: "listen-together", "abandoned room cleanup failed"),
        _ => {}
    }
}

async fn post_room_operation<T: serde::Serialize + ?Sized>(
    http: &crate::api::transport::FallbackHttp,
    credentials: &LtSessionHttpCredentials,
    operation: &str,
    body: &T,
    timeout: Duration,
) -> Result<LtControlResponse, String> {
    let url = format!(
        "{}/api/rooms/{}/{operation}",
        credentials.base_url, credentials.room_id
    );
    tokio::time::timeout(timeout, async {
        let resp = http
            .send_once(|client| {
                client
                    .post(&url)
                    .bearer_auth(&credentials.token)
                    .json(body)
                    .timeout(timeout)
            })
            .await
            .map_err(|e| format!("HTTP error: {e}"))?;

        let status = resp.status();
        if status.is_client_error() {
            // 服务端拒绝控制时回 4xx + {ok:false,error}，这是应答而不是传输失败，
            // 交给前端走与 WebSocket 拒绝相同的恢复路径（对齐 Android HttpControlFallbackOwner）
            let body = resp.bytes().await.map_err(|e| format!("HTTP error: {e}"))?;
            return serde_json::from_slice::<LtControlResponse>(&body).map_err(|_| {
                format!("HTTP {}: listen-together {operation} rejected", status.as_u16())
            });
        }
        crate::api::transport::parse_json_response(resp, &format!("listen-together {operation}"))
            .await
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| format!("Listen-together {operation} timed out"))?
}

#[tauri::command]
pub async fn lt_connect_ws(
    ws_url: String,
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // 替换期间持有同一把锁，避免离房后在途连接再次写回会话
    let ws_arc = state.lt_session.lock().ws_client.clone();
    let mut ws = ws_arc.lock().await;
    if state.lt_session.lock().http_credentials().is_none() {
        return Err("Listen-together session is unavailable".to_string());
    }
    if let Some(old) = ws.take() {
        old.disconnect().await;
    }

    let client = LtWsClient::connect(&ws_url, app_handle).await?;
    *ws = Some(client);

    Ok(())
}

#[tauri::command]
pub async fn lt_disconnect_ws(state: State<'_, AppState>) -> Result<(), String> {
    let ws_arc = state.lt_session.lock().ws_client.clone();
    let mut ws = ws_arc.lock().await;
    if let Some(client) = ws.take() {
        client.disconnect().await;
    }
    state.lt_session.lock().reset();
    Ok(())
}

#[tauri::command]
pub async fn lt_send_event(event: LtEvent, state: State<'_, AppState>) -> Result<bool, String> {
    let ws_arc = state.lt_session.lock().ws_client.clone();
    let ws = ws_arc.lock().await;
    match ws.as_ref() {
        Some(client) => {
            let json =
                serde_json::to_string(&event).map_err(|e| format!("Serialize error: {e}"))?;
            client.send(&json)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

#[tauri::command]
pub async fn lt_send_ping(
    t: Option<i64>,
    legacy: Option<bool>,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let ws_arc = state.lt_session.lock().ws_client.clone();
    let ws = ws_arc.lock().await;
    match ws.as_ref() {
        Some(client) => {
            client.send_ping(t, legacy.unwrap_or(false))?;
            Ok(true)
        }
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::transport::FallbackHttp;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn http() -> FallbackHttp {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        FallbackHttp::new(&client, "listen-together-test")
    }

    async fn mock_server(
        response_body: Option<&'static str>,
    ) -> (LtSessionHttpCredentials, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut buffer = [0_u8; 4096];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0, "request closed before the JSON body arrived");
                request.extend_from_slice(&buffer[..count]);
                let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            let Some(body) = response_body else {
                std::future::pending::<()>().await;
                unreachable!();
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
            String::from_utf8(request).unwrap()
        });
        (
            LtSessionHttpCredentials {
                base_url,
                room_id: "ABC123".into(),
                token: "test-token".into(),
            },
            server,
        )
    }

    #[tokio::test]
    async fn control_fallback_posts_the_same_event_with_session_authorization() {
        let (credentials, server) = mock_server(Some(r#"{"ok":true}"#)).await;
        let event: LtEvent = serde_json::from_value(serde_json::json!({
            "type": "REQUEST_SEEK",
            "eventId": "event-1",
            "clientInstanceId": "instance-1",
            "clientSequence": 2,
            "positionMs": 500,
            "requestTrackStableKey": "netease:42"
        }))
        .unwrap();
        let response = post_room_operation(
            &http(),
            &credentials,
            "control",
            &event,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert!(response.ok);

        let request = server.await.unwrap();
        assert!(request.starts_with("POST /api/rooms/ABC123/control HTTP/1.1\r\n"));
        let (headers, body) = request.split_once("\r\n\r\n").unwrap();
        assert!(headers
            .to_ascii_lowercase()
            .contains("authorization: bearer test-token"));
        let payload: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["eventId"], "event-1");
        assert_eq!(payload["clientSequence"], 2);
        assert_eq!(payload["requestTrackStableKey"], "netease:42");
        assert_eq!(payload["positionMs"], 500);
    }

    #[tokio::test]
    async fn leave_posts_an_empty_body_and_preserves_server_rejection() {
        let (credentials, server) =
            mock_server(Some(r#"{"ok":false,"error":"room_closed"}"#)).await;
        let response = post_room_operation(
            &http(),
            &credentials,
            "leave",
            &serde_json::json!({}),
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert!(!response.ok);
        assert_eq!(response.error.as_deref(), Some("room_closed"));

        let request = server.await.unwrap();
        assert!(request.starts_with("POST /api/rooms/ABC123/leave HTTP/1.1\r\n"));
        let (headers, body) = request.split_once("\r\n\r\n").unwrap();
        assert!(headers
            .to_ascii_lowercase()
            .contains("authorization: bearer test-token"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            serde_json::json!({})
        );
    }

    #[tokio::test]
    async fn abandoned_create_cleanup_uses_only_the_response_room_credentials() {
        let (credentials, server) = mock_server(Some(r#"{"ok":true}"#)).await;
        let response: LtRoomResponse = serde_json::from_value(serde_json::json!({
            "ok": true,
            "roomId": "LATE12",
            "token": "abandoned-token"
        }))
        .unwrap();

        cleanup_abandoned_room(&http(), &credentials.base_url, &response).await;

        let request = server.await.unwrap();
        assert!(request.starts_with("POST /api/rooms/LATE12/leave HTTP/1.1\r\n"));
        let (headers, _) = request.split_once("\r\n\r\n").unwrap();
        assert!(headers
            .to_ascii_lowercase()
            .contains("authorization: bearer abandoned-token"));
    }

    #[tokio::test]
    async fn room_operation_stops_when_the_server_does_not_respond() {
        let (credentials, server) = mock_server(None).await;
        let started = tokio::time::Instant::now();
        let result = post_room_operation(
            &http(),
            &credentials,
            "leave",
            &serde_json::json!({}),
            Duration::from_millis(100),
        )
        .await;
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
