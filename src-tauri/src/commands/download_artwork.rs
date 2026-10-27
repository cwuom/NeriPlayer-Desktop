use crate::error::{AppError, AppResult};
use base64::{engine::general_purpose::STANDARD, Engine};
use image::{ImageEncoder, ImageFormat};
use reqwest::header::{ACCEPT, REFERER, USER_AGENT};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use url::Url;

const MAX_COVER_BYTES: usize = 8 * 1024 * 1024;
const COVER_TIMEOUT: Duration = Duration::from_secs(20);
const COVER_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

pub(super) async fn fetch_cover(
    client: &reqwest::Client,
    raw_url: &str,
) -> AppResult<(Vec<u8>, &'static str)> {
    let source = raw_url.trim();
    let bytes = if source.starts_with("data:") {
        read_data_image(source)?
    } else if source.starts_with("file:") {
        let path = Url::parse(source)
            .ok()
            .and_then(|url| url.to_file_path().ok())
            .ok_or_else(invalid_address)?;
        read_local_image(&path).await?
    } else if !source.starts_with("//") && Path::new(source).is_absolute() {
        read_local_image(Path::new(source)).await?
    } else if let Some(path) = asset_image_path(source)? {
        read_local_image(&path).await?
    } else {
        let url = normalize_http_url(source)?;
        let mut retries = 0;
        loop {
            match fetch_http_image(client, &url).await {
                Ok(bytes) => break bytes,
                Err((_, true)) if retries < 2 => {
                    tokio::time::sleep(Duration::from_millis(200 << retries)).await;
                    retries += 1;
                }
                Err((error, _)) => return Err(error),
            }
        }
    };
    tokio::task::spawn_blocking(move || validate_image(bytes))
        .await
        .map_err(|_| AppError::Metadata("封面图片处理失败，请重试或更换封面".into()))?
}

fn invalid_address() -> AppError {
    AppError::Metadata(
        "封面地址无效或暂不支持，请使用 HTTP、HTTPS、绝对本地图片路径、Tauri asset 或 base64 图片地址".into(),
    )
}

fn too_large() -> AppError {
    AppError::Metadata("封面图片超过 8 MiB，请更换较小的图片".into())
}

fn asset_image_path(source: &str) -> AppResult<Option<PathBuf>> {
    let Ok(url) = Url::parse(source) else {
        return Ok(None);
    };
    let asset_scheme = url.scheme() == "asset";
    let asset_host =
        matches!(url.scheme(), "http" | "https") && url.host_str() == Some("asset.localhost");
    if !asset_scheme && !asset_host {
        return Ok(None);
    }
    let expected_host = if asset_scheme {
        "localhost"
    } else {
        "asset.localhost"
    };
    if url.host_str() != Some(expected_host)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err(invalid_address());
    }
    // Tauri 对整个文件路径做 encodeURIComponent，读取时只解码一次
    let encoded = url.path().strip_prefix('/').ok_or_else(invalid_address)?;
    let decoded = urlencoding::decode(encoded).map_err(|_| invalid_address())?;
    let path = PathBuf::from(decoded.as_ref());
    if !path.is_absolute() {
        return Err(invalid_address());
    }
    tauri::path::SafePathBuf::new(path)
        .map(|path| Some(path.into()))
        .map_err(|_| invalid_address())
}

fn normalize_http_url(source: &str) -> AppResult<Url> {
    let normalized = if source.starts_with("//") {
        format!("https:{source}")
    } else {
        source.to_string()
    };
    let url = Url::parse(&normalized).map_err(|_| invalid_address())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.host_str() == Some("asset.localhost")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(invalid_address());
    }
    Ok(url)
}

fn read_data_image(source: &str) -> AppResult<Vec<u8>> {
    let (metadata, encoded) = source.split_once(',').ok_or_else(invalid_address)?;
    if !metadata.starts_with("data:image/") || !metadata.ends_with(";base64") {
        return Err(invalid_address());
    }
    if encoded.len() > MAX_COVER_BYTES.div_ceil(3) * 4 {
        return Err(too_large());
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| AppError::Metadata("封面 base64 图片内容无效，请更换封面".into()))?;
    if bytes.len() > MAX_COVER_BYTES {
        return Err(too_large());
    }
    Ok(bytes)
}

