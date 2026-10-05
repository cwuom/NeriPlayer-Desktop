use std::collections::HashSet;

use serde_json::{json, Value};

use super::client::{YouTubeClient, YtSearchResult};
use crate::error::AppResult;

const SONG_PARAMS: &str = "EgWKAQIIAWoKEAkQBRAKEAMQBA%3D%3D";
const VIDEO_PARAMS: &str = "EgWKAQIQAWoKEAkQChAFEAMQBA%3D%3D";
const RESULT_LIMIT: usize = 30;
const PAGE_LIMIT: usize = 80;

fn text(node: &Value) -> String {
    node["simpleText"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            node["runs"]
                .as_array()
                .map(|runs| runs.iter().filter_map(|run| run["text"].as_str()).collect())
                .unwrap_or_default()
        })
}

fn column(renderer: &Value, index: usize) -> &Value {
    &renderer["flexColumns"][index]["musicResponsiveListItemFlexColumnRenderer"]["text"]
}

fn video_id(renderer: &Value) -> Option<&str> {
    for path in [
        "/playlistItemData/videoId",
        "/navigationEndpoint/watchEndpoint/videoId",
        "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId",
    ] {
        if let Some(value) = renderer.pointer(path).and_then(Value::as_str).filter(|value| !value.trim().is_empty()) { return Some(value); }
    }
    if let Some(id) = renderer["flexColumns"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|col| {
            col["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .find_map(|run| {
            run.pointer("/navigationEndpoint/watchEndpoint/videoId")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
        })
    {
        return Some(id);
    }
    renderer.pointer("/menu/menuRenderer/items").and_then(Value::as_array)?.iter().find_map(|item| {
        ["/menuNavigationItemRenderer/navigationEndpoint/watchEndpoint/videoId", "/menuServiceItemRenderer/serviceEndpoint/queueAddEndpoint/queueTarget/videoId", "/menuServiceItemRenderer/serviceEndpoint/queueAddEndpoint/queueTarget/onEmptyQueue/watchEndpoint/videoId"]
            .iter().find_map(|path| item.pointer(path).and_then(Value::as_str).filter(|value| !value.trim().is_empty()))
    })
}

fn thumbnail(renderer: &Value) -> Option<String> {
    for path in [
        "/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails",
        "/thumbnail/croppedSquareThumbnailRenderer/thumbnail/thumbnails",
        "/thumbnail/thumbnail/thumbnails",
        "/thumbnail/thumbnails",
        "/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails",
    ] {
        if let Some(url) = renderer
            .pointer(path)
            .and_then(Value::as_array)
            .and_then(|array| array.last())
            .and_then(|item| item["url"].as_str())
            .filter(|value| !value.trim().is_empty())
        {
            return Some(if url.starts_with("//") {
                format!("https:{url}")
            } else {
                url.trim().into()
            });
        }
    }
    None
}

fn parse_row(renderer: &Value) -> Option<YtSearchResult> {
    let title = text(column(renderer, 0));
    if title.trim().is_empty() {
        return None;
    }
    let id = video_id(renderer)?;
    let mut artists = Vec::new();
    let mut album = String::new();
    let runs = column(renderer, 1)["runs"].as_array();
    for (index, run) in runs.into_iter().flatten().enumerate().step_by(2) {
        let label = run["text"].as_str().unwrap_or_default().trim();
        let endpoint = &run["navigationEndpoint"]["browseEndpoint"];
        let browse_id = endpoint["browseId"].as_str().unwrap_or_default();
        let page_type = endpoint
            .pointer(
                "/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType",
            )
            .and_then(Value::as_str)
            .unwrap_or_default();
        let normalized = label.to_lowercase().replace(' ', "");
        if label.is_empty()
            || (index == 0
                && runs.is_some_and(|runs| runs.len() >= 3)
                && matches!(
                    normalized.as_str(),
                    "song" | "songs" | "video" | "videos" | "歌曲" | "曲" | "视频" | "mv"
                ))
            || (label.contains(':') && label.split(':').all(|part| part.parse::<u64>().is_ok()))
            || [
                "播放",
                "观看",
                "views",
                "view",
                "listeners",
                "listener",
                "monthly",
                "观众",
                "订阅者",
                "subscriber",
            ]
            .iter()
            .any(|token| normalized.contains(token))
        {
            continue;
        }
        if page_type == "MUSIC_PAGE_TYPE_ALBUM"
            || browse_id.starts_with("MPRE")
            || browse_id.contains("release_detail")
        {
            album = label.into();
        } else if !artists.iter().any(|artist| artist == label) {
            artists.push(label.to_owned());
        }
    }
    if runs.is_none() {
        if let Some(label) = text(column(renderer, 1))
            .split(['•', '·'])
            .map(str::trim)
            .find(|value| {
                !(value.is_empty()
                    || value.contains(':')
                        && value.split(':').all(|part| part.parse::<u64>().is_ok()))
            })
        {
            artists.push(label.into());
        }
    }
    if album.is_empty() && artists.len() >= 2 {
        album = artists.pop().unwrap_or_default();
    }
    Some(YtSearchResult {
        video_id: id.into(),
        title,
        artist: artists.join(" / "),
        album,
        duration_ms: super::duration::extract_track_duration_ms(renderer),
        thumbnail_url: thumbnail(renderer),
    })
}

pub(super) fn parse_page(root: &Value, limit: usize) -> Vec<YtSearchResult> {
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![root];
    let mut visited = 0;
    while let Some(node) = stack.pop() {
        visited += 1;
        if visited > 8000 || rows.len() >= limit {
            break;
        }
        if let Some(renderer) = node.get("musicResponsiveListItemRenderer") {
            if let Some(row) = parse_row(renderer) {
                if seen.insert(row.video_id.clone()) {
                    rows.push(row);
                }
            }
        }
        match node {
            Value::Object(object) => stack.extend(object.values().rev()),
            Value::Array(array) => stack.extend(array.iter().rev()),
            _ => {}
        }
    }
    rows
}

pub(super) fn request_body(query: &str, params: &str, continuation: Option<&str>) -> Value {
    let mut body = json!({"query": query, "params": params});
    if let Some(token) = continuation {
        body["continuation"] = json!(token);
    }
    body
}

async fn collect_with<F, Fut>(
    query: &str,
    params: &str,
    mut send: F,
) -> AppResult<Vec<YtSearchResult>>
where
    F: FnMut(Value) -> Fut,
    Fut: std::future::Future<Output = AppResult<Value>>,
{
    let mut rows = Vec::new();
    let mut seen_rows = HashSet::new();
    let mut seen_tokens = HashSet::new();
    let mut continuation = None;
    for page in 0..PAGE_LIMIT {
        let body = request_body(query, params, continuation.as_deref());
        let response = match send(body).await {
            Ok(value) => value,
            Err(error) if page == 0 => return Err(error),
            Err(_) => break,
        };
        for row in parse_page(&response, RESULT_LIMIT) {
            if seen_rows.insert(row.video_id.clone()) {
                rows.push(row);
            }
            if rows.len() >= RESULT_LIMIT {
                return Ok(rows);
            }
        }
        continuation = super::playlist::extract_continuation_token(&response);
        if !continuation
            .as_ref()
            .is_some_and(|token| seen_tokens.insert(token.clone()))
        {
            break;
        }
    }
    Ok(rows)
}

async fn collect(
    client: &YouTubeClient,
    query: &str,
    params: &str,
) -> AppResult<Vec<YtSearchResult>> {
    collect_with(query, params, |body| async move {
        client.innertube_post("search", &body).await
    })
    .await
}

pub(super) async fn search_tracks(
    client: &YouTubeClient,
    query: &str,
) -> AppResult<Vec<YtSearchResult>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    // 桌面搜索没有单独的歌曲/视频切换，合并 Android 的两种筛选结果
    let (songs, videos) = tokio::join!(
        collect(client, query, SONG_PARAMS),
        collect(client, query, VIDEO_PARAMS)
    );
    let mut result = match (songs, videos) {
        (Err(error), Err(_)) => return Err(error),
        (Ok(songs), Err(_)) | (Err(_), Ok(songs)) => songs,
        (Ok(mut songs), Ok(videos)) => {
            songs.extend(videos);
            songs
        }
    };
    let mut seen = HashSet::new();
    result.retain(|row| seen.insert(row.video_id.clone()));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str) -> Value {
        json!({"musicResponsiveListItemRenderer": {
            "navigationEndpoint": {"watchEndpoint": {"videoId": id}},
            "flexColumns": [
                {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "完整"}, {"text": "标题"}]}}},
                {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [
                    {"text": "Artist A", "navigationEndpoint": {"browseEndpoint": {"browseId": "UC-artist"}}},
                    {"text": " / "},
                    {"text": "Artist B", "navigationEndpoint": {"browseEndpoint": {"browseId": "UC-second"}}},
                    {"text": " • "},
                    {"text": "Album", "navigationEndpoint": {"browseEndpoint": {"browseId": "MPRE-album"}}},
                    {"text": " • "}, {"text": "3:45"}
                ]}}}
            ],
            "thumbnail": {"musicThumbnailRenderer": {"thumbnail": {"thumbnails": [{"url": "//i.ytimg.com/cover.jpg"}]}}}
        }})
    }

    #[test]
    fn parses_android_nested_search_video_metadata_and_continuation_rows() {
        let first = row("first");
        let root = json!({"contents": {"tabbedSearchResultsRenderer": {"tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": [
            {"itemSectionRenderer": {"contents": [{"musicShelfRenderer": {"contents": [first.clone(), first]}}]}}
        ]}}}}]}}, "continuationContents": {"musicShelfContinuation": {"contents": [row("video")]}}});
        let rows = parse_page(&root, 30);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].title, "完整标题");
        assert_eq!(rows[0].artist, "Artist A / Artist B");
        assert_eq!(rows[0].album, "Album");
        assert_eq!(rows[0].duration_ms, 225_000);
        assert_eq!(
            rows[0].thumbnail_url.as_deref(),
            Some("https://i.ytimg.com/cover.jpg")
        );
        assert_eq!(parse_page(&root, 1).len(), 1);
    }

    #[test]
    fn request_filters_and_continuations_match_android_contract() {
        assert_eq!(
            request_body("query", SONG_PARAMS, None),
            json!({"query":"query", "params":"EgWKAQIIAWoKEAkQBRAKEAMQBA%3D%3D"})
        );
        assert_eq!(
            request_body("query", VIDEO_PARAMS, Some("next")),
            json!({"query":"query", "params":"EgWKAQIQAWoKEAkQChAFEAMQBA%3D%3D", "continuation":"next"})
        );
        assert!(parse_page(&json!({"musicResponsiveListItemRenderer": {"navigationEndpoint": {"watchEndpoint": {"videoId": "missing-title"}}}}), 30).is_empty());
    }

    #[test]
    fn menu_video_identity_and_unlinked_metadata_match_android() {
        let mut fixture = row("remove");
        let renderer = fixture.get_mut("musicResponsiveListItemRenderer").unwrap();
        renderer
            .as_object_mut()
            .unwrap()
            .remove("navigationEndpoint");
        renderer["menu"] = json!({"menuRenderer": {"items": [{"menuServiceItemRenderer": {"serviceEndpoint": {"queueAddEndpoint": {"queueTarget": {"onEmptyQueue": {"watchEndpoint": {"videoId": "menu-video"}}}}}}}]}});
        renderer["flexColumns"][1]["musicResponsiveListItemFlexColumnRenderer"]["text"] = json!({"runs": [{"text":"Song"}, {"text":" • "}, {"text":"Plain artist"}, {"text":" • "}, {"text":"Plain album"}, {"text":" • "}, {"text":"12M views"}, {"text":" • "}, {"text":"3:45"}]});
        let rows = parse_page(&fixture, 30);
        assert_eq!(rows[0].video_id, "menu-video");
        assert_eq!(rows[0].artist, "Plain artist");
        assert_eq!(rows[0].album, "Plain album");
        assert_eq!(rows[0].duration_ms, 225_000);
    }

    #[tokio::test]
    async fn collects_pages_without_duplicates_or_repeated_token_requests() {
        let mut requests = Vec::new();
        let mut page = 0;
        let rows = collect_with("query", VIDEO_PARAMS, |body| {
            requests.push(body);
            page += 1;
            let values = if page == 1 { vec![row("first")] } else { vec![row("first"), row("second")] };
            std::future::ready(Ok(json!({"continuationContents": {"musicShelfContinuation": {
                "contents": values, "continuations": [{"nextContinuationData": {"continuation": "same-token"}}]
            }}})))
        }).await.unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| row.video_id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(requests.len(), 2);
        assert!(requests[0].get("continuation").is_none());
        assert_eq!(requests[1]["continuation"], "same-token");
        assert_eq!(requests[1]["params"], VIDEO_PARAMS);
    }
}
