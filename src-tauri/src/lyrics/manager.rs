// 歌词管理器：多源瀑布获取
// 跨平台(YouTube/B站/本地)优先 LRCLIB(带时长精确匹配), 再 QQ/网易,
// 候选必须过时长硬门槛, 避免同名不同版本歌词(如王心凌「爱你」Rap 版)。
use crate::api::lrclib::LrcLibClient;
use crate::api::netease::client::NeteaseClient;
use crate::api::qq::client::QqMusicClient;
use crate::api::youtube::client::YouTubeClient;
use crate::api::transport::FallbackHttp;
use crate::error::AppResult;
use crate::lyrics::parser::{self, LyricLine};
use std::path::{Path, PathBuf};

pub struct LyricsManager {
    transport: FallbackHttp,
    netease_csrf: String,
}

impl LyricsManager {
    pub async fn fetch_word_timed_lyrics(
        &self,
        track_title: &str,
        track_artist: &str,
        duration_ms: u64,
    ) -> AppResult<Vec<LyricLine>> {
        Ok(
            super::external::ExternalLyricsClient::new(self.transport.clone())
                .fetch(track_title, track_artist, duration_ms, true)
                .await,
        )
    }

    pub fn new(http: &reqwest::Client) -> Self {
        Self {
            transport: FallbackHttp::new(http, "lyrics"),
            netease_csrf: String::new(),
        }
    }

    pub fn with_transport(transport: FallbackHttp, netease_csrf: String) -> Self {
        Self {
            transport,
            netease_csrf,
        }
    }

