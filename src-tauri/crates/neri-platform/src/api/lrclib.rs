// LRCLIB 歌词 API（开放 API，无鉴权）
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::api::transport::FallbackHttp;
use neri_common::error::AppResult;

const BASE_URL: &str = "https://lrclib.net/api";

pub struct LrcLibClient {
    http: FallbackHttp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LrcLibResult {
    pub id: u64,
    #[serde(rename = "trackName")]
    pub track_name: String,
    #[serde(rename = "artistName")]
    pub artist_name: String,
    pub duration: f64,
    #[serde(rename = "syncedLyrics")]
    pub synced_lyrics: Option<String>,
    #[serde(rename = "plainLyrics")]
    pub plain_lyrics: Option<String>,
}

impl LrcLibClient {
    pub fn with_transport(http: FallbackHttp) -> Self {
        Self { http }
    }

    pub fn new(http: &Client) -> Self {
        Self::with_transport(FallbackHttp::new(http, "lrclib"))
    }

    /// 精确匹配歌词
    pub async fn get_lyrics(
        &self,
        track: &str,
        artist: &str,
        duration_secs: u64,
    ) -> AppResult<Option<LrcLibResult>> {
        let duration_text = duration_secs.to_string();
        let resp = self
            .http
            .send(|client| {
                client
                    .get(format!("{}/get", BASE_URL))
                    .query(&[
                        ("track_name", track),
                        ("artist_name", artist),
                        ("duration", duration_text.as_str()),
                    ])
                    .header("User-Agent", "NeriPlayer/1.0.0")
            })
            .await?;

        if resp.status() == 404 {
            return Ok(None);
        }

        let result: LrcLibResult = resp.json().await?;
        Ok(Some(result))
    }

    /// 模糊搜索歌词
    pub async fn search(&self, query: &str) -> AppResult<Vec<LrcLibResult>> {
        let resp = self
            .http
            .send(|client| {
                client
                    .get(format!("{}/search", BASE_URL))
                    .query(&[("q", query)])
                    .header("User-Agent", "NeriPlayer/1.0.0")
            })
            .await?;

        let results: Vec<LrcLibResult> = resp.json().await?;
        Ok(results)
    }
}
