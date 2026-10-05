// 平台请求的传输层兜底
//
// 单一的「绕过代理」开关覆盖不了现实：系统代理配错时只有直连能通，
// 处在需要代理才能出网的网络里又只有走代理能通，而用户不一定知道该拨哪边。
// 这里统一提供「主客户端失败后用相反代理设置重试一次」的能力，
// 各平台客户端共用，避免每个平台各写一份。

use reqwest::{Client, RequestBuilder, Response};

/// 是否属于「换个代理设置可能就通」的失败
///
/// 只认连接建立阶段的错误。HTTP 状态码错误在 reqwest 里是 `Ok(response)`，
/// 本来就到不了这里；解析错误重试也没有意义，只会把一次业务失败放大成两次请求。
pub fn is_transport_failure(error: &reqwest::Error) -> bool {
    error.is_connect() || error.is_timeout() || (error.is_request() && !error.is_decode())
}

/// 带代理兜底的 HTTP 通道
#[derive(Clone)]
pub struct FallbackHttp {
    primary: Client,
    /// 代理设置与 primary 相反；None 表示不做兜底
    fallback: Option<Client>,
    /// 日志标签，用于区分是哪个平台在兜底
    target: &'static str,
}

impl FallbackHttp {
    pub fn new(primary: &Client, target: &'static str) -> Self {
        Self {
            primary: primary.clone(),
            fallback: None,
            target,
        }
    }

    pub fn with_fallback(primary: &Client, fallback: &Client, target: &'static str) -> Self {
        Self {
            primary: primary.clone(),
            fallback: Some(fallback.clone()),
            target,
        }
    }

    /// 主客户端，供不需要兜底或需要自行构造请求的调用方使用
    pub fn primary(&self) -> &Client {
        &self.primary
    }

    /// 发送幂等请求；传输层失败时用相反代理设置重试一次
    ///
    /// `build` 会被调用最多两次，因此必须是可重复执行的纯构造，
    /// 不要在里面做带副作用的操作。
    pub async fn send(
        &self,
        build: impl Fn(&Client) -> RequestBuilder,
    ) -> Result<Response, reqwest::Error> {
        self.send_with_predicate(build, is_transport_failure).await
    }

    /// 发送非幂等写请求；只在连接建立阶段失败时换代理重试一次
    ///
    /// 读超时发生在请求体已送达服务端之后，此时重发会造成重复提交
    /// （建歌单、加歌、点赞等），因此写请求不做超时/解码重试，只认
    /// `is_connect()`——此时 TCP/TLS/代理握手尚未完成，服务端不可能已处理。
    pub async fn send_once(
        &self,
        build: impl Fn(&Client) -> RequestBuilder,
    ) -> Result<Response, reqwest::Error> {
        self.send_with_predicate(build, |error| error.is_connect())
            .await
    }

    async fn send_with_predicate(
        &self,
        build: impl Fn(&Client) -> RequestBuilder,
        retryable: impl Fn(&reqwest::Error) -> bool,
    ) -> Result<Response, reqwest::Error> {
        let error = match build(&self.primary).send().await {
            Ok(response) => return Ok(response),
            Err(error) => error.without_url(),
        };
        let Some(fallback) = self.fallback.as_ref().filter(|_| retryable(&error)) else {
            return Err(error);
        };
        log::warn!(
            target: "transport",
            "{} primary transport failed, retrying with the opposite proxy setting: {}",
            self.target,
            error,
        );
        build(fallback)
            .send()
            .await
            .map_err(reqwest::Error::without_url)
    }
}

/// 解析 JSON 响应，先判状态码再解析
///
/// 直接 `resp.json()` 会把错误响应当成正常内容：
/// - 4xx/5xx 的 JSON 错误体解析成功后，业务层看到的是"合法但没有内容"，
///   界面只会渲染成空列表，整条链路不报错（YouTube 云端歌单就栽在这里）
/// - 返回 HTML 错误页时，用户看到的是「Parse error: expected value」，
///   完全指不到真正的问题
pub async fn parse_json_response<T: serde::de::DeserializeOwned>(
    response: Response,
    context: &str,
) -> Result<T, crate::error::AppError> {
    use crate::error::AppError;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| {
            AppError::Api(format!("{context}: failed to read body: {}", error.without_url()))
        })?;

    if !status.is_success() {
        return Err(AppError::Api(format!(
            "{context}: HTTP {status}{}",
            summarize_body(&body)
        )));
    }

    serde_json::from_str(&body).map_err(|error| {
        AppError::Api(format!(
            "{context}: invalid JSON ({error}){}",
            summarize_body(&body)
        ))
    })
}