    /// 多源获取歌词
    /// 顺序:
    /// 1. 本地 sidecar
    /// 2. 明确 QQ songmid / 网易 id
    /// 3. 无平台 id 时: LRCLIB 精确(含时长) → LRCLIB 搜索(时长过滤)
    /// 4. QQ / 网易 搜索(标题+歌手+时长硬门槛)
    /// 5. YouTube 原生歌词(若有 video_id)
    /// 6. LRCLIB 再兜底一次(搜索)
    // 播放/下载编排函数的参数都是相互独立的运行时上下文，聚成结构体只是换个地方堆字段
    #[allow(clippy::too_many_arguments)]
    pub async fn fetch_lyrics(
        &self,
        track_title: &str,
        track_artist: &str,
        duration_secs: u64,
        audio_path: Option<&str>,
        netease_id: Option<u64>,
        qq_song_mid: Option<&str>,
        youtube_video_id: Option<&str>,
    ) -> AppResult<Vec<LyricLine>> {
        log::info!(
            target: "lyrics",
            "fetch: title={}, artist={}, dur={}s, netease_id={:?}, qq_song_mid={:?}, yt={:?}",
            track_title,
            track_artist,
            duration_secs,
            netease_id,
            qq_song_mid,
            youtube_video_id
        );

        if let Some(path) = audio_path {
            if let Some(lines) = load_local_sidecar_lyrics(path) {
                log::info!(target: "lyrics", "found local sidecar: {} lines", lines.len());
                return Ok(lines);
            }
        }

        let target_duration_ms = duration_secs.saturating_mul(1000);
        let has_platform_id = netease_id.is_some()
            || qq_song_mid.map(|s| !s.trim().is_empty()).unwrap_or(false);

        // 明确平台 id: 直接取对应源
        if let Some(song_mid) = qq_song_mid.filter(|id| !id.trim().is_empty()) {
            let qq = QqMusicClient::with_transport(self.transport.clone());
            match self.parse_qq_lyrics(&qq, song_mid).await {
                Ok(Some(lines)) => {
                    if lyrics_duration_acceptable(&lines, target_duration_ms) {
                        return Ok(lines);
                    }
                    log::info!(
                        target: "lyrics",
                        "QQ lyrics for song_mid={} rejected by duration gate (target={}ms)",
                        song_mid,
                        target_duration_ms
                    );
                }
                Ok(None) => log::info!(target: "lyrics", "QQ lyrics empty for song_mid={}", song_mid),
                Err(e) => log::warn!(
                    target: "lyrics",
                    "QQ get_lyrics failed for song_mid={}: {}",
                    song_mid,
                    e
                ),
            }
        }

        if let Some(id) = netease_id {
            let client = NeteaseClient::with_transport(self.transport.clone())
                .with_csrf(self.netease_csrf.clone());
            if let Some(lines) = self.fetch_netease_lyrics(&client, id, target_duration_ms).await {
                return Ok(lines);
            }
        }

        // 跨平台: 先 LRCLIB (API 自带 duration, 最不容易错版)
        if !has_platform_id {
            if let Some(lines) = self
                .fetch_lrclib_lyrics(track_title, track_artist, duration_secs, target_duration_ms)
                .await
            {
                return Ok(lines);
            }
        }

        // QQ 搜索匹配 (标题+歌手, 时长硬门槛)
        if netease_id.is_none() && qq_song_mid.map(|s| s.trim().is_empty()).unwrap_or(true) {
            let qq = QqMusicClient::with_transport(self.transport.clone());
            match self
                .search_qq_song_mid(&qq, track_title, track_artist, target_duration_ms)
                .await
            {
                Some(song_mid) => match self.parse_qq_lyrics(&qq, &song_mid).await {
                    Ok(Some(lines)) => {
                        if lyrics_duration_acceptable(&lines, target_duration_ms) {
                            return Ok(lines);
                        }
                        log::info!(
                            target: "lyrics",
                            "matched QQ lyrics rejected by lyric-span duration gate"
                        );
                    }
                    Ok(None) => log::info!(
                        target: "lyrics",
                        "matched QQ lyrics empty for song_mid={}",
                        song_mid
                    ),
                    Err(e) => log::warn!(
                        target: "lyrics",
                        "matched QQ get_lyrics failed for song_mid={}: {}",
                        song_mid,
                        e
                    ),
                },
                None => log::info!(
                    target: "lyrics",
                    "QQ candidate not found for {} / {}",
                    track_title,
                    track_artist
                ),
            }
        }

        // 网易云搜索匹配
        if netease_id.is_none() {
            let client = NeteaseClient::with_transport(self.transport.clone())
                .with_csrf(self.netease_csrf.clone());
            if let Some(id) = self
                .search_netease_id(&client, track_title, track_artist, target_duration_ms)
                .await
            {
                if let Some(lines) = self.fetch_netease_lyrics(&client, id, target_duration_ms).await
                {
                    return Ok(lines);
                }
            }
        }

        // YouTube 原生歌词 (plain 居多, 有 timed 则用)
        if let Some(video_id) = youtube_video_id.filter(|id| !id.trim().is_empty()) {
            if let Some(lines) = self
                .fetch_youtube_lyrics(video_id, target_duration_ms)
                .await
            {
                return Ok(lines);
            }
        }

        // 最后再试 LRCLIB (平台 id 路径失败时)
        if has_platform_id {
            if let Some(lines) = self
                .fetch_lrclib_lyrics(track_title, track_artist, duration_secs, target_duration_ms)
                .await
            {
                return Ok(lines);
            }
        }

        let external = super::external::ExternalLyricsClient::new(self.transport.clone())
            .fetch(track_title, track_artist, target_duration_ms, false)
            .await;
        if !external.is_empty() {
            return Ok(external);
        }

        log::info!(
            target: "lyrics",
            "no lyrics found for: {} - {}",
            track_title,
            track_artist
        );
        Ok(Vec::new())
    }

