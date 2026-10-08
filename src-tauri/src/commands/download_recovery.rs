use std::future::Future;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use crate::api::youtube::client::{YtAudioStream, YtStreamType};
use crate::error::{AppError, AppResult};

#[derive(Clone, Debug)]
pub(super) struct Source {
    pub url: String,
    pub stream_type: YtStreamType,
    pub content_length: Option<u64>,
    pub content_md5: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Retry {
    Stop,
    Refresh,
    Playable,
}

pub(super) fn retry(error: &AppError) -> Retry {
    let status = match error {
        AppError::Api(message) => message.strip_prefix("HTTP ").and_then(http_status),
        AppError::Audio(message) => {
            if message.starts_with("HLS: request failed:")
                || message.starts_with("HLS: response interrupted:")
            {
                return Retry::Refresh;
            }
            message
                .strip_prefix("HLS: request returned HTTP ")
                .and_then(http_status)
        }
        AppError::Network(error)
            if error.is_connect()
                || error.is_timeout()
                || error.is_body()
                || error.is_request() =>
        {
            return Retry::Refresh
        }
        AppError::Other(message) if message.starts_with("Download stalled:") => {
            return Retry::Refresh
        }
        _ => None,
    };
    match status {
        Some(403) => Retry::Playable,
        Some(401 | 408 | 409 | 410 | 416 | 425 | 429 | 500..=599) => Retry::Refresh,
        _ => Retry::Stop,
    }
}

fn http_status(message: &str) -> Option<u16> {
    let code = message.split_ascii_whitespace().next()?;
    if code.len() != 3 {
        return None;
    }
    code.parse().ok()
}

pub(super) async fn request(
    client: &reqwest::Client,
    url: &str,
    referer: &str,
    user_agent: &str,
) -> AppResult<reqwest::Response> {
    let response = client
        .get(url)
        .header("Referer", referer)
        .header("User-Agent", user_agent)
        .send()
        .await
        .map_err(|error| AppError::Network(error.without_url()))?;
    if !response.status().is_success() {
        return Err(AppError::Api(format!("HTTP {}", response.status())));
    }
    Ok(response)
}

pub(super) async fn cancellable<T>(
    cancel: &Arc<AtomicBool>,
    future: impl Future<Output = AppResult<T>>,
) -> AppResult<T> {
    if cancel.load(Ordering::Acquire) {
        return Err(AppError::Other("Download cancelled".into()));
    }
    tokio::pin!(future);
    loop {
        tokio::select! {
            result = &mut future => return result,
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                if cancel.load(Ordering::Acquire) {
                    return Err(AppError::Other("Download cancelled".into()));
                }
            }
        }
    }
}

pub(super) async fn run<T, R, RF, D, DF>(
    initial: Source,
    cancel: Arc<AtomicBool>,
    delay_unit: Duration,
    mut resolve: R,
    mut download: D,
) -> AppResult<T>
where
    R: FnMut(bool) -> RF,
    RF: Future<Output = AppResult<Source>>,
    D: FnMut(Source) -> DF,
    DF: Future<Output = AppResult<T>>,
{
    let mut source = initial;
    let mut avoid_direct = false;
    for attempt in 0..4 {
        let error = match cancellable(&cancel, download(source)).await {
            Ok(result) => return Ok(result),
            Err(error) => error,
        };
        let policy = retry(&error);
        if policy == Retry::Stop || attempt == 3 {
            return Err(error);
        }
        avoid_direct |= policy == Retry::Playable;
        // 同一后台任务保留取消标志，刷新前释放旧 transfer 的临时文件
        cancellable(&cancel, async {
            tokio::time::sleep(delay_unit.saturating_mul(1 << attempt)).await;
            Ok(())
        })
        .await?;
        source = cancellable(&cancel, resolve(avoid_direct)).await?;
    }
    unreachable!("the fourth attempt returns its result")
}

pub(super) fn resolve_plan(force_refresh: bool, avoid_direct: bool) -> Vec<(bool, bool)> {
    let mut plan = Vec::with_capacity(4);
    if !avoid_direct {
        if !force_refresh {
            plan.push((false, true));
        }
        plan.push((true, true));
    }
    if !force_refresh {
        plan.push((false, false));
    }
    plan.push((true, false));
    plan
}

