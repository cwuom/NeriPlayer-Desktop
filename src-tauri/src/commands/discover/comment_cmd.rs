// 评论（对齐 Android CommentRepository / NeteaseCommentMapper / BiliCommentMapper）：
// 网易云歌曲评论与 B 站视频评论的读取、发表、回复与点赞
use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, State};

use crate::platform_webview::netease_comment_token::mint_comment_token;
use crate::state::AppState;
use neri_common::error::{AppError, AppResult};

const PAGE_SIZE: u32 = 20;
const NETEASE_LENGTH_LIMIT: usize = 140;
const BILIBILI_LENGTH_LIMIT: usize = 1000;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CommentQuote {
    pub user_name: String,
    /// None 表示被回复的评论已删除
    pub content: Option<String>,
}

/// 正文里的内联表情：placeholder 是正文中出现的标记文本（`[doge]` / `[大笑]`）
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CommentEmote {
    pub placeholder: String,
    pub url: String,
}

/// 评论配图；宽高未知时为 0，前端回退 1:1 占位
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CommentImage {
    pub url: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CommentItem {
    pub id: String,
    pub user_name: String,
    pub avatar_url: Option<String>,
    pub user_level: Option<u32>,
    pub content: String,
    pub time_ms: i64,
    pub like_count: u64,
    pub liked: bool,
    /// None 表示接口没给回复数（不能当成 0 条）
    pub reply_count: Option<u64>,
    pub quotes: Vec<CommentQuote>,
    pub preview_replies: Vec<CommentItem>,
    pub emotes: Vec<CommentEmote>,
    pub images: Vec<CommentImage>,
}

#[derive(Debug, Serialize)]
pub struct CommentPage {
    pub items: Vec<CommentItem>,
    pub total: u64,
    pub has_more: bool,
    pub cursor: Option<String>,
}

/// 写操作结果：业务失败按 Android CommentError 分类返回，网络层失败走 Err（结果不确定）
#[derive(Debug, Serialize, PartialEq)]
pub struct CommentActionResult {
    pub ok: bool,
    pub code: Option<i64>,
    /// permission | not_found | closed | server | verification | api
    pub reason: Option<&'static str>,
}

impl CommentActionResult {
    fn success() -> Self {
        Self {
            ok: true,
            code: None,
            reason: None,
        }
    }

    fn failure(reason: &'static str, code: Option<i64>) -> Self {
        Self {
            ok: false,
            code,
            reason: Some(reason),
        }
    }
}

fn https(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    Some(if let Some(rest) = url.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        url.replacen("http://", "https://", 1)
    })
}

fn str_of(value: &Value) -> String {
    value.as_str().map(str::to_string).unwrap_or_else(|| {
        if value.is_number() {
            value.to_string()
        } else {
            String::new()
        }
    })
}

fn u64_of(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
}

fn i64_of(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
}

// ---------- 网易云 ----------

/// 网易云网页版内置表情「名称 -> CDN 图片 id」（对齐 Android NeteaseEmoteCatalog，取自 web core.js）
const NETEASE_EMOTE_IDS: &[(&str, &str)] = &[
    ("大笑", "86"),
    ("可爱", "85"),
    ("憨笑", "359"),
    ("色", "95"),
    ("亲亲", "363"),
    ("惊恐", "96"),
    ("流泪", "356"),
    ("亲", "362"),
    ("呆", "352"),
    ("哀伤", "342"),
    ("呲牙", "343"),
    ("吐舌", "348"),
    ("撇嘴", "353"),
    ("怒", "361"),
    ("奸笑", "341"),
    ("汗", "97"),
    ("痛苦", "346"),
    ("惶恐", "354"),
    ("生病", "350"),
    ("口罩", "351"),
    ("大哭", "357"),
    ("晕", "355"),
    ("发怒", "115"),
    ("开心", "360"),
    ("鬼脸", "94"),
    ("皱眉", "87"),
    ("流感", "358"),
    ("爱心", "33"),
    ("心碎", "34"),
    ("钟情", "303"),
    ("星星", "309"),
    ("生气", "314"),
    ("便便", "89"),
    ("强", "13"),
    ("弱", "372"),
    ("拜", "14"),
    ("牵手", "379"),
    ("跳舞", "380"),
    ("禁止", "374"),
    ("这边", "262"),
    ("爱意", "106"),
    ("示爱", "376"),
    ("嘴唇", "367"),
    ("狗", "81"),
    ("猫", "78"),
    ("猪", "100"),
    ("兔子", "459"),
    ("小鸡", "450"),
    ("公鸡", "461"),
    ("幽灵", "116"),
    ("圣诞", "411"),
    ("外星", "101"),
    ("钻石", "52"),
    ("礼物", "107"),
    ("男孩", "0"),
    ("女孩", "1"),
    ("蛋糕", "337"),
    ("圈", "312"),
    ("叉", "313"),
];