    async fn fetch_netease_lyrics(
        &self,
        client: &NeteaseClient,
        id: u64,
        target_duration_ms: u64,
    ) -> Option<Vec<LyricLine>> {
        match client.get_lyrics(id).await {
            Ok(lyrics_data) => {
                log::info!(
                    target: "lyrics",
                    "netease lyrics for id={}: lrc={}, tlyric={}, yrc={}, romalrc={}",
                    id,
                    lyrics_data.lrc.as_ref().map_or(0, |s| s.len()),
                    lyrics_data.tlyric.as_ref().map_or(0, |s| s.len()),
                    lyrics_data.yrc.as_ref().map_or(0, |s| s.len()),
                    lyrics_data.romalrc.as_ref().map_or(0, |s| s.len()),
                );

                let translation = lyrics_data
                    .ytlrc
                    .as_deref()
                    .or(lyrics_data.tlyric.as_deref())
                    .filter(|s| !s.is_empty());

                if let Some(ref yrc_str) = lyrics_data.yrc {
                    if !yrc_str.trim().is_empty() {
                        let mut lines = parser::parse_yrc(yrc_str);
                        if !lines.is_empty() {
                            if let Some(tl) = translation {
                                parser::merge_translation(&mut lines, tl);
                            }
                            if let Some(roman) = lyrics_data
                                .romalrc
                                .as_deref()
                                .filter(|s| !s.trim().is_empty())
                            {
                                parser::merge_roman(&mut lines, roman);
                            }
                            if lyrics_duration_acceptable(&lines, target_duration_ms) {
                                log::info!(
                                    target: "lyrics",
                                    "using netease YRC: {} lines",
                                    lines.len()
                                );
                                return Some(lines);
                            }
                            log::info!(
                                target: "lyrics",
                                "netease YRC rejected by duration gate id={}",
                                id
                            );
                        }
                    }
                }

                if let Some(ref lrc_str) = lyrics_data.lrc {
                    let mut lines = parser::parse_auto(lrc_str);
                    if !lines.is_empty() {
                        if let Some(tl) = translation {
                            parser::merge_translation(&mut lines, tl);
                        }
                        if let Some(roman) = lyrics_data
                            .romalrc
                            .as_deref()
                            .filter(|s| !s.trim().is_empty())
                        {
                            parser::merge_roman(&mut lines, roman);
                        }
                        if lyrics_duration_acceptable(&lines, target_duration_ms) {
                            log::info!(target: "lyrics", "using netease LRC: {} lines", lines.len());
                            return Some(lines);
                        }
                        log::info!(
                            target: "lyrics",
                            "netease LRC rejected by duration gate id={}",
                            id
                        );
                    }
                }
                None
            }
            Err(e) => {
                log::warn!(target: "lyrics", "netease get_lyrics failed for id={}: {}", id, e);
                None
            }
        }
    }

    async fn fetch_lrclib_lyrics(
        &self,
        track_title: &str,
        track_artist: &str,
        duration_secs: u64,
        target_duration_ms: u64,
    ) -> Option<Vec<LyricLine>> {
        let lrclib = LrcLibClient::with_transport(self.transport.clone());

        // 精确匹配 (API 按 duration 查)
        if duration_secs > 0 {
            if let Ok(Some(result)) = lrclib
                .get_lyrics(track_title, track_artist, duration_secs)
                .await
            {
                if let Some(synced) = result.synced_lyrics {
                    let lines = parser::parse_lrc(&synced);
                    if !lines.is_empty() && lyrics_duration_acceptable(&lines, target_duration_ms) {
                        log::info!(
                            target: "lyrics",
                            "using LRCLIB exact: {} lines, api_dur={}",
                            lines.len(),
                            result.duration
                        );
                        return Some(lines);
                    }
                }
            }
            // duration ±2s 再试, LRCLIB 有时按 206/208 入库
            for delta in [-2i64, -1, 1, 2] {
                let d = (duration_secs as i64 + delta).max(1) as u64;
                if d == duration_secs {
                    continue;
                }
                if let Ok(Some(result)) = lrclib.get_lyrics(track_title, track_artist, d).await {
                    if let Some(synced) = result.synced_lyrics {
                        let lines = parser::parse_lrc(&synced);
                        if !lines.is_empty()
                            && lyrics_duration_acceptable(&lines, target_duration_ms)
                        {
                            log::info!(
                                target: "lyrics",
                                "using LRCLIB exact(delta={}): {} lines",
                                delta,
                                lines.len()
                            );
                            return Some(lines);
                        }
                    }
                }
            }
        }

        // 模糊搜索 + 时长过滤
        let query = format!("{} {}", track_title, track_artist);
        if let Ok(results) = lrclib.search(&query).await {
            let target_title = normalize_match_text(track_title);
            let target_artist = normalize_match_text(track_artist);
            let target_artists = normalize_artists(track_artist);

            let mut ranked: Vec<_> = results
                .into_iter()
                .filter(|r| r.synced_lyrics.as_ref().map(|s| !s.trim().is_empty()).unwrap_or(false))
                .map(|r| {
                    let cand_dur_ms = (r.duration * 1000.0).round().max(0.0) as u64;
                    let score = score_lyric_candidate(
                        &r.track_name,
                        &r.artist_name,
                        cand_dur_ms,
                        &target_title,
                        &target_artist,
                        &target_artists,
                        target_duration_ms,
                    );
                    (r, score, cand_dur_ms)
                })
                .filter(|(_, score, _)| *score >= MINIMUM_MATCH_SCORE)
                .collect();
            ranked.sort_by_key(|entry| std::cmp::Reverse(entry.1));

            for (r, score, cand_dur) in ranked {
                let synced = r.synced_lyrics.as_deref().unwrap_or("");
                let lines = parser::parse_lrc(synced);
                if lines.is_empty() {
                    continue;
                }
                if !lyrics_duration_acceptable(&lines, target_duration_ms) {
                    continue;
                }
                // 候选元数据时长也要过硬门槛
                if target_duration_ms > 0
                    && cand_dur > 0
                    && !duration_within_hard_tolerance(cand_dur, target_duration_ms)
                {
                    continue;
                }
                log::info!(
                    target: "lyrics",
                    "using LRCLIB search: score={}, dur={}ms, lines={}",
                    score,
                    cand_dur,
                    lines.len()
                );
                return Some(lines);
            }
        }
        None
    }

