// 探索页：分类搜索、分享链接识别（对齐 Android ExploreViewModel / ExploreLinkRecognizer）
use serde::Serialize;
use serde_json::Value;
use tauri::State;

use super::search_cmd::{bili_search_rows, strip_bili_highlight, upgrade_youtube_thumbnail_url};
use crate::api::bilibili::client::BiliVideoInfo;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

const NETEASE_PAGE_SIZE: u32 = 30;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExploreItem {
    Song {
        id: String,
        title: String,
        artist: String,
        album: String,
        duration_ms: u64,
        source: String,
        cover_url: Option<String>,
    },
    Playlist {
        platform: String,
        id: String,
        name: String,
        cover_url: Option<String>,
        track_count: u64,
        subtitle: String,
    },
    Artist {
        platform: String,
        id: String,
        name: String,
        cover_url: Option<String>,
        subtitle: String,
    },
    /// 识别到了链接但桌面端打不开（Android 同款提示）
    Notice { reason: String },
}

#[derive(Debug, Serialize)]
pub struct ExplorePage {
    pub items: Vec<ExploreItem>,
    pub has_more: bool,
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
        if value.is_number() { value.to_string() } else { String::new() }
    })
}

// ---------- 网易云 ----------

fn netease_song(song: &Value) -> Option<ExploreItem> {
    let id = song["id"].as_u64()?;
    let artists = song["ar"].as_array().or_else(|| song["artists"].as_array());
    let album = if song["al"].is_object() { &song["al"] } else { &song["album"] };
    Some(ExploreItem::Song {
        id: format!("netease:{id}"),
        title: str_of(&song["name"]),
        artist: artists
            .map(|list| list.iter().filter_map(|artist| artist["name"].as_str()).collect::<Vec<_>>().join(" / "))
            .unwrap_or_default(),
        album: str_of(&album["name"]),
        duration_ms: song["dt"].as_u64().or_else(|| song["duration"].as_u64()).unwrap_or(0),
        source: "netease".into(),
        cover_url: album["picUrl"].as_str().and_then(https),
    })
}

fn netease_playlist(playlist: &Value) -> Option<ExploreItem> {
    let id = playlist["id"].as_u64()?;
    Some(ExploreItem::Playlist {
        platform: "netease".into(),
        id: id.to_string(),
        name: str_of(&playlist["name"]),
        cover_url: playlist["coverImgUrl"].as_str().or_else(|| playlist["picUrl"].as_str()).and_then(https),
        track_count: playlist["trackCount"].as_u64().unwrap_or(0),
        subtitle: str_of(&playlist["creator"]["nickname"]),
    })
}

fn netease_artist(artist: &Value) -> Option<ExploreItem> {
    let id = artist["id"].as_u64()?;
    let songs = artist["musicSize"].as_u64().unwrap_or(0);
    let albums = artist["albumSize"].as_u64().unwrap_or(0);
    Some(ExploreItem::Artist {
        platform: "netease".into(),
        id: id.to_string(),
        name: str_of(&artist["name"]),
        cover_url: artist["picUrl"].as_str().or_else(|| artist["img1v1Url"].as_str()).and_then(https),
        subtitle: if songs + albums > 0 { format!("{songs}|{albums}") } else { String::new() },
    })
}

fn netease_page(body: &Value, kind: &str, offset: u32) -> ExplorePage {
    let result = &body["result"];
    let (list, total_key, mapper): (&str, &str, fn(&Value) -> Option<ExploreItem>) = match kind {
        "playlists" => ("playlists", "playlistCount", netease_playlist),
        "artists" => ("artists", "artistCount", netease_artist),
        _ => ("songs", "songCount", netease_song),
    };
    let raw = result[list].as_array().cloned().unwrap_or_default();
    let items: Vec<ExploreItem> = raw.iter().filter_map(mapper).collect();
    let loaded = u64::from(offset) + raw.len() as u64;
    let has_more = match result[total_key].as_u64() {
        Some(total) => loaded < total && !raw.is_empty(),
        None => raw.len() as u32 >= NETEASE_PAGE_SIZE,
    };
    ExplorePage { items, has_more }
}

