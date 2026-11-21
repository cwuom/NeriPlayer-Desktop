use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::api::transport::FallbackHttp;
use neri_common::error::{AppError, AppResult};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36";

pub struct QqMusicClient {
    http: FallbackHttp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QqSongUrl {
    pub url: Option<String>,
    pub bitrate: u64,
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QqSearchResult {
    pub song_mid: String,
    pub song_name: String,
    pub artists: Vec<String>,
    pub album_name: String,
    pub album_mid: Option<String>,
    pub singer_mid: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Deserialize)]
struct QqSearchResponse {
    data: Option<QqSearchData>,
}

#[derive(Debug, Deserialize)]
struct QqSearchData {
    song: Option<QqSearchSong>,
}

#[derive(Debug, Deserialize)]
struct QqSearchSong {
    list: Option<Vec<QqSongSummary>>,
}

#[derive(Debug, Deserialize)]
struct QqSongSummary {
    #[serde(rename = "songmid")]
    song_mid: String,
    #[serde(rename = "songname")]
    song_name: String,
    singer: Vec<QqArtist>,
    #[serde(rename = "albummid")]
    album_mid: Option<String>,
    #[serde(rename = "albumname")]
    album_name: Option<String>,
    interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct QqArtist {
    name: String,
    #[serde(default)]
    mid: Option<String>,
}

#[derive(Debug, Deserialize)]
struct QqLyricResponse {
    lyric: Option<String>,
    trans: Option<String>,
}

#[derive(Debug, Deserialize)]
struct QqVkeyEnvelope {
    req_0: Option<QqVkeyResponse>,
}

#[derive(Debug, Deserialize)]
struct QqVkeyResponse {
    data: Option<QqVkeyData>,
}

#[derive(Debug, Deserialize)]
struct QqVkeyData {
    sip: Option<Vec<String>>,
    midurlinfo: Option<Vec<QqMidUrlInfo>>,
}

#[derive(Debug, Deserialize)]
struct QqMidUrlInfo {
    purl: Option<String>,
    #[serde(default)]
    songmid: String,
    #[serde(default)]
    filename: String,
}

impl QqMusicClient {
    pub fn with_transport(http: FallbackHttp) -> Self {
        Self { http }
    }

    pub fn new(http: &Client) -> Self {
        Self::with_transport(FallbackHttp::new(http, "qq"))
    }

    pub async fn search(
        &self,
        keyword: &str,
        page: u32,
        limit: u32,
    ) -> AppResult<Vec<QqSearchResult>> {
        let limit_text = limit.to_string();
        let page_text = page.to_string();
        let resp_text = self
            .http
            .send(|client| {
                client
                    .get("https://c.y.qq.com/soso/fcgi-bin/client_search_cp")
                    .query(&[
                        ("format", "json"),
                        ("n", limit_text.as_str()),
                        ("p", page_text.as_str()),
                        ("w", keyword),
                        ("cr", "1"),
                        ("g_tk", "5381"),
                    ])
                    .header("User-Agent", USER_AGENT)
                    .header("Referer", "https://y.qq.com")
            })
            .await?
            .text()
            .await?;

        let value: QqSearchResponse = serde_json::from_str(&unwrap_jsonp(&resp_text)?)?;
        let songs = value
            .data
            .and_then(|d| d.song)
            .and_then(|s| s.list)
            .unwrap_or_default();

        Ok(songs
            .into_iter()
            .map(|song| {
                let singer_mid = song
                    .singer
                    .iter()
                    .find_map(|a| a.mid.as_deref().filter(|mid| !mid.is_empty()))
                    .map(String::from);
                QqSearchResult {
                    song_mid: song.song_mid,
                    song_name: song.song_name,
                    artists: song.singer.into_iter().map(|a| a.name).collect(),
                    album_name: song.album_name.unwrap_or_default(),
                    album_mid: song.album_mid,
                    singer_mid,
                    duration_ms: song.interval.unwrap_or(0) * 1000,
                }
            })
            .collect())
    }

    pub async fn get_lyrics(&self, song_mid: &str) -> AppResult<(Option<String>, Option<String>)> {
        let response_text = self
            .http
            .send(|client| {
                client
                    .get("https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg")
                    .query(&[
                        ("songmid", song_mid),
                        ("format", "json"),
                        ("inCharset", "utf8"),
                        ("outCharset", "utf-8"),
                        ("nobase64", "1"),
                        ("g_tk", "5381"),
                    ])
                    .header("User-Agent", USER_AGENT)
                    .header("Referer", "https://y.qq.com")
            })
            .await?
            .text()
            .await?;
        let response: QqLyricResponse = serde_json::from_str(&unwrap_jsonp(&response_text)?)?;

        Ok((
            decode_qq_lyric_payload(response.lyric),
            strip_untranslated_placeholder_lines(decode_qq_lyric_payload(response.trans)),
        ))
    }