    async fn fetch_youtube_lyrics(
        &self,
        video_id: &str,
        target_duration_ms: u64,
    ) -> Option<Vec<LyricLine>> {
        let yt = YouTubeClient::with_transport(self.transport.clone());
        match yt.get_lyrics(video_id).await {
            Ok(Some(text)) if !text.trim().is_empty() => {
                // timed LRC / 纯文本
                let lines = if text.contains('[') && text.contains(':') {
                    parser::parse_auto(&text)
                } else {
                    plain_text_to_lines(&text, target_duration_ms)
                };
                if lines.is_empty() {
                    return None;
                }
                log::info!(
                    target: "lyrics",
                    "using YouTube native lyrics: {} lines for {}",
                    lines.len(),
                    video_id
                );
                Some(lines)
            }
            Ok(_) => {
                log::info!(target: "lyrics", "YouTube lyrics empty for {}", video_id);
                None
            }
            Err(e) => {
                log::warn!(
                    target: "lyrics",
                    "YouTube get_lyrics failed for {}: {}",
                    video_id,
                    e
                );
                None
            }
        }
    }

    async fn parse_qq_lyrics(
        &self,
        client: &QqMusicClient,
        song_mid: &str,
    ) -> AppResult<Option<Vec<LyricLine>>> {
        let (lrc, translated) = client.get_lyrics(song_mid).await?;
        let Some(lrc) = lrc else {
            return Ok(None);
        };
        let mut lines = parser::parse_lrc(&lrc);
        if lines.is_empty() {
            return Ok(None);
        }
        if let Some(tl) = translated.as_deref() {
            parser::merge_translation(&mut lines, tl);
        }
        log::info!(target: "lyrics", "using QQ LRC: {} lines", lines.len());
        Ok(Some(lines))
    }

    async fn search_qq_song_mid(
        &self,
        client: &QqMusicClient,
        title: &str,
        artist: &str,
        target_duration_ms: u64,
    ) -> Option<String> {
        // 带上歌手一起搜, 避免「爱你」命中王心凌
        let query = if artist.trim().is_empty() {
            title.to_string()
        } else {
            format!("{} {}", title, artist)
        };
        let mut results = client.search(&query, 1, 15).await.ok().unwrap_or_default();
        if results.is_empty() && !artist.trim().is_empty() {
            // 回退仅标题, 但仍走时长硬门槛
            results = client.search(title, 1, 15).await.ok()?;
        }
        if results.is_empty() {
            return None;
        }

        let target_title = normalize_match_text(title);
        let target_artist = normalize_match_text(artist);
        let target_artists = normalize_artists(artist);

        let best = results
            .into_iter()
            .map(|candidate| {
                let score = score_lyric_candidate(
                    &candidate.song_name,
                    &candidate.artists.join(" / "),
                    candidate.duration_ms,
                    &target_title,
                    &target_artist,
                    &target_artists,
                    target_duration_ms,
                );
                (candidate, score)
            })
            .max_by_key(|(_, score)| *score)?;

        if best.1 < MINIMUM_MATCH_SCORE {
            log::info!(
                target: "lyrics",
                "no confident QQ match for {} / {}, best_score={}, target_dur={}ms",
                title,
                artist,
                best.1,
                target_duration_ms
            );
            return None;
        }

        // 有目标时长时必须过硬门槛
        if target_duration_ms > 0
            && best.0.duration_ms > 0
            && !duration_within_hard_tolerance(best.0.duration_ms, target_duration_ms)
        {
            log::info!(
                target: "lyrics",
                "best QQ match rejected by hard duration: mid={}, cand={}ms, target={}ms, score={}",
                best.0.song_mid,
                best.0.duration_ms,
                target_duration_ms,
                best.1
            );
            return None;
        }

        log::info!(
            target: "lyrics",
            "matched QQ song_mid={}, score={}, name={}, dur={}ms (target={}ms)",
            best.0.song_mid,
            best.1,
            best.0.song_name,
            best.0.duration_ms,
            target_duration_ms
        );
        Some(best.0.song_mid)
    }

