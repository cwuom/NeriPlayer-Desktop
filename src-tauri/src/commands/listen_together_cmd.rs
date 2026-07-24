use crate::listen_together::protocol::*;
use crate::listen_together::ws_client::LtWsClient;
use crate::state::AppState;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::{AppHandle, State};

const LT_WS_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

#[tauri::command]
pub async fn lt_create_room(
    base_url: String,
    user_uuid: String,
    nickname: String,
    initial_snapshot: LtInitialSnapshot,
    state: State<'_, AppState>,
) -> Result<LtRoomResponse, String> {
    let http = state.http();
    let url = format!("{}/api/rooms", base_url.trim_end_matches('/'));

    let body = LtCreateRoomRequest {
        user_uuid: user_uuid.clone(),
        nickname: nickname.clone(),
        initial_snapshot,
    };

    let resp = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("HTTP error: {e}"))?;

    let room_resp: LtRoomResponse = resp.json().await.map_err(|e| format!("Parse error: {e}"))?;

    if room_resp.ok {
        let mut session = state.lt_session.lock();
        session.base_url = Some(base_url);
        session.room_id = room_resp.room_id.clone();
        session.token = room_resp.token.clone();
        session.ws_url = room_resp.ws_url.clone();
        session.user_uuid = user_uuid;
        session.nickname = nickname;
    }

    Ok(room_resp)
}

#[tauri::command]
pub async fn lt_join_room(
    base_url: String,
    room_id: String,
    user_uuid: String,
    nickname: String,
    state: State<'_, AppState>,
) -> Result<LtRoomResponse, String> {
    let http = state.http();
    let url = format!(
        "{}/api/rooms/{}/join",
        base_url.trim_end_matches('/'),
        room_id
    );

    let body = LtJoinRoomRequest {
        user_uuid: user_uuid.clone(),
        nickname: nickname.clone(),
    };

    let resp = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("HTTP error: {e}"))?;

    let room_resp: LtRoomResponse = resp.json().await.map_err(|e| format!("Parse error: {e}"))?;

    if room_resp.ok {
        let mut session = state.lt_session.lock();
        session.base_url = Some(base_url);
        session.room_id = Some(room_id);
        session.token = room_resp.token.clone();
        session.ws_url = room_resp.ws_url.clone();
        session.user_uuid = user_uuid;
        session.nickname = nickname;
    }

    Ok(room_resp)
}

#[tauri::command]
pub async fn lt_get_room_state(
    base_url: String,
    room_id: String,
    state: State<'_, AppState>,
) -> Result<LtStateResponse, String> {
    let http = state.http();
    let token = state.lt_session.lock().token.clone().unwrap_or_default();
    let url = format!(
        "{}/api/rooms/{}/state",
        base_url.trim_end_matches('/'),
        room_id
    );

    let resp = http
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("HTTP error: {e}"))?;

    resp.json::<LtStateResponse>()
        .await
        .map_err(|e| format!("Parse error: {e}"))
}

#[tauri::command]
pub async fn lt_connect_ws(
    ws_url: String,
    session_id: u64,
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let (ws_arc, ws_revision, ws_session_id, ws_generation) = {
        let session = state.lt_session.lock();
        (
            session.ws_client.clone(),
            session.ws_revision.clone(),
            session.ws_session_id.clone(),
            session.ws_generation.clone(),
        )
    };
    let old = {
        let mut ws = ws_arc.lock().await;
        if session_id < ws_generation.load(Ordering::Acquire) {
            return Ok(());
        }
        ws_generation.store(session_id, Ordering::Release);
        ws_session_id.store(session_id, Ordering::Release);
        let revision = ws_revision.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        // Keep the replacement revision paired with the client slot update.
        // Sending holds this same lock, so it linearizes before or after replacement.
        let old = ws.take();
        (old, revision)
    };
    let (old, revision) = old;
    if let Some(old) = old {
        old.disconnect().await;
    }

    let client = tokio::time::timeout(
        LT_WS_CONNECT_TIMEOUT,
        LtWsClient::connect(&ws_url, session_id, app_handle.clone()),
    )
    .await
    .map_err(|_| "WebSocket connect timed out".to_string())??;
    let activation = client.activation_handle();
    let mut ws = ws_arc.lock().await;
    if ws_revision.load(Ordering::Acquire) != revision {
        drop(ws);
        client.disconnect().await;
        return Ok(());
    }
    *ws = Some(client);
    drop(ws);
    activation.activate(&app_handle);

    Ok(())
}

