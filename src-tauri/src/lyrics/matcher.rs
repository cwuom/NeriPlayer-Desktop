//! 编辑器「匹配歌词」：按所选平台搜索候选歌词，统一打分排序后交给用户挑选
//!
//! 打分、置信度和排序逐条对齐 Android EditableLyricsMatcher / EditableLyricMatchPolicy /
//! SearchTextMatcher（关键字模糊匹配不含拼音候选）。与自动取歌词不同，这里把所有像样的
//! 候选都列出来；每个候选带完整歌词行，前端可以先预览再应用。各平台并发搜索。

use super::external::{self, AmllEntry, ExternalLyricsClient, KugouSong};
use super::parser::{self, LyricLine};
use super::LyricSource;
use crate::api::lrclib::{LrcLibClient, LrcLibResult};
use crate::api::netease::client::{NeteaseClient, NeteaseLyrics, NeteaseSearchResult};
use crate::api::qq::client::{QqMusicClient, QqSearchResult};
use crate::api::transport::FallbackHttp;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::cmp::{Ordering, Reverse};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

const MAX_SOURCE_RESULTS: usize = 5;
const MAX_DETAIL_RESULTS: usize = 4;
const MAX_RESULTS: usize = 20;
/// 单个平台的总预算：慢的平台不拖住已经返回的结果太久
const SOURCE_BUDGET: Duration = Duration::from_secs(15);

const MIN_MATCH_SCORE: i32 = 35;
const WORD_TIMED_BONUS: i32 = 36;
const MIN_RELIABLE_TITLE_SCORE: i32 = 52;
const MIN_RELIABLE_ARTIST_SCORE: i32 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchSource {
    Kugou,
    Netease,
    Qq,
    AmllTtml,
    Lrclib,
}

impl MatchSource {
    /// Android 逐个平台取的顺序
    const SEARCH_ORDER: [MatchSource; 5] = [Self::Kugou, Self::Netease, Self::Qq, Self::Lrclib, Self::AmllTtml];

    fn priority(self) -> i32 {
        match self {
            Self::Kugou => 5,
            Self::Netease => 4,
            Self::Qq => 3,
            Self::Lrclib => 2,
            Self::AmllTtml => 1,
        }
    }

    /// Android 枚举声明顺序，排序的最后兜底
    fn ordinal(self) -> i32 {
        match self {
            Self::Kugou => 0,
            Self::Netease => 1,
            Self::Qq => 2,
            Self::AmllTtml => 3,
            Self::Lrclib => 4,
        }
    }

    /// 国内平台的搜索词先繁转简
    fn domestic(self) -> bool {
        matches!(self, Self::Kugou | Self::Netease | Self::Qq)
    }

    fn quality(self) -> i32 {
        match self {
            Self::Kugou | Self::Netease | Self::Qq => 4,
            Self::Lrclib => 2,
            Self::AmllTtml => 1,
        }
    }