    /// 通过搜索网易云获取匹配歌曲 ID (标题/歌手/时长综合打分 + 硬门槛)
    async fn search_netease_id(
        &self,
        client: &NeteaseClient,
        title: &str,
        artist: &str,
        target_duration_ms: u64,
    ) -> Option<u64> {
        let query = format!("{} {}", title, artist);
        let results = client.search(&query, 15, 0).await.ok()?;
        if results.is_empty() {
            return None;
        }

        let target_title = normalize_match_text(title);
        let target_artist = normalize_match_text(artist);
        let target_artists = normalize_artists(artist);

        let best = results
            .into_iter()
            .map(|candidate| {
                let score = score_lyric_candidate(
                    &candidate.name,
                    &candidate.artists.join(" / "),
                    candidate.duration_ms,
                    &target_title,
                    &target_artist,
                    &target_artists,
                    target_duration_ms,
                );
                (candidate, score)
            })
            .max_by_key(|(_, score)| *score)?;

        if best.1 < MINIMUM_MATCH_SCORE {
            log::info!(
                target: "lyrics",
                "no confident netease match for {} / {}, best_score={}, target_dur={}ms",
                title,
                artist,
                best.1,
                target_duration_ms
            );
            return None;
        }

        if target_duration_ms > 0
            && best.0.duration_ms > 0
            && !duration_within_hard_tolerance(best.0.duration_ms, target_duration_ms)
        {
            log::info!(
                target: "lyrics",
                "best netease match rejected by hard duration: id={}, cand={}ms, target={}ms, score={}",
                best.0.id,
                best.0.duration_ms,
                target_duration_ms,
                best.1
            );
            return None;
        }

        log::info!(
            target: "lyrics",
            "matched netease id={}, score={}, name={}, dur={}ms (target={}ms)",
            best.0.id,
            best.1,
            best.0.name,
            best.0.duration_ms,
            target_duration_ms
        );
        Some(best.0.id)
    }
}

/// 最低置信分: 标题至少部分匹配
const MINIMUM_MATCH_SCORE: i32 = 60;

/// 歌词候选打分: 标题 + 歌手 + 时长
/// 有目标时长时: 时长不在硬容差内直接淘汰(返回 0)
fn score_lyric_candidate(
    candidate_title: &str,
    candidate_artist: &str,
    candidate_duration_ms: u64,
    target_title: &str,
    target_artist: &str,
    target_artists: &std::collections::HashSet<String>,
    target_duration_ms: u64,
) -> i32 {
    let candidate_title = normalize_match_text(candidate_title);
    let candidate_artist_normalized = normalize_match_text(candidate_artist);
    let candidate_artists = normalize_artists(candidate_artist);

    // 有目标时长 + 候选时长时, 先过硬门槛 (防止王心凌 219s 抢走陈芳语 207s)
    if target_duration_ms > 0
        && candidate_duration_ms > 0
        && !duration_within_hard_tolerance(candidate_duration_ms, target_duration_ms)
    {
        return 0;
    }

    let mut score = if candidate_title == target_title {
        100
    } else if !target_title.is_empty()
        && !candidate_title.is_empty()
        && (candidate_title.contains(target_title) || target_title.contains(&candidate_title))
    {
        60
    } else {
        0
    };

    // 标题完全对不上时不靠歌手/时长硬凑
    if score == 0 {
        return 0;
    }

    if !target_artist.is_empty() || !target_artists.is_empty() {
        score += if candidate_artist_normalized == target_artist {
            40
        } else if !candidate_artists.is_disjoint(target_artists) {
            25
        } else if !target_artist.is_empty()
            && (candidate_artist_normalized.contains(target_artist)
                || target_artist.contains(&candidate_artist_normalized))
        {
            15
        } else {
            // 歌手对不上: 有目标时长时重罚, 无时长时轻微罚
            if target_duration_ms > 0 {
                -35
            } else {
                -10
            }
        };
    }

    score += duration_match_bonus(candidate_duration_ms, target_duration_ms);
    score
}

