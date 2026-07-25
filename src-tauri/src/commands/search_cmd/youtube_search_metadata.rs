use serde_json::Value;

pub(super) struct YouTubeSearchMetadata {
    pub(super) artist: String,
    pub(super) duration_ms: u64,
}

pub(super) fn parse_youtube_search_metadata(renderer: &Value) -> YouTubeSearchMetadata {
    let runs = renderer["flexColumns"].get(1).and_then(|column| {
        column["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"].as_array()
    });

    let mut endpoint_artists = Vec::new();
    let mut fallback_artist = None;
    if let Some(runs) = runs {
        for run in runs {
            let Some(text) = run["text"].as_str().map(str::trim) else {
                continue;
            };
            if is_metadata_separator(text)
                || is_search_type_label(text)
                || parse_duration_ms(text).is_some()
                || is_search_stat_text(text)
            {
                continue;
            }

            let browse_id = run["navigationEndpoint"]["browseEndpoint"]["browseId"]
                .as_str()
                .unwrap_or("");
            if is_album_browse_id(browse_id) {
                continue;
            }
            if is_artist_browse_id(browse_id) {
                endpoint_artists.push(text.to_string());
            } else if fallback_artist.is_none() {
                fallback_artist = Some(text.to_string());
            }
        }
    }

    let artist = if endpoint_artists.is_empty() {
        fallback_artist.unwrap_or_default()
    } else {
        endpoint_artists.join(" / ")
    };

    let duration_ms = find_duration_ms(
        renderer,
        "fixedColumns",
        "musicResponsiveListItemFixedColumnRenderer",
    )
    .or_else(|| {
        find_duration_ms(
            renderer,
            "flexColumns",
            "musicResponsiveListItemFlexColumnRenderer",
        )
    })
    .unwrap_or(0);

    YouTubeSearchMetadata {
        artist,
        duration_ms,
    }
}

fn find_duration_ms(renderer: &Value, columns_key: &str, column_renderer_key: &str) -> Option<u64> {
    renderer[columns_key].as_array()?.iter().find_map(|column| {
        let text = &column[column_renderer_key]["text"];
        text["simpleText"]
            .as_str()
            .and_then(parse_duration_ms)
            .or_else(|| {
                text["runs"]
                    .as_array()?
                    .iter()
                    .find_map(|run| run["text"].as_str().and_then(parse_duration_ms))
            })
    })
}

fn parse_duration_ms(text: &str) -> Option<u64> {
    let parts = text
        .trim()
        .split(':')
        .map(|part| part.trim().parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()?;

    let seconds = match parts.as_slice() {
        [minutes, seconds] if *seconds < 60 => minutes.checked_mul(60)?.checked_add(*seconds)?,
        [hours, minutes, seconds] if *minutes < 60 && *seconds < 60 => hours
            .checked_mul(60)?
            .checked_add(*minutes)?
            .checked_mul(60)?
            .checked_add(*seconds)?,
        _ => return None,
    };
    seconds.checked_mul(1000)
}

fn is_search_type_label(text: &str) -> bool {
    let normalized = text
        .trim()
        .to_lowercase()
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "song"
            | "songs"
            | "video"
            | "videos"
            | "mv"
            | "\u{6b4c}\u{66f2}"
            | "\u{66f2}"
            | "\u{89c6}\u{9891}"
    )
}

fn is_metadata_separator(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.is_empty()
        || trimmed
            .chars()
            .all(|character| matches!(character, '\u{2022}' | '\u{00b7}' | '\u{30fb}' | '|'))
}

fn is_album_browse_id(browse_id: &str) -> bool {
    browse_id.starts_with("MPRE") || browse_id.contains("release_detail")
}

fn is_artist_browse_id(browse_id: &str) -> bool {
    browse_id.starts_with("UC")
        || browse_id.starts_with("MPLA")
        || browse_id.contains("artist_detail")
}

fn is_search_stat_text(text: &str) -> bool {
    let normalized = text.trim().to_lowercase();
    [
        "view",
        "listener",
        "subscriber",
        "monthly",
        "\u{64ad}\u{653e}",
        "\u{89c2}\u{770b}",
        "\u{89c2}\u{4f17}",
        "\u{8ba2}\u{9605}\u{8005}",
    ]
    .iter()
    .any(|token| normalized.contains(token))
}

#[cfg(test)]
mod tests {
    use super::parse_youtube_search_metadata;
    use serde_json::json;

    #[test]
    fn parses_song_artist_after_localized_type_label() {
        let renderer = json!({
            "flexColumns": [
                {
                    "musicResponsiveListItemFlexColumnRenderer": {
                        "text": { "runs": [{ "text": "Track title" }] }
                    }
                },
                {
                    "musicResponsiveListItemFlexColumnRenderer": {
                        "text": {
                            "runs": [
                                { "text": "\u{6b4c}\u{66f2}" },
                                { "text": " \u{2022} " },
                                {
                                    "text": "Correct Artist",
                                    "navigationEndpoint": {
                                        "browseEndpoint": { "browseId": "UC-artist" }
                                    }
                                },
                                { "text": " \u{2022} " },
                                {
                                    "text": "Album title",
                                    "navigationEndpoint": {
                                        "browseEndpoint": { "browseId": "MPREb_album" }
                                    }
                                },
                                { "text": " \u{2022} " },
                                { "text": "3:42" }
                            ]
                        }
                    }
                }
            ]
        });

        let metadata = parse_youtube_search_metadata(&renderer);

        assert_eq!(metadata.artist, "Correct Artist");
        assert_eq!(metadata.duration_ms, 222_000);
    }

    #[test]
    fn preserves_video_artist_and_parses_hour_duration_column() {
        let renderer = json!({
            "flexColumns": [
                {
                    "musicResponsiveListItemFlexColumnRenderer": {
                        "text": { "runs": [{ "text": "Long video" }] }
                    }
                },
                {
                    "musicResponsiveListItemFlexColumnRenderer": {
                        "text": {
                            "runs": [
                                { "text": "Video" },
                                { "text": " \u{2022} " },
                                { "text": "Video Creator" },
                                { "text": " \u{2022} " },
                                { "text": "1.2M views" }
                            ]
                        }
                    }
                }
            ],
            "fixedColumns": [
                {
                    "musicResponsiveListItemFixedColumnRenderer": {
                        "text": { "runs": [{ "text": "1:02:03" }] }
                    }
                }
            ]
        });

        let metadata = parse_youtube_search_metadata(&renderer);

        assert_eq!(metadata.artist, "Video Creator");
        assert_eq!(metadata.duration_ms, 3_723_000);
    }

    #[test]
    fn leaves_missing_metadata_empty() {
        let renderer = json!({
            "flexColumns": [
                {
                    "musicResponsiveListItemFlexColumnRenderer": {
                        "text": { "runs": [{ "text": "Track title" }] }
                    }
                },
                {
                    "musicResponsiveListItemFlexColumnRenderer": {
                        "text": { "runs": [{ "text": "Song" }] }
                    }
                }
            ]
        });

        let metadata = parse_youtube_search_metadata(&renderer);

        assert_eq!(metadata.artist, "");
        assert_eq!(metadata.duration_ms, 0);
    }
}
