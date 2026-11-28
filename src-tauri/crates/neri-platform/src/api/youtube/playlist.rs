// YouTube Music 歌单 browse / continuation
use serde_json::{json, Value};

use neri_common::error::{AppError, AppResult};

use super::client::YouTubeClient;
use crate::auth::state::YouTubeAuth;

/// 提取 continuation token（多种布局）
pub fn extract_continuation_token(root: &Value) -> Option<String> {
    // 直接 continuationContents
    if let Some(token) = root
        .pointer("/continuationContents/musicPlaylistShelfContinuation/continuations/0/nextContinuationData/continuation")
        .and_then(|v| v.as_str())
    {
        return Some(token.to_string());
    }
    if let Some(token) = root
        .pointer("/continuationContents/musicShelfContinuation/continuations/0/nextContinuationData/continuation")
        .and_then(|v| v.as_str())
    {
        return Some(token.to_string());
    }

    // section 内 shelf continuations
    let mut stack = vec![root];
    let mut visited = 0usize;
    while let Some(node) = stack.pop() {
        visited += 1;
        if visited > 8000 {
            break;
        }
        if let Some(obj) = node.as_object() {
            if let Some(token) = obj
                .get("nextContinuationData")
                .and_then(|v| v.get("continuation"))
                .and_then(|v| v.as_str())
            {
                if !token.is_empty() {
                    return Some(token.to_string());
                }
            }
            if let Some(token) = obj
                .get("continuationCommand")
                .and_then(|value| value.get("token"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Some(token.to_string());
            }
            for value in obj.values() {
                stack.push(value);
            }
        } else if let Some(arr) = node.as_array() {
            for value in arr {
                stack.push(value);
            }
        }
    }
    None
}

const PLAYLIST_SECTION_PATHS: [&str; 3] = [
    "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents",
    "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents",
    "/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents",
];

/// 定位曲目 shelf（对齐 Android findPlaylistShelfRenderer）
fn find_playlist_shelf(root: &Value) -> Option<&Value> {
    PLAYLIST_SECTION_PATHS
        .iter()
        .filter_map(|path| root.pointer(path).and_then(Value::as_array))
        .flatten()
        .find_map(|section| {
            section
                .get("musicPlaylistShelfRenderer")
                .or_else(|| section.get("musicShelfRenderer"))
        })
        .or_else(|| root.pointer("/continuationContents/musicPlaylistShelfContinuation"))
        .or_else(|| root.pointer("/continuationContents/musicShelfContinuation"))
}

/// 当前页的曲目条目：首屏/旧分页在 shelf.contents，新分页在 appendContinuationItemsAction
fn playlist_page_items(root: &Value) -> Option<&Vec<Value>> {
    find_playlist_shelf(root)
        .and_then(|shelf| shelf.get("contents"))
        .and_then(Value::as_array)
        .or_else(|| {
            root.get("onResponseReceivedActions")?
                .as_array()?
                .iter()
                .find_map(|action| {
                    action
                        .pointer("/appendContinuationItemsAction/continuationItems")
                        .and_then(Value::as_array)
                })
        })
}

/// 只从曲目 shelf 和当前页条目取下一页 token。通用深搜会先命中 sectionListRenderer
/// 上「推荐歌曲」的续页 token，翻页请求拿不到曲目，于是在首页约 100 首处静默结束（#73）
pub fn extract_playlist_continuation_token(root: &Value) -> Option<String> {
    let non_empty = |token: Option<&Value>| {
        token
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(str::to_string)
    };
    find_playlist_shelf(root)
        .and_then(|shelf| shelf.get("continuations"))
        .and_then(Value::as_array)
        .and_then(|continuations| {
            continuations
                .iter()
                .find_map(|item| non_empty(item.pointer("/nextContinuationData/continuation")))
        })
        .or_else(|| {
            playlist_page_items(root)?.iter().find_map(|item| {
                non_empty(item.pointer(
                    "/continuationItemRenderer/continuationEndpoint/continuationCommand/token",
                ))
            })
        })
}

/// 拉取歌单详情，并尽量展开 continuation 分页
pub async fn fetch_playlist_detail_pages(
    client: &YouTubeClient,
    browse_id: &str,
    auth: &YouTubeAuth,
    max_pages: usize,
) -> AppResult<(Value, Option<YouTubeAuth>)> {
    let (root, first_auth) = client
        .get_playlist_detail_with_session(browse_id, auth)
        .await?;
    let latest_auth = parking_lot::Mutex::new(first_auth);
    let root = expand_playlist_continuations(root, max_pages, |token| {
        let latest_auth = &latest_auth;
        async move {
            let current = latest_auth.lock().clone().unwrap_or_else(|| auth.clone());
            let (next, updated) = client
                .continue_playlist_with_session(&token, &current)
                .await?;
            if let Some(updated) = updated {
                *latest_auth.lock() = Some(updated);
            }
            Ok(next)
        }
    })
    .await;
    Ok((root, latest_auth.into_inner()))
}

async fn expand_playlist_continuations<F, Fut>(
    mut root: Value,
    max_pages: usize,
    mut fetch_continuation: F,
) -> Value
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = AppResult<Value>>,
{
    let mut page = 1usize;
    let mut token = extract_playlist_continuation_token(&root);
    let mut seen_tokens = std::collections::HashSet::new();
    while page < max_pages {
        let Some(next_token) = token.take() else {
            break;
        };
        if !seen_tokens.insert(next_token.clone()) {
            break;
        }
        // 对齐 Android / Metrolist：后续页失败只停止翻页并保留已加载的曲目。
        // 整体报错会让前端回退到旧缓存，长歌单又只剩首页约 100 首（#73）
        let next = match fetch_continuation(next_token).await {
            Ok(next) => next,
            Err(error) => {
                log::warn!(
                    target: "youtube",
                    "playlist continuation stopped after page {page}: {error}"
                );
                break;
            }
        };
        // 下一页 token 只从刚收到的页面获取，旧 root 中可能仍有其它 shelf 的 token
        token = extract_playlist_continuation_token(&next);
        merge_continuation_items(&mut root, &next);
        page += 1;
    }
    root
}

fn merge_continuation_items(root: &mut Value, next: &Value) {
    let next_items = playlist_page_items(next).cloned().unwrap_or_default();

    if next_items.is_empty() {
        return;
    }

    // 尝试追加到现有 shelf
    for path in PLAYLIST_SECTION_PATHS {
        if let Some(contents) = root.pointer_mut(path).and_then(|v| v.as_array_mut()) {
            for section in contents.iter_mut() {
                for shelf_key in ["musicPlaylistShelfRenderer", "musicShelfRenderer"] {
                    if let Some(items) = section
                        .pointer_mut(&format!("/{shelf_key}/contents"))
                        .and_then(|v| v.as_array_mut())
                    {
                        items.retain(|item| item.get("continuationItemRenderer").is_none());
                        items.extend(next_items.clone());
                        // 更新 continuation token
                        if let Some(conts) = next
                            .pointer("/continuationContents/musicPlaylistShelfContinuation/continuations")
                            .or_else(|| {
                                next.pointer(
                                    "/continuationContents/musicShelfContinuation/continuations",
                                )
                            })
                        {
                            if let Some(shelf) = section
                                .as_object_mut()
                                .and_then(|obj| obj.get_mut(shelf_key))
                                .and_then(Value::as_object_mut)
                            {
                                shelf.insert("continuations".into(), conts.clone());
                            }
                        } else if let Some(shelf) = section
                            .as_object_mut()
                            .and_then(|obj| obj.get_mut(shelf_key))
                            .and_then(|v| v.as_object_mut())
                        {
                            shelf.remove("continuations");
                        }
                        return;
                    }
                }
            }
        }
    }

    // 兜底: 把 continuation 结果挂到根上, 供前端深搜
    if let Some(obj) = root.as_object_mut() {
        match obj.get_mut("_neriMergedContinuationItems") {
            Some(Value::Array(existing)) => {
                existing.extend(next_items);
            }
            _ => {
                obj.insert("_neriMergedContinuationItems".into(), json!(next_items));
            }
        }
    }
}

pub fn ensure_browse_id(browse_id: &str) -> AppResult<String> {
    let id = browse_id.trim();
    if id.is_empty() {
        return Err(AppError::Api("empty youtube playlist id".into()));
    }
    Ok(normalize_playlist_browse_id(id))
}

/// 歌单 browseId 规范化: 裸 playlistId 需加 VL 前缀才能 browse 到歌单内容
/// 已是完整 browseId 的 VL/MP/FE 前缀保持不变(对齐 Android normalizePlaylistBrowseId)
pub fn normalize_playlist_browse_id(browse_id: &str) -> String {
    let trimmed = browse_id.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("VL")
        || trimmed.starts_with("MP")
        || trimmed.starts_with("FE")
    {
        return trimmed.to_string();
    }
    format!("VL{trimmed}")
}

#[cfg(test)]
mod tests {
    use super::{
        expand_playlist_continuations, extract_continuation_token,
        extract_playlist_continuation_token, merge_continuation_items,
        normalize_playlist_browse_id,
    };
    use serde_json::{json, Value};

    fn fixture(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    fn shelf_video_ids(root: &Value) -> Vec<String> {
        root.pointer("/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicPlaylistShelfRenderer/contents")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(|item| {
                item.pointer("/musicResponsiveListItemRenderer/playlistItemData/videoId")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect()
    }

    #[test]
    fn playlist_token_ignores_section_list_suggestion_continuation() {
        // #73：首屏 sectionListRenderer.continuations 是推荐区续页，不是曲目分页
        let root = fixture(include_str!("fixtures/playlist_first_page.json"));
        assert_eq!(
            extract_playlist_continuation_token(&root).as_deref(),
            Some("SHELF_PAGE_2")
        );
        let next = fixture(include_str!("fixtures/playlist_continuation_page.json"));
        assert_eq!(
            extract_playlist_continuation_token(&next).as_deref(),
            Some("SHELF_PAGE_3")
        );
        let last = fixture(include_str!("fixtures/playlist_last_page.json"));
        assert_eq!(extract_playlist_continuation_token(&last), None);
        let legacy = json!({"continuationContents": {"musicPlaylistShelfContinuation": {"contents": [], "continuations": [{"nextContinuationData": {"continuation": "LEGACY"}}]}}});
        assert_eq!(
            extract_playlist_continuation_token(&legacy).as_deref(),
            Some("LEGACY")
        );
    }

    #[tokio::test]
    async fn playlist_pagination_follows_shelf_tokens_across_all_pages() {
        let root = fixture(include_str!("fixtures/playlist_first_page.json"));
        let mut requested = Vec::new();
        let merged = expand_playlist_continuations(root, 80, |token| {
            requested.push(token.clone());
            let page = match token.as_str() {
                "SHELF_PAGE_2" => Some(include_str!("fixtures/playlist_continuation_page.json")),
                "SHELF_PAGE_3" => Some(include_str!("fixtures/playlist_last_page.json")),
                _ => None,
            };
            async move {
                page.map(fixture).ok_or_else(|| {
                    neri_common::error::AppError::Api(format!("unexpected token {token}"))
                })
            }
        })
        .await;
        assert_eq!(requested, ["SHELF_PAGE_2", "SHELF_PAGE_3"]);
        assert_eq!(shelf_video_ids(&merged), ["v1", "v2", "v3", "v4", "v5"]);
        assert!(!merged.to_string().contains("continuationItemRenderer"));
    }

    #[tokio::test]
    async fn playlist_pagination_keeps_loaded_pages_when_a_later_page_fails() {
        // #73：中途某页失败不能让整次详情报错，否则前端回退到只有首页的旧缓存
        let root = fixture(include_str!("fixtures/playlist_first_page.json"));
        let mut requested = Vec::new();
        let merged = expand_playlist_continuations(root, 80, |token| {
            requested.push(token.clone());
            async move {
                match token.as_str() {
                    "SHELF_PAGE_2" => Ok(fixture(include_str!(
                        "fixtures/playlist_continuation_page.json"
                    ))),
                    _ => Err(neri_common::error::AppError::Api("HTTP 503".into())),
                }
            }
        })
        .await;
        assert_eq!(requested, ["SHELF_PAGE_2", "SHELF_PAGE_3"]);
        assert_eq!(shelf_video_ids(&merged), ["v1", "v2", "v3", "v4"]);
    }

    #[tokio::test]
    async fn playlist_pagination_respects_page_limit() {
        let root = fixture(include_str!("fixtures/playlist_first_page.json"));
        let mut calls = 0;
        let merged = expand_playlist_continuations(root, 2, |_| {
            calls += 1;
            async {
                Ok(fixture(include_str!(
                    "fixtures/playlist_continuation_page.json"
                )))
            }
        })
        .await;
        assert_eq!(calls, 1);
        assert_eq!(shelf_video_ids(&merged), ["v1", "v2", "v3", "v4"]);
    }

    #[test]
    fn extract_token_from_playlist_shelf() {
        let root = json!({
            "continuationContents": {
                "musicPlaylistShelfContinuation": {
                    "continuations": [{
                        "nextContinuationData": { "continuation": "TOKEN123" }
                    }]
                }
            }
        });
        assert_eq!(
            extract_continuation_token(&root).as_deref(),
            Some("TOKEN123")
        );
    }

    #[test]
    fn normalize_prefixes_bare_playlist_id() {
        assert_eq!(normalize_playlist_browse_id("PLabc"), "VLPLabc");
        assert_eq!(normalize_playlist_browse_id("OLAK5uy_x"), "VLOLAK5uy_x");
        assert_eq!(normalize_playlist_browse_id("LM"), "VLLM");
        assert_eq!(normalize_playlist_browse_id("VLPLabc"), "VLPLabc");
        assert_eq!(normalize_playlist_browse_id("MPREb_x"), "MPREb_x");
        assert_eq!(normalize_playlist_browse_id("FEmusic_home"), "FEmusic_home");
    }

    #[test]
    fn recognizes_android_append_action_continuation_command() {
        let root = json!({"onResponseReceivedActions": [{"appendContinuationItemsAction": {"continuationItems": [{"continuationItemRenderer": {"continuationEndpoint": {"continuationCommand": {"token": "NEXT"}}}}]}}]});
        assert_eq!(extract_continuation_token(&root).as_deref(), Some("NEXT"));
        assert!(
            extract_continuation_token(&json!({"continuationCommand": {"token": ""}})).is_none()
        );
    }

    #[test]
    fn merges_pages_without_retaining_previous_continuation_items() {
        let path = "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicPlaylistShelfRenderer";
        let mut root = json!({"contents": {"twoColumnBrowseResultsRenderer": {"secondaryContents": {"sectionListRenderer": {"contents": [{"musicPlaylistShelfRenderer": {"contents": [{"musicResponsiveListItemRenderer": {"playlistItemData": {"videoId": "first"}}}]}}]}}}}});
        let next = json!({"continuationContents": {"musicPlaylistShelfContinuation": {"contents": [{"musicResponsiveListItemRenderer": {"playlistItemData": {"videoId": "second"}}}], "continuations": [{"nextContinuationData": {"continuation": "THIRD"}}]}}});
        merge_continuation_items(&mut root, &next);
        assert_eq!(
            root.pointer(&format!("{path}/contents"))
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            root.pointer(&format!(
                "{path}/continuations/0/nextContinuationData/continuation"
            ))
            .unwrap(),
            "THIRD"
        );
        let final_page = json!({"onResponseReceivedActions": [{"appendContinuationItemsAction": {"continuationItems": [{"musicResponsiveListItemRenderer": {"playlistItemData": {"videoId": "third"}}}]}}]});
        merge_continuation_items(&mut root, &final_page);
        assert_eq!(
            root.pointer(&format!("{path}/contents"))
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert!(root.pointer(&format!("{path}/continuations")).is_none());
    }
}