    pub async fn get_song_url(&self, song_mid: &str, quality: &str) -> AppResult<QqSongUrl> {
        let candidates = qq_filename_candidates(song_mid, quality);

        for (filename, bitrate, format) in candidates {
            let data = serde_json::json!({
                "comm": {
                    "uin": "0",
                    "format": "json",
                    "ct": 24,
                    "cv": 0
                },
                "req_0": {
                    "module": "vkey.GetVkeyServer",
                    "method": "CgiGetVkey",
                    "param": {
                        "guid": "10000",
                        "songmid": [song_mid],
                        "songtype": [0],
                        "filename": [filename],
                        "uin": "0",
                        "loginflag": 1,
                        "platform": "20"
                    }
                }
            });

            let data_text = data.to_string();
            let response_text = self
                .http
                .send(|client| {
                    client
                        .get("https://u.y.qq.com/cgi-bin/musicu.fcg")
                        .query(&[("data", data_text.as_str())])
                        .header("User-Agent", USER_AGENT)
                        .header("Referer", "https://y.qq.com")
                })
                .await?
                .text()
                .await?;

            let response: QqVkeyEnvelope = serde_json::from_str(&unwrap_jsonp(&response_text)?)?;
            let Some(data) = response.req_0.and_then(|r| r.data) else {
                continue;
            };
            let Some(info) = data
                .midurlinfo
                .unwrap_or_default()
                .into_iter()
                .find(|info| info.songmid.is_empty() || info.songmid == song_mid)
            else {
                continue;
            };
            let Some(purl) = info.purl.filter(|v| !v.trim().is_empty()) else {
                continue;
            };

            let base = data
                .sip
                .unwrap_or_default()
                .into_iter()
                .find(|s| s.starts_with("http"))
                .unwrap_or_else(|| "https://dl.stream.qqmusic.qq.com/".to_string());
            let url = if purl.starts_with("http") {
                purl
            } else {
                format!(
                    "{}{}",
                    base.trim_end_matches('/'),
                    ensure_leading_slash(&purl)
                )
            };

            let resolved_format = if !info.filename.is_empty() {
                format_from_filename(&info.filename).unwrap_or(format)
            } else {
                format
            };

            return Ok(QqSongUrl {
                url: Some(url),
                bitrate,
                format: resolved_format,
            });
        }

        Ok(QqSongUrl {
            url: None,
            bitrate: 0,
            format: String::new(),
        })
    }
}

fn unwrap_jsonp(raw: &str) -> AppResult<String> {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') {
        return Ok(trimmed.to_string());
    }

    let start = trimmed
        .find('{')
        .ok_or_else(|| AppError::Api("QQ search response missing JSON body".into()))?;
    let end = trimmed
        .rfind('}')
        .ok_or_else(|| AppError::Api("QQ search response missing JSON end".into()))?;
    if end <= start {
        return Err(AppError::Api("QQ search JSONP format invalid".into()));
    }

    Ok(trimmed[start..=end].to_string())
}

fn decode_qq_lyric_payload(input: Option<String>) -> Option<String> {
    let raw = input?.trim().to_string();
    if raw.is_empty() {
        return None;
    }

    let unescaped = html_unescape(&raw);
    if let Some(decoded) = decode_base64_lrc(&unescaped) {
        return Some(html_unescape(&decoded));
    }
    Some(unescaped)
}

// QQ 用只含 // 的一行表示这句未翻译，原样保留会在歌词视图里显示成整排斜杠
// 去掉时间标签后内容恰为 // 的行整行丢弃，其余翻译行仍各自对准原句
fn strip_untranslated_placeholder_lines(input: Option<String>) -> Option<String> {
    let source = input?;
    let filtered = source
        .lines()
        .filter(|line| {
            let content = match line.rfind(']') {
                Some(idx) => &line[idx + 1..],
                None => line,
            };
            content.trim() != "//"
        })
        .collect::<Vec<_>>()
        .join("\n");
    if filtered.trim().is_empty() {
        None
    } else {
        Some(filtered)
    }
}

fn decode_base64_lrc(value: &str) -> Option<String> {
    if !value.len().is_multiple_of(4) {
        return None;
    }
    if !value.chars().all(|c| {
        c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' || c == '\r' || c == '\n'
    }) {
        return None;
    }

    let decoded = STANDARD.decode(value.as_bytes()).ok()?;
    let as_text = String::from_utf8(decoded).ok()?;
    if as_text.contains('[') || as_text.contains('\n') || as_text.contains('\r') {
        return Some(as_text);
    }
    None
}

fn html_unescape(value: &str) -> String {
    value
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

fn qq_filename_candidates(song_mid: &str, quality: &str) -> Vec<(String, u64, String)> {
    let ordered = match quality {
        "lossless" | "flac" => vec![
            ("F000", "flac", 999_000),
            ("M800", "mp3", 320_000),
            ("M500", "mp3", 128_000),
            ("C400", "m4a", 96_000),
        ],
        "high" | "exhigh" | "320k" => vec![
            ("M800", "mp3", 320_000),
            ("M500", "mp3", 128_000),
            ("C400", "m4a", 96_000),
        ],
        _ => vec![
            ("M500", "mp3", 128_000),
            ("C400", "m4a", 96_000),
            ("M800", "mp3", 320_000),
        ],
    };

    ordered
        .into_iter()
        .map(|(prefix, ext, bitrate)| {
            (
                format!("{}{}.{}", prefix, song_mid, ext),
                bitrate,
                ext.to_string(),
            )
        })
        .collect()
}

fn ensure_leading_slash(value: &str) -> String {
    if value.starts_with('/') {
        value.to_string()
    } else {
        format!("/{}", value)
    }
}

fn format_from_filename(filename: &str) -> Option<String> {
    filename.rsplit_once('.').map(|(_, ext)| ext.to_lowercase())
}