// ---------- 分类搜索 ----------

/// platform: netease | youtube | bilibili；kind: songs | playlists | artists | videos | creators
#[tauri::command]
pub async fn explore_search(
    platform: String,
    kind: String,
    query: String,
    page: Option<u32>,
    state: State<'_, AppState>,
) -> AppResult<ExplorePage> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Ok(ExplorePage { items: Vec::new(), has_more: false });
    }
    let page = page.unwrap_or(1).max(1);
    match platform.as_str() {
        "netease" => {
            let search_type = match kind.as_str() {
                "playlists" => 1000,
                "artists" => 100,
                _ => 1,
            };
            let offset = (page - 1) * NETEASE_PAGE_SIZE;
            let body = state.netease().cloudsearch(&query, search_type, NETEASE_PAGE_SIZE, offset).await?;
            Ok(netease_page(&body, &kind, offset))
        }
        "youtube" => {
            let client = state.youtube();
            // YouTube 搜索结果由后端一次取满，前端不分页（对齐 Android searchHasMore=false）
            if page > 1 {
                return Ok(ExplorePage { items: Vec::new(), has_more: false });
            }
            let items = if kind == "creators" {
                client
                    .search_creators(&query)
                    .await?
                    .into_iter()
                    .map(|creator| ExploreItem::Artist {
                        platform: "youtube".into(),
                        id: creator.browse_id,
                        name: creator.title,
                        cover_url: creator.cover_url.as_deref().map(upgrade_youtube_thumbnail_url),
                        subtitle: creator.subtitle,
                    })
                    .collect()
            } else {
                client
                    .search_tracks_of(&query, kind == "videos")
                    .await?
                    .into_iter()
                    .map(|row| ExploreItem::Song {
                        id: format!("youtube:{}", row.video_id),
                        title: row.title,
                        artist: row.artist,
                        album: row.album,
                        duration_ms: row.duration_ms,
                        source: "youtube".into(),
                        cover_url: row.thumbnail_url.as_deref().map(upgrade_youtube_thumbnail_url),
                    })
                    .collect()
            };
            Ok(ExplorePage { items, has_more: false })
        }
        "bilibili" => {
            let body = state.bilibili().search_videos(&query, page).await?;
            let rows = bili_search_rows(&body);
            let pages = body["data"]["numPages"].as_u64().unwrap_or(1);
            let has_more = !rows.is_empty() && u64::from(page) < pages;
            let items = rows
                .into_iter()
                .map(|row| ExploreItem::Song {
                    id: row.id,
                    title: row.title,
                    artist: row.artist,
                    album: row.album,
                    duration_ms: row.duration_ms,
                    source: row.source,
                    cover_url: row.cover_url,
                })
                .collect();
            Ok(ExplorePage { items, has_more })
        }
        other => Err(AppError::Other(format!("unsupported explore platform: {other}"))),
    }
}

// ---------- 分享链接识别 ----------

#[derive(Debug, Clone, PartialEq)]
enum LinkTarget {
    NeteaseSong(u64),
    NeteasePlaylist(u64),
    NeteaseArtist(u64),
    NeteaseShort,
    BiliVideo { bvid: Option<String>, avid: Option<u64>, page: Option<u64>, cid: Option<u64> },
    BiliShort,
    BiliFavorite(u64),
    BiliUploader(u64),
    YouTubeVideo(String),
    YouTubePlaylist(String),
    YouTubeChannel(String),
    Unsupported,
}