static NETEASE_EMOTES: LazyLock<HashMap<&'static str, String>> = LazyLock::new(|| {
    NETEASE_EMOTE_IDS
        .iter()
        .map(|(name, id)| {
            (
                *name,
                format!("https://s1.music.126.net/style/web2/emt/emoji_{id}.png"),
            )
        })
        .collect()
});

/// 只展开表里有的 `[名称]`，未知标记留作纯文本；同一标记只产出一次
fn netease_emotes(content: &str) -> Vec<CommentEmote> {
    let mut emotes: Vec<CommentEmote> = Vec::new();
    let mut rest = content;
    while let Some(start) = rest.find('[') {
        let after = &rest[start + 1..];
        let Some(end) = after.find(']') else { break };
        let inner = &after[..end];
        if inner.is_empty() || inner.contains('[') {
            rest = after;
            continue;
        }
        if let Some(url) = NETEASE_EMOTES.get(inner) {
            let placeholder = format!("[{inner}]");
            if !emotes.iter().any(|emote| emote.placeholder == placeholder) {
                emotes.push(CommentEmote {
                    placeholder,
                    url: url.clone(),
                });
            }
        }
        rest = &after[end + 1..];
    }
    emotes
}

/// 回复数在老接口放顶层 replyCount，新接口放 showFloorComment.replyCount；都没有时为 None
fn netease_reply_count(comment: &Value) -> Option<u64> {
    u64_of(&comment["replyCount"]).or_else(|| u64_of(&comment["showFloorComment"]["replyCount"]))
}

