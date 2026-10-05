use super::parser::{self, LyricLine, LyricWord};
use crate::api::transport::FallbackHttp;
use base64::{engine::general_purpose::STANDARD, Engine};
use regex::Regex;
use serde_json::Value;
use std::{sync::OnceLock, time::Duration};

const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_TIME_MS: u64 = 24 * 60 * 60 * 1000;
const USER_AGENT: &str = "NeriPlayer/1.0 (https://github.com/cwuom/NeriPlayer)";
const PROVIDER_BUDGET: Duration = Duration::from_secs(12);

pub(super) struct ExternalLyricsClient {
    transport: FallbackHttp,
    amll_base: String,
    kugou_song_endpoint: String,
    kugou_base: String,
}
impl ExternalLyricsClient {
    pub(super) fn new(transport: FallbackHttp) -> Self {
        Self {
            transport,
            amll_base: "https://amlldb.bikonoo.com".into(),
            kugou_song_endpoint: "http://mobilecdn.kugou.com/api/v3/search/song".into(),
            kugou_base: "https://lyrics.kugou.com".into(),
        }
    }

    pub(super) async fn fetch(
        &self,
        title: &str,
        artist: &str,
        duration_ms: u64,
        word_only: bool,
    ) -> Vec<LyricLine> {
        if title.trim().is_empty()
            || artist.trim().is_empty()
            || duration_ms == 0
            || duration_ms > MAX_TIME_MS
        {
            return Vec::new();
        }
        if let Ok(Some(lines)) =
            tokio::time::timeout(PROVIDER_BUDGET, self.fetch_amll(title, artist, duration_ms)).await
        {
            return lines;
        }
        tokio::time::timeout(
            PROVIDER_BUDGET,
            self.fetch_kugou(title, artist, duration_ms, word_only),
        )
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
    }

    async fn read(
        &self,
        build: impl Fn(&reqwest::Client) -> reqwest::RequestBuilder,
    ) -> Option<String> {
        let mut response = self
            .transport
            .send(|client| {
                build(client)
                    .header("User-Agent", USER_AGENT)
                    .timeout(Duration::from_secs(4))
            })
            .await
            .ok()?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|length| length > MAX_BYTES as u64)
        {
            return None;
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.ok()? {
            if chunk.len() > MAX_BYTES.saturating_sub(bytes.len()) {
                return None;
            }
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes)
            .ok()
            .filter(|body| !body.trim().is_empty())
    }