pub(super) fn select_stream(
    streams: Vec<YtAudioStream>,
    quality: &str,
    require_direct: bool,
) -> Option<Source> {
    let minimum = match quality.trim().to_ascii_lowercase().as_str() {
        "medium" => 96_000,
        "high" | "higher" => 128_000,
        "low" | "standard" => 0,
        _ => 160_000,
    };
    streams
        .into_iter()
        .filter(|stream| !require_direct || stream.stream_type == YtStreamType::Direct)
        .filter(|stream| crate::api::youtube::hls::is_trusted_hls_url(&stream.url))
        .filter(|stream| {
            let mime = stream.mime_type.to_ascii_lowercase();
            !["opus", "webm", "ec-3", "eac3", "ac-3", "ac3"]
                .iter()
                .any(|codec| mime.contains(codec))
                && (stream.stream_type == YtStreamType::Hls || m4a(&mime))
        })
        .max_by_key(|stream| {
            (
                m4a(&stream.mime_type.to_ascii_lowercase()),
                stream.bitrate >= minimum,
                stream.stream_type == YtStreamType::Direct,
                stream.bitrate,
                stream.content_length,
            )
        })
        .map(|stream| Source {
            url: stream.url,
            stream_type: stream.stream_type,
            content_length: (stream.stream_type == YtStreamType::Direct
                && stream.content_length > 0)
                .then_some(stream.content_length),
            content_md5: None,
        })
}