async fn read_local_image(path: &Path) -> AppResult<Vec<u8>> {
    let read_error =
        |_| AppError::Metadata("读取本地封面失败，请检查图片文件是否存在及读取权限".into());
    let file = tokio::fs::File::open(path).await.map_err(read_error)?;
    if file.metadata().await.map_err(read_error)?.len() > MAX_COVER_BYTES as u64 {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    file.take(MAX_COVER_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(read_error)?;
    if bytes.len() > MAX_COVER_BYTES {
        return Err(too_large());
    }
    Ok(bytes)
}

async fn fetch_http_image(
    client: &reqwest::Client,
    url: &Url,
) -> Result<Vec<u8>, (AppError, bool)> {
    let mut response = client
        .get(url.clone())
        .header(REFERER, crate::commands::image_cmd::cover_referer(url))
        .header(USER_AGENT, COVER_USER_AGENT)
        .header(
            ACCEPT,
            "image/webp,image/png,image/jpeg,image/*;q=0.8,*/*;q=0.1",
        )
        .timeout(COVER_TIMEOUT)
        .send()
        .await
        .map_err(network_error)?;
    let status = response.status();
    if !status.is_success() {
        return Err((
            AppError::Metadata(format!(
                "封面请求失败 (HTTP {})，请稍后重试或更换封面地址",
                status.as_u16()
            )),
            status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS,
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_COVER_BYTES as u64)
    {
        return Err((too_large(), false));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if chunk.len() > MAX_COVER_BYTES - bytes.len() {
            return Err((too_large(), false));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn network_error(error: reqwest::Error) -> (AppError, bool) {
    let retryable =
        error.is_timeout() || error.is_connect() || error.is_body() || error.is_decode();
    let message = if error.is_timeout() {
        "封面请求超时，请检查网络后重试"
    } else {
        "封面网络请求失败，请检查网络、代理或封面地址后重试"
    };
    (AppError::Metadata(message.into()), retryable)
}

struct CoverImageBuffer(Vec<u8>);

impl Write for CoverImageBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_COVER_BYTES - self.0.len() {
            return Err(std::io::Error::other("封面图片超过 8 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn validate_image(bytes: Vec<u8>) -> AppResult<(Vec<u8>, &'static str)> {
    if bytes.is_empty() {
        return Err(AppError::Metadata("封面响应为空，请重试或更换封面".into()));
    }
    let image_error = |error| match error {
        image::ImageError::Limits(_) => AppError::Metadata(
            "封面图片超过解码限制，请使用不超过 8192x8192、解码内存不超过 128 MiB 的图片".into(),
        ),
        _ => AppError::Metadata(
            "封面图片无效或格式不受支持，请更换有效的 JPEG、PNG、GIF 或 WebP 图片".into(),
        ),
    };
    let format = image::guess_format(&bytes).map_err(image_error)?;
    let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(image_error)?;
    if decoded.width() == 0 || decoded.height() == 0 {
        return Err(AppError::Metadata(
            "封面图片尺寸为空，请更换有效图片".into(),
        ));
    }
    match format {
        ImageFormat::Jpeg => Ok((bytes, "jpg")),
        ImageFormat::Png => Ok((bytes, "png")),
        _ => {
            let mut encoded = CoverImageBuffer(Vec::new());
            image::codecs::png::PngEncoder::new(&mut encoded)
                .write_image(
                    decoded.as_bytes(),
                    decoded.width(),
                    decoded.height(),
                    decoded.color().into(),
                )
                .map_err(|_| {
                    AppError::Metadata("封面转换 PNG 失败或转换后超过 8 MiB，请更换封面图片".into())
                })?;
            Ok((encoded.0, "png"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{fetch_cover, normalize_http_url};
    use base64::{engine::general_purpose::STANDARD, Engine};
    use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
    use std::io::Cursor;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct CoverServer {
        address: SocketAddr,
        requests: Arc<Mutex<Vec<String>>>,
        count: Arc<AtomicUsize>,
        task: tokio::task::JoinHandle<()>,
    }

    impl CoverServer {
        async fn start(responses: Vec<Vec<u8>>) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let seen = requests.clone();
            let count = Arc::new(AtomicUsize::new(0));
            let received = count.clone();
            let task = tokio::spawn(async move {
                loop {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        let mut chunk = [0; 1024];
                        let length = socket.read(&mut chunk).await.unwrap();
                        assert!(length > 0 && request.len() + length <= 8192);
                        request.extend_from_slice(&chunk[..length]);
                    }
                    seen.lock()
                        .unwrap()
                        .push(String::from_utf8(request).unwrap());
                    let index = received.fetch_add(1, Ordering::Relaxed);
                    let response = &responses[index.min(responses.len() - 1)];
                    let _ = socket.write_all(response).await;
                }
            });
            Self {
                address,
                requests,
                count,
                task,
            }
        }

        fn url(&self) -> String {
            format!("http://{}/cover?signature=private-token", self.address)
        }

        fn attempts(&self) -> usize {
            self.count.load(Ordering::Relaxed)
        }
    }

    impl Drop for CoverServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    fn response(status: &str, content_type: &str, bytes: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len(),
        ).into_bytes();
        response.extend_from_slice(bytes);
        response
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    fn image(format: ImageFormat) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(RgbImage::from_pixel(2, 2, Rgb([40, 80, 120])))
            .write_to(&mut bytes, format)
            .unwrap();
        bytes.into_inner()
    }

    fn assert_private_error(error: &crate::error::AppError) -> String {
        let text = error.to_string();
        assert!(text.contains("封面"), "{text}");
        assert!(!text.contains("private-token"), "{text}");
        assert!(!text.contains("signature="), "{text}");
        text
    }

    #[test]
    fn normalizes_protocol_relative_cover_addresses_to_https() {
        assert_eq!(
            normalize_http_url("//i0.hdslb.com/bfs/cover.jpg")
                .unwrap()
                .as_str(),
            "https://i0.hdslb.com/bfs/cover.jpg",
        );
    }

    #[tokio::test]
    async fn keeps_valid_jpeg_and_png_when_content_type_is_incorrect() {
        for (format, extension) in [(ImageFormat::Jpeg, "jpg"), (ImageFormat::Png, "png")] {
            let original = image(format);
            let server = CoverServer::start(vec![response(
                "200 OK",
                "application/octet-stream",
                &original,
            )])
            .await;
            let (bytes, actual_extension) =
                fetch_cover(&client(), &format!("  {}\n", server.url()))
                    .await
                    .unwrap();
            assert_eq!(bytes, original);
            assert_eq!(actual_extension, extension);
            assert_eq!(server.attempts(), 1);
        }
    }

    #[tokio::test]
    async fn sends_the_same_platform_referer_as_the_cover_proxy() {
        for (host, referer) in [
            ("i0.hdslb.com", "https://www.bilibili.com/"),
            ("y.gtimg.cn", "https://y.qq.com/"),
            ("i.ytimg.com", "https://music.youtube.com/"),
            ("p1.music.126.net", "https://music.163.com/"),
        ] {
            let server = CoverServer::start(vec![response(
                "200 OK",
                "image/png",
                &image(ImageFormat::Png),
            )])
            .await;
            let client = reqwest::Client::builder()
                .no_proxy()
                .resolve(host, server.address)
                .build()
                .unwrap();
            fetch_cover(
                &client,
                &format!("http://{host}:{}/cover", server.address.port()),
            )
            .await
            .unwrap();
            let request = server.requests.lock().unwrap()[0].to_ascii_lowercase();
            assert!(
                request.contains(&format!("referer: {referer}\r\n")),
                "{request}"
            );
            assert!(request.contains("user-agent: mozilla/5.0"), "{request}");
        }
    }

    #[tokio::test]
    async fn rejects_html_error_pages_without_retrying_or_exposing_the_url() {
        let server = CoverServer::start(vec![response(
            "200 OK",
            "image/jpeg",
            b"<html>rate limit</html>",
        )])
        .await;
        let error = fetch_cover(&client(), &server.url()).await.unwrap_err();
        assert!(assert_private_error(&error).contains("图片"));
        assert_eq!(server.attempts(), 1);
    }

    #[tokio::test]
    async fn reports_permanent_http_failures_without_retrying_or_exposing_the_url() {
        let server =
            CoverServer::start(vec![response("404 Not Found", "text/html", b"not found")]).await;
        let error = fetch_cover(&client(), &server.url()).await.unwrap_err();
        assert!(assert_private_error(&error).contains("404"));
        assert_eq!(server.attempts(), 1);
    }

    #[tokio::test]
    async fn retries_temporary_http_errors_and_recovers_within_three_attempts() {
        let expected = image(ImageFormat::Png);
        let server = CoverServer::start(vec![
            response("503 Service Unavailable", "text/plain", b"later"),
            response("429 Too Many Requests", "text/plain", b"later"),
            response("200 OK", "image/png", &expected),
        ])
        .await;
        assert_eq!(
            fetch_cover(&client(), &server.url()).await.unwrap(),
            (expected, "png")
        );
        assert_eq!(server.attempts(), 3);
    }

    #[tokio::test]
    async fn stops_after_three_temporary_http_failures() {
        let server = CoverServer::start(vec![response(
            "503 Service Unavailable",
            "text/plain",
            b"later",
        )])
        .await;
        let error = fetch_cover(&client(), &server.url()).await.unwrap_err();
        assert!(assert_private_error(&error).contains("503"));
        assert_eq!(server.attempts(), 3);
    }

    #[tokio::test]
    async fn retries_an_interrupted_response_body() {
        let expected = image(ImageFormat::Png);
        let server = CoverServer::start(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 500\r\nConnection: close\r\n\r\nshort".to_vec(),
            response("200 OK", "image/png", &expected),
        ])
        .await;
        assert_eq!(
            fetch_cover(&client(), &server.url()).await.unwrap(),
            (expected, "png")
        );
        assert_eq!(server.attempts(), 2);
    }

    #[tokio::test]
    async fn rejects_empty_responses_without_retrying() {
        let server = CoverServer::start(vec![response("200 OK", "image/png", b"")]).await;
        let error = fetch_cover(&client(), &server.url()).await.unwrap_err();
        assert!(assert_private_error(&error).contains("空"));
        assert_eq!(server.attempts(), 1);
    }

    #[tokio::test]
    async fn rejects_announced_oversized_responses_before_reading_the_body() {
        let server = CoverServer::start(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 8388609\r\nConnection: close\r\n\r\n".to_vec(),
        ])
        .await;
        let error = fetch_cover(&client(), &server.url()).await.unwrap_err();
        assert!(assert_private_error(&error).contains("8 MiB"));
        assert_eq!(server.attempts(), 1);
    }

    #[tokio::test]
    async fn bounds_streamed_bodies_without_a_content_length() {
        let mut body = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
        body.resize(body.len() + 8 * 1024 * 1024 + 1, 0);
        let server = CoverServer::start(vec![body]).await;
        let error = fetch_cover(&client(), &server.url()).await.unwrap_err();
        assert!(assert_private_error(&error).contains("8 MiB"));
        assert_eq!(server.attempts(), 1);
    }

    #[tokio::test]
    async fn converts_webp_and_gif_to_png_without_changing_pixels() {
        for format in [ImageFormat::WebP, ImageFormat::Gif] {
            let server = CoverServer::start(vec![response(
                "200 OK",
                "application/octet-stream",
                &image(format),
            )])
            .await;
            let (bytes, extension) = fetch_cover(&client(), &server.url()).await.unwrap();
            assert_eq!(extension, "png");
            assert_eq!(image::guess_format(&bytes).unwrap(), ImageFormat::Png);
            assert_eq!(
                image::load_from_memory(&bytes).unwrap().to_rgb8(),
                RgbImage::from_pixel(2, 2, Rgb([40, 80, 120]))
            );
        }
    }

    #[tokio::test]
    async fn rejects_corrupt_images_even_with_a_recognizable_signature() {
        let server = CoverServer::start(vec![response(
            "200 OK",
            "image/png",
            b"\x89PNG\r\n\x1a\ninvalid",
        )])
        .await;
        let error = fetch_cover(&client(), &server.url()).await.unwrap_err();
        assert!(assert_private_error(&error).contains("图片"));
        assert_eq!(server.attempts(), 1);
    }

    #[tokio::test]
    async fn rejects_images_exceeding_the_decode_dimension_limit() {
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(RgbImage::from_pixel(8193, 1, Rgb([0, 0, 0])))
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        let server =
            CoverServer::start(vec![response("200 OK", "image/png", &bytes.into_inner())]).await;
        let error = fetch_cover(&client(), &server.url()).await.unwrap_err();
        assert!(assert_private_error(&error).contains("图片"));
        assert_eq!(server.attempts(), 1);
    }

    #[tokio::test]
    async fn rejects_image_headers_exceeding_the_decode_memory_limit() {
        let mut bytes = image(ImageFormat::Png);
        bytes[16..20].copy_from_slice(&8192u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&8192u32.to_be_bytes());
        let mut crc = u32::MAX;
        for byte in &bytes[12..29] {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
            }
        }
        bytes[29..33].copy_from_slice(&(!crc).to_be_bytes());
        let server = CoverServer::start(vec![response("200 OK", "image/png", &bytes)]).await;
        let error = fetch_cover(&client(), &server.url()).await.unwrap_err();
        assert!(assert_private_error(&error).contains("128 MiB"));
        assert_eq!(server.attempts(), 1);
    }

    #[tokio::test]
    async fn reads_validated_local_paths_and_file_urls() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("自定义封面.png");
        let expected = image(ImageFormat::Png);
        std::fs::write(&path, &expected).unwrap();
        for source in [
            path.to_string_lossy().into_owned(),
            url::Url::from_file_path(&path).unwrap().into(),
        ] {
            assert_eq!(
                fetch_cover(&client(), &source).await.unwrap(),
                (expected.clone(), "png")
            );
        }
    }

    #[tokio::test]
    async fn decodes_base64_image_data_and_uses_the_actual_format() {
        let expected = image(ImageFormat::Png);
        let data = format!("data:image/jpeg;base64,{}", STANDARD.encode(&expected));
        assert_eq!(
            fetch_cover(&client(), &data).await.unwrap(),
            (expected, "png")
        );
    }

    #[tokio::test]
    async fn rejects_invalid_or_oversized_local_and_data_images() {
        let root = tempfile::tempdir().unwrap();
        let oversized = root.path().join("oversized.png");
        std::fs::File::create(&oversized)
            .unwrap()
            .set_len(8 * 1024 * 1024 + 1)
            .unwrap();
        let error = fetch_cover(&client(), oversized.to_str().unwrap())
            .await
            .unwrap_err();
        assert!(assert_private_error(&error).contains("8 MiB"));
        let invalid = fetch_cover(&client(), "data:image/png;base64,invalid%%")
            .await
            .unwrap_err();
        assert_private_error(&invalid);
        let data = format!("data:image/png;base64,{}", "A".repeat(11_184_813));
        let error = fetch_cover(&client(), &data).await.unwrap_err();
        assert!(assert_private_error(&error).contains("8 MiB"));
    }

    #[tokio::test]
    async fn reads_tauri_asset_urls_using_the_encoded_absolute_local_path() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("自定义 封面 +#%25.png");
        let expected = image(ImageFormat::Png);
        std::fs::write(&path, &expected).unwrap();
        let encoded = urlencoding::encode(path.to_str().unwrap());
        for origin in [
            "asset://localhost",
            "http://asset.localhost",
            "https://asset.localhost",
        ] {
            let source = format!("{origin}/{encoded}");
            assert_eq!(
                fetch_cover(&client(), &source).await.unwrap(),
                (expected.clone(), "png")
            );
        }
    }

    #[tokio::test]
    async fn rejects_asset_urls_without_an_absolute_local_path() {
        for source in [
            "asset://localhost/relative.png",
            "http://asset.localhost/relative.png",
            "asset://remote.example/cover.png",
            "asset://localhost/%FF",
        ] {
            let error = fetch_cover(&client(), source).await.unwrap_err();
            assert!(assert_private_error(&error).contains("地址"));
        }
    }

    #[tokio::test]
    async fn enforces_the_image_byte_limit_for_tauri_asset_paths() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("oversized.png");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(8 * 1024 * 1024 + 1)
            .unwrap();
        let source = format!(
            "asset://localhost/{}",
            urlencoding::encode(path.to_str().unwrap())
        );
        let error = fetch_cover(&client(), &source).await.unwrap_err();
        assert!(assert_private_error(&error).contains("8 MiB"));
    }
}