/// 把响应体压成一行短摘要，避免整页 HTML 灌进错误信息与日志
fn summarize_body(body: &str) -> String {
    const MAX: usize = 160;
    let normalized = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return String::new();
    }
    if normalized.chars().count() <= MAX {
        return format!(" - {normalized}");
    }
    let truncated: String = normalized.chars().take(MAX).collect();
    format!(" - {truncated}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpSocket};

    /// 测试必须绕开系统代理：mock server 监听回环地址
    fn client() -> Client {
        Client::builder()
            .no_proxy()
            .build()
            .expect("failed to build test client")
    }

    /// 起一个只回固定状态码的服务，并统计收到的请求数
    async fn mock_server(status: &'static str) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { break };
                counter.fetch_add(1, Ordering::SeqCst);
                let mut buffer = [0_u8; 1024];
                let _ = socket.read(&mut buffer).await;
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        (format!("http://{address}"), hits)
    }

    // 保留未监听的端口，避免并行测试的服务复用它并把失败请求接走
    fn dead_address() -> (String, TcpSocket) {
        let socket = TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let address = socket.local_addr().unwrap();
        (format!("http://{address}"), socket)
    }

    #[tokio::test]
    async fn primary_success_never_touches_the_fallback() {
        let (primary_url, primary_hits) = mock_server("200 OK").await;
        let (_fallback_url, fallback_hits) = mock_server("200 OK").await;
        let transport = FallbackHttp::with_fallback(&client(), &client(), "test");

        let response = transport
            .send(|client| client.get(&primary_url))
            .await
            .unwrap();

        assert!(response.status().is_success());
        assert_eq!(primary_hits.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn transport_failure_retries_once_on_the_fallback() {
        let (dead, _reservation) = dead_address();
        let (live_url, live_hits) = mock_server("200 OK").await;
        // 保留未监听端口，避免并行测试复用端口后误判主路成功
        let transport = FallbackHttp::with_fallback(&client(), &client(), "test");
        let attempt = AtomicUsize::new(0);

        let response = transport
            .send(|client| {
                let url = if attempt.fetch_add(1, Ordering::SeqCst) == 0 {
                    dead.clone()
                } else {
                    live_url.clone()
                };
                client.get(url)
            })
            .await
            .unwrap();

        assert!(response.status().is_success());
        assert_eq!(attempt.load(Ordering::SeqCst), 2, "should have retried once");
        assert_eq!(live_hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn http_error_status_is_not_retried() {
        // 5xx 在 reqwest 里是 Ok(response)，重试只会把一次业务失败放大成两次请求
        let (url, hits) = mock_server("500 Internal Server Error").await;
        let transport = FallbackHttp::with_fallback(&client(), &client(), "test");

        let response = transport.send(|client| client.get(&url)).await.unwrap();

        assert_eq!(response.status().as_u16(), 500);
        assert_eq!(hits.load(Ordering::SeqCst), 1, "must not double the request");
    }

    #[tokio::test]
    async fn json_parse_rejects_error_statuses_instead_of_returning_empty_content() {
        // 这正是 YouTube 云端歌单静默变空的形态：错误体也是合法 JSON
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 1024];
            let _ = socket.read(&mut buffer).await;
            let payload = r#"{"error":{"message":"session expired"}}"#;
            let response = format!(
                "HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        });

        let response = client()
            .get(format!("http://{address}"))
            .send()
            .await
            .unwrap();
        let error = parse_json_response::<serde_json::Value>(response, "test")
            .await
            .unwrap_err();
        let message = error.to_string();

        assert!(message.contains("401"), "{message}");
        assert!(message.contains("session expired"), "{message}");
    }

    #[tokio::test]
    async fn json_parse_reports_non_json_bodies_readably() {
        let (url, _) = mock_server("200 OK").await;
        let response = client().get(&url).send().await.unwrap();

        let error = parse_json_response::<serde_json::Value>(response, "test")
            .await
            .unwrap_err();

        assert!(error.to_string().contains("invalid JSON"), "{error}");
    }

    #[tokio::test]
    async fn transport_errors_preserve_failure_kind_without_signed_urls() {
        let (address, _reservation) = dead_address();
        let dead = format!("{address}?pot=private-fixture-token");
        for transport in [
            FallbackHttp::new(&client(), "test"),
            FallbackHttp::with_fallback(&client(), &client(), "test"),
        ] {
            let error = transport.send(|client| client.get(&dead)).await.unwrap_err();
            assert!(is_transport_failure(&error));
            assert!(error.url().is_none());
            assert!(!error.to_string().contains("private-fixture-token"));
        }
    }

    #[tokio::test]
    async fn without_a_fallback_the_original_error_is_returned() {
        let (dead, reservation) = dead_address();
        assert!(TcpListener::bind(reservation.local_addr().unwrap()).await.is_err());
        let transport = FallbackHttp::new(&client(), "test");

        let error = transport.send(|client| client.get(&dead)).await.unwrap_err();

        assert!(is_transport_failure(&error));
    }
}