/// 从分享文案里取第一个链接，并去掉粘在末尾的中英文标点
fn extract_url(text: &str) -> Option<url::Url> {
    static HTTP: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    // 只收 URL 合法的 ASCII 字符：分享文案常把中文说明紧贴在链接后面
    let regex = HTTP.get_or_init(|| {
        regex::Regex::new(r"(?i)https?://[A-Za-z0-9\-._~:/?#\[\]@!$&'()*+,;=%]+").expect("url regex")
    });
    let raw = regex.find(text)?.as_str();
    let trimmed = raw.trim_end_matches(|c: char| {
        matches!(c, '。' | '，' | '、' | '！' | '？' | '）' | '】' | '」' | '》' | ',' | '.' | '!' | '?' | ')' | ']' | '"' | '\'' | '>')
    });
    url::Url::parse(trimmed).ok()
}

fn query_param(url: &url::Url, key: &str) -> Option<String> {
    url.query_pairs().find(|(name, _)| name == key).map(|(_, value)| value.into_owned()).filter(|value| !value.is_empty())
}

fn recognize(url: &url::Url) -> LinkTarget {
    static BVID: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static AVID: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static MEDIA_LIST: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let bvid_re = BVID.get_or_init(|| regex::Regex::new(r"BV[0-9A-Za-z]{10}").expect("bvid regex"));
    let avid_re = AVID.get_or_init(|| regex::Regex::new(r"(?:/video/av|[?&]aid=)(\d+)").expect("avid regex"));
    let media_list_re = MEDIA_LIST.get_or_init(|| {
        regex::Regex::new(r"(?i)/medialist/(?:detail|play)/(?:ml)?(\d+)").expect("medialist regex")
    });

    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let raw = url.as_str();

    if host == "music.163.com" || host.ends_with(".music.163.com") || host == "y.music.163.com" {
        // 网易云网页链接的路由常在 # 之后：/#/song?id=1
        let route = format!("{}{}", url.path(), url.fragment().map(|fragment| format!("#{fragment}")).unwrap_or_default());
        let id = query_param(url, "id").or_else(|| {
            url.fragment()
                .and_then(|fragment| fragment.split_once('?'))
                .and_then(|(_, query)| url::form_urlencoded::parse(query.as_bytes()).find(|(key, _)| key == "id"))
                .map(|(_, value)| value.into_owned())
        }).and_then(|id| id.parse::<u64>().ok());
        let Some(id) = id else { return LinkTarget::Unsupported };
        if route.contains("/song") {
            return LinkTarget::NeteaseSong(id);
        }
        if route.contains("/playlist") {
            return LinkTarget::NeteasePlaylist(id);
        }
        if route.contains("/artist") {
            return LinkTarget::NeteaseArtist(id);
        }
        return LinkTarget::Unsupported;
    }
    if host == "163cn.tv" {
        return LinkTarget::NeteaseShort;
    }

    let is_bili = host == "bilibili.com" || host.ends_with(".bilibili.com") || host == "b23.tv";
    if is_bili {
        let page = query_param(url, "p").map(|value| value.parse().unwrap_or(0));
        let cid = query_param(url, "cid").map(|value| value.parse().unwrap_or(0));
        if let Some(found) = bvid_re.find(raw) {
            return LinkTarget::BiliVideo { bvid: Some(found.as_str().to_string()), avid: None, page, cid };
        }
        if let Some(captures) = avid_re.captures(raw) {
            let avid = captures[1].parse().ok();
            return LinkTarget::BiliVideo { bvid: None, avid, page, cid };
        }
        if host == "b23.tv" {
            return LinkTarget::BiliShort;
        }
        if let Some(captures) = media_list_re.captures(raw) {
            if let Ok(id) = captures[1].parse() {
                return LinkTarget::BiliFavorite(id);
            }
        }
        if host == "space.bilibili.com" {
            if let Some(fid) = query_param(url, "fid").map(|fid| fid.trim_start_matches("ml").to_string()).and_then(|fid| fid.parse().ok()) {
                return LinkTarget::BiliFavorite(fid);
            }
            // UP 主主页与其合集链接都打开 UP 主页（合集在 UP 主页的合集分区里）
            if let Some(mid) = url.path_segments().and_then(|mut segments| segments.next()).and_then(|mid| mid.parse().ok()) {
                return LinkTarget::BiliUploader(mid);
            }
        }
        return LinkTarget::Unsupported;
    }

    if host == "youtu.be" {
        if let Some(id) = url.path_segments().and_then(|mut segments| segments.next()).filter(|id| !id.is_empty()) {
            return LinkTarget::YouTubeVideo(id.to_string());
        }
        return LinkTarget::Unsupported;
    }
    if host == "youtube.com" || host.ends_with(".youtube.com") || host.ends_with("youtube-nocookie.com") {
        if let Some(video) = query_param(url, "v") {
            return LinkTarget::YouTubeVideo(video);
        }
        let segments: Vec<&str> = url.path_segments().map(|segments| segments.collect()).unwrap_or_default();
        if let (Some(&first), Some(&id)) = (segments.first(), segments.get(1)) {
            if matches!(first, "embed" | "shorts" | "live") && !id.is_empty() {
                return LinkTarget::YouTubeVideo(id.to_string());
            }
            if first == "channel" && id.starts_with("UC") {
                return LinkTarget::YouTubeChannel(id.to_string());
            }
            if first == "browse" && (id.starts_with("UC") || id.starts_with("MPLA")) {
                return LinkTarget::YouTubeChannel(id.to_string());
            }
            if first == "browse" && (id.starts_with("VL") || id.starts_with("MPRE")) {
                return LinkTarget::YouTubePlaylist(id.to_string());
            }
        }
        if let Some(list) = query_param(url, "list") {
            return LinkTarget::YouTubePlaylist(list);
        }
        return LinkTarget::Unsupported;
    }
    LinkTarget::Unsupported
}