/// 硬容差: 对齐 Android isAmllDurationCompatible 偏紧版
/// 目标 207s 时约允许 ±12~20s, 卡住 219s 王心凌 vs 207s 陈芳语仍靠歌手分区分,
/// 但 234s 云汐 / 明显不同版本直接淘汰
fn duration_within_hard_tolerance(candidate_duration_ms: u64, target_duration_ms: u64) -> bool {
    if candidate_duration_ms == 0 || target_duration_ms == 0 {
        return true;
    }
    let delta = (candidate_duration_ms as i64 - target_duration_ms as i64).unsigned_abs();
    let tolerance = std::cmp::max(12_000, target_duration_ms / 10).min(25_000);
    delta <= tolerance
}

/// 时长匹配加分
fn duration_match_bonus(candidate_duration_ms: u64, target_duration_ms: u64) -> i32 {
    if candidate_duration_ms == 0 || target_duration_ms == 0 {
        return 0;
    }
    let delta = (candidate_duration_ms as i64 - target_duration_ms as i64).unsigned_abs();
    if delta <= 3_000 {
        30
    } else if delta <= 6_000 {
        20
    } else if delta <= 12_000 {
        10
    } else if delta <= 25_000 {
        0
    } else {
        -40
    }
}

/// 用歌词时间轴估算总长, 与目标时长比对 (歌词末行常早于音频结束, 允许更宽下限)
fn lyrics_duration_acceptable(lines: &[LyricLine], target_duration_ms: u64) -> bool {
    if target_duration_ms == 0 || lines.is_empty() {
        return true;
    }
    let mut end_ms = 0u64;
    for line in lines {
        let line_end = line.start_ms.saturating_add(line.duration_ms.max(1));
        if line_end > end_ms {
            end_ms = line_end;
        }
        for w in &line.words {
            let we = w.start_ms.saturating_add(w.duration_ms.max(1));
            if we > end_ms {
                end_ms = we;
            }
        }
    }
    if end_ms == 0 {
        return true;
    }
    // 歌词跨度远短于歌曲(如只有前奏/错版) 或 明显超长
    let short_tol = std::cmp::max(35_000, (target_duration_ms * 20) / 100).min(70_000);
    let long_tol = std::cmp::max(15_000, target_duration_ms / 8).min(35_000);
    if end_ms + short_tol < target_duration_ms {
        // 末行过早: 可能是短版/截断, 仍接受 (片尾空白常见); 但若短超过 50% 则拒
        if end_ms.saturating_mul(2) < target_duration_ms {
            return false;
        }
        return true;
    }
    if end_ms > target_duration_ms.saturating_add(long_tol) {
        return false;
    }
    true
}

fn plain_text_to_lines(text: &str, target_duration_ms: u64) -> Vec<LyricLine> {
    let rows: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if rows.is_empty() {
        return Vec::new();
    }
    let total = target_duration_ms.max(rows.len() as u64 * 3000);
    let step = (total / rows.len() as u64).max(2000);
    rows.into_iter()
        .enumerate()
        .map(|(i, line)| LyricLine {
            start_ms: i as u64 * step,
            duration_ms: step,
            text: line.to_string(),
            translation: None,
            roman: None,
            words: Vec::new(),
        })
        .collect()
}

fn normalize_match_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn normalize_artists(value: &str) -> std::collections::HashSet<String> {
    let normalized = value
        .replace(" feat. ", "/")
        .replace(" feat ", "/")
        .replace(" ft. ", "/")
        .replace(" ft ", "/")
        .replace(" x ", "/")
        .replace(" X ", "/");
    normalized
        .split(['/', ',', '、', '，', '&'])
        .map(normalize_match_text)
        .filter(|s| !s.is_empty())
        .collect()
}