    pub fn lyric_source(self) -> LyricSource {
        match self {
            Self::Kugou => LyricSource::Kugou,
            Self::Netease => LyricSource::Netease,
            Self::Qq => LyricSource::Qq,
            Self::AmllTtml => LyricSource::AmllTtml,
            Self::Lrclib => LyricSource::Lrclib,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LyricFormat {
    Lrc,
    Yrc,
    Ttml,
    Plain,
}

impl LyricFormat {
    fn quality(self) -> i32 {
        match self {
            Self::Ttml => 16,
            Self::Yrc => 14,
            Self::Lrc => 10,
            Self::Plain => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchConfidence {
    Low,
    Medium,
    High,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchRequest {
    /// 用户可改的搜索关键字，默认「歌名 歌手」
    pub keyword: String,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default = "default_true")]
    pub prefer_word_timed: bool,
    pub sources: Vec<MatchSource>,
}

#[derive(Debug, Clone)]
pub struct MatchCandidate {
    pub id: String,
    pub source: MatchSource,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub lines: Vec<LyricLine>,
    pub format: LyricFormat,
    /// 平台自身给的可信度，0–20
    pub source_score: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankedMatch {
    pub id: String,
    pub source: MatchSource,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub format: LyricFormat,
    pub score: i32,
    pub duration_delta_ms: Option<u64>,
    pub confidence: MatchConfidence,
    pub word_timed: bool,
    pub has_translation: bool,
    pub has_romanization: bool,
    pub lines: Vec<LyricLine>,
}

pub struct LyricMatcher {
    transport: FallbackHttp,
    /// 网易云搜索要带登录态和 cookie 存储，未登录的匿名请求直接被拒
    netease: NeteaseClient,
}

impl LyricMatcher {
    pub fn new(transport: FallbackHttp, netease: NeteaseClient) -> Self {
        Self { transport, netease }
    }

    pub async fn find(&self, request: MatchRequest) -> Vec<RankedMatch> {
        let request = MatchRequest {
            keyword: request.keyword.trim().to_owned(),
            title: request.title.trim().to_owned(),
            artist: request.artist.trim().to_owned(),
            album: request.album.trim().to_owned(),
            ..request
        };
        if request.keyword.is_empty() || request.sources.is_empty() {
            return Vec::new();
        }
        let searches = MatchSource::SEARCH_ORDER
            .into_iter()
            .filter(|source| request.sources.contains(source))
            .map(|source| {
                let request = &request;
                async move {
                    let started = Instant::now();
                    let found = match tokio::time::timeout(SOURCE_BUDGET, self.search(source, request)).await {
                        Ok(found) => found,
                        Err(_) => {
                            log::warn!(target: "lyric-match", "source={source:?} timed out after {}s", SOURCE_BUDGET.as_secs());
                            Vec::new()
                        }
                    };
                    log::info!(
                        target: "lyric-match",
                        "source={source:?} candidates={} elapsed_ms={}",
                        found.len(),
                        started.elapsed().as_millis(),
                    );
                    found
                }
            });
        let candidates = futures_util::future::join_all(searches).await.into_iter().flatten().collect();
        rank(&request, candidates)
    }

    async fn search(&self, source: MatchSource, request: &MatchRequest) -> Vec<MatchCandidate> {
        let queries = search_queries(request, source.domestic());
        match source {
            MatchSource::Netease => self.search_netease(request, &queries).await,
            MatchSource::Qq => self.search_qq(request, &queries).await,
            MatchSource::Kugou => self.search_kugou(request, &queries).await,
            MatchSource::AmllTtml => self.search_amll(request, &queries).await,
            MatchSource::Lrclib => self.search_lrclib(request, &queries).await,
        }
    }

    async fn search_netease(&self, request: &MatchRequest, queries: &[String]) -> Vec<MatchCandidate> {
        let client = &self.netease;
        let hits = collect(queries, |song: &NeteaseSearchResult| song.id.to_string(), |query| {
            async move { log_failure("netease search", client.search(&query, MAX_SOURCE_RESULTS as u32, 0).await) }
        })
        .await;
        let mut found = Vec::new();
        for song in for_detail(request, hits, |song| (song.name.clone(), song.artists.join("/"), song.duration_ms)) {
            let Some(lyrics) = log_failure("netease lyric", client.get_lyrics(song.id).await) else { continue };
            let Some((lines, format)) = netease_lines(&lyrics) else { continue };
            found.push(MatchCandidate {
                id: song.id.to_string(),
                source: MatchSource::Netease,
                artist: song.artists.join("/"),
                title: song.name,
                album: song.album,
                duration_ms: song.duration_ms,
                lines,
                format,
                source_score: 4,
            });
        }
        found
    }

    async fn search_qq(&self, request: &MatchRequest, queries: &[String]) -> Vec<MatchCandidate> {
        let client = QqMusicClient::with_transport(self.transport.clone());
        let hits = collect(queries, |song: &QqSearchResult| song.song_mid.clone(), |query| {
            let client = &client;
            async move { log_failure("qq search", client.search(&query, 1, MAX_SOURCE_RESULTS as u32).await) }
        })
        .await;
        let mut found = Vec::new();
        for song in for_detail(request, hits, |song| (song.song_name.clone(), song.artists.join("/"), song.duration_ms)) {
            let Some((Some(lrc), translation)) = log_failure("qq lyric", client.get_lyrics(&song.song_mid).await) else {
                continue;
            };
            let mut lines = parser::parse_lrc(&lrc);
            if lines.is_empty() {
                continue;
            }
            if let Some(translation) = translation.as_deref().filter(|value| !value.trim().is_empty()) {
                parser::merge_translation(&mut lines, translation);
            }
            found.push(MatchCandidate {
                id: song.song_mid,
                source: MatchSource::Qq,
                artist: song.artists.join("/"),
                title: song.song_name,
                album: song.album_name,
                duration_ms: song.duration_ms,
                lines,
                format: LyricFormat::Lrc,
                source_score: 3,
            });
        }
        found
    }

    async fn search_kugou(&self, request: &MatchRequest, queries: &[String]) -> Vec<MatchCandidate> {
        let client = ExternalLyricsClient::new(self.transport.clone());
        let hits = collect(queries, |song: &KugouSong| song.hash.clone(), |query| {
            let client = &client;
            async move { client.kugou_songs(&query).await }
        })
        .await;
        let mut found = Vec::new();
        for song in for_detail(request, hits, |song| (song.title.clone(), song.artist.clone(), song.duration)) {
            let Some(lines) = client.kugou_lyrics(&song).await else { continue };
            let format = if external::has_word_timing(&lines) { LyricFormat::Yrc } else { LyricFormat::Lrc };
            found.push(MatchCandidate {
                id: song.hash,
                source: MatchSource::Kugou,
                title: song.title,
                artist: song.artist,
                album: song.album,
                duration_ms: song.duration,
                lines,
                format,
                source_score: 4,
            });
        }
        found
    }

    async fn search_amll(&self, request: &MatchRequest, queries: &[String]) -> Vec<MatchCandidate> {
        let client = ExternalLyricsClient::new(self.transport.clone());
        let mut hits = collect(queries, |entry: &AmllEntry| entry.file.clone(), |query| {
            let client = &client;
            async move { client.amll_search(&query).await }
        })
        .await;
        hits.sort_by_cached_key(|entry| {
            Reverse((external::identity_score(&request.title, &request.artist, &entry.titles, &entry.artists), entry.score))
        });
        let mut found = Vec::new();
        for entry in hits.into_iter().take(MAX_DETAIL_RESULTS) {
            let Some(lines) = client.amll_lyrics(&entry.file).await else { continue };
            found.push(MatchCandidate {
                title: entry.titles.first().cloned().unwrap_or_default(),
                artist: entry.artists.join("/"),
                album: entry.albums.first().cloned().unwrap_or_default(),
                duration_ms: external::lyric_end(&lines),
                source_score: (entry.score / 20).min(10) as i32,
                id: entry.file,
                source: MatchSource::AmllTtml,
                lines,
                format: LyricFormat::Ttml,
            });
        }
        found
    }

    async fn search_lrclib(&self, request: &MatchRequest, queries: &[String]) -> Vec<MatchCandidate> {
        let client = LrcLibClient::with_transport(self.transport.clone());
        let hits = collect(
            queries,
            |result: &LrcLibResult| format!("{}:{}:{}", result.track_name, result.artist_name, result.duration.round()),
            |query| {
                let client = &client;
                async move { log_failure("lrclib search", client.search(&query).await) }
            },
        )
        .await;
        hits.into_iter()
            .take(MAX_SOURCE_RESULTS)
            .filter_map(|result| {
                let duration_ms = (result.duration * 1000.0).round().max(0.0) as u64;
                let (lines, format, source_score) = match (result.synced_lyrics.as_deref(), result.plain_lyrics.as_deref()) {
                    (Some(synced), _) if !synced.trim().is_empty() => (parser::parse_lrc(synced), LyricFormat::Lrc, 4),
                    (_, Some(plain)) if !plain.trim().is_empty() => {
                        let span = if duration_ms > 0 { duration_ms } else { request.duration_ms };
                        (super::manager::plain_text_to_lines(plain, span), LyricFormat::Plain, 1)
                    }
                    _ => return None,
                };
                (!lines.is_empty()).then(|| MatchCandidate {
                    id: format!("{}:{}:{}", result.track_name, result.artist_name, result.duration.round()),
                    source: MatchSource::Lrclib,
                    title: result.track_name,
                    artist: result.artist_name,
                    album: String::new(),
                    duration_ms,
                    lines,
                    format,
                    source_score,
                })
            })
            .collect()
    }
}

fn log_failure<T>(what: &str, result: crate::error::AppResult<T>) -> Option<T> {
    result.map_err(|error| log::info!(target: "lyric-match", "{what} failed: {error}")).ok()
}

/// 网易云：有逐字 YRC 用 YRC，否则 LRC；翻译、音译并到行上
fn netease_lines(lyrics: &NeteaseLyrics) -> Option<(Vec<LyricLine>, LyricFormat)> {
    let nonblank = |value: &Option<String>| value.as_deref().filter(|value| !value.trim().is_empty()).map(str::to_owned);
    let (mut lines, format) = match nonblank(&lyrics.yrc).map(|yrc| parser::parse_yrc(&yrc)).filter(|lines| !lines.is_empty()) {
        Some(lines) => (lines, LyricFormat::Yrc),
        None => (parser::parse_auto(&nonblank(&lyrics.lrc)?), LyricFormat::Lrc),
    };
    if lines.is_empty() {
        return None;
    }
    if let Some(translation) = nonblank(&lyrics.ytlrc).or_else(|| nonblank(&lyrics.tlyric)) {
        parser::merge_translation(&mut lines, &translation);
    }
    if let Some(roman) = nonblank(&lyrics.romalrc) {
        parser::merge_roman(&mut lines, &roman);
    }
    Some((lines, format))
}

/// 搜索词：关键字 →「歌名 歌手」→ 歌名，按规整后去重；国内平台先繁转简
fn search_queries(request: &MatchRequest, domestic: bool) -> Vec<String> {
    let metadata = [request.title.trim(), request.artist.trim()]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut seen = HashSet::new();
    [request.keyword.as_str(), metadata.as_str(), request.title.as_str()]
        .into_iter()
        .map(|query| if domestic { fast2s::convert(query.trim()) } else { query.trim().to_owned() })
        .filter(|query| !query.is_empty() && seen.insert(normalize_text(query)))
        .collect()
}

/// 每个搜索词取前 5 条，按 key 去重保序
async fn collect<T, Search, Fut>(queries: &[String], key: impl Fn(&T) -> String, search: Search) -> Vec<T>
where
    Search: Fn(String) -> Fut,
    Fut: Future<Output = Option<Vec<T>>>,
{
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for query in queries {
        let Some(items) = search(query.clone()).await else { continue };
        for item in items.into_iter().take(MAX_SOURCE_RESULTS) {
            if seen.insert(key(&item)) {
                found.push(item);
            }
        }
    }
    found
}

/// 取详情前的初筛：时长明显不对的不取，其余按「时长相符、标题、歌手、时长」排前 4 个
fn for_detail<T>(request: &MatchRequest, items: Vec<T>, identity: impl Fn(&T) -> (String, String, u64)) -> Vec<T> {
    let mut items = items
        .into_iter()
        .filter(|item| {
            let duration = identity(item).2;
            request.duration_ms == 0 || duration == 0 || duration_compatible(request.duration_ms, duration)
        })
        .collect::<Vec<_>>();
    items.sort_by_cached_key(|item| {
        let (title, artist, duration) = identity(item);
        Reverse((
            request.duration_ms > 0 && duration > 0 && duration_compatible(request.duration_ms, duration),
            score_title(&request.title, &title),
            score_artist(&request.artist, &artist),
            score_duration(request.duration_ms, duration),
        ))
    });
    items.truncate(MAX_DETAIL_RESULTS);
    items
}

// ---- 打分与排序（对齐 EditableLyricMatchPolicy） ----

pub fn rank(request: &MatchRequest, candidates: Vec<MatchCandidate>) -> Vec<RankedMatch> {
    let usable = candidates
        .into_iter()
        .map(|mut candidate| {
            let lines = std::mem::take(&mut candidate.lines);
            candidate.lines = super::sanitize::sanitize_matched_lines(
                lines, &candidate.title, &candidate.artist, &candidate.album,
            );
            candidate
        })
        .filter(|candidate| candidate.lines.iter().any(|line| !line.text.trim().is_empty()))
        .filter(|candidate| !has_collapsed_timeline(&candidate.lines))
        .collect::<Vec<_>>();
    let mut ranked = Vec::new();
    let mut keys = HashSet::new();
    for candidate in &usable {
        if let Some(result) = score_candidate(request, candidate) {
            keys.insert(identity_key(candidate));
            ranked.push(result);
        }
    }
    // 没过身份门槛但标题/关键字和歌手对得上的，作为低置信度结果列在后面，由用户核对
    for candidate in &usable {
        if keys.contains(&identity_key(candidate)) || !has_match_signal(request, candidate) {
            continue;
        }
        let score = score_title(&request.title, &candidate.title)
            + score_artist(&request.artist, &candidate.artist)
            + score_duration(request.duration_ms, candidate.duration_ms)
            + score_keyword(&request.keyword, candidate)
            + candidate.source_score.clamp(0, 20);
        ranked.push(ranked_match(request, candidate, score, MatchConfidence::Low));
    }
    let mut seen = HashSet::new();
    ranked.retain(|result| {
        seen.insert(format!(
            "{:?}:{}:{}:{}",
            result.source,
            normalize_text(&result.title),
            normalize_text(&result.artist),
            lines_hash(&result.lines)
        ))
    });
    ranked.sort_by(|left, right| compare_results(left, right, request.prefer_word_timed));
    ranked.truncate(MAX_RESULTS);
    ranked
}

fn identity_key(candidate: &MatchCandidate) -> String {
    format!(
        "{:?}:{}:{}:{}",
        candidate.source,
        normalize_text(&candidate.title),
        normalize_text(&candidate.artist),
        lines_hash(&candidate.lines)
    )
}

fn lines_hash(lines: &[LyricLine]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for line in lines {
        line.start_ms.hash(&mut hasher);
        line.text.hash(&mut hasher);
    }
    hasher.finish()
}

fn score_candidate(request: &MatchRequest, candidate: &MatchCandidate) -> Option<RankedMatch> {
    let title_score = score_title(&request.title, &candidate.title);
    let artist_score = score_artist(&request.artist, &candidate.artist);
    let album_score = score_album(&request.album, &candidate.album);
    let duration_score = score_duration(request.duration_ms, candidate.duration_ms);
    let quality_score = score_quality(candidate);
    let word_timing_score =
        if request.prefer_word_timed && external::has_word_timing(&candidate.lines) { WORD_TIMED_BONUS } else { 0 };
    let keyword_score = score_keyword(&request.keyword, candidate);
    let keyword_fallback = is_placeholder(&request.title) || is_placeholder(&request.artist);
    let primary_artist = has_primary_artist(&request.artist, &candidate.artist);
    let duration_signal = request.duration_ms == 0
        || candidate.duration_ms == 0
        || duration_compatible(request.duration_ms, candidate.duration_ms);
    let reliable = is_reliable_identity(&request.title, &request.artist, &candidate.title, &candidate.artist);
    let plausible = is_plausible_identity(
        &request.title,
        &request.artist,
        &candidate.title,
        &candidate.artist,
        duration_signal,
    ) || (keyword_fallback && (keyword_score > 0 || title_score >= 20));
    if !plausible {
        return None;
    }
    let score = title_score
        + artist_score
        + album_score
        + duration_score
        + quality_score
        + word_timing_score
        + keyword_score
        + candidate.source_score.clamp(0, 20);
    if score < MIN_MATCH_SCORE {
        return None;
    }
    let confidence = if reliable && duration_signal {
        MatchConfidence::High
    } else if (primary_artist && duration_signal && title_score >= 20)
        || (title_score >= MIN_RELIABLE_TITLE_SCORE && duration_signal)
    {
        MatchConfidence::Medium
    } else {
        MatchConfidence::Low
    };
    Some(ranked_match(request, candidate, score, confidence))
}

fn ranked_match(request: &MatchRequest, candidate: &MatchCandidate, score: i32, confidence: MatchConfidence) -> RankedMatch {
    RankedMatch {
        id: candidate.id.clone(),
        source: candidate.source,
        title: candidate.title.clone(),
        artist: candidate.artist.clone(),
        album: candidate.album.clone(),
        duration_ms: candidate.duration_ms,
        format: candidate.format,
        score,
        duration_delta_ms: (request.duration_ms > 0 && candidate.duration_ms > 0)
            .then(|| request.duration_ms.abs_diff(candidate.duration_ms)),
        confidence,
        word_timed: external::has_word_timing(&candidate.lines),
        has_translation: candidate.lines.iter().any(|line| line.translation.as_deref().is_some_and(|value| !value.trim().is_empty())),
        has_romanization: candidate.lines.iter().any(|line| line.roman.as_deref().is_some_and(|value| !value.trim().is_empty())),
        lines: candidate.lines.clone(),
    }
}

/// 偏好逐字时逐字结果整体提一档，再按置信度、分数、时长差、平台排
fn compare_results(left: &RankedMatch, right: &RankedMatch, prefer_word_timed: bool) -> Ordering {
    let word_tier = if prefer_word_timed { right.word_timed.cmp(&left.word_timed) } else { Ordering::Equal };
    word_tier
        .then(right.confidence.cmp(&left.confidence))
        .then(right.score.cmp(&left.score))
        .then(
            left.duration_delta_ms
                .unwrap_or(u64::MAX)
                .cmp(&right.duration_delta_ms.unwrap_or(u64::MAX)),
        )
        .then(right.source.priority().cmp(&left.source.priority()))
        .then(left.source.ordinal().cmp(&right.source.ordinal()))
        .then_with(|| normalize_text(&left.title).cmp(&normalize_text(&right.title)))
}

fn has_match_signal(request: &MatchRequest, candidate: &MatchCandidate) -> bool {
    let title_score = score_title(&request.title, &candidate.title);
    let artist_score = score_artist(&request.artist, &candidate.artist);
    let keyword_score = score_keyword(&request.keyword, candidate);
    (title_score >= 20 || keyword_score > 0)
        && (artist_score >= MIN_RELIABLE_ARTIST_SCORE || candidate.artist.trim().is_empty())
}

/// 至少 3 行、所有行时间戳都一样：时间轴塌缩，不能用来同步
fn has_collapsed_timeline(lines: &[LyricLine]) -> bool {
    let timed = lines.iter().filter(|line| !line.text.trim().is_empty()).map(|line| line.start_ms).collect::<Vec<_>>();
    timed.len() >= 3 && timed.iter().collect::<HashSet<_>>().len() < 2
}

pub fn duration_compatible(expected: u64, candidate: u64) -> bool {
    if expected == 0 || candidate == 0 {
        return false;
    }
    let tolerance = 7_000.max(expected * 6 / 100).min(15_000);
    expected.abs_diff(candidate) <= tolerance
}

pub fn score_title(expected: &str, candidate: &str) -> i32 {
    let expected = normalize_text(expected);
    let candidate = normalize_text(candidate);
    if expected.is_empty() || candidate.is_empty() {
        return 0;
    }
    if candidate == expected {
        80
    } else if candidate.starts_with(&format!("{expected} ")) {
        68
    } else if expected.starts_with(&format!("{candidate} ")) {
        62
    } else if candidate.contains(&expected) || expected.contains(&candidate) {
        52
    } else {
        (token_overlap(&expected, &candidate) * 44.0).round() as i32
    }
}

pub fn score_artist(expected: &str, candidate: &str) -> i32 {
    let expected_artists = split_artists(expected);
    let candidate_artists = split_artists(candidate);
    if expected_artists.is_empty() || candidate_artists.is_empty() {
        return 0;
    }
    if expected_artists == candidate_artists {
        return 55;
    }
    if expected_artists.is_subset(&candidate_artists) {
        return 46;
    }
    let shared = expected_artists.intersection(&candidate_artists).count() as i32;
    if shared > 0 {
        return 28 + 18 * shared / (expected_artists.len() as i32).max(1);
    }
    expected_artists
        .iter()
        .flat_map(|expected| {
            candidate_artists.iter().map(move |candidate| {
                if expected == candidate || candidate.starts_with(&format!("{expected} ")) {
                    24
                } else {
                    (token_overlap(expected, candidate) * 20.0).round() as i32
                }
            })
        })
        .max()
        .unwrap_or(0)
}

fn score_album(expected: &str, candidate: &str) -> i32 {
    let expected = normalize_text(expected);
    let candidate = normalize_text(candidate);
    if expected.is_empty() || candidate.is_empty() {
        0
    } else if candidate == expected {
        8
    } else if candidate.contains(&expected) || expected.contains(&candidate) {
        4
    } else {
        0
    }
}

pub fn score_duration(expected: u64, candidate: u64) -> i32 {
    if expected == 0 || candidate == 0 {
        return 0;
    }
    let delta = expected.abs_diff(candidate);
    if duration_compatible(expected, candidate) {
        (42 - (delta / 500) as i32).max(22)
    } else {
        -((delta / 3_000).min(48) as i32)
    }
}

fn score_quality(candidate: &MatchCandidate) -> i32 {
    let translation = candidate
        .lines
        .iter()
        .any(|line| line.translation.as_deref().is_some_and(|value| !value.trim().is_empty()));
    candidate.format.quality() + if translation { 5 } else { 0 } + candidate.source.quality()
}

fn score_keyword(keyword: &str, candidate: &MatchCandidate) -> i32 {
    let query = fast2s::convert(keyword.trim());
    if query.trim().is_empty() {
        return 0;
    }
    let values = [&candidate.title, &candidate.artist, &candidate.album].map(|value| fast2s::convert(value));
    match fuzzy_score(&query, &values) {
        Some(fuzzy) => (48 - fuzzy / 2).clamp(12, 48),
        None => 0,
    }
}

fn is_placeholder(value: &str) -> bool {
    matches!(
        normalize_text(value).as_str(),
        "unknown" | "unknown artist" | "unknown song" | "unknown title" | "未知" | "未知歌手" | "未知歌曲" | "未知标题"
    )
}

pub fn is_reliable_identity(expected_title: &str, expected_artist: &str, candidate_title: &str, candidate_artist: &str) -> bool {
    if [expected_title, expected_artist, candidate_title, candidate_artist].iter().any(|value| value.trim().is_empty()) {
        return false;
    }
    if version_signature(expected_title) != version_signature(candidate_title) {
        return false;
    }
    let Some(primary) = artist_segments(expected_artist).into_iter().next() else { return false };
    let primary_matches = split_artists(candidate_artist)
        .iter()
        .any(|name| *name == primary || has_collaborator_suffix(name, &primary));
    primary_matches
        && canonical_title(expected_title) == canonical_title(candidate_title)
        && score_title(expected_title, candidate_title) >= MIN_RELIABLE_TITLE_SCORE
        && score_artist(expected_artist, candidate_artist) >= MIN_RELIABLE_ARTIST_SCORE
}

fn is_plausible_identity(
    expected_title: &str,
    expected_artist: &str,
    candidate_title: &str,
    candidate_artist: &str,
    duration_compatible: bool,
) -> bool {
    if is_reliable_identity(expected_title, expected_artist, candidate_title, candidate_artist) {
        return true;
    }
    let title_score = score_title(expected_title, candidate_title);
    let artist_score = score_artist(expected_artist, candidate_artist);
    (has_primary_artist(expected_artist, candidate_artist) && title_score >= 20 && duration_compatible)
        || (title_score >= MIN_RELIABLE_TITLE_SCORE && duration_compatible && candidate_artist.trim().is_empty())
        || (title_score >= 20 && artist_score >= MIN_RELIABLE_ARTIST_SCORE && duration_compatible)
}

fn has_primary_artist(expected: &str, candidate: &str) -> bool {
    let Some(primary) = artist_segments(expected).into_iter().next() else { return false };
    split_artists(candidate).contains(&primary)
}

/// 「主歌手 and/with/x 嘉宾」算同一主歌手，「主歌手 Tribute」不算
fn has_collaborator_suffix(candidate: &str, primary: &str) -> bool {
    let Some(rest) = candidate.strip_prefix(&format!("{primary} ")) else { return false };
    let connective = rest.trim_start().split(' ').next().unwrap_or_default();
    matches!(connective, "and" | "with" | "x" | "vs" | "versus" | "和" | "与")
}

fn split_artists(value: &str) -> HashSet<String> {
    std::iter::once(normalize_text(value))
        .chain(artist_segments(value))
        .filter(|value| !value.is_empty())
        .collect()
}

fn artist_segments(value: &str) -> Vec<String> {
    static FEATURED: OnceLock<Regex> = OnceLock::new();
    static HARD: OnceLock<Regex> = OnceLock::new();
    let featured = FEATURED.get_or_init(|| Regex::new(r"(?i)\b(?:feat\.?|ft\.?|featuring)\b").unwrap());
    let hard = HARD.get_or_init(|| Regex::new(r"[/,，、&+]|\s+[xX]\s+").unwrap());
    featured
        .split(value)
        .flat_map(|segment| hard.split(segment))
        .map(normalize_text)
        .filter(|value| !value.is_empty())
        .collect()
}

fn version_signature(value: &str) -> HashSet<String> {
    version_modifier()
        .find_iter(&normalize_text(value))
        .map(|found| if found.as_str() == "remastered" { "remaster".to_owned() } else { found.as_str().to_owned() })
        .collect()
}

fn version_modifier() -> &'static Regex {
    static VERSION: OnceLock<Regex> = OnceLock::new();
    VERSION.get_or_init(|| {
        Regex::new(r"\b(?:remaster(?:ed)?|remix|live|acoustic|instrumental|karaoke|demo|cover|rework|slowed|sped\s+up|version|edit|extended|radio|clean|explicit)\b").unwrap()
    })
}

fn canonical_title(value: &str) -> String {
    static DESCRIPTOR: OnceLock<Regex> = OnceLock::new();
    static SPACES: OnceLock<Regex> = OnceLock::new();
    let words = "official|audio|video|lyrics?|visualizer|hd|hq|4k|mv|官方|官方版|官方视频|音频|歌词|歌词版|高清|完整版";
    let descriptor = DESCRIPTOR.get_or_init(|| Regex::new(&format!(r"(?:\s+|^)(?:{words})(?:\s+(?:{words}))*$")).unwrap());
    let without_descriptor = descriptor.replace(&normalize_text(value), " ").into_owned();
    let without_version = version_modifier().replace_all(&without_descriptor, " ");
    SPACES.get_or_init(|| Regex::new(r"\s+").unwrap()).replace_all(&without_version, " ").trim().to_owned()
}

fn token_overlap(left: &str, right: &str) -> f64 {
    let left = left.split(' ').filter(|token| !token.is_empty()).collect::<HashSet<_>>();
    let right = right.split(' ').filter(|token| !token.is_empty()).collect::<HashSet<_>>();
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    left.intersection(&right).count() as f64 / left.len().max(right.len()) as f64
}

/// 与 Android normalizeLyricMatchText 一致：繁转简、NFKC、小写、去 feat 和括号、非字母数字变空格
pub fn normalize_text(value: &str) -> String {
    static FEAT: OnceLock<Regex> = OnceLock::new();
    static BRACKETS: OnceLock<Regex> = OnceLock::new();
    static NON_WORD: OnceLock<Regex> = OnceLock::new();
    static SPACES: OnceLock<Regex> = OnceLock::new();
    let text = fast2s::convert(value).nfkc().collect::<String>().to_lowercase().replace('&', " and ");
    let text = FEAT.get_or_init(|| Regex::new(r"\b(?:feat|ft|featuring)\.?\b").unwrap()).replace_all(&text, " ");
    let text = BRACKETS.get_or_init(|| Regex::new(r"[(){}\[\]【】（）]").unwrap()).replace_all(&text, " ");
    let text = NON_WORD.get_or_init(|| Regex::new(r"[^\p{L}\p{N}]+").unwrap()).replace_all(&text, " ");
    SPACES.get_or_init(|| Regex::new(r"\s+").unwrap()).replace_all(text.trim(), " ").into_owned()
}

// ---- 关键字模糊匹配（对齐 Android SearchTextMatcher，不含拼音候选） ----

const WHOLE_TEXT_BIAS: i32 = 0;
const SPLIT_TOKEN_BIAS: i32 = 2;
const COMPACT_TOKEN_BIAS: i32 = 4;
const ACRONYM_BIAS: i32 = 10;
const PREFIX_MATCH_SCORE: i32 = 16;
const CONTAINS_MATCH_SCORE: i32 = 48;
const FUZZY_MATCH_SCORE: i32 = 96;

/// 每个查询词取最佳候选的分数之和，越小越好；有词完全对不上时为 None
fn fuzzy_score(query: &str, values: &[String]) -> Option<i32> {
    let tokens = search_tokens(&fold_search_text(query, true));
    if tokens.is_empty() {
        return Some(0);
    }
    let mut best_bias = HashMap::<String, i32>::new();
    for value in values {
        for (text, bias) in candidate_tokens(value) {
            best_bias.entry(text).and_modify(|current| *current = (*current).min(bias)).or_insert(bias);
        }
    }
    if best_bias.is_empty() {
        return None;
    }
    tokens.iter().try_fold(0, |total, token| {
        best_bias
            .iter()
            .filter_map(|(text, bias)| match_score(token, text, *bias))
            .min()
            .map(|best| total + best)
    })
}

fn fold_search_text(value: &str, lowercase: bool) -> String {
    value
        .trim()
        .nfkd()
        .filter(|character| !is_combining_mark(*character))
        .map(|character| if character == '\u{3000}' { ' ' } else { character })
        .flat_map(|character| {
            let lowered: Vec<char> = if lowercase { character.to_lowercase().collect() } else { vec![character] };
            lowered
        })
        .collect()
}

fn search_tokens(value: &str) -> Vec<String> {
    static SEPARATOR: OnceLock<Regex> = OnceLock::new();
    SEPARATOR
        .get_or_init(|| Regex::new(r"[^\p{L}\p{Nd}]+").unwrap())
        .split(value)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

fn split_camel(value: &str) -> Vec<String> {
    let characters = value.chars().collect::<Vec<_>>();
    let mut parts = Vec::new();
    let mut start = 0;
    for index in 1..characters.len() {
        if characters[index - 1].is_lowercase() && characters[index].is_uppercase() {
            parts.push(characters[start..index].iter().collect::<String>().to_lowercase());
            start = index;
        }
    }
    parts.push(characters[start..].iter().collect::<String>().to_lowercase());
    parts
}

fn candidate_tokens(value: &str) -> Vec<(String, i32)> {
    let normalized = fold_search_text(value, true);
    if normalized.trim().is_empty() {
        return Vec::new();
    }
    let split = search_tokens(&fold_search_text(value, false))
        .iter()
        .flat_map(|token| split_camel(token))
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let compact = split.concat();
    let acronym = split.iter().filter_map(|token| token.chars().next()).collect::<String>();
    let mut tokens = vec![(normalized, WHOLE_TEXT_BIAS)];
    tokens.extend(split.into_iter().map(|token| (token, SPLIT_TOKEN_BIAS)));
    if !compact.is_empty() {
        tokens.push((compact, COMPACT_TOKEN_BIAS));
    }
    if acronym.chars().count() > 1 {
        tokens.push((acronym, ACRONYM_BIAS));
    }
    tokens
}

fn match_score(query: &str, text: &str, bias: i32) -> Option<i32> {
    let query_length = query.chars().count() as i32;
    let text_length = text.chars().count() as i32;
    if query == text {
        Some(bias)
    } else if text.starts_with(query) {
        Some(PREFIX_MATCH_SCORE + bias + (text_length - query_length))
    } else if let Some(byte_index) = text.find(query) {
        Some(CONTAINS_MATCH_SCORE + bias + text[..byte_index].chars().count() as i32 * 2)
    } else if query_length > 1 {
        let gap = subsequence_gap(text, query)?;
        allows_fuzzy(query, text, gap).then(|| FUZZY_MATCH_SCORE + bias + gap * 4 + (text_length - query_length))
    } else {
        None
    }
}

fn allows_fuzzy(query: &str, text: &str, gap: i32) -> bool {
    let ascii_token = |value: &str| !value.is_empty() && value.chars().all(|character| character.is_ascii_lowercase() || character.is_ascii_digit());
    if !ascii_token(query) || !ascii_token(text) {
        return true;
    }
    if query.chars().next() != text.chars().next() {
        return false;
    }
    gap <= (query.chars().count() as i32).max(1)
}

/// query 作为子序列出现在 text 中时，首尾之间多出来的字符数
fn subsequence_gap(text: &str, query: &str) -> Option<i32> {
    let query = query.chars().collect::<Vec<_>>();
    let mut matched = 0;
    let mut start = None;
    let mut end = 0;
    for (index, character) in text.chars().enumerate() {
        if matched < query.len() && character == query[matched] {
            start.get_or_insert(index);
            end = index;
            matched += 1;
        }
    }
    let start = start?;
    (matched == query.len()).then(|| (end - start + 1) as i32 - query.len() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(start_ms: u64, text: &str) -> LyricLine {
        LyricLine { start_ms, duration_ms: 1_000, text: text.into(), translation: None, roman: None, words: Vec::new() }
    }

    fn lrc_lines(count: u64) -> Vec<LyricLine> {
        (0..count).map(|index| line(index * 4_000, &format!("line {index}"))).collect()
    }

    fn request(title: &str, artist: &str, duration_ms: u64) -> MatchRequest {
        MatchRequest {
            keyword: format!("{title} {artist}"),
            title: title.into(),
            artist: artist.into(),
            album: String::new(),
            duration_ms,
            prefer_word_timed: true,
            sources: MatchSource::SEARCH_ORDER.to_vec(),
        }
    }

    fn candidate(source: MatchSource, title: &str, artist: &str, duration_ms: u64) -> MatchCandidate {
        MatchCandidate {
            id: format!("{source:?}:{title}:{artist}"),
            source,
            title: title.into(),
            artist: artist.into(),
            album: String::new(),
            duration_ms,
            lines: lrc_lines(6),
            format: LyricFormat::Lrc,
            source_score: 4,
        }
    }

    #[test]
    fn text_is_normalized_like_android() {
        assert_eq!(normalize_text("  Hello (feat. Someone) & Friends!  "), "hello someone and friends");
        assert_eq!(normalize_text("愛你【Live】"), "爱你 live", "繁转简、去括号");
        assert_eq!(normalize_text("ＡＢＣ　１２３"), "abc 123", "NFKC 把全角变半角");
    }

    #[test]
    fn title_and_artist_scores_follow_the_android_table() {
        assert_eq!(score_title("Lemon", "Lemon"), 80);
        assert_eq!(score_title("Lemon", "Lemon Remix"), 68);
        assert_eq!(score_title("Lemon Remix", "Lemon"), 62);
        assert_eq!(score_title("unravel", "unravel acoustic version"), 68);
        assert_eq!(score_title("Lemon", "Peach"), 0);
        assert_eq!(score_artist("米津玄師", "米津玄师"), 55, "繁简不同也算同一歌手");
        assert_eq!(score_artist("A", "A / B"), 46, "期望歌手全在候选里");
        assert_eq!(score_artist("A / B", "A/B/C"), 40, "共享两位：28 + 18 × 2 / 3（整体名也算一个）");
        assert_eq!(score_artist("A / B", "A / C"), 34);
        assert_eq!(score_artist("Artist One", "Artist One Tribute"), 24);
    }

    #[test]
    fn duration_tolerance_and_score_follow_android() {
        assert!(duration_compatible(100_000, 107_000), "7 s 下限");
        assert!(!duration_compatible(100_000, 107_001));
        assert!(duration_compatible(200_000, 212_000), "6%");
        assert!(duration_compatible(300_000, 315_000), "6% 封顶 15 s");
        assert!(!duration_compatible(300_000, 315_001));
        assert_eq!(score_duration(200_000, 200_000), 42);
        assert_eq!(score_duration(200_000, 206_000), 30);
        assert_eq!(score_duration(200_000, 260_000), -20);
        assert_eq!(score_duration(0, 200_000), 0);
    }

    #[test]
    fn version_mismatch_is_not_a_reliable_identity() {
        assert!(is_reliable_identity("Lemon", "米津玄師", "Lemon", "米津玄师"));
        assert!(!is_reliable_identity("Lemon", "米津玄師", "Lemon (Live)", "米津玄师"));
        assert!(is_reliable_identity("Lemon", "米津玄師", "Lemon (Official Audio)", "米津玄師"), "官方音频这类描述不算版本");
        assert!(is_reliable_identity("Song", "Artist One", "Song", "Artist One and Guest"));
        assert!(!is_reliable_identity("Song", "Artist One", "Song", "Artist One Tribute"));
    }

    #[test]
    fn confidence_reflects_identity_and_duration() {
        let request = request("Lemon", "米津玄師", 255_000);
        let ranked = rank(&request, vec![
            candidate(MatchSource::Netease, "Lemon", "米津玄师", 256_000),
            candidate(MatchSource::Qq, "Lemon", "米津玄师", 300_000),
            candidate(MatchSource::Lrclib, "Lemon", "", 255_000),
            candidate(MatchSource::Kugou, "Lemon", "Someone Else", 255_000),
        ]);
        let by_source = |source| ranked.iter().find(|result| result.source == source);
        assert_eq!(by_source(MatchSource::Netease).unwrap().confidence, MatchConfidence::High);
        assert_eq!(by_source(MatchSource::Netease).unwrap().duration_delta_ms, Some(1_000));
        assert_eq!(by_source(MatchSource::Qq).unwrap().confidence, MatchConfidence::Low, "时长差 45 s 只能列为低置信度");
        assert_eq!(by_source(MatchSource::Lrclib).unwrap().confidence, MatchConfidence::Medium, "没有歌手信息，但标题一致、时长对得上");
        assert!(by_source(MatchSource::Kugou).is_none(), "同名但歌手不同的不列出");
        assert_eq!(ranked.iter().map(|result| result.source).collect::<Vec<_>>(), [MatchSource::Netease, MatchSource::Lrclib, MatchSource::Qq]);
    }

    #[test]
    fn word_timed_results_form_their_own_tier_when_preferred() {
        let mut word_timed = candidate(MatchSource::Kugou, "Lemon", "米津玄師", 300_000);
        word_timed.lines[0].words = vec![parser::LyricWord { start_ms: 0, duration_ms: 400, text: "line".into() }];
        word_timed.format = LyricFormat::Yrc;
        let line_timed = candidate(MatchSource::Netease, "Lemon", "米津玄師", 255_000);
        let mut request = request("Lemon", "米津玄師", 255_000);
        let ranked = rank(&request, vec![line_timed.clone(), word_timed.clone()]);
        assert_eq!(ranked[0].source, MatchSource::Kugou, "偏好逐字时，逐字结果整体排在逐行前面，哪怕置信度低");
        assert!(ranked[0].word_timed);
        request.prefer_word_timed = false;
        let ranked = rank(&request, vec![line_timed, word_timed]);
        assert_eq!(ranked[0].source, MatchSource::Netease, "不偏好逐字时按置信度排");
    }

    #[test]
    fn unusable_and_unrelated_candidates_are_dropped() {
        let request = request("Lemon", "米津玄師", 255_000);
        let mut collapsed = candidate(MatchSource::Qq, "Lemon", "米津玄師", 255_000);
        collapsed.lines = (0..4).map(|index| line(0, &format!("row {index}"))).collect();
        let mut empty = candidate(MatchSource::Kugou, "Lemon", "米津玄師", 255_000);
        empty.lines = vec![line(0, "  ")];
        let unrelated = candidate(MatchSource::Netease, "Peach", "Somebody", 255_000);
        assert!(rank(&request, vec![collapsed, empty, unrelated]).is_empty());
    }

    #[test]
    fn duplicates_from_one_source_are_listed_once() {
        let request = request("Lemon", "米津玄師", 255_000);
        let mut again = candidate(MatchSource::Netease, "Lemon", "米津玄師", 255_000);
        again.id = "other-id".into();
        let ranked = rank(&request, vec![candidate(MatchSource::Netease, "Lemon", "米津玄師", 255_000), again]);
        assert_eq!(ranked.len(), 1);
    }

    #[test]
    fn keyword_fuzzy_score_matches_android_search_text_matcher() {
        let values = ["Lemon".to_owned(), "米津玄師".to_owned(), String::new()];
        assert_eq!(fuzzy_score("lemon", &values), Some(0), "整词相等");
        assert_eq!(fuzzy_score("lem", &values), Some(PREFIX_MATCH_SCORE + 2), "前缀匹配 lemon 比 lem 长 2");
        assert_eq!(fuzzy_score("mon", &values), Some(CONTAINS_MATCH_SCORE + 2 * 2));
        assert_eq!(fuzzy_score("peach", &values), None);
        let lemon = candidate(MatchSource::Netease, "Lemon", "米津玄師", 255_000);
        assert_eq!(score_keyword("Lemon 米津玄師", &lemon), 48);
        assert_eq!(score_keyword("Peach", &lemon), 0);
    }

    #[test]
    fn search_queries_try_keyword_then_metadata_then_title() {
        let mut request = request("愛你", "王心凌", 200_000);
        request.keyword = "愛你 王心凌".into();
        assert_eq!(search_queries(&request, false), ["愛你 王心凌", "愛你"]);
        assert_eq!(search_queries(&request, true), ["爱你 王心凌", "爱你"], "国内平台繁转简后搜");
    }

    /// 访问真实平台，手动运行：cargo test --lib live_match -- --ignored --nocapture
    /// 这里没有登录态，网易云会拒绝匿名搜索；应用里用的是带 cookie 的客户端
    #[tokio::test]
    #[ignore = "访问真实歌词平台"]
    async fn live_match_lists_candidates_from_every_platform() {
        let http = reqwest::Client::builder().build().unwrap();
        let transport = FallbackHttp::new(&http, "lyrics");
        let matcher = LyricMatcher::new(transport.clone(), NeteaseClient::with_transport(transport));
        for (title, artist, duration_ms) in [("Lemon", "米津玄師", 255_000), ("unravel", "TK from 凛として時雨", 239_000)] {
            let started = Instant::now();
            let results = matcher.find(request(title, artist, duration_ms)).await;
            eprintln!("== {title} / {artist}: {} results in {} ms", results.len(), started.elapsed().as_millis());
            for result in &results {
                eprintln!(
                    "{:?} {:?} score={} delta={:?} word={} trans={} roman={} lines={} | {} - {} ({})",
                    result.source, result.confidence, result.score, result.duration_delta_ms, result.word_timed,
                    result.has_translation, result.has_romanization, result.lines.len(), result.title, result.artist, result.album,
                );
            }
            assert!(!results.is_empty());
        }
    }

    #[test]
    fn requests_from_the_frontend_default_to_word_timed() {
        let request: MatchRequest = serde_json::from_value(serde_json::json!({
            "keyword": "Lemon", "title": "Lemon", "artist": "米津玄師", "durationMs": 255000,
            "sources": ["amll_ttml", "netease", "kugou"]
        }))
        .unwrap();
        assert!(request.prefer_word_timed);
        assert_eq!(request.sources, [MatchSource::AmllTtml, MatchSource::Netease, MatchSource::Kugou]);
        let ranked = serde_json::to_value(rank(&request, vec![candidate(MatchSource::Netease, "Lemon", "米津玄師", 255_000)])).unwrap();
        assert_eq!(ranked[0]["source"], "netease");
        assert_eq!(ranked[0]["confidence"], "high");
        assert_eq!(ranked[0]["durationDeltaMs"], 0);
        assert_eq!(ranked[0]["lines"][0]["start_ms"], 0, "歌词行沿用后端的 snake_case，前端已有映射");
    }
}