fn netease_comment(comment: &Value, include_preview: bool) -> Option<CommentItem> {
    let id = u64_of(&comment["commentId"]).filter(|id| *id > 0)?;
    let content = str_of(&comment["content"]);
    let user = &comment["user"];
    Some(CommentItem {
        id: id.to_string(),
        user_name: str_of(&user["nickname"]),
        avatar_url: user["avatarUrl"].as_str().and_then(https),
        user_level: u64_of(&user["level"])
            .filter(|level| *level > 0)
            .map(|level| level as u32),
        time_ms: i64_of(&comment["time"]).unwrap_or(0),
        like_count: u64_of(&comment["likedCount"]).unwrap_or(0),
        liked: comment["liked"].as_bool().unwrap_or(false),
        reply_count: netease_reply_count(comment),
        quotes: comment["beReplied"]
            .as_array()
            .map(|list| {
                list.iter()
                    .map(|quote| CommentQuote {
                        user_name: str_of(&quote["user"]["nickname"]),
                        content: (i64_of(&quote["status"]) != Some(-5)
                            && !quote["content"].is_null())
                        .then(|| str_of(&quote["content"])),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        preview_replies: if include_preview {
            comment["showFloorComment"]["comments"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .filter_map(|reply| netease_comment(reply, false))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        },
        emotes: netease_emotes(&content),
        images: Vec::new(),
        content,
    })
}

fn netease_items(data: &Value) -> Vec<CommentItem> {
    data["comments"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|comment| netease_comment(comment, true))
                .collect()
        })
        .unwrap_or_default()
}

/// 网易云业务码分类（对齐 Android neteaseCommentError；250 是易盾校验未通过）
fn netease_failure(code: i64) -> &'static str {
    match code {
        301 | 315 | 401 | 403 | -460 => "permission",
        250 => "verification",
        404 => "not_found",
        500..=599 => "server",
        _ => "api",
    }
}

fn netease_result(body: &Value) -> CommentActionResult {
    match i64_of(&body["code"]) {
        Some(200) => CommentActionResult::success(),
        code => CommentActionResult::failure(netease_failure(code.unwrap_or(-1)), code),
    }
}

// ---------- B 站 ----------

fn bili_emotes(content: &Value) -> Vec<CommentEmote> {
    content["emote"]
        .as_object()
        .map(|emotes| {
            emotes
                .iter()
                .filter_map(|(placeholder, emote)| {
                    let url = emote["url"].as_str().and_then(https)?;
                    Some(CommentEmote {
                        placeholder: placeholder.clone(),
                        url,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn bili_images(content: &Value) -> Vec<CommentImage> {
    content["pictures"]
        .as_array()
        .map(|pictures| {
            pictures
                .iter()
                .filter_map(|picture| {
                    Some(CommentImage {
                        url: picture["img_src"].as_str().and_then(https)?,
                        width: u64_of(&picture["img_width"]).unwrap_or(0) as u32,
                        height: u64_of(&picture["img_height"]).unwrap_or(0) as u32,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn bili_comment(reply: &Value, include_preview: bool) -> Option<CommentItem> {
    let id = u64_of(&reply["rpid"]).filter(|id| *id > 0)?;
    let member = &reply["member"];
    let content = &reply["content"];
    Some(CommentItem {
        id: id.to_string(),
        user_name: str_of(&member["uname"]),
        avatar_url: member["avatar"].as_str().and_then(https),
        user_level: u64_of(&member["level_info"]["current_level"])
            .filter(|level| *level > 0)
            .map(|level| level as u32),
        content: str_of(&content["message"]),
        time_ms: i64_of(&reply["ctime"]).unwrap_or(0) * 1000,
        like_count: u64_of(&reply["like"]).unwrap_or(0),
        liked: i64_of(&reply["action"]) == Some(1),
        // rcount 为楼中楼数量，缺失时回退 count
        reply_count: u64_of(&reply["rcount"]).or_else(|| u64_of(&reply["count"])),
        quotes: Vec::new(),
        preview_replies: if include_preview {
            reply["replies"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .filter_map(|reply| bili_comment(reply, false))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        },
        emotes: bili_emotes(content),
        images: bili_images(content),
    })
}

fn bili_items(data: &Value) -> Vec<CommentItem> {
    data["replies"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|reply| bili_comment(reply, true))
                .collect()
        })
        .unwrap_or_default()
}

/// B 站业务码分类（对齐 Android biliCommentError）
fn bili_failure(code: i64) -> &'static str {
    match code {
        -101 | -102 | -111 | -403 | -412 | 12004 => "permission",
        -404 | 12006 => "not_found",
        12061 | 12002 => "closed",
        500..=599 => "server",
        _ => "api",
    }
}

fn bili_result(body: &Value) -> CommentActionResult {
    match i64_of(&body["code"]) {
        Some(0) => CommentActionResult::success(),
        code => CommentActionResult::failure(bili_failure(code.unwrap_or(-1)), code),
    }
}

async fn bili_aid(state: &AppState, video: &str) -> AppResult<u64> {
    if let Ok(aid) = video.trim_start_matches("av").parse::<u64>() {
        return Ok(aid);
    }
    let info = state.bilibili().get_video_info(video).await?;
    if info.aid == 0 {
        return Err(AppError::Api("bilibili video has no aid".into()));
    }
    Ok(info.aid)
}

fn parse_id(value: &str, what: &str) -> AppResult<u64> {
    value
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| AppError::Other(format!("invalid {what}")))
}

fn validate_content(content: &str, limit: usize) -> AppResult<()> {
    // 与前端一致按 UTF-16 码元计长度（Android String.length 同口径）
    if content.trim().is_empty() || content.encode_utf16().count() > limit {
        return Err(AppError::Other("invalid comment content".into()));
    }
    Ok(())
}

// ---------- 命令 ----------

/// platform: netease（target = 歌曲 id）| bilibili（target = BV 号或 av 号）
/// sort: hot | newest | recommended（B 站没有推荐，按热门处理）
#[tauri::command]
pub async fn get_comments(
    platform: String,
    target: String,
    sort: String,
    page: Option<u32>,
    cursor: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<CommentPage> {
    let page = page.unwrap_or(1).max(1);
    match platform.as_str() {
        "netease" => {
            let song_id = parse_id(&target, "netease song id")?;
            let sort_type = match sort.as_str() {
                "newest" => 3,
                "recommended" => 99,
                _ => 2,
            };
            let body = state
                .netease()
                .get_song_comments(song_id, page, PAGE_SIZE, sort_type, cursor.as_deref())
                .await?;
            let data = &body["data"];
            let items = netease_items(data);
            Ok(CommentPage {
                total: u64_of(&data["totalCount"]).unwrap_or(0),
                has_more: data["hasMore"].as_bool().unwrap_or(false) && !items.is_empty(),
                cursor: data["cursor"]
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| data["cursor"].as_i64().map(|value| value.to_string())),
                items,
            })
        }
        "bilibili" => {
            let aid = bili_aid(&state, &target).await?;
            let body = state
                .bilibili()
                .get_video_comments(aid, page, PAGE_SIZE, if sort == "newest" { 0 } else { 1 })
                .await?;
            let data = &body["data"];
            let items = bili_items(data);
            let total = u64_of(&data["page"]["count"]).unwrap_or(0);
            Ok(CommentPage {
                has_more: !items.is_empty() && u64::from(page) * u64::from(PAGE_SIZE) < total,
                total,
                cursor: None,
                items,
            })
        }
        other => Err(AppError::Other(format!(
            "comments are not available for {other}"
        ))),
    }
}

/// 楼中楼：网易云用时间游标，B 站用页码（cursor 里放页码）
#[tauri::command]
pub async fn get_comment_replies(
    platform: String,
    target: String,
    comment_id: String,
    cursor: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<CommentPage> {
    let root = parse_id(&comment_id, "comment id")?;
    match platform.as_str() {
        "netease" => {
            let song_id = parse_id(&target, "netease song id")?;
            let body = state
                .netease()
                .get_song_comment_replies(song_id, root, PAGE_SIZE, cursor.as_deref())
                .await?;
            let data = &body["data"];
            let items = netease_items(data);
            // 楼中楼按时间翻页：优先取 data.time，缺失时退回本页最后一条的时间
            let next_cursor = i64_of(&data["time"])
                .filter(|time| *time > 0)
                .or_else(|| {
                    items
                        .last()
                        .map(|item| item.time_ms)
                        .filter(|time| *time > 0)
                })
                .map(|time| time.to_string());
            let has_more = data["hasMore"].as_bool().unwrap_or(false)
                && !items.is_empty()
                && next_cursor.is_some()
                && next_cursor != cursor;
            Ok(CommentPage {
                total: u64_of(&data["totalCount"]).unwrap_or(0),
                has_more,
                cursor: next_cursor,
                items,
            })
        }
        "bilibili" => {
            let aid = bili_aid(&state, &target).await?;
            let page: u32 = cursor
                .as_deref()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1);
            let body = state
                .bilibili()
                .get_video_comment_replies(aid, root, page, PAGE_SIZE)
                .await?;
            let data = &body["data"];
            let items: Vec<CommentItem> = data["replies"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .filter_map(|reply| bili_comment(reply, false))
                        .collect()
                })
                .unwrap_or_default();
            let total = u64_of(&data["page"]["count"]).unwrap_or(0);
            let has_more = !items.is_empty() && u64::from(page) * u64::from(PAGE_SIZE) < total;
            Ok(CommentPage {
                has_more,
                total,
                cursor: has_more.then(|| (page + 1).to_string()),
                items,
            })
        }
        other => Err(AppError::Other(format!(
            "comments are not available for {other}"
        ))),
    }
}

/// 发表评论：root_id / reply_to_id 都为空时是一级评论；回复楼中楼时 root_id 为所在一级评论
#[tauri::command]
pub async fn send_comment(
    platform: String,
    target: String,
    content: String,
    root_id: Option<String>,
    reply_to_id: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<CommentActionResult> {
    let content = content.trim().to_string();
    match platform.as_str() {
        "netease" => {
            validate_content(&content, NETEASE_LENGTH_LIMIT)?;
            let song_id = parse_id(&target, "netease song id")?;
            let reply_to = reply_to_id
                .as_deref()
                .map(|id| parse_id(id, "comment id"))
                .transpose()?;
            if !state.netease().has_comment_login() {
                return Ok(CommentActionResult::failure("permission", Some(301)));
            }
            let Ok(token) = mint_comment_token(&app).await else {
                return Ok(CommentActionResult::failure("verification", None));
            };
            let body = state
                .netease()
                .send_song_comment(song_id, &content, reply_to, &token)
                .await?;
            Ok(netease_result(&body))
        }
        "bilibili" => {
            validate_content(&content, BILIBILI_LENGTH_LIMIT)?;
            let reply = match (root_id.as_deref(), reply_to_id.as_deref()) {
                (Some(root), Some(parent)) => Some((
                    parse_id(root, "comment id")?,
                    parse_id(parent, "comment id")?,
                )),
                (None, None) => None,
                _ => {
                    return Err(AppError::Other(
                        "bilibili replies need both root and parent".into(),
                    ))
                }
            };
            if !state.bilibili().has_comment_login() {
                return Ok(CommentActionResult::failure("permission", Some(-101)));
            }
            let aid = bili_aid(&state, &target).await?;
            let body = state
                .bilibili()
                .send_video_comment(aid, &content, reply)
                .await?;
            Ok(bili_result(&body))
        }
        other => Err(AppError::Other(format!(
            "comments are not available for {other}"
        ))),
    }
}

#[tauri::command]
pub async fn set_comment_liked(
    platform: String,
    target: String,
    comment_id: String,
    liked: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<CommentActionResult> {
    let comment_id = parse_id(&comment_id, "comment id")?;
    match platform.as_str() {
        "netease" => {
            let song_id = parse_id(&target, "netease song id")?;
            if !state.netease().has_comment_login() {
                return Ok(CommentActionResult::failure("permission", Some(301)));
            }
            let Ok(token) = mint_comment_token(&app).await else {
                return Ok(CommentActionResult::failure("verification", None));
            };
            let body = state
                .netease()
                .set_song_comment_liked(song_id, comment_id, liked, &token)
                .await?;
            Ok(netease_result(&body))
        }
        "bilibili" => {
            if !state.bilibili().has_comment_login() {
                return Ok(CommentActionResult::failure("permission", Some(-101)));
            }
            let aid = bili_aid(&state, &target).await?;
            let body = state
                .bilibili()
                .set_video_comment_liked(aid, comment_id, liked)
                .await?;
            Ok(bili_result(&body))
        }
        other => Err(AppError::Other(format!(
            "comments are not available for {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn netease_emotes_expand_known_markers_once() {
        let emotes = netease_emotes("[大笑][大笑] [未知] [[可爱] 文本[");
        assert_eq!(
            emotes
                .iter()
                .map(|emote| emote.placeholder.as_str())
                .collect::<Vec<_>>(),
            ["[大笑]", "[可爱]"]
        );
        assert_eq!(
            emotes[0].url,
            "https://s1.music.126.net/style/web2/emt/emoji_86.png"
        );
        assert!(netease_emotes("没有表情").is_empty());
    }

    #[test]
    fn netease_comment_maps_quotes_preview_and_reply_count() {
        let comment = json!({
            "commentId": 11, "content": "好听[爱心]", "time": 1700000000000i64, "likedCount": 5, "liked": true,
            "user": {"nickname": "A", "avatarUrl": "http://p1.music.126.net/a.jpg"},
            "beReplied": [{"user": {"nickname": "B"}, "content": "原文"}, {"user": {"nickname": "C"}, "content": null, "status": -5}],
            "showFloorComment": {"replyCount": 2, "comments": [{"commentId": 12, "content": "楼中楼", "user": {"nickname": "D"}}]},
        });
        let item = netease_comment(&comment, true).unwrap();
        assert_eq!(item.reply_count, Some(2));
        assert!(item.liked);
        assert_eq!(
            item.avatar_url.as_deref(),
            Some("https://p1.music.126.net/a.jpg")
        );
        assert_eq!(
            item.quotes,
            vec![
                CommentQuote {
                    user_name: "B".into(),
                    content: Some("原文".into())
                },
                CommentQuote {
                    user_name: "C".into(),
                    content: None
                },
            ]
        );
        assert_eq!(item.preview_replies.len(), 1);
        assert_eq!(item.emotes.len(), 1);
        assert_eq!(
            netease_comment(&json!({"commentId": 1, "content": ""}), true)
                .unwrap()
                .reply_count,
            None
        );
    }

    #[test]
    fn bili_comment_maps_emotes_images_level_and_like_state() {
        let reply = json!({
            "rpid": 7, "ctime": 1700000000, "like": 3, "action": 1, "rcount": 4,
            "member": {"uname": "U", "avatar": "//i0.hdslb.com/a.jpg", "level_info": {"current_level": 6}},
            "content": {
                "message": "哈[doge]",
                "emote": {"[doge]": {"url": "//i0.hdslb.com/bfs/emote/doge.png"}, "[bad]": {"url": ""}},
                "pictures": [{"img_src": "//i0.hdslb.com/p.jpg", "img_width": 1200, "img_height": 800}],
            },
            "replies": [{"rpid": 8, "member": {"uname": "R"}, "content": {"message": "回复"}}],
        });
        let item = bili_comment(&reply, true).unwrap();
        assert_eq!(item.time_ms, 1_700_000_000_000);
        assert_eq!(item.user_level, Some(6));
        assert!(item.liked);
        assert_eq!(item.reply_count, Some(4));
        assert_eq!(
            item.emotes,
            vec![CommentEmote {
                placeholder: "[doge]".into(),
                url: "https://i0.hdslb.com/bfs/emote/doge.png".into()
            }]
        );
        assert_eq!(
            item.images,
            vec![CommentImage {
                url: "https://i0.hdslb.com/p.jpg".into(),
                width: 1200,
                height: 800
            }]
        );
        assert_eq!(item.preview_replies[0].id, "8");
        assert!(item.preview_replies[0].preview_replies.is_empty());
    }

    #[test]
    fn action_results_classify_platform_codes() {
        assert_eq!(
            netease_result(&json!({"code": 200})),
            CommentActionResult::success()
        );
        assert_eq!(
            netease_result(&json!({"code": 301})),
            CommentActionResult::failure("permission", Some(301))
        );
        assert_eq!(
            netease_result(&json!({"code": 250})).reason,
            Some("verification")
        );
        assert_eq!(
            bili_result(&json!({"code": 0})),
            CommentActionResult::success()
        );
        assert_eq!(bili_result(&json!({"code": 12061})).reason, Some("closed"));
        assert_eq!(
            bili_result(&json!({"code": -101})).reason,
            Some("permission")
        );
        assert_eq!(bili_result(&json!({})).reason, Some("api"));
    }

    #[test]
    fn content_length_counts_utf16_units() {
        assert!(validate_content("  ", 10).is_err());
        assert!(validate_content(&"字".repeat(140), NETEASE_LENGTH_LIMIT).is_ok());
        assert!(validate_content(&"字".repeat(141), NETEASE_LENGTH_LIMIT).is_err());
        assert!(validate_content(&"😀".repeat(71), NETEASE_LENGTH_LIMIT).is_err());
    }
}
