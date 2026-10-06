use std::collections::HashSet;

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct YtFollowedArtist {
    pub browse_id: String,
    pub name: String,
    pub cover_url: Option<String>,
    pub subtitle: String,
}

pub(super) struct FollowedArtistsPage {
    pub(super) artists: Vec<YtFollowedArtist>,
    pub(super) continuation: Option<String>,
}

fn source<'a>(renderer: &'a Value, key: &str) -> Option<(&'a Value, &'a Vec<Value>)> {
    Some((renderer, renderer.get(key)?.as_array()?))
}

fn section_source(section: &Value) -> Option<(&Value, &Vec<Value>)> {
    for (key, items) in [
        ("gridRenderer", "items"),
        ("musicPlaylistShelfRenderer", "contents"),
        ("musicShelfRenderer", "contents"),
    ] {
        if let Some(renderer) = section.get(key) {
            return source(renderer, items);
        }
    }
    section
        .pointer("/itemSectionRenderer/contents")
        .and_then(Value::as_array)?
        .iter()
        .find_map(section_source)
}

fn page_source(root: &Value) -> Option<(&Value, &Vec<Value>)> {
    if root
        .pointer("/continuationContents/musicCarouselShelfContinuation")
        .is_some()
    {
        return None;
    }
    for (key, items) in [
        ("gridContinuation", "items"),
        ("musicPlaylistShelfContinuation", "contents"),
        ("musicShelfContinuation", "contents"),
    ] {
        if let Some(renderer) = root
            .get("continuationContents")
            .and_then(|contents| contents.get(key))
        {
            return source(renderer, items);
        }
    }
    if let Some(actions) = root
        .get("onResponseReceivedActions")
        .and_then(Value::as_array)
    {
        if let Some(value) = actions
            .iter()
            .find_map(|action| action.get("appendContinuationItemsAction"))
        {
            return source(value, "continuationItems");
        }
    }
    if let Some(sections) = root
        .pointer("/contents/sectionListRenderer/contents")
        .and_then(Value::as_array)
    {
        if let Some(source) = sections.iter().find_map(section_source) {
            return Some(source);
        }
    }
    for column in [
        "singleColumnBrowseResultsRenderer",
        "twoColumnBrowseResultsRenderer",
    ] {
        let Some(tabs) = root
            .get("contents")
            .and_then(|contents| contents.get(column))
            .and_then(|column| column.get("tabs"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        let legacy_tab = if tabs.len() < 3 { 1 } else { 2 };
        for index in [0, legacy_tab] {
            let Some(sections) = tabs
                .get(index)
                .and_then(|tab| tab.pointer("/tabRenderer/content/sectionListRenderer/contents"))
                .and_then(Value::as_array)
            else {
                continue;
            };
            if let Some(source) = sections.iter().find_map(section_source) {
                return Some(source);
            }
        }
    }
    None
}

fn text(node: Option<&Value>) -> String {
    let Some(node) = node else {
        return String::new();
    };
    node.get("runs")
        .and_then(Value::as_array)
        .map(|runs| {
            runs.iter()
                .filter_map(|run| run.get("text").and_then(Value::as_str))
                .collect::<String>()
        })
        .unwrap_or_else(|| {
            node.get("simpleText")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        })
        .trim()
        .to_owned()
}

fn run_endpoint(node: &Value) -> Option<&Value> {
    node.get("runs")?
        .as_array()?
        .iter()
        .find_map(|run| run.pointer("/navigationEndpoint/browseEndpoint"))
}

fn parse_artist(item: &Value) -> Option<YtFollowedArtist> {
    let two_row = item.get("musicTwoRowItemRenderer");
    let renderer = two_row.or_else(|| item.get("musicResponsiveListItemRenderer"))?;
    if renderer.pointer("/navigationEndpoint/watchEndpoint").is_some()
        || renderer.pointer("/playlistItemData/videoId").and_then(Value::as_str).is_some_and(|id| !id.is_empty())
        || renderer.pointer("/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId").and_then(Value::as_str).is_some_and(|id| !id.is_empty()) { return None; }
    let columns = renderer.get("flexColumns").and_then(Value::as_array);
    let endpoint = renderer
        .pointer("/navigationEndpoint/browseEndpoint")
        .or_else(|| renderer.get("title").and_then(run_endpoint))
        .or_else(|| {
            columns.and_then(|columns| {
                columns
                    .iter()
                    .filter_map(|column| {
                        column.pointer("/musicResponsiveListItemFlexColumnRenderer/text")
                    })
                    .find_map(run_endpoint)
            })
        })?;
    let browse_id = endpoint.get("browseId")?.as_str()?.trim();
    let page_type = endpoint
        .pointer("/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if browse_id.is_empty() || !(browse_id.starts_with("UC") || page_type.contains("ARTIST")) {
        return None;
    }
    let column_text = |index: usize| {
        text(
            columns
                .and_then(|columns| columns.get(index))
                .and_then(|column| {
                    column.pointer("/musicResponsiveListItemFlexColumnRenderer/text")
                }),
        )
    };
    let name = if two_row.is_some() {
        text(renderer.get("title"))
    } else {
        column_text(0)
    };
    if name.is_empty() {
        return None;
    }
    let subtitle = if two_row.is_some() {
        text(renderer.get("subtitle"))
    } else {
        column_text(1)
    };
    let cover_url = [
        "/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails",
        "/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails",
        "/thumbnail/thumbnails",
    ]
    .into_iter()
    .filter_map(|path| renderer.pointer(path).and_then(Value::as_array))
    .flat_map(|items| items.iter())
    .filter(|item| {
        item.get("url")
            .and_then(Value::as_str)
            .is_some_and(|url| !url.is_empty())
    })
    .max_by_key(|item| item.get("width").and_then(Value::as_u64).unwrap_or(0))
    .and_then(|item| item.get("url").and_then(Value::as_str))
    .map(str::to_owned);
    Some(YtFollowedArtist {
        browse_id: browse_id.to_owned(),
        name,
        cover_url,
        subtitle,
    })
}

pub(super) fn parse_followed_artists_page(root: &Value) -> Option<FollowedArtistsPage> {
    let (renderer, items) = page_source(root)?;
    let mut seen = HashSet::new();
    Some(FollowedArtistsPage {
        artists: items
            .iter()
            .filter_map(parse_artist)
            .filter(|artist| seen.insert(artist.browse_id.clone()))
            .collect(),
        continuation: super::playlist::extract_continuation_token(renderer),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn artist(id: &str) -> Value {
        json!({"musicTwoRowItemRenderer":{
            "title":{"runs":[{"text":"Demo "},{"text":"Artist"}]},
            "subtitle":{"simpleText":"12K subscribers"},
            "navigationEndpoint":{"browseEndpoint":{"browseId":id}},
            "thumbnailRenderer":{"musicThumbnailRenderer":{"thumbnail":{"thumbnails":[{"url":"https://example.test/a.jpg","width":120},{"url":"https://example.test/b.jpg","width":480}]}}}
        }})
    }

    #[test]
    fn followed_library_preserves_identity_text_thumbnail_and_pagination() {
        let root = json!({"contents":{"sectionListRenderer":{"contents":[{"musicShelfRenderer":{
            "contents":[artist("UCdemo"),artist("UCdemo"),artist("MPREalbum")],
            "continuations":[{"nextContinuationData":{"continuation":"next-page"}}]
        }}]}}});
        let page = parse_followed_artists_page(&root).unwrap();
        assert_eq!(page.artists.len(), 1);
        assert_eq!(page.artists[0].browse_id, "UCdemo");
        assert_eq!(page.artists[0].name, "Demo Artist");
        assert_eq!(page.artists[0].subtitle, "12K subscribers");
        assert_eq!(
            page.artists[0].cover_url.as_deref(),
            Some("https://example.test/b.jpg")
        );
        assert_eq!(page.continuation.as_deref(), Some("next-page"));
    }

    #[test]
    fn known_empty_library_is_distinct_from_unknown_and_recommendations() {
        let empty = json!({"contents":{"sectionListRenderer":{"contents":[{"gridRenderer":{"items":[]}}]}}});
        assert!(parse_followed_artists_page(&empty)
            .unwrap()
            .artists
            .is_empty());
        for root in [
            json!({}),
            json!({"contents":{"sectionListRenderer":{"contents":[{"messageRenderer":{"text":{"simpleText":"Sign in"}}}]}}}),
            json!({"contents":{"sectionListRenderer":{"contents":[{"musicCarouselShelfRenderer":{"contents":[artist("UCrecommendation")]}}]}}}),
            json!({"continuationContents":{"musicCarouselShelfContinuation":{"contents":[artist("UCrecommendation")]}}}),
        ] {
            assert!(parse_followed_artists_page(&root).is_none());
        }
    }

    #[test]
    fn legacy_library_uses_third_tab_and_excludes_downloaded_artist_tab() {
        let root = json!({"contents":{"twoColumnBrowseResultsRenderer":{"tabs":[
            {"tabRenderer":{"content":{"sectionListRenderer":{"contents":[{"musicCarouselShelfRenderer":{"contents":[artist("UCrecommended")]}}]}}}},
            {"tabRenderer":{"content":{"sectionListRenderer":{"contents":[{"gridRenderer":{"items":[artist("UCdownloaded")]}}]}}}},
            {"tabRenderer":{"content":{"sectionListRenderer":{"contents":[{"itemSectionRenderer":{"contents":[{"gridRenderer":{"items":[artist("UCfollowed")]}}]}}]}}}}
        ]}}});
        let page = parse_followed_artists_page(&root).unwrap();
        assert_eq!(
            page.artists
                .iter()
                .map(|artist| artist.browse_id.as_str())
                .collect::<Vec<_>>(),
            ["UCfollowed"]
        );
    }

    #[test]
    fn continuation_rows_do_not_import_song_authors() {
        let root = json!({"onResponseReceivedActions":[{"appendContinuationItemsAction":{"continuationItems":[
            {"musicResponsiveListItemRenderer":{"playlistItemData":{"videoId":"song"},"flexColumns":[{"musicResponsiveListItemFlexColumnRenderer":{"text":{"simpleText":"Song"}}},{"musicResponsiveListItemFlexColumnRenderer":{"text":{"runs":[{"text":"Song author","navigationEndpoint":{"browseEndpoint":{"browseId":"UCauthor"}}}]}}}]}},
            {"musicResponsiveListItemRenderer":{"flexColumns":[{"musicResponsiveListItemFlexColumnRenderer":{"text":{"runs":[{"text":"Followed Artist","navigationEndpoint":{"browseEndpoint":{"browseId":"UCfollowed"}}}]}}},{"musicResponsiveListItemFlexColumnRenderer":{"text":{"simpleText":"100 subscribers"}}}]}},
            {"continuationItemRenderer":{"continuationEndpoint":{"continuationCommand":{"token":"next"}}}}
        ]}}]});
        let page = parse_followed_artists_page(&root).unwrap();
        assert_eq!(page.artists.len(), 1);
        assert_eq!(page.artists[0].browse_id, "UCfollowed");
        assert_eq!(page.artists[0].name, "Followed Artist");
        assert_eq!(page.continuation.as_deref(), Some("next"));
    }
}