fn load_local_sidecar_lyrics(audio_path: &str) -> Option<Vec<LyricLine>> {
    let path = Path::new(audio_path);
    let primary = find_nearby_lyric(path);
    let fallback = find_nearby_lyric_with_suffixes(path, &["lrc", "txt"]);
    let mut lines = [primary, fallback].into_iter().flatten().find_map(|lyric_path| {
        let content = std::fs::read_to_string(&lyric_path).ok()?;
        let lines = parse_sidecar_lyrics_text(&content);
        (!lines.is_empty()).then_some(lines)
    })?;

    if let Some(translation_path) = find_nearby_translation(path) {
        if let Ok(translation) = std::fs::read_to_string(&translation_path) {
            parser::merge_translation(&mut lines, &translation);
        }
    }

    Some(lines)
}

fn parse_sidecar_lyrics_text(content: &str) -> Vec<LyricLine> {
    let parsed = parser::parse_auto(content);
    if !parsed.is_empty() {
        return parsed;
    }

    if super::ttml::looks_like(content) {
        return Vec::new();
    }

    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .enumerate()
        .map(|(index, text)| LyricLine {
            start_ms: index as u64 * 3000,
            duration_ms: 3000,
            text: text.to_string(),
            translation: None,
            roman: None,
            words: Vec::new(),
        })
        .collect()
}

fn find_nearby_lyric(audio_path: &Path) -> Option<PathBuf> {
    find_nearby_lyric_with_suffixes(audio_path, &["ttml", "lrc", "txt"])
}

fn find_nearby_lyric_with_suffixes(audio_path: &Path, suffixes: &[&str]) -> Option<PathBuf> {
    let parent = audio_path.parent()?;
    let file_name = audio_path.file_name()?;
    let stem = audio_path.file_stem()?.to_str()?;

    // 下载器新写入的 sidecar 带完整音频名，例如 Song.m4a.lrc。
    // 先查这一形式，避免 Song.m4a 和 Song.flac 共用旧 Song.lrc 时串词
    if let Some(path) = find_audio_scoped_sidecar(parent, file_name, suffixes) {
        return Some(path);
    }

    let lyrics_dir = parent.join("Lyrics");
    if let Some(path) = find_audio_scoped_sidecar(&lyrics_dir, file_name, suffixes) {
        return Some(path);
    }

    // 保留旧版 stem 命名兼容，已有本地歌词无需迁移
    if let Some(path) = find_stem_sidecar(parent, stem, suffixes) {
        return Some(path);
    }
    if let Some(path) = find_stem_sidecar(&lyrics_dir, stem, suffixes) {
        return Some(path);
    }

    None
}

fn find_nearby_translation(audio_path: &Path) -> Option<PathBuf> {
    let parent = audio_path.parent()?;
    let file_name = audio_path.file_name()?;
    let stem = audio_path.file_stem()?.to_str()?;
    let suffixes = ["tlrc", "translated.lrc", "translation.lrc"];

    if let Some(path) = find_audio_scoped_sidecar(parent, file_name, &suffixes) {
        return Some(path);
    }

    let lyrics_dir = parent.join("Lyrics");
    if let Some(path) = find_audio_scoped_sidecar(&lyrics_dir, file_name, &suffixes) {
        return Some(path);
    }
    if let Some(path) = find_stem_sidecar(parent, stem, &suffixes) {
        return Some(path);
    }
    if let Some(path) = find_stem_sidecar(&lyrics_dir, stem, &suffixes) {
        return Some(path);
    }

    None
}

fn find_audio_scoped_sidecar(
    dir: &Path,
    audio_file_name: &std::ffi::OsStr,
    suffixes: &[&str],
) -> Option<PathBuf> {
    suffixes.iter().find_map(|suffix| {
        let mut name = audio_file_name.to_os_string();
        name.push(format!(".{suffix}"));
        let path = dir.join(name);
        path.is_file().then_some(path)
    })
}

fn find_stem_sidecar(dir: &Path, stem: &str, suffixes: &[&str]) -> Option<PathBuf> {
    suffixes.iter().find_map(|suffix| {
        let path = dir.join(format!("{stem}.{suffix}"));
        path.is_file().then_some(path)
    })
}


#[cfg(test)]
mod tests {
    use super::{
        duration_match_bonus, duration_within_hard_tolerance, lyrics_duration_acceptable,
        score_lyric_candidate, LyricLine,
    };
    use std::collections::HashSet;

    fn artists(value: &str) -> HashSet<String> {
        super::normalize_artists(value)
    }

