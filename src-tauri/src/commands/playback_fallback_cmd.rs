use crate::error::{AppError, AppResult};
use crate::library::playlist;
use crate::state::{AppState, TrackInfo, TrackSource};
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use tauri::State;

const LOCAL_DURATION_TOLERANCE_MS: u64 = 8_000;
const BILI_MIN_ACCEPT_SCORE: i32 = 70;

fn compact(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn local_match_rank(
    song_id: &str,
    title: &str,
    artist: &str,
    duration_ms: u64,
    track: &TrackInfo,
) -> Option<(u8, u64)> {
    if track.source != TrackSource::Local {
        return None;
    }
    let difference = duration_ms.abs_diff(track.duration_ms);
    let known_duration = duration_ms > 0 && track.duration_ms > 0;
    if known_duration && difference > LOCAL_DURATION_TOLERANCE_MS {
        return None;
    }
    let payload = track.sync_payload.as_ref();
    let exact = payload.is_some_and(|song| {
        matches!(
            song.matched_lyric_source.as_deref(),
            Some("CLOUD_MUSIC" | "NETEASE")
        ) && song.matched_song_id.as_deref() == Some(song_id)
    });
    if exact {
        return Some((0, if known_duration { difference } else { u64::MAX }));
    }
    if !known_duration || compact(title).is_empty() {
        return None;
    }
    let display_title = payload
        .and_then(|song| song.custom_name.as_deref())
        .unwrap_or(&track.title);
    let display_artist = payload
        .and_then(|song| song.custom_artist.as_deref())
        .unwrap_or(&track.artist);
    if compact(display_title) != compact(title)
        || (!compact(artist).is_empty() && compact(display_artist) != compact(artist))
    {
        return None;
    }
    Some((1, difference))
}

fn readable_local_path(track: &TrackInfo) -> Option<PathBuf> {
    let raw = if track.url.trim().is_empty() {
        track.sync_payload.as_ref()?.media_uri.as_str()
    } else {
        track.url.as_str()
    };
    let path = if raw.starts_with("file:") {
        url::Url::parse(raw).ok()?.to_file_path().ok()?
    } else {
        if raw.contains("://") || raw.trim().is_empty() {
            return None;
        }
        PathBuf::from(raw)
    };
    let path = path.canonicalize().ok()?;
    if !path.is_file() || std::fs::File::open(&path).is_err() {
        return None;
    }
    Some(path)
}

#[tauri::command]
pub async fn find_netease_local_sources(
    song_id: String,
    title: String,
    artist: String,
    duration_ms: u64,
) -> AppResult<Vec<TrackInfo>> {
    tokio::task::spawn_blocking(move || {
        let mut candidates: Vec<_> = playlist::load_all_tracks(Some(TrackSource::Local))?
            .into_iter()
            .filter_map(|track| {
                local_match_rank(&song_id, &title, &artist, duration_ms, &track)
                    .map(|rank| (rank, track))
            })
            .collect();
        candidates.sort_by_key(|(rank, _)| *rank);
        let mut seen = HashSet::new();
        Ok(candidates
            .into_iter()
            .filter_map(|(_, mut track)| {
                let path = readable_local_path(&track)?;
                if !seen.insert(path.clone()) {
                    return None;
                }
                track.url = path.to_string_lossy().into_owned();
                Some(track)
            })
            .collect())
    })
    .await
    .map_err(|error| AppError::Other(error.to_string()))?
}

fn score_bili_text(
    title: &str,
    artist: &str,
    duration_ms: u64,
    candidate_title: &str,
    author: &str,
    duration_sec: u64,
) -> i32 {
    let normalized_title = compact(candidate_title);
    let needle = compact(title);
    let title_hit = needle.chars().count() >= 2 && normalized_title.contains(&needle);
    let mut score = if title_hit {
        55
    } else {
        let tokens: Vec<_> = title
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|token| token.chars().count() >= 2)
            .map(str::to_owned)
            .collect();
        let hits = tokens
            .iter()
            .filter(|token| normalized_title.contains(token.as_str()))
            .count();
        if hits > 0 && hits == tokens.len() {
            35
        } else if hits > 0 {
            18
        } else {
            0
        }
    };
    let artist_needle = compact(artist);
    if artist_needle.chars().count() >= 2
        && (normalized_title.contains(&artist_needle) || compact(author).contains(&artist_needle))
    {
        score += 25;
    }
    if duration_ms > 0 && duration_sec > 0 {
        let candidate_ms = duration_sec.saturating_mul(1000);
        let difference = duration_ms.abs_diff(candidate_ms);
        score += if difference <= 8_000 {
            30
        } else if difference <= 20_000 {
            22
        } else if difference <= 45_000 {
            12
        } else if candidate_ms > duration_ms.saturating_mul(2) {
            -15
        } else {
            0
        };
    }
    score
}