#[tauri::command]
pub async fn lt_disconnect_ws(session_id: u64, state: State<'_, AppState>) -> Result<(), String> {
    let (ws_arc, ws_revision, ws_session_id, ws_generation) = {
        let session = state.lt_session.lock();
        (
            session.ws_client.clone(),
            session.ws_revision.clone(),
            session.ws_session_id.clone(),
            session.ws_generation.clone(),
        )
    };
    let client = {
        let mut ws = ws_arc.lock().await;
        if ws_generation.load(Ordering::Acquire) > session_id {
            return Ok(());
        }
        ws_generation.fetch_max(session_id.saturating_add(1), Ordering::AcqRel);
        let active_session_id = ws_session_id.load(Ordering::Acquire);
        if active_session_id > session_id {
            return Ok(());
        }
        ws_session_id.store(0, Ordering::Release);
        ws_revision.fetch_add(1, Ordering::AcqRel);
        let client = ws.take();
        state.lt_session.lock().reset();
        client
    };
    if let Some(client) = client {
        client.disconnect().await;
    }
    Ok(())
}

#[tauri::command]
pub async fn lt_send_event(
    event: LtEvent,
    session_id: u64,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let (ws_arc, expected_session_id) = {
        let session = state.lt_session.lock();
        (session.ws_client.clone(), session.ws_session_id.clone())
    };
    if expected_session_id.load(Ordering::Acquire) != session_id {
        return Ok(false);
    }
    let ws = ws_arc.lock().await;
    if expected_session_id.load(Ordering::Acquire) != session_id {
        return Ok(false);
    }
    match ws.as_ref() {
        Some(client) if client.session_id() == session_id => {
            let json =
                serde_json::to_string(&event).map_err(|e| format!("Serialize error: {e}"))?;
            client.send(&json)?;
            Ok(true)
        }
        Some(_) => Ok(false),
        None => Ok(false),
    }
}