async fn expand_short_link(state: &AppState, url: &url::Url) -> AppResult<url::Url> {
    let response = state
        .http()
        .get(url.as_str())
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await?;
    Ok(response.url().clone())
}

async fn youtube_oembed(state: &AppState, target: &str) -> Option<Value> {
    let response = state
        .http()
        .get("https://www.youtube.com/oembed")
        .query(&[("format", "json"), ("url", target)])
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<Value>().await.ok()
}

fn notice(reason: &str) -> ExploreItem {
    ExploreItem::Notice { reason: reason.into() }
}

fn bili_video_explore_item(info: &BiliVideoInfo, page: Option<u64>, cid: Option<u64>) -> ExploreItem {
    if info.bvid.is_empty() {
        return notice("not_found");
    }
    let part = match (cid, page) {
        (Some(cid), _) => info.pages.iter().find(|part| cid > 0 && part.cid == cid),
        (None, Some(page)) => info.pages.iter().find(|part| page > 0 && part.page == page),
        (None, None) => None,
    };
    if (cid.is_some() || page.is_some()) && part.is_none() {
        return notice("not_found");
    }
    ExploreItem::Song {
        id: part.map(|part| format!("bilibili:{}:{}", info.aid, part.cid))
            .unwrap_or_else(|| format!("bilibili:{}", info.bvid)),
        title: match part {
            Some(part) if !part.part.is_empty() => format!("{} - {}", info.title, part.part),
            _ => info.title.clone(),
        },
        artist: info.owner.clone(),
        album: part.map(|part| format!("Bilibili|{}|{}", part.cid, info.bvid)).unwrap_or_default(),
        duration_ms: part.map(|part| part.duration).unwrap_or(info.duration) * 1000,
        source: "bilibili".into(),
        cover_url: https(&info.cover),
    }
}