    #[test]
    fn duration_bonus_prefers_close_lengths() {
        assert_eq!(duration_match_bonus(207_000, 207_000), 30);
        assert_eq!(duration_match_bonus(210_000, 207_000), 30);
        assert_eq!(duration_match_bonus(212_000, 207_000), 20);
        assert_eq!(duration_match_bonus(218_000, 207_000), 10);
        assert_eq!(duration_match_bonus(230_000, 207_000), 0);
        assert_eq!(duration_match_bonus(300_000, 207_000), -40);
        assert_eq!(duration_match_bonus(0, 207_000), 0);
        assert_eq!(duration_match_bonus(207_000, 0), 0);
    }

    #[test]
    fn hard_tolerance_rejects_far_versions() {
        // 207s vs 219s = 12s, 边界内
        assert!(duration_within_hard_tolerance(219_000, 207_000));
        // 207s vs 234s = 27s, 淘汰
        assert!(!duration_within_hard_tolerance(234_000, 207_000));
        // 207s vs 260s 淘汰
        assert!(!duration_within_hard_tolerance(260_000, 207_000));
    }

    #[test]
    fn score_prefers_correct_artist_same_title() {
        let target_title = "爱你";
        let target_artist = "陈芳语";
        let target_artists = artists(target_artist);
        let target_dur = 207_000;

        let cyndi = score_lyric_candidate(
            "爱你",
            "陈芳语",
            207_000,
            target_title,
            target_artist,
            &target_artists,
            target_dur,
        );
        let wang = score_lyric_candidate(
            "爱你",
            "王心凌",
            219_000,
            target_title,
            target_artist,
            &target_artists,
            target_dur,
        );
        assert!(cyndi > wang, "cyndi={cyndi} wang={wang}");
        assert!(cyndi >= 60);
        // 王心凌歌手不对应被扣分, 即使时长擦边
        assert!(wang < cyndi);
    }

    #[test]
    fn score_rejects_far_duration_even_same_title() {
        let target_artists = artists("陈芳语");
        let score = score_lyric_candidate(
            "爱你",
            "云汐",
            234_000,
            "爱你",
            "陈芳语",
            &target_artists,
            207_000,
        );
        assert_eq!(score, 0);
    }

    #[test]
    fn score_rejects_title_mismatch() {
        let target_artists = artists("罗言");
        let score = score_lyric_candidate(
            "完全不同的歌",
            "罗言",
            161_000,
            "红",
            "罗言",
            &target_artists,
            161_000,
        );
        assert_eq!(score, 0);
    }

    #[test]
    fn lyrics_span_rejects_half_length() {
        let lines = vec![LyricLine {
            start_ms: 0,
            duration_ms: 5_000,
            text: "a".into(),
            translation: None,
            roman: None,
            words: vec![],
        }];
        assert!(!lyrics_duration_acceptable(&lines, 207_000));
    }

    #[test]
    fn audio_scoped_sidecars_override_legacy_stem_sidecars() {
        let dir = std::env::temp_dir().join(format!(
            "neri-lyrics-sidecar-scope-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("Song.m4a");
        std::fs::write(&audio, b"audio").unwrap();
        let legacy_lyric = dir.join("Song.lrc");
        let scoped_lyric = dir.join("Song.m4a.lrc");
        let legacy_translation = dir.join("Song.tlrc");
        let scoped_translation = dir.join("Song.m4a.tlrc");
        std::fs::write(&legacy_lyric, "[00:00.00]legacy").unwrap();
        std::fs::write(&scoped_lyric, "[00:00.00]scoped").unwrap();
        std::fs::write(&legacy_translation, "[00:00.00]旧翻译").unwrap();
        std::fs::write(&scoped_translation, "[00:00.00]新翻译").unwrap();

        assert_eq!(super::find_nearby_lyric(&audio), Some(scoped_lyric));
        assert_eq!(
            super::find_nearby_translation(&audio),
            Some(scoped_translation)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_ttml_sidecar_keeps_valid_existing_lrc() {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("Song.m4a");
        std::fs::write(dir.path().join("Song.m4a.ttml"), "<tt><body>unfinished").unwrap();
        std::fs::write(dir.path().join("Song.m4a.lrc"), "[00:01.00]valid baseline").unwrap();
        let lines = super::load_local_sidecar_lyrics(audio.to_str().unwrap()).unwrap();
        assert_eq!(lines[0].text, "valid baseline");
    }
}