fn m4a(mime: &str) -> bool {
    matches!(
        mime.split(';').next().unwrap_or("").trim(),
        "audio/mp4" | "audio/m4a" | "audio/aac"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn initial() -> Source {
        Source {
            url: "fixture:stale".into(),
            stream_type: YtStreamType::Direct,
            content_length: Some(100),
            content_md5: None,
        }
    }

    #[test]
    fn youtube_download_signed_and_transient_http_failures_refresh_but_forbidden_uses_playable() {
        for code in [401, 408, 409, 410, 416, 425, 429, 500, 502, 503, 504] {
            assert_eq!(
                retry(&AppError::Api(format!("HTTP {code}"))),
                Retry::Refresh
            );
        }
        assert_eq!(
            retry(&AppError::Api("HTTP 403 Forbidden".into())),
            Retry::Playable
        );
        assert_eq!(
            retry(&AppError::Audio(
                "HLS: request returned HTTP 403 Forbidden".into()
            )),
            Retry::Playable
        );
        assert_eq!(
            retry(&AppError::Audio(
                "HLS: response interrupted: connection reset".into()
            )),
            Retry::Refresh
        );
        for error in [
            AppError::Api("HTTP 404".into()),
            AppError::Audio("Download MD5 mismatch".into()),
            AppError::Audio("Downloaded audio is unreadable: HTTP 403".into()),
            AppError::Other("Download cancelled".into()),
        ] {
            assert_eq!(retry(&error), Retry::Stop);
        }
    }

    #[test]
    fn youtube_download_resolve_order_matches_android_and_forbidden_never_revisits_direct() {
        assert_eq!(
            resolve_plan(false, false),
            [(false, true), (true, true), (false, false), (true, false)]
        );
        assert_eq!(resolve_plan(true, false), [(true, true), (true, false)]);
        assert_eq!(resolve_plan(true, true), [(true, false)]);
    }

    #[test]
    fn youtube_download_prefers_aac_and_actual_quality_instead_of_second_bitrate() {
        let stream = |bitrate, stream_type, mime: &str| YtAudioStream {
            url: format!("https://rr1.googlevideo.com/videoplayback?rate={bitrate}"),
            bitrate,
            stream_type,
            mime_type: mime.into(),
            content_length: bitrate,
        };
        let selected = select_stream(
            vec![
                stream(
                    128_000,
                    YtStreamType::Direct,
                    "audio/mp4; codecs=\"mp4a.40.2\"",
                ),
                stream(48_000, YtStreamType::Direct, "audio/mp4"),
            ],
            "high",
            true,
        )
        .unwrap();
        assert_eq!(selected.content_length, Some(128_000));
        let selected = select_stream(
            vec![
                stream(128_000, YtStreamType::Direct, "audio/mp4"),
                stream(160_000, YtStreamType::Hls, "audio/mp4"),
                stream(256_000, YtStreamType::Direct, "audio/webm; codecs=\"opus\""),
            ],
            "very_high",
            false,
        )
        .unwrap();
        assert_eq!(selected.stream_type, YtStreamType::Hls);
        assert_eq!(selected.content_length, None);
        assert!(select_stream(
            vec![stream(
                256_000,
                YtStreamType::Direct,
                "audio/webm; codecs=\"opus\""
            )],
            "high",
            false
        )
        .is_none());
    }

    #[tokio::test]
    async fn youtube_download_real_http_expired_link_refreshes_once_in_same_runner() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = server.local_addr().unwrap();
        let task = tokio::spawn(async move {
            for status in ["410 Gone", "200 OK"] {
                let (mut socket, _) = server.accept().await.unwrap();
                let mut request = [0u8; 1024];
                assert!(socket.read(&mut request).await.unwrap() > 0);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: 3\r\nConnection: close\r\n\r\naac"
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut source = initial();
        source.url = format!("http://{address}/expired");
        let result = run(
            source,
            Arc::new(AtomicBool::new(false)),
            Duration::ZERO,
            move |_| async move {
                Ok(Source {
                    url: format!("http://{address}/fresh"),
                    content_length: Some(3),
                    ..initial()
                })
            },
            move |source| {
                let client = client.clone();
                async move {
                    let response = request(&client, &source.url, "fixture", "fixture").await?;
                    let body = response
                        .bytes()
                        .await
                        .map_err(|error| AppError::Network(error.without_url()))?;
                    assert_eq!(source.content_length, Some(3));
                    Ok(body)
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(result.as_ref(), b"aac");
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn youtube_download_same_operation_refreshes_source_metadata_before_retry() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let resolve_events = events.clone();
        let download_events = events.clone();
        let result = run(
            initial(),
            Arc::new(AtomicBool::new(false)),
            Duration::ZERO,
            move |avoid_direct| {
                let events = resolve_events.clone();
                async move {
                    assert!(!avoid_direct);
                    events.lock().unwrap().push("fresh".to_string());
                    Ok(Source {
                        url: "fixture:fresh".into(),
                        stream_type: YtStreamType::Direct,
                        content_length: Some(200),
                        content_md5: None,
                    })
                }
            },
            move |source| {
                let events = download_events.clone();
                async move {
                    events.lock().unwrap().push(source.url.clone());
                    if source.url == "fixture:stale" {
                        Err(AppError::Api("HTTP 410".into()))
                    } else {
                        assert_eq!(source.content_length, Some(200));
                        Ok(7)
                    }
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(result, 7);
        assert_eq!(
            *events.lock().unwrap(),
            ["fixture:stale", "fresh", "fixture:fresh"]
        );
    }

    #[tokio::test]
    async fn youtube_download_forbidden_switches_to_hls_and_keeps_retry_budget_bounded() {
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let downloads = count.clone();
        let result: AppResult<()> = run(
            initial(),
            Arc::new(AtomicBool::new(false)),
            Duration::ZERO,
            |avoid_direct| async move {
                assert!(avoid_direct);
                Ok(Source {
                    url: "fixture:hls".into(),
                    stream_type: YtStreamType::Hls,
                    content_length: None,
                    content_md5: None,
                })
            },
            move |source| {
                let count = downloads.clone();
                async move {
                    let current = count.fetch_add(1, Ordering::SeqCst);
                    if current == 0 {
                        Err(AppError::Api("HTTP 403".into()))
                    } else {
                        assert_eq!(source.stream_type, YtStreamType::Hls);
                        Err(AppError::Api("HTTP 503".into()))
                    }
                }
            },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(count.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn youtube_download_integrity_and_cancellation_do_not_resolve_again() {
        let result: AppResult<()> = run(
            initial(),
            Arc::new(AtomicBool::new(false)),
            Duration::ZERO,
            |_| async { panic!("integrity failure must not resolve a new stream") },
            |_| async { Err(AppError::Audio("Download duration is too short".into())) },
        )
        .await;
        assert!(result.is_err());
        let cancel = Arc::new(AtomicBool::new(true));
        let result: AppResult<()> = run(
            initial(),
            cancel,
            Duration::ZERO,
            |_| async { panic!("cancelled operation must not resolve") },
            |_| async { panic!("cancelled operation must not start transfer") },
        )
        .await;
        assert!(result.is_err());
    }
}