async fn resolve_target(state: &AppState, target: LinkTarget) -> AppResult<ExploreItem> {
    match target {
        LinkTarget::NeteaseSong(id) => {
            let detail = state.netease().get_song_detail(&[id]).await?;
            Ok(detail["songs"].as_array().and_then(|songs| songs.first()).and_then(netease_song).unwrap_or_else(|| notice("not_found")))
        }
        LinkTarget::NeteasePlaylist(id) => {
            let detail = state.netease().get_playlist(id).await?;
            Ok(netease_playlist(&detail["playlist"]).unwrap_or_else(|| notice("not_found")))
        }
        LinkTarget::NeteaseArtist(id) => {
            let detail = state.netease().get_artist_detail(id).await.unwrap_or(Value::Null);
            let artist = &detail["data"]["artist"];
            Ok(ExploreItem::Artist {
                platform: "netease".into(),
                id: id.to_string(),
                name: artist["name"].as_str().unwrap_or_default().to_string(),
                cover_url: artist["avatar"].as_str().or_else(|| artist["cover"].as_str()).and_then(https),
                subtitle: String::new(),
            })
        }
        LinkTarget::BiliVideo { bvid, avid, page, cid } => {
            let client = state.bilibili();
            let info = match (bvid, avid) {
                (Some(bvid), _) => client.get_video_info(&bvid).await?,
                (None, Some(avid)) => client.get_video_info_by_avid(avid).await?,
                _ => return Ok(notice("not_found")),
            };
            Ok(bili_video_explore_item(&info, page, cid))
        }
        LinkTarget::BiliFavorite(media_id) => {
            let info = state.bilibili().get_fav_folder_info(media_id).await?;
            let data = &info["data"];
            Ok(ExploreItem::Playlist {
                platform: "bilibili".into(),
                id: media_id.to_string(),
                name: strip_bili_highlight(data["title"].as_str().unwrap_or_default()),
                cover_url: data["cover"].as_str().and_then(https),
                track_count: data["media_count"].as_u64().unwrap_or(0),
                subtitle: str_of(&data["upper"]["name"]),
            })
        }
        LinkTarget::BiliUploader(mid) => {
            let profile = state.bilibili().get_uploader_profile(mid).await.unwrap_or(Value::Null);
            let data = &profile["data"];
            Ok(ExploreItem::Artist {
                platform: "bilibili".into(),
                id: mid.to_string(),
                name: str_of(&data["name"]),
                cover_url: data["face"].as_str().and_then(https),
                subtitle: str_of(&data["sign"]),
            })
        }
        LinkTarget::YouTubeVideo(video_id) => {
            let meta = youtube_oembed(state, &format!("https://www.youtube.com/watch?v={video_id}")).await.unwrap_or(Value::Null);
            Ok(ExploreItem::Song {
                id: format!("youtube:{video_id}"),
                title: meta["title"].as_str().unwrap_or(&video_id).to_string(),
                artist: meta["author_name"].as_str().unwrap_or("YouTube").trim_end_matches(" - Topic").to_string(),
                album: String::new(),
                duration_ms: 0,
                source: "youtube".into(),
                cover_url: meta["thumbnail_url"].as_str().and_then(https)
                    .or_else(|| Some(format!("https://i.ytimg.com/vi/{video_id}/hqdefault.jpg"))),
            })
        }
        LinkTarget::YouTubePlaylist(list) => {
            let list_id = list.trim_start_matches("VL").to_string();
            let meta = youtube_oembed(state, &format!("https://www.youtube.com/playlist?list={list_id}")).await.unwrap_or(Value::Null);
            Ok(ExploreItem::Playlist {
                platform: "youtube".into(),
                id: list,
                name: meta["title"].as_str().unwrap_or_default().to_string(),
                cover_url: meta["thumbnail_url"].as_str().and_then(https),
                track_count: 0,
                subtitle: str_of(&meta["author_name"]),
            })
        }
        LinkTarget::YouTubeChannel(browse_id) => Ok(ExploreItem::Artist {
            platform: "youtube".into(),
            id: browse_id,
            name: String::new(),
            cover_url: None,
            subtitle: String::new(),
        }),
        LinkTarget::NeteaseShort | LinkTarget::BiliShort | LinkTarget::Unsupported => Ok(notice("unsupported")),
    }
}