#[tauri::command]
pub async fn lt_send_ping(session_id: u64, state: State<'_, AppState>) -> Result<bool, String> {
    let (ws_arc, expected_session_id) = {
        let session = state.lt_session.lock();
        (session.ws_client.clone(), session.ws_session_id.clone())
    };
    if expected_session_id.load(Ordering::Acquire) != session_id {
        return Ok(false);
    }
    let ws = ws_arc.lock().await;
    if expected_session_id.load(Ordering::Acquire) != session_id {
        return Ok(false);
    }
    match ws.as_ref() {
        Some(client) if client.session_id() == session_id => {
            client.send_ping()?;
            Ok(true)
        }
        Some(_) => Ok(false),
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use std::sync::OnceLock;
    use std::time::Duration;
    use tauri::{Listener, Manager};
    use tokio::net::TcpListener;
    use tokio::sync::{mpsc, oneshot, Mutex as TokioMutex};
    use tokio::task::JoinHandle;
    use tokio_tungstenite::tungstenite::Message;

    static TEST_GATE: TokioMutex<()> = TokioMutex::const_new(());

    struct TestServer {
        ws_url: String,
        commands: mpsc::UnboundedSender<ServerCommand>,
        messages: TokioMutex<mpsc::UnboundedReceiver<String>>,
        task: JoinHandle<()>,
    }

    struct ServerCommand {
        text: String,
        sent: oneshot::Sender<()>,
    }

    impl TestServer {
        async fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let (commands, mut command_rx) = mpsc::unbounded_channel::<ServerCommand>();
            let (message_tx, messages) = mpsc::unbounded_channel::<String>();
            let task = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                loop {
                    tokio::select! {
                        command = command_rx.recv() => {
                            let Some(command) = command else { break };
                            let _ = socket.send(Message::Text(command.text.into())).await;
                            let _ = command.sent.send(());
                        }
                        inbound = socket.next() => {
                            match inbound {
                                Some(Ok(Message::Text(text))) => {
                                    let _ = message_tx.send(text.to_string());
                                }
                                Some(Ok(_)) => {}
                                Some(Err(_)) | None => break,
                            }
                        }
                    }
                }
            });

            Self {
                ws_url: format!("ws://{address}"),
                commands,
                messages: TokioMutex::new(messages),
                task,
            }
        }

        async fn send_text(&self, text: &str) {
            let (sent, received) = oneshot::channel();
            self.commands
                .send(ServerCommand {
                    text: text.to_string(),
                    sent,
                })
                .unwrap();
            received.await.unwrap();
        }

        async fn receive_text(&self, wait: Duration) -> Option<String> {
            tokio::time::timeout(wait, self.messages.lock().await.recv())
                .await
                .ok()
                .flatten()
        }

        fn stop(self) {
            self.task.abort();
        }
    }

    fn test_app_handle() -> AppHandle {
        static HANDLE: OnceLock<AppHandle> = OnceLock::new();
        HANDLE
            .get_or_init(|| {
                let app = tauri::Builder::default()
                    .manage(AppState::new())
                    .build(tauri::generate_context!())
                    .expect("failed to build test app");
                let handle = app.handle().clone();
                Box::leak(Box::new(app));
                handle
            })
            .clone()
    }

    async fn reset_connection(app_handle: &AppHandle) {
        let (session_id, ws_generation) = {
            let session = app_handle.state::<AppState>().lt_session.lock();
            (
                session.ws_session_id.load(Ordering::Acquire),
                session.ws_generation.clone(),
            )
        };
        lt_disconnect_ws(session_id, app_handle.state())
            .await
            .unwrap();
        ws_generation.store(0, Ordering::Release);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn connected_is_emitted_after_client_is_installed() {
        let _gate = TEST_GATE.lock().await;
        let app_handle = test_app_handle();
        reset_connection(&app_handle).await;
        let server = TestServer::start().await;

        let ws_client = app_handle
            .state::<AppState>()
            .lt_session
            .lock()
            .ws_client
            .clone();
        let (observed_tx, mut observed_rx) = mpsc::unbounded_channel();
        let listener = app_handle.listen("lt:connected", move |event| {
            let installed = ws_client
                .try_lock()
                .map(|client| client.is_some())
                .unwrap_or(false);
            let payload = serde_json::from_str::<serde_json::Value>(event.payload()).unwrap();
            let _ = observed_tx.send((installed, payload));
        });

        lt_connect_ws(
            server.ws_url.clone(),
            1,
            app_handle.clone(),
            app_handle.state(),
        )
        .await
        .unwrap();
        let (installed_when_emitted, connected_payload) =
            tokio::time::timeout(Duration::from_secs(1), observed_rx.recv())
                .await
                .expect("connected event was not emitted")
                .unwrap();

        app_handle.unlisten(listener);
        reset_connection(&app_handle).await;
        server.stop();

        assert!(
            installed_when_emitted,
            "connected was emitted before the client was installed"
        );
        assert_eq!(connected_payload["sessionId"], 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn inbound_message_is_emitted_with_its_backend_session_id() {
        let _gate = TEST_GATE.lock().await;
        let app_handle = test_app_handle();
        reset_connection(&app_handle).await;
        let server = TestServer::start().await;
        let (observed_tx, mut observed_rx) = mpsc::unbounded_channel();
        let listener = app_handle.listen("lt:message", move |event| {
            let payload = serde_json::from_str::<serde_json::Value>(event.payload()).unwrap();
            let _ = observed_tx.send(payload);
        });

        lt_connect_ws(
            server.ws_url.clone(),
            42,
            app_handle.clone(),
            app_handle.state(),
        )
        .await
        .unwrap();
        server.send_text(r#"{"type":"pong","ok":true}"#).await;
        let message_payload = tokio::time::timeout(Duration::from_secs(1), observed_rx.recv())
            .await
            .expect("message event was not emitted")
            .unwrap();

        app_handle.unlisten(listener);
        reset_connection(&app_handle).await;
        server.stop();

        assert_eq!(message_payload["sessionId"], 42);
        assert_eq!(message_payload["envelope"]["type"], "pong");
        assert_eq!(message_payload["envelope"]["ok"], true);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn replaced_connection_cannot_emit_late_events() {
        let _gate = TEST_GATE.lock().await;
        let app_handle = test_app_handle();
        reset_connection(&app_handle).await;
        let old_server = TestServer::start().await;
        let current_server = TestServer::start().await;

        lt_connect_ws(
            old_server.ws_url.clone(),
            1,
            app_handle.clone(),
            app_handle.state(),
        )
        .await
        .unwrap();
        lt_connect_ws(
            current_server.ws_url.clone(),
            2,
            app_handle.clone(),
            app_handle.state(),
        )
        .await
        .unwrap();

        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let message_tx = event_tx.clone();
        let message_listener = app_handle.listen("lt:message", move |_| {
            let _ = message_tx.send("message");
        });
        let disconnected_listener = app_handle.listen("lt:disconnected", move |_| {
            let _ = event_tx.send("disconnected");
        });

        old_server.send_text(r#"{"type":"OLD_CONNECTION"}"#).await;
        let late_event = tokio::time::timeout(Duration::from_millis(250), event_rx.recv()).await;

        app_handle.unlisten(message_listener);
        app_handle.unlisten(disconnected_listener);
        reset_connection(&app_handle).await;
        old_server.stop();
        current_server.stop();

        assert!(
            late_event.is_err(),
            "a replaced connection emitted a late event: {late_event:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stale_session_cannot_send_through_the_current_connection() {
        let _gate = TEST_GATE.lock().await;
        let app_handle = test_app_handle();
        reset_connection(&app_handle).await;
        let server = TestServer::start().await;

        lt_connect_ws(
            server.ws_url.clone(),
            2,
            app_handle.clone(),
            app_handle.state(),
        )
        .await
        .unwrap();

        let stale_event = serde_json::from_value(serde_json::json!({
            "type": "PAUSE",
            "eventId": "stale-event"
        }))
        .unwrap();
        let stale_sent = lt_send_event(stale_event, 1, app_handle.state())
            .await
            .unwrap();
        assert!(!stale_sent);
        assert_eq!(server.receive_text(Duration::from_millis(100)).await, None);

        let current_event = serde_json::from_value(serde_json::json!({
            "type": "PAUSE",
            "eventId": "current-event"
        }))
        .unwrap();
        let current_sent = lt_send_event(current_event, 2, app_handle.state())
            .await
            .unwrap();
        assert!(current_sent);
        assert!(server.receive_text(Duration::from_secs(1)).await.is_some());

        reset_connection(&app_handle).await;
        server.stop();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stale_disconnect_cannot_close_the_current_connection() {
        let _gate = TEST_GATE.lock().await;
        let app_handle = test_app_handle();
        reset_connection(&app_handle).await;
        let old_server = TestServer::start().await;
        let current_server = TestServer::start().await;

        lt_connect_ws(
            old_server.ws_url.clone(),
            1,
            app_handle.clone(),
            app_handle.state(),
        )
        .await
        .unwrap();
        lt_connect_ws(
            current_server.ws_url.clone(),
            2,
            app_handle.clone(),
            app_handle.state(),
        )
        .await
        .unwrap();

        lt_disconnect_ws(1, app_handle.state()).await.unwrap();

        let (ws_client, session_id) = {
            let session = app_handle.state::<AppState>().lt_session.lock();
            (session.ws_client.clone(), session.ws_session_id.clone())
        };
        assert_eq!(session_id.load(Ordering::Acquire), 2);
        assert_eq!(
            ws_client.lock().await.as_ref().map(LtWsClient::session_id),
            Some(2)
        );

        reset_connection(&app_handle).await;
        old_server.stop();
        current_server.stop();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn newer_disconnect_closes_an_older_active_connection() {
        let _gate = TEST_GATE.lock().await;
        let app_handle = test_app_handle();
        reset_connection(&app_handle).await;
        let server = TestServer::start().await;

        lt_connect_ws(
            server.ws_url.clone(),
            1,
            app_handle.clone(),
            app_handle.state(),
        )
        .await
        .unwrap();
        lt_disconnect_ws(2, app_handle.state()).await.unwrap();

        let (ws_client, session_id) = {
            let session = app_handle.state::<AppState>().lt_session.lock();
            (session.ws_client.clone(), session.ws_session_id.clone())
        };
        assert_eq!(session_id.load(Ordering::Acquire), 0);
        assert!(ws_client.lock().await.is_none());

        reset_connection(&app_handle).await;
        server.stop();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stale_connect_cannot_replace_the_current_connection() {
        let _gate = TEST_GATE.lock().await;
        let app_handle = test_app_handle();
        reset_connection(&app_handle).await;
        let stale_server = TestServer::start().await;
        let current_server = TestServer::start().await;

        lt_connect_ws(
            current_server.ws_url.clone(),
            2,
            app_handle.clone(),
            app_handle.state(),
        )
        .await
        .unwrap();
        lt_connect_ws(
            stale_server.ws_url.clone(),
            1,
            app_handle.clone(),
            app_handle.state(),
        )
        .await
        .unwrap();

        let (ws_client, session_id) = {
            let session = app_handle.state::<AppState>().lt_session.lock();
            (session.ws_client.clone(), session.ws_session_id.clone())
        };
        assert_eq!(session_id.load(Ordering::Acquire), 2);
        assert_eq!(
            ws_client.lock().await.as_ref().map(LtWsClient::session_id),
            Some(2)
        );

        reset_connection(&app_handle).await;
        stale_server.stop();
        current_server.stop();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn older_handshake_completing_last_cannot_replace_the_current_connection() {
        let _gate = TEST_GATE.lock().await;
        let app_handle = test_app_handle();
        reset_connection(&app_handle).await;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (accepted_tx, accepted_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let mut old_server_task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let _ = accepted_tx.send(());
            release_rx.await.unwrap();
            let _socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            std::future::pending::<()>().await;
        });

        let old_connect_app = app_handle.clone();
        let mut old_connect_task = tokio::spawn(async move {
            let command_app = old_connect_app.clone();
            lt_connect_ws(
                format!("ws://{address}"),
                1,
                command_app,
                old_connect_app.state(),
            )
            .await
        });
        match tokio::time::timeout(Duration::from_secs(5), accepted_rx).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                old_connect_task.abort();
                old_server_task.abort();
                let _ = (&mut old_connect_task).await;
                let _ = (&mut old_server_task).await;
                reset_connection(&app_handle).await;
                panic!("older WebSocket server stopped before TCP accept: {error}");
            }
            Err(_) => {
                old_connect_task.abort();
                old_server_task.abort();
                let _ = (&mut old_connect_task).await;
                let _ = (&mut old_server_task).await;
                reset_connection(&app_handle).await;
                panic!("older WebSocket server did not accept TCP within five seconds");
            }
        }

        let current_server = TestServer::start().await;
        let current_connect_result = lt_connect_ws(
            current_server.ws_url.clone(),
            2,
            app_handle.clone(),
            app_handle.state(),
        )
        .await;
        if let Err(error) = current_connect_result {
            old_connect_task.abort();
            old_server_task.abort();
            let _ = (&mut old_connect_task).await;
            let _ = (&mut old_server_task).await;
            reset_connection(&app_handle).await;
            current_server.stop();
            panic!("newer WebSocket connect returned an error: {error}");
        }
        let (ws_client, session_id) = {
            let session = app_handle.state::<AppState>().lt_session.lock();
            (session.ws_client.clone(), session.ws_session_id.clone())
        };
        let active_session_id = session_id.load(Ordering::Acquire);
        let installed_session_id = ws_client.lock().await.as_ref().map(LtWsClient::session_id);
        if active_session_id != 2 || installed_session_id != Some(2) {
            old_connect_task.abort();
            old_server_task.abort();
            let _ = (&mut old_connect_task).await;
            let _ = (&mut old_server_task).await;
            reset_connection(&app_handle).await;
            current_server.stop();
            panic!(
                "newer WebSocket was not installed before releasing the older handshake: active={active_session_id}, installed={installed_session_id:?}"
            );
        }

        if release_tx.send(()).is_err() {
            old_connect_task.abort();
            old_server_task.abort();
            let _ = (&mut old_connect_task).await;
            let _ = (&mut old_server_task).await;
            reset_connection(&app_handle).await;
            current_server.stop();
            panic!("older WebSocket server stopped before its handshake was released");
        }
        let old_connect_result =
            tokio::time::timeout(Duration::from_secs(5), &mut old_connect_task).await;
        if old_connect_result.is_err() {
            old_connect_task.abort();
            old_server_task.abort();
            let _ = (&mut old_connect_task).await;
            let _ = (&mut old_server_task).await;
            reset_connection(&app_handle).await;
            current_server.stop();
            panic!("older WebSocket connect did not finish after its handshake was released");
        }
        old_server_task.abort();
        let _ = (&mut old_server_task).await;
        match old_connect_result.unwrap() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                reset_connection(&app_handle).await;
                current_server.stop();
                panic!("older WebSocket connect returned an error: {error}");
            }
            Err(error) => {
                reset_connection(&app_handle).await;
                current_server.stop();
                panic!("older WebSocket connect task panicked: {error}");
            }
        }

        let (ws_client, session_id) = {
            let session = app_handle.state::<AppState>().lt_session.lock();
            (session.ws_client.clone(), session.ws_session_id.clone())
        };
        let active_session_id = session_id.load(Ordering::Acquire);
        let installed_session_id = ws_client.lock().await.as_ref().map(LtWsClient::session_id);

        reset_connection(&app_handle).await;
        current_server.stop();

        assert_eq!(active_session_id, 2);
        assert_eq!(installed_session_id, Some(2));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn replacing_handshake_immediately_invalidates_the_old_session() {
        let _gate = TEST_GATE.lock().await;
        let app_handle = test_app_handle();
        reset_connection(&app_handle).await;
        let old_server = TestServer::start().await;

        lt_connect_ws(
            old_server.ws_url.clone(),
            10,
            app_handle.clone(),
            app_handle.state(),
        )
        .await
        .unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (accepted_tx, accepted_rx) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            let _ = accepted_tx.send(());
            std::future::pending::<()>().await;
        });
        let connect_app = app_handle.clone();
        let connect_task = tokio::spawn(async move {
            let command_app = connect_app.clone();
            lt_connect_ws(
                format!("ws://{address}"),
                11,
                command_app,
                connect_app.state(),
            )
            .await
        });
        accepted_rx.await.unwrap();

        let old_event = serde_json::from_value(serde_json::json!({
            "type": "PAUSE",
            "eventId": "old-event-during-replacement"
        }))
        .unwrap();
        let old_sent = lt_send_event(old_event, 10, app_handle.state())
            .await
            .unwrap();

        connect_task.abort();
        let _ = connect_task.await;
        server_task.abort();
        let _ = server_task.await;
        reset_connection(&app_handle).await;
        old_server.stop();

        assert!(!old_sent);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn disconnect_is_not_blocked_by_a_stalled_handshake() {
        let _gate = TEST_GATE.lock().await;
        let app_handle = test_app_handle();
        reset_connection(&app_handle).await;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (accepted_tx, accepted_rx) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            let _ = accepted_tx.send(());
            std::future::pending::<()>().await;
        });

        let connect_app = app_handle.clone();
        let connect_task = tokio::spawn(async move {
            let command_app = connect_app.clone();
            lt_connect_ws(
                format!("ws://{address}"),
                3,
                command_app,
                connect_app.state(),
            )
            .await
        });
        accepted_rx.await.unwrap();

        let disconnect_result = tokio::time::timeout(
            Duration::from_millis(250),
            lt_disconnect_ws(3, app_handle.state()),
        )
        .await;

        connect_task.abort();
        let _ = connect_task.await;
        server_task.abort();
        let _ = server_task.await;
        reset_connection(&app_handle).await;

        assert!(
            disconnect_result.is_ok(),
            "disconnect waited for the stalled WebSocket handshake"
        );
    }
}