    async fn fetch_amll(&self, title: &str, artist: &str, duration: u64) -> Option<Vec<LyricLine>> {
        let endpoint = format!("{}/api/search-lyrics", self.amll_base.trim_end_matches('/'));
        let query = serde_json::json!({"query":title.trim(),"type":"title"});
        let body = self
            .read(|client| client.post(&endpoint).json(&query))
            .await?;
        let response: Value = serde_json::from_str(&body).ok()?;
        let mut candidates = response
            .as_array()?
            .iter()
            .take(200)
            .filter_map(|item| {
                let file = item.get("file")?.as_str()?.trim();
                if !safe_ttml_file(file) {
                    return None;
                }
                let score = identity_score(
                    title,
                    artist,
                    &aliases(item, "titles", "title"),
                    &aliases(item, "artists", "artist"),
                );
                (score >= 70).then(|| {
                    (
                        score,
                        number(item.get("score")).unwrap_or(0),
                        file.to_owned(),
                    )
                })
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|candidate| std::cmp::Reverse((candidate.0, candidate.1)));
        for (_, _, file) in candidates.into_iter().take(5) {
            let mut url = reqwest::Url::parse(&self.amll_base).ok()?;
            url.path_segments_mut()
                .ok()?
                .pop_if_empty()
                .push("raw-lyrics")
                .push(&file);
            let Some(body) = self.read(|client| client.get(url.clone())).await else {
                continue;
            };
            let Ok(lines) = super::ttml::parse(&body) else {
                continue;
            };
            if has_word_timing(&lines) && amll_duration_compatible(duration, lyric_end(&lines)) {
                return Some(lines);
            }
        }
        None
    }

    async fn fetch_kugou(
        &self,
        title: &str,
        artist: &str,
        duration: u64,
        word_only: bool,
    ) -> Option<Vec<LyricLine>> {
        let keyword = format!("{} {}", title.trim(), artist.trim());
        let body = self
            .read(|client| {
                client.get(&self.kugou_song_endpoint).query(&[
                    ("format", "json"),
                    ("keyword", keyword.as_str()),
                    ("page", "1"),
                    ("pagesize", "8"),
                    ("showtype", "1"),
                ])
            })
            .await?;
        let response: Value = serde_json::from_str(&body).ok()?;
        let mut songs = response
            .get("data")?
            .get("info")?
            .as_array()?
            .iter()
            .take(8)
            .filter_map(|item| {
                let song_title = item.get("songname")?.as_str()?.trim();
                let song_artist = item.get("singername")?.as_str()?.trim();
                let hash = item.get("hash")?.as_str()?.trim();
                let song_duration = number(item.get("duration"))?.checked_mul(1000)?;
                let score = identity_score(
                    title,
                    artist,
                    &[song_title.to_owned()],
                    &[song_artist.to_owned()],
                );
                if hash.is_empty()
                    || score < 70
                    || !external_duration_compatible(duration, song_duration)
                {
                    return None;
                }
                Some((
                    score,
                    song_duration.abs_diff(duration),
                    Song {
                        title: song_title.to_owned(),
                        artist: song_artist.to_owned(),
                        hash: hash.to_owned(),
                        duration: song_duration,
                    },
                ))
            })
            .collect::<Vec<_>>();
        songs.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
        for (_, _, song) in songs.into_iter().take(4) {
            if let Some(lines) = self.fetch_kugou_song(&song, duration, word_only).await {
                return Some(lines);
            }
        }
        None
    }

    async fn fetch_kugou_song(
        &self,
        song: &Song,
        duration: u64,
        word_only: bool,
    ) -> Option<Vec<LyricLine>> {
        let endpoint = format!("{}/search", self.kugou_base.trim_end_matches('/'));
        let keyword = format!("{} - {}", song.artist, song.title);
        let song_duration = song.duration.to_string();
        let body = self
            .read(|client| {
                client.get(&endpoint).query(&[
                    ("ver", "1"),
                    ("man", "yes"),
                    ("client", "pc"),
                    ("keyword", keyword.as_str()),
                    ("duration", song_duration.as_str()),
                    ("hash", song.hash.as_str()),
                ])
            })
            .await?;
        let response: Value = serde_json::from_str(&body).ok()?;
        let mut candidates = response
            .get("candidates")?
            .as_array()?
            .iter()
            .take(64)
            .filter_map(|item| {
                let id = item.get("id").and_then(string_number)?;
                let key = item.get("accesskey")?.as_str()?.trim();
                let candidate_duration = number(item.get("duration"))?;
                if id.is_empty()
                    || key.is_empty()
                    || !external_duration_compatible(duration, candidate_duration)
                {
                    return None;
                }
                Some((
                    number(item.get("score")).unwrap_or(0),
                    candidate_duration.abs_diff(song.duration),
                    id,
                    key.to_owned(),
                ))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
        candidates.truncate(8);
        let endpoint = format!("{}/download", self.kugou_base.trim_end_matches('/'));
        // Android 先试完逐字候选，避免首个 LRC 抢占后续可用 KRC
        for format in if word_only {
            &["krc"][..]
        } else {
            &["krc", "lrc"][..]
        } {
            for (_, _, id, key) in &candidates {
                let body = self
                    .read(|client| {
                        client.get(&endpoint).query(&[
                            ("ver", "1"),
                            ("client", if *format == "krc" { "mobi" } else { "pc" }),
                            ("id", id.as_str()),
                            ("accesskey", key.as_str()),
                            ("fmt", *format),
                            ("charset", "utf8"),
                        ])
                    })
                    .await;
                let Some(body) = body else { continue };
                let lines = if *format == "krc" {
                    decode_krc(&body)
                } else {
                    decode_lrc(&body)
                };
                let Some(lines) = lines else { continue };
                if !lines.is_empty()
                    && (!word_only || has_word_timing(&lines))
                    && amll_duration_compatible(duration, lyric_end(&lines))
                {
                    return Some(lines);
                }
            }
        }
        None
    }
}

struct Song {
    title: String,
    artist: String,
    hash: String,
    duration: u64,
}

fn number(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    value
        .as_u64()
        .or_else(|| value.as_str()?.trim().parse().ok())
}
fn string_number(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(|value| value.trim().to_owned())
        .or_else(|| value.as_u64().map(|value| value.to_string()))
}
fn aliases(value: &Value, plural: &str, singular: &str) -> Vec<String> {
    let mut result = value
        .get(plural)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .take(16)
                .filter_map(Value::as_str)
                .filter(|value| value.len() <= 512)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Some(value) = value
        .get(singular)
        .and_then(Value::as_str)
        .filter(|value| value.len() <= 512)
    {
        result.push(value.to_owned());
    }
    result
}

fn normalized(value: &str) -> String {
    let value = value
        .chars()
        .map(|character| match character {
            '\u{ff01}'..='\u{ff5e}' => char::from_u32(character as u32 - 0xfee0).unwrap(),
            '\u{3000}' => ' ',
            _ => character,
        })
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .replace('&', " and ");
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty() && !matches!(*token, "feat" | "ft" | "featuring"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn version_tags(value: &str) -> Vec<&'static str> {
    [
        "live",
        "remix",
        "acoustic",
        "instrumental",
        "karaoke",
        "cover",
        "sped",
        "slowed",
        "rap",
        "remaster",
        "remastered",
        "demo",
        "现场",
        "伴奏",
        "翻唱",
        "纯音乐",
    ]
    .into_iter()
    .filter(|tag| {
        if tag.is_ascii() {
            value.split_whitespace().any(|token| token == *tag)
        } else {
            value.contains(tag)
        }
    })
    .collect()
}

fn identity_score(title: &str, artist: &str, titles: &[String], artists: &[String]) -> i32 {
    static ARTISTS: OnceLock<Regex> = OnceLock::new();
    static FEATURED: OnceLock<Regex> = OnceLock::new();
    let requested = normalized(title);
    if requested.is_empty() {
        return 0;
    }
    let requested_artists = ARTISTS
        .get_or_init(|| Regex::new(r"(?i)[/,，、&+]|\s+x\s+").unwrap())
        .split(artist)
        .map(normalized)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if requested_artists.is_empty() {
        return 0;
    }
    let title_score = titles
        .iter()
        .map(|title| {
            let candidate = normalized(title);
            if version_tags(&requested) != version_tags(&candidate) {
                return 0;
            }
            if candidate == requested {
                90
            } else if FEATURED
                .get_or_init(|| Regex::new(r"(?i)\b(?:feat|ft|featuring)\.?\b").unwrap())
                .find(title)
                .is_some_and(|marker| normalized(&title[..marker.start()]) == requested)
            {
                78
            } else {
                0
            }
        })
        .max()
        .unwrap_or(0);
    if title_score == 0 {
        return 0;
    }
    let artist_score = artists
        .iter()
        .map(|value| normalized(value))
        .flat_map(|candidate| {
            requested_artists.iter().map(move |requested| {
                if candidate.is_empty() {
                    0
                } else if candidate == *requested {
                    55
                } else if format!(" {candidate} ").contains(&format!(" {requested} "))
                    || format!(" {requested} ").contains(&format!(" {candidate} "))
                {
                    40
                } else {
                    candidate
                        .split_whitespace()
                        .filter(|token| {
                            requested
                                .split_whitespace()
                                .any(|requested| requested == *token)
                        })
                        .count() as i32
                        * 8
                }
            })
        })
        .max()
        .unwrap_or(0);
    if artist_score < 30 {
        0
    } else {
        title_score + artist_score
    }
}

fn amll_duration_compatible(target: u64, candidate: u64) -> bool {
    if target == 0 || candidate == 0 || target > MAX_TIME_MS || candidate > MAX_TIME_MS {
        return false;
    }
    let tolerance = if candidate < target {
        30_000.max(target * 15 / 100).min(60_000)
    } else {
        12_000.max(target / 10).min(30_000)
    };
    target.abs_diff(candidate) <= tolerance
}
fn external_duration_compatible(target: u64, candidate: u64) -> bool {
    target > 0
        && candidate > 0
        && target <= MAX_TIME_MS
        && candidate <= MAX_TIME_MS
        && target.abs_diff(candidate) <= 7_000.max(target * 6 / 100).min(15_000)
}
fn safe_ttml_file(file: &str) -> bool {
    !file.is_empty()
        && file.len() <= 512
        && file.ends_with(".ttml")
        && file.trim() == file
        && !file.contains("..")
        && !file.chars().any(|character| {
            character.is_control() || matches!(character, '/' | '\\' | ':' | '%' | '?' | '#')
        })
}
pub(super) fn has_word_timing(lines: &[LyricLine]) -> bool {
    lines.iter().any(|line| {
        line.words
            .iter()
            .any(|word| word.duration_ms > 0 && !word.text.trim().is_empty())
    })
}
fn lyric_end(lines: &[LyricLine]) -> u64 {
    lines
        .iter()
        .flat_map(|line| {
            std::iter::once(line.start_ms.saturating_add(line.duration_ms)).chain(
                line.words
                    .iter()
                    .map(|word| word.start_ms.saturating_add(word.duration_ms)),
            )
        })
        .max()
        .unwrap_or(0)
}

fn decoded_content(body: &str) -> Option<Vec<u8>> {
    if body.len() > MAX_BYTES {
        return None;
    }
    let root: Value = serde_json::from_str(body).ok()?;
    if number(root.get("status")) != Some(200) && number(root.get("error_code")) != Some(0) {
        return None;
    }
    STANDARD
        .decode(root.get("content")?.as_str()?)
        .ok()
        .filter(|bytes| bytes.len() <= MAX_BYTES)
}
fn decode_lrc(body: &str) -> Option<Vec<LyricLine>> {
    let content = String::from_utf8(decoded_content(body)?).ok()?;
    let lines = parser::parse_lrc(content.trim_start_matches('\u{feff}').trim());
    (!lines.is_empty()).then_some(lines)
}
fn decode_krc(body: &str) -> Option<Vec<LyricLine>> {
    let bytes = decoded_content(body)?;
    if bytes.get(..4)? != b"krc1" {
        return None;
    }
    const KEY: [u8; 16] = [
        0x40, 0x47, 0x61, 0x77, 0x5e, 0x32, 0x74, 0x47, 0x51, 0x36, 0x31, 0x2d, 0xce, 0xd2, 0x6e,
        0x69,
    ];
    let compressed = bytes[4..]
        .iter()
        .enumerate()
        .map(|(index, byte)| byte ^ KEY[index % KEY.len()])
        .collect::<Vec<_>>();
    let mut plaintext = vec![0; MAX_BYTES + 1];
    let mut inflater = flate2::Decompress::new(true);
    if inflater
        .decompress(&compressed, &mut plaintext, flate2::FlushDecompress::Finish)
        .ok()?
        != flate2::Status::StreamEnd
        || inflater.total_out() > MAX_BYTES as u64
        || inflater.total_in() != compressed.len() as u64
    {
        return None;
    }
    plaintext.truncate(inflater.total_out() as usize);
    parse_krc(&String::from_utf8(plaintext).ok()?)
}

fn parse_krc(content: &str) -> Option<Vec<LyricLine>> {
    static LINE: OnceLock<Regex> = OnceLock::new();
    static WORD: OnceLock<Regex> = OnceLock::new();
    let line_re = LINE.get_or_init(|| Regex::new(r"^\[(\d+),\s*(\d+)\](.*)$").unwrap());
    let word_re =
        WORD.get_or_init(|| Regex::new(r"<(\d+),\s*(\d+),\s*[-\d]+>([^<\n\r]*)").unwrap());
    let translations = krc_translations(content);
    let mut lines = Vec::new();
    let mut row_index = 0;
    for raw in content.lines() {
        let Some(captures) = line_re.captures(raw.trim_start_matches('\u{feff}').trim()) else {
            continue;
        };
        let start: u64 = captures[1].parse().ok()?;
        let duration: u64 = captures[2].parse().ok()?;
        let end = start
            .checked_add(duration)
            .filter(|value| *value <= MAX_TIME_MS)?;
        let translation = translations
            .as_ref()
            .and_then(|rows| rows.get(row_index))
            .cloned()
            .filter(|value| !value.is_empty());
        row_index += 1;
        let mut words = Vec::new();
        for word in word_re.captures_iter(&captures[3]) {
            let start = start.checked_add(word[1].parse::<u64>().ok()?)?;
            let duration = word[2].parse::<u64>().ok()?;
            if start.checked_add(duration)? > end.saturating_add(500) || start > MAX_TIME_MS {
                return None;
            }
            if duration > 0 && !word[3].trim().is_empty() {
                words.push(LyricWord {
                    start_ms: start,
                    duration_ms: duration,
                    text: word[3].to_owned(),
                });
            }
        }
        if words.is_empty() {
            continue;
        }
        let text = words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<String>()
            .trim()
            .to_owned();
        lines.push(LyricLine {
            start_ms: start,
            duration_ms: duration,
            text,
            translation,
            roman: None,
            words,
        });
        if row_index > 20_000 {
            return None;
        }
    }
    lines.sort_by_key(|line| line.start_ms);
    has_word_timing(&lines).then_some(lines)
}

fn krc_translations(content: &str) -> Option<Vec<String>> {
    let encoded = content
        .lines()
        .find_map(|line| line.trim().strip_prefix("[language:")?.strip_suffix(']'))?;
    let encoded = encoded
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect::<String>();
    let bytes = STANDARD.decode(encoded).ok()?;
    let root: Value = serde_json::from_slice(&bytes).ok()?;
    let rows = root
        .get("content")?
        .as_array()?
        .iter()
        .find(|entry| number(entry.get("type")) == Some(1))?
        .get("lyricContent")?
        .as_array()?;
    Some(
        rows.iter()
            .take(20_000)
            .map(|row| match row {
                Value::String(value) => value.trim().to_owned(),
                Value::Array(values) => values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => String::new(),
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use std::io::{Read, Write};

    fn krc_response(text: &str) -> String {
        let mut zip = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        zip.write_all(text.as_bytes()).unwrap();
        let mut bytes = b"krc1".to_vec();
        let key = [
            0x40, 0x47, 0x61, 0x77, 0x5e, 0x32, 0x74, 0x47, 0x51, 0x36, 0x31, 0x2d, 0xce, 0xd2,
            0x6e, 0x69,
        ];
        bytes.extend(
            zip.finish()
                .unwrap()
                .into_iter()
                .enumerate()
                .map(|(i, b)| b ^ key[i % key.len()]),
        );
        serde_json::json!({"status":200,"content":STANDARD.encode(bytes)}).to_string()
    }

    #[test]
    fn external_identity_requires_artist_and_rejects_version_mismatch() {
        let titles = vec!["Ｈｅｌｌｏ (feat. Guest)".into(), "Hello".into()];
        assert!(identity_score("Hello", "Alice / Guest", &titles, &["Alice".into()]) >= 70);
        assert_eq!(
            identity_score("Hello", "Bob", &titles, &["Alice".into()]),
            0
        );
        assert_eq!(
            identity_score(
                "Hello",
                "Alice",
                &["Hello (Live)".into()],
                &["Alice".into()]
            ),
            0
        );
    }

    #[test]
    fn external_identity_rejects_different_prefix_title_and_artist_substring() {
        assert_eq!(
            identity_score("Hello", "Alice", &["Hello Again".into()], &["Alice".into()]),
            0
        );
        assert_eq!(
            identity_score("Hello", "Alice", &["Hello".into()], &["Malice".into()]),
            0
        );
        assert_eq!(
            identity_score("爱你", "歌手", &["爱你 (现场版)".into()], &["歌手".into()]),
            0
        );
    }

    type Requests = std::sync::Arc<std::sync::Mutex<Vec<String>>>;
    fn fixture_server(bodies: Vec<String>) -> (String, Requests, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = requests.clone();
        let worker = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            for body in bodies {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(_) if std::time::Instant::now() < deadline => {
                            std::thread::sleep(Duration::from_millis(5))
                        }
                        Err(_) => return,
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut request = Vec::new();
                loop {
                    let mut buf = [0; 4096];
                    let n = stream.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&request);
                    if let Some((headers, body)) = text.split_once("\r\n\r\n") {
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|length| length.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if body.len() >= length {
                            break;
                        }
                    }
                    assert!(request.len() < 16_384);
                }
                captured
                    .lock()
                    .unwrap()
                    .push(String::from_utf8(request).unwrap());
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });
        (base, requests, worker)
    }

    fn fixture_client(base: &str) -> ExternalLyricsClient {
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut client = ExternalLyricsClient::new(FallbackHttp::new(&http, "lyrics-test"));
        client.amll_base = base.to_owned();
        client.kugou_song_endpoint = format!("{base}/song");
        client.kugou_base = base.to_owned();
        client
    }

    #[tokio::test]
    async fn external_kugou_http_tries_all_krc_before_lrc_and_uses_android_parameters() {
        for word_only in [true, false] {
            let good = if word_only {
                krc_response("[1000,199000]<0,199000,0>Hello")
            } else {
                serde_json::json!({"error_code":0,"content":STANDARD.encode("[00:01.00]Hello\n[03:20.00]End")}).to_string()
            };
            let mut bodies = vec!["[]".into(), r#"{"data":{"info":[{"hash":"fixture","songname":"Hello","singername":"Alice","duration":"200"}]}}"#.into(), r#"{"candidates":[{"id":"one","accesskey":"fixture-1","duration":200000,"score":100},{"id":"two","accesskey":"fixture-2","duration":200000,"score":90}]}"#.into(), "{}".into()];
            if !word_only {
                bodies.push("{}".into());
            }
            bodies.push(good);
            let (base, requests, worker) = fixture_server(bodies);
            let lines = fixture_client(&base)
                .fetch("Hello", "Alice", 200_000, word_only)
                .await;
            worker.join().unwrap();
            assert!(!lines.is_empty());
            assert_eq!(has_word_timing(&lines), word_only);
            let requests = requests.lock().unwrap();
            let song_url = reqwest::Url::parse(&format!(
                "{base}{}",
                requests[1].split_whitespace().nth(1).unwrap()
            ))
            .unwrap();
            assert!(song_url
                .query_pairs()
                .any(|(key, value)| key == "pagesize" && value == "8"));
            let search_url = reqwest::Url::parse(&format!(
                "{base}{}",
                requests[2].split_whitespace().nth(1).unwrap()
            ))
            .unwrap();
            assert!(search_url
                .query_pairs()
                .any(|(key, value)| key == "duration" && value == "200000"));
            assert!(search_url
                .query_pairs()
                .any(|(key, value)| key == "keyword" && value == "Alice - Hello"));
            let formats = requests
                .iter()
                .skip(3)
                .map(|request| {
                    let url = reqwest::Url::parse(&format!(
                        "{base}{}",
                        request.split_whitespace().nth(1).unwrap()
                    ))
                    .unwrap();
                    let format = url
                        .query_pairs()
                        .find(|(key, _)| key == "fmt")
                        .unwrap()
                        .1
                        .into_owned();
                    assert!(url.query_pairs().any(|(key, value)| key == "client"
                        && value == if format == "krc" { "mobi" } else { "pc" }));
                    format
                })
                .collect::<Vec<_>>();
            assert_eq!(
                formats,
                if word_only {
                    vec!["krc", "krc"]
                } else {
                    vec!["krc", "krc", "lrc"]
                }
            );
        }
    }

    #[tokio::test]
    async fn external_amll_skips_unsafe_wrong_identity_and_plain_candidates() {
        let (base, requests, worker) = fixture_server(vec![
            r#"[{"file":"../unsafe.ttml","title":"Hello","artist":"Alice"},{"file":"wrong.ttml","title":"Hello","artist":"Bob"},{"file":"plain.ttml","title":"Hello","artist":"Alice"},{"file":"timed.ttml","title":"Hello","artist":"Alice"}]"#.into(),
            r#"<tt><body><p begin="1s" end="200s">plain</p></body></tt>"#.into(),
            r#"<tt><body><p begin="1s" end="200s"><span begin="1s" end="200s">timed</span></p></body></tt>"#.into(),
        ]);
        let lines = fixture_client(&base)
            .fetch("Hello", "Alice", 200_000, true)
            .await;
        worker.join().unwrap();
        assert_eq!(lines[0].text, "timed");
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[1].starts_with("GET /raw-lyrics/plain.ttml "));
        assert!(requests[2].starts_with("GET /raw-lyrics/timed.ttml "));
        let query: Value =
            serde_json::from_str(requests[0].split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(query, serde_json::json!({"query":"Hello","type":"title"}));
    }

    #[test]
    fn external_duration_limits_follow_android_and_require_known_duration() {
        assert!(amll_duration_compatible(200_000, 170_000));
        assert!(!amll_duration_compatible(200_000, 169_999));
        assert!(amll_duration_compatible(200_000, 220_000));
        assert!(!amll_duration_compatible(200_000, 220_001));
        assert!(external_duration_compatible(200_000, 212_000));
        assert!(!external_duration_compatible(200_000, 212_001));
        assert!(!external_duration_compatible(0, 200_000));
        assert!(!amll_duration_compatible(200_000, 0));
    }

    #[test]
    fn external_rejects_amll_path_escape_and_accepts_filename() {
        assert!(safe_ttml_file("123456.ttml"));
        for file in [
            "../x.ttml",
            "x/y.ttml",
            "x\\y.ttml",
            "%2e%2e.ttml",
            "x:bad.ttml",
            "x.ttml?bad",
            "x.ttml\n",
        ] {
            assert!(!safe_ttml_file(file));
        }
    }

    #[test]
    fn external_krc_decodes_real_compression_word_offsets_and_translation_rows() {
        let language = STANDARD
            .encode(r#"{"content":[{"type":1,"lyricContent":[["纯文本译文"],["你好","世界"]]}]}"#);
        let body = krc_response(&format!("[language:{language}]\n[0,1000]plain without words\n[1000,2000]<0,500,0>Hello <500,1500,0>world"));
        let lines = decode_krc(&body).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Hello world");
        assert_eq!(lines[0].words[1].start_ms, 1500);
        assert_eq!(lines[0].translation.as_deref(), Some("你好 世界"));
    }

    #[test]
    fn external_krc_rejects_corrupt_base64_header_and_inflate_bomb() {
        assert!(decode_krc(r#"{"status":200,"content":"not base64!"}"#).is_none());
        assert!(decode_krc(
            &serde_json::json!({"status":200,"content":STANDARD.encode(b"nopebad")}).to_string()
        )
        .is_none());
        assert!(decode_krc(&krc_response(&"x".repeat(2 * 1024 * 1024 + 1))).is_none());
    }

    #[tokio::test]
    async fn external_amll_uses_search_post_then_safe_raw_and_real_ttml() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = requests.clone();
        let worker = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            for body in [
                r#"[{"file":"fixture.ttml","title":"Hello","artist":"Alice"}]"#,
                r#"<tt><body><p begin="1s" end="200s"><span begin="1s" end="200s">Hello</span></p></body></tt>"#,
            ] {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(_) if std::time::Instant::now() < deadline => {
                            std::thread::sleep(std::time::Duration::from_millis(5))
                        }
                        Err(_) => return,
                    }
                };
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                    .unwrap();
                let mut buf = [0; 8192];
                let n = stream.read(&mut buf).unwrap();
                captured
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..n]).into_owned());
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut client = ExternalLyricsClient::new(FallbackHttp::new(&http, "lyrics-test"));
        client.amll_base = base;
        let lines = client.fetch("Hello", "Alice", 200_000, true).await;
        worker.join().unwrap();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].words[0].duration_ms > 0);
        let requests = requests.lock().unwrap();
        assert!(requests[0].starts_with("POST /api/search-lyrics "));
        assert!(requests[1].starts_with("GET /raw-lyrics/fixture.ttml "));
    }
}