/// 识别粘贴的分享文案或链接。短链先跟随跳转再识别一次
#[tauri::command]
pub async fn resolve_share_link(text: String, state: State<'_, AppState>) -> AppResult<ExploreItem> {
    let Some(url) = extract_url(&text) else {
        return Ok(notice("no_link"));
    };
    let mut target = recognize(&url);
    if matches!(target, LinkTarget::NeteaseShort | LinkTarget::BiliShort) {
        let expanded = expand_short_link(&state, &url).await?;
        target = recognize(&expanded);
    }
    resolve_target(&state, target).await
}

/// 获取视频及全部分 P，保留每个分 P 的 CID 和时长
#[tauri::command]
pub async fn get_bili_video_info(
    bvid: Option<String>,
    avid: Option<u64>,
    state: State<'_, AppState>,
) -> AppResult<BiliVideoInfo> {
    let client = state.bilibili();
    match (bvid.as_deref().map(str::trim).filter(|bvid| !bvid.is_empty()), avid.filter(|avid| *avid > 0)) {
        (Some(bvid), _) => client.get_video_info(bvid).await,
        (None, Some(avid)) => client.get_video_info_by_avid(avid).await,
        _ => Err(AppError::Other("Bilibili video requires a BV or AV ID".into())),
    }
}

// ---------- 创作者 / UP 主解析（播放页点歌手名） ----------

#[derive(Debug, Serialize)]
pub struct CreatorLink {
    pub id: String,
    pub name: String,
    pub cover_url: Option<String>,
}

fn normalize_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// 对齐 Android resolveYouTubeMusicCreators：按分隔符拆出各位歌手，逐个搜创作者并只保留名字完全一致的
#[tauri::command]
pub async fn resolve_youtube_creators(artist: String, state: State<'_, AppState>) -> AppResult<Vec<CreatorLink>> {
    let names: Vec<String> = artist
        .split(['/', '&', '、', ','])
        .flat_map(|part| part.split(" feat. ").flat_map(|part| part.split(" x ")))
        .map(|part| part.trim().trim_end_matches(" - Topic").to_string())
        .filter(|part| !part.is_empty())
        .collect();
    let client = state.youtube();
    let mut links: Vec<CreatorLink> = Vec::new();
    for name in names {
        let wanted = normalize_name(&name);
        for creator in client.search_creators(&name).await.unwrap_or_default() {
            if normalize_name(&creator.title) == wanted && !links.iter().any(|link| link.id == creator.browse_id) {
                links.push(CreatorLink {
                    id: creator.browse_id,
                    name: creator.title,
                    cover_url: creator.cover_url.as_deref().map(upgrade_youtube_thumbnail_url),
                });
                break;
            }
        }
    }
    Ok(links)
}

