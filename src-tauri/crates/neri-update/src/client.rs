//! HTTP 下载：清单与签名一次取完，文件内容按 Range 分段取。
//! 服务器或代理不支持 Range 时退回到读整个响应、跳过前面的字节，结果一样只是更慢

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::StatusCode;

use crate::error::{UpdateError, UpdateResult};
use crate::manifest::{BlobRef, MAX_MANIFEST_BYTES};
use crate::stage::RangeFetcher;

pub struct HttpFetcher {
    client: reqwest::Client,
    cancel: Arc<AtomicBool>,
}

impl HttpFetcher {
    pub fn new(client: reqwest::Client, cancel: Arc<AtomicBool>) -> Self {
        Self { client, cancel }
    }

    /// 应用内其它请求可能配置了 Cookie 或平台专用的请求头，这里单独建一个干净的客户端
    pub fn default_client(user_agent: &str) -> UpdateResult<reqwest::Client> {
        reqwest::Client::builder()
            .user_agent(user_agent)
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(60))
            .build()
            .map_err(network)
    }

    /// 取清单或签名这类小文件，超过上限直接放弃
    pub async fn fetch_small(&self, url: &str) -> UpdateResult<Vec<u8>> {
        let response = self.client.get(url).send().await.map_err(network)?;
        if !response.status().is_success() {
            return Err(UpdateError::Network(format!(
                "{url}: HTTP {}",
                response.status()
            )));
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            body.extend_from_slice(&chunk.map_err(network)?);
            if body.len() > MAX_MANIFEST_BYTES {
                return Err(UpdateError::InvalidManifest(format!("{url} is too large")));
            }
        }
        Ok(body)
    }
}

impl RangeFetcher for HttpFetcher {
    async fn fetch(
        &self,
        blob: &BlobRef,
        offset: u64,
        length: u64,
        on_bytes: &mut (dyn FnMut(u64) + Send),
    ) -> UpdateResult<Vec<u8>> {
        let end = offset + length - 1;
        let response = self
            .client
            .get(&blob.url)
            .header(RANGE, format!("bytes={offset}-{end}"))
            .send()
            .await
            .map_err(network)?;
        let mut skip = match response.status() {
            StatusCode::PARTIAL_CONTENT => {
                let range = response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default();
                if range_start(range) != Some(offset) {
                    return Err(UpdateError::Network(format!(
                        "unexpected Content-Range {range:?}"
                    )));
                }
                0
            }
            StatusCode::OK => {
                log::info!(target: "update", "server ignored Range, streaming {}", blob.url);
                offset
            }
            status => return Err(UpdateError::Network(format!("{}: HTTP {status}", blob.url))),
        };

        let capacity =
            usize::try_from(length).map_err(|_| UpdateError::Other("range too large".into()))?;
        let mut body = Vec::with_capacity(capacity);
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(UpdateError::Cancelled);
            }
            let chunk = chunk.map_err(network)?;
            let mut data: &[u8] = &chunk;
            if skip > 0 {
                let dropped = (skip as usize).min(data.len());
                data = &data[dropped..];
                skip -= dropped as u64;
            }
            let wanted = (length as usize - body.len()).min(data.len());
            body.extend_from_slice(&data[..wanted]);
            on_bytes(wanted as u64);
            if body.len() as u64 == length {
                break;
            }
        }
        if body.len() as u64 != length {
            return Err(UpdateError::Network(format!("{} ended early", blob.url)));
        }
        Ok(body)
    }
}

/// `bytes 100-199/1000` → 100
fn range_start(header: &str) -> Option<u64> {
    header
        .strip_prefix("bytes ")?
        .split('-')
        .next()?
        .trim()
        .parse()
        .ok()
}

fn network(error: reqwest::Error) -> UpdateError {
    UpdateError::Network(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_range_start() {
        assert_eq!(range_start("bytes 100-199/1000"), Some(100));
        assert_eq!(range_start("bytes 0-0/*"), Some(0));
        assert_eq!(range_start("items 1-2/3"), None);
        assert_eq!(range_start(""), None);
    }
}