fn search_duration(value: &Value) -> u64 {
    if let Some(seconds) = value.as_u64() {
        return seconds;
    }
    value
        .as_str()
        .unwrap_or_default()
        .split(':')
        .try_fold(0_u64, |total, part| {
            part.parse::<u64>()
                .ok()
                .and_then(|seconds| total.checked_mul(60)?.checked_add(seconds))
        })
        .unwrap_or(0)
}

fn check_generation(state: &AppState, generation: Option<u64>) -> AppResult<()> {
    if generation.is_some_and(|generation| {
        state
            .playback_generation
            .load(std::sync::atomic::Ordering::Acquire)
            > generation
    }) {
        return Err(AppError::Audio("Playback request superseded".into()));
    }
    Ok(())
}

#[tauri::command]
pub async fn find_netease_bili_sources(
    title: String,
    artist: String,
    duration_ms: u64,
    request_generation: Option<u64>,
    state: State<'_, AppState>,
) -> AppResult<Vec<TrackInfo>> {
    check_generation(&state, request_generation)?;
    let client = state.bilibili();
    let mut queries = Vec::new();
    for value in [
        format!("{title} {artist}"),
        format!("{artist} {title}"),
        title.clone(),
    ] {
        let query = value.split_whitespace().collect::<Vec<_>>().join(" ");
        if !query.is_empty() && !queries.contains(&query) {
            queries.push(query);
        }
    }
    let duration_filter = match duration_ms / 1000 {
        0 => 0,
        1..600 => 1,
        600..1800 => 2,
        1800..3600 => 3,
        _ => 4,
    };
    let mut visited = HashSet::new();
    let mut sources = Vec::new();
    for query in queries {
        check_generation(&state, request_generation)?;
        let keyword = format!("{query} 无损");
        let response = client.search_with_duration(&keyword, duration_filter).await;
        check_generation(&state, request_generation)?;
        let response = match response {
            Ok(response)
                if response["data"]["result"]
                    .as_array()
                    .is_some_and(|items| !items.is_empty())
                    || duration_filter == 0 =>
            {
                response
            }
            _ => match client.search_with_duration(&keyword, 0).await {
                Ok(response) => response,
                Err(_) => continue,
            },
        };
        let mut candidates: Vec<_> = response["data"]["result"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|item| {
                let text = item["title"]
                    .as_str()
                    .unwrap_or_default()
                    .replace("<em class=\"keyword\">", "")
                    .replace("</em>", "");
                let score = score_bili_text(
                    &title,
                    &artist,
                    duration_ms,
                    &text,
                    item["author"].as_str().unwrap_or_default(),
                    search_duration(&item["duration"]),
                );
                (score, item)
            })
            .collect();
        candidates.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        for (candidate_score, candidate) in candidates.into_iter().take(6) {
            check_generation(&state, request_generation)?;
            let bvid = candidate["bvid"].as_str().unwrap_or_default();
            let aid = candidate["aid"].as_u64().unwrap_or(0);
            let key = if bvid.is_empty() {
                aid.to_string()
            } else {
                bvid.to_string()
            };
            if !visited.insert(key) {
                continue;
            }
            let info = if bvid.is_empty() {
                client.get_video_info_by_avid(aid).await
            } else {
                client.get_video_info(bvid).await
            };
            check_generation(&state, request_generation)?;
            let Ok(info) = info else {
                continue;
            };
            let page = info
                .pages
                .iter()
                .filter(|page| page.cid > 0)
                .map(|page| {
                    let score = candidate_score
                        + score_bili_text(
                            &title,
                            &artist,
                            duration_ms,
                            &format!("{} {}", page.part, info.title),
                            &info.owner,
                            page.duration,
                        );
                    (score, page)
                })
                .max_by_key(|(score, _)| *score);
            let Some((score, page)) = page else {
                continue;
            };
            if score < BILI_MIN_ACCEPT_SCORE {
                continue;
            }
            sources.push(TrackInfo {
                id: format!("bilibili:{}", info.bvid),
                title: info.title.clone(),
                artist: info.owner.clone(),
                album: format!("Bilibili|{}", page.cid),
                duration_ms: page.duration.saturating_mul(1000),
                source: TrackSource::Bilibili,
                url: String::new(),
                cover_url: Some(info.cover.clone()),
                added_at: 0,
                sync_payload: None,
                playlist_key: None,
            });
            if sources.len() >= 3 {
                return Ok(sources);
            }
        }
    }
    Ok(sources)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> TrackInfo {
        TrackInfo {
            id: "local:fixture".into(),
            title: "今、歩き出す君へ。".into(),
            artist: "Ceui".into(),
            album: String::new(),
            duration_ms: 240_000,
            source: TrackSource::Local,
            url: String::new(),
            cover_url: None,
            added_at: 0,
            sync_payload: None,
            playlist_key: None,
        }
    }

    #[test]
    fn local_metadata_requires_matching_title_artist_and_duration() {
        assert_eq!(
            local_match_rank("1", "今歩き出す君へ", "CEUI", 248_000, &track()),
            Some((1, 8_000))
        );
        assert!(local_match_rank("1", "今歩き出す君へ", "other", 240_000, &track()).is_none());
        assert!(local_match_rank("1", "今歩き出す君へ", "Ceui", 248_001, &track()).is_none());
        assert!(local_match_rank("1", "今歩き出す君へ", "Ceui", 0, &track()).is_none());
    }

    #[test]
    fn matched_id_precedes_metadata_and_allows_unknown_duration() {
        let mut local = track();
        local.sync_payload = Some(crate::sync::models::SyncSong {
            matched_lyric_source: Some("CLOUD_MUSIC".into()),
            matched_song_id: Some("7".into()),
            ..Default::default()
        });
        assert_eq!(
            local_match_rank("7", "different", "artist", 0, &local),
            Some((0, u64::MAX))
        );
        assert!(local_match_rank("7", "different", "artist", 260_000, &local).is_none());
    }

    #[test]
    fn bili_scoring_uses_android_weights_and_duration_penalties() {
        assert_eq!(
            score_bili_text(
                "Song Name",
                "Singer",
                180_000,
                "Singer Song Name 无损",
                "uploader",
                180
            ),
            110
        );
        assert_eq!(
            score_bili_text("Song Name", "Singer", 180_000, "unrelated", "other", 600),
            -15
        );
        assert_eq!(search_duration(&serde_json::json!("1:02:03")), 3723);
    }

    #[test]
    fn local_sources_must_be_readable_files() {
        let folder = std::env::temp_dir().join(format!("neri-fallback-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join("audio.flac");
        std::fs::write(&path, b"fixture").unwrap();
        let mut local = track();
        local.url = url::Url::from_file_path(&path).unwrap().to_string();
        assert_eq!(
            readable_local_path(&local),
            Some(path.canonicalize().unwrap())
        );
        local.url = folder.to_string_lossy().into_owned();
        assert!(readable_local_path(&local).is_none());
        local.url = "https://example.com/audio".into();
        assert!(readable_local_path(&local).is_none());
        std::fs::remove_dir_all(folder).unwrap();
    }
}