/// 对齐 Android resolveBiliUploader：取视频信息里的 UP 主
#[tauri::command]
pub async fn resolve_bili_uploader(bvid: String, state: State<'_, AppState>) -> AppResult<Option<CreatorLink>> {
    let info = state.bilibili().get_video_info(&bvid).await?;
    Ok((info.owner_mid > 0).then(|| CreatorLink {
        id: info.owner_mid.to_string(),
        name: info.owner,
        cover_url: https(&info.owner_face),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::search_cmd::parse_clock_duration_ms;

    fn target(text: &str) -> LinkTarget {
        recognize(&extract_url(text).expect("url"))
    }

    #[test]
    fn extracts_url_from_share_text_and_trims_punctuation() {
        let url = extract_url("分享歌曲：「浴室」https://music.163.com/song?id=1234&uct=x。（来自@网易云音乐）").unwrap();
        assert_eq!(url.as_str(), "https://music.163.com/song?id=1234&uct=x");
        assert!(extract_url("没有链接").is_none());
    }

    #[test]
    fn recognizes_netease_routes_including_hash_routes() {
        assert_eq!(target("https://music.163.com/song?id=1"), LinkTarget::NeteaseSong(1));
        assert_eq!(target("https://music.163.com/#/playlist?id=22"), LinkTarget::NeteasePlaylist(22));
        assert_eq!(target("https://y.music.163.com/m/artist?id=3"), LinkTarget::NeteaseArtist(3));
        assert_eq!(target("https://music.163.com/album?id=4"), LinkTarget::Unsupported);
        assert_eq!(target("https://163cn.tv/abcd"), LinkTarget::NeteaseShort);
    }

    #[test]
    fn recognizes_bilibili_videos_favorites_and_uploaders() {
        assert_eq!(
            target("https://www.bilibili.com/video/BV1xx411c7mD?p=2"),
            LinkTarget::BiliVideo { bvid: Some("BV1xx411c7mD".into()), avid: None, page: Some(2), cid: None }
        );
        assert_eq!(target("https://www.bilibili.com/video/av170001"), LinkTarget::BiliVideo { bvid: None, avid: Some(170001), page: None, cid: None });
        assert_eq!(target("https://b23.tv/xyz"), LinkTarget::BiliShort);
        assert_eq!(target("https://www.bilibili.com/medialist/detail/ml123"), LinkTarget::BiliFavorite(123));
        assert_eq!(target("https://space.bilibili.com/42/favlist?fid=ml99"), LinkTarget::BiliFavorite(99));
        assert_eq!(target("https://space.bilibili.com/42"), LinkTarget::BiliUploader(42));
    }

    #[test]
    fn recognizes_bilibili_explicit_pages_and_cids() {
        for (query, page, cid) in [
            ("p=1", Some(1), None),
            ("cid=1002", None, Some(1002)),
            ("p=1&cid=1002", Some(1), Some(1002)),
            ("p=bad", Some(0), None),
            ("cid=bad", None, Some(0)),
        ] {
            assert_eq!(
                target(&format!("https://www.bilibili.com/video/BV14Dn16tELC?{query}")),
                LinkTarget::BiliVideo {
                    bvid: Some("BV14Dn16tELC".into()), avid: None, page, cid
                }
            );
        }
        assert_eq!(
            target("https://www.bilibili.com/video/av114?cid=1002"),
            LinkTarget::BiliVideo { bvid: None, avid: Some(114), page: None, cid: Some(1002) }
        );
    }

    #[test]
    fn recognizes_youtube_videos_playlists_and_channels() {
        assert_eq!(target("https://youtu.be/abcdefghijk?si=1"), LinkTarget::YouTubeVideo("abcdefghijk".into()));
        assert_eq!(target("https://music.youtube.com/watch?v=abc&list=PL1"), LinkTarget::YouTubeVideo("abc".into()));
        assert_eq!(target("https://www.youtube.com/shorts/xyz"), LinkTarget::YouTubeVideo("xyz".into()));
        assert_eq!(target("https://music.youtube.com/playlist?list=PL1"), LinkTarget::YouTubePlaylist("PL1".into()));
        assert_eq!(target("https://www.youtube.com/channel/UCabc"), LinkTarget::YouTubeChannel("UCabc".into()));
        assert_eq!(target("https://www.youtube.com/@someone"), LinkTarget::Unsupported);
    }

    #[test]
    fn netease_page_maps_each_search_type_and_paging() {
        let body = serde_json::json!({"result": {
            "songs": [{"id": 1, "name": "S", "ar": [{"name": "A"}, {"name": "B"}], "al": {"name": "Al", "picUrl": "http://p1.music.126.net/x.jpg"}, "dt": 1000}],
            "songCount": 31,
            "playlists": [{"id": 2, "name": "P", "coverImgUrl": "http://p1/x.jpg", "trackCount": 5, "creator": {"nickname": "C"}}],
            "playlistCount": 1,
            "artists": [{"id": 3, "name": "Ar", "img1v1Url": "http://p1/a.jpg", "musicSize": 10, "albumSize": 2}],
            "artistCount": 1,
        }});
        let songs = netease_page(&body, "songs", 0);
        assert!(songs.has_more);
        assert_eq!(songs.items[0], ExploreItem::Song {
            id: "netease:1".into(), title: "S".into(), artist: "A / B".into(), album: "Al".into(),
            duration_ms: 1000, source: "netease".into(), cover_url: Some("https://p1.music.126.net/x.jpg".into()),
        });
        let playlists = netease_page(&body, "playlists", 0);
        assert!(!playlists.has_more);
        assert!(matches!(&playlists.items[0], ExploreItem::Playlist { id, track_count: 5, .. } if id == "2"));
        let artists = netease_page(&body, "artists", 0);
        assert!(matches!(&artists.items[0], ExploreItem::Artist { subtitle, .. } if subtitle == "10|2"));
    }

    #[test]
    fn bilibili_clock_durations_and_highlights() {
        assert_eq!(parse_clock_duration_ms("3:45"), 225_000);
        assert_eq!(parse_clock_duration_ms("1:02:03"), 3_723_000);
        assert_eq!(strip_bili_highlight("<em class=\"keyword\">浴室</em> &amp; 现场"), "浴室 & 现场");
    }

    fn multipart_bili_video() -> BiliVideoInfo {
        serde_json::from_value(serde_json::json!({
            "bvid": "BV14Dn16tELC", "aid": 114, "title": "世界纯音乐100首", "owner": "UP",
            "cover": "http://i0.hdslb.com/cover.jpg", "cid": 1001, "duration": 600,
            "pages": [
                {"cid": 1001, "page": 1, "part": "第一首", "duration": 120},
                {"cid": 1002, "page": 2, "part": "第二首", "duration": 480}
            ]
        })).unwrap()
    }

    #[test]
    fn bilibili_share_first_page_keeps_its_identity_and_duration() {
        assert!(matches!(
            bili_video_explore_item(&multipart_bili_video(), Some(1), None),
            ExploreItem::Song { id, album, duration_ms: 120_000, .. }
                if id == "bilibili:114:1001" && album == "Bilibili|1001|BV14Dn16tELC"
        ));
    }

    #[test]
    fn bilibili_share_pages_have_distinct_ids() {
        let info = multipart_bili_video();
        let ExploreItem::Song { id: first_id, .. } = bili_video_explore_item(&info, Some(1), None) else {
            panic!("first page should resolve");
        };
        assert!(matches!(
            bili_video_explore_item(&info, Some(2), None),
            ExploreItem::Song { id, album, duration_ms: 480_000, .. }
                if id == "bilibili:114:1002" && id != first_id && album == "Bilibili|1002|BV14Dn16tELC"
        ));
    }

    #[test]
    fn bilibili_share_invalid_explicit_page_does_not_fall_back() {
        let info = multipart_bili_video();
        for page in [0, 3] {
            assert_eq!(bili_video_explore_item(&info, Some(page), None), notice("not_found"));
        }
    }

    #[test]
    fn bilibili_share_without_page_remains_a_parent_video() {
        assert!(matches!(
            bili_video_explore_item(&multipart_bili_video(), None, None),
            ExploreItem::Song { id, title, album, duration_ms: 600_000, .. }
                if id == "bilibili:BV14Dn16tELC" && title == "世界纯音乐100首" && album.is_empty()
        ));
    }

    #[test]
    fn bilibili_share_cid_wins_over_page_number() {
        assert!(matches!(
            bili_video_explore_item(&multipart_bili_video(), Some(1), Some(1002)),
            ExploreItem::Song { id, album, duration_ms: 480_000, .. }
                if id == "bilibili:114:1002" && album == "Bilibili|1002|BV14Dn16tELC"
        ));
    }

    #[test]
    fn bilibili_share_invalid_explicit_cid_does_not_fall_back() {
        let info = multipart_bili_video();
        for cid in [0, 1003] {
            assert_eq!(bili_video_explore_item(&info, Some(1), Some(cid)), notice("not_found"));
        }
    }
}
