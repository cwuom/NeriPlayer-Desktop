// B 站分类搜索（对齐 Android BiliClient.searchVideos / searchMusicVideos / searchUploaders）
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

pub const SEARCH_TYPE_URL: &str = "https://api.bilibili.com/x/web-interface/wbi/search/type";

/// 音乐分区：「歌曲」分类即限定在该分区内的视频搜索
pub const MUSIC_ZONE_TID: u32 = 3;

#[derive(Debug, Clone, PartialEq)]
pub struct BiliSearchPage<T> {
    pub page: u32,
    pub num_pages: u32,
    pub items: Vec<T>,
}

impl<T> BiliSearchPage<T> {
    /// 空页即使 numPages 更大也视为到底，避免无限触底加载
    pub fn has_more(&self) -> bool {
        self.page < self.num_pages && !self.items.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BiliSearchVideo {
    pub aid: u64,
    pub bvid: String,
    pub title: String,
    pub author: String,
    pub mid: u64,
    pub cover_url: Option<String>,
    pub duration_sec: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BiliSearchUploader {
    pub mid: u64,
    pub name: String,
    pub face_url: Option<String>,
    pub sign: String,
    pub fans: Option<u64>,
    pub videos: Option<u64>,
}

/// 综合排序、全部时长；tids 为 0 表示全站
pub fn video_search_params(keyword: &str, page: u32, tids: u32) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("search_type".into(), "video".into()),
        ("keyword".into(), keyword.into()),
        ("order".into(), "totalrank".into()),
        ("duration".into(), "0".into()),
        ("tids".into(), tids.to_string()),
        ("page".into(), page.max(1).to_string()),
    ])
}

pub fn uploader_search_params(keyword: &str, page: u32) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("search_type".into(), "bili_user".into()),
        ("keyword".into(), keyword.into()),
        ("page".into(), page.max(1).to_string()),
    ])
}

pub fn parse_video_search(body: &Value, requested_page: u32) -> BiliSearchPage<BiliSearchVideo> {
    let mut seen = HashSet::new();
    let items = results_of(body, "video")
        .filter_map(|item| {
            let aid = item["aid"].as_u64().filter(|aid| *aid > 0)?;
            let title = plain_search_text(search_string(&item["title"]));
            if title.trim().is_empty() || !seen.insert(aid) {
                return None;
            }
            let bvid = Some(search_string(&item["bvid"]).trim())
                .filter(|bvid| !bvid.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| aid_to_bvid(aid));
            Some(BiliSearchVideo {
                aid,
                bvid,
                title,
                author: plain_search_text(search_string(&item["author"])),
                mid: item["mid"].as_u64().unwrap_or(0),
                cover_url: normalize_cover_url(search_string(&item["pic"])),
                duration_sec: parse_duration_sec(item["duration"].as_str().unwrap_or_default()),
            })
        })
        .collect();
    page_of(body, requested_page, items)
}

pub fn parse_uploader_search(
    body: &Value,
    requested_page: u32,
) -> BiliSearchPage<BiliSearchUploader> {
    let mut seen = HashSet::new();
    let items = results_of(body, "bili_user")
        .filter_map(|item| {
            let mid = item["mid"].as_u64().filter(|mid| *mid > 0)?;
            let name = plain_search_text(search_string(&item["uname"]));
            if name.trim().is_empty() || !seen.insert(mid) {
                return None;
            }
            Some(BiliSearchUploader {
                mid,
                name,
                face_url: normalize_cover_url(search_string(&item["upic"])),
                sign: plain_search_text(search_string(&item["usign"])),
                fans: item["fans"].as_u64(),
                videos: item["videos"].as_u64(),
            })
        })
        .collect();
    page_of(body, requested_page, items)
}

fn results_of<'a>(body: &'a Value, kind: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
    // 综合类结果可能混入其它 type，只收本分类的条目
    body["data"]["result"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(move |item| item["type"].as_str() == Some(kind))
}

fn page_of<T>(body: &Value, requested_page: u32, items: Vec<T>) -> BiliSearchPage<T> {
    let data = &body["data"];
    let number = |key: &str| {
        data[key]
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
    };
    BiliSearchPage {
        page: number("page").unwrap_or(requested_page.max(1)),
        num_pages: number("numPages").unwrap_or(1),
        items,
    }
}

/// 只接受 JSON 字符串：null 或数字不应被当成标题
fn search_string(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

/// 去掉关键词高亮标签并解码常见实体；&amp; 最后处理，避免把 &amp;lt; 二次解码成 <
pub fn plain_search_text(raw: &str) -> String {
    static HIGHLIGHT: OnceLock<regex::Regex> = OnceLock::new();
    static HEX_APOS: OnceLock<regex::Regex> = OnceLock::new();
    let highlight = HIGHLIGHT
        .get_or_init(|| regex::Regex::new(r"(?i)</?em\b[^>]*>").expect("bili highlight regex"));
    let hex_apos = HEX_APOS.get_or_init(|| regex::Regex::new(r"(?i)&#x27;").expect("apos regex"));
    let stripped = highlight.replace_all(raw, "");
    hex_apos
        .replace_all(&stripped, "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", "\u{a0}")
        .replace("&amp;", "&")
}

fn normalize_cover_url(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    Some(if let Some(rest) = url.strip_prefix("//") {
        format!("https://{rest}")
    } else if let Some(rest) = url.strip_prefix("http://") {
        format!("https://{rest}")
    } else {
        url.to_string()
    })
}

/// 搜索结果时长是「m:ss」或「h:mm:ss」，个别段不是数字时跳过该段
fn parse_duration_sec(text: &str) -> u64 {
    text.split(':')
        .filter_map(|part| part.trim().parse::<u64>().ok())
        .fold(0, |total, part| total * 60 + part)
}

/// avid 转 BV 号（B 站 2023 年后的新算法），用于结果缺 bvid 时补齐播放标识
pub fn aid_to_bvid(aid: u64) -> String {
    const XOR_CODE: u64 = 23_442_827_791_579;
    const MAX_AID: u64 = 1 << 51;
    const ALPHABET: &[u8] = b"FcwAPNKTMug3GV5Lj7EJnHpWsx4tb8haYeviqBz6rkCy12mUSDQX9RdoZf";
    let mut bytes = *b"BV1000000000";
    let mut index = bytes.len() - 1;
    let mut value = (MAX_AID | aid) ^ XOR_CODE;
    while value > 0 && index > 2 {
        bytes[index] = ALPHABET[(value % 58) as usize];
        value /= 58;
        index -= 1;
    }
    bytes.swap(3, 9);
    bytes.swap(4, 7);
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn music_search_is_scoped_to_music_zone_and_video_search_to_all_zones() {
        let music = video_search_params("Mili", 2, MUSIC_ZONE_TID);
        let video = video_search_params("Mili", 0, 0);
        assert_eq!(music["search_type"], "video");
        assert_eq!(music["tids"], "3");
        assert_eq!(music["page"], "2");
        assert_eq!(music["order"], "totalrank");
        assert_eq!(video["tids"], "0");
        assert_eq!(video["page"], "1", "页码至少从 1 开始");

        let uploader = uploader_search_params("Mili", 2);
        assert_eq!(uploader["search_type"], "bili_user");
        assert_eq!(uploader["keyword"], "Mili");
        assert_eq!(uploader["page"], "2");
        assert!(!uploader.contains_key("tids"));
    }

    #[test]
    fn video_search_keeps_valid_unique_videos_and_derives_missing_bvid() {
        let body = json!({"code": 0, "data": {"page": 2, "numPages": 3, "result": [
            {"type": "video", "aid": 170001, "title": "<em class=\"keyword\">Mili</em> &amp; Co",
             "author": "<em>UP</em>", "mid": 81, "pic": "//i0.hdslb.com/a.jpg", "duration": "1:02:03"},
            {"type": "video", "aid": 170001, "bvid": "BV1dup", "title": "duplicate aid"},
            {"type": "video", "aid": 0, "title": "missing identity"},
            {"type": "video", "aid": 9, "title": null},
            {"type": "ketang", "aid": 10, "title": "other type"},
            {"type": "video", "aid": 11, "bvid": "BV1xx411c7mD", "title": "Second",
             "pic": "http://i1.hdslb.com/b.jpg", "duration": "4:5"}
        ]}});
        let page = parse_video_search(&body, 2);
        assert_eq!(page.page, 2);
        assert_eq!(page.num_pages, 3);
        assert!(page.has_more());
        assert_eq!(
            page.items[0],
            BiliSearchVideo {
                aid: 170001,
                bvid: "BV17x411w7KC".into(),
                title: "Mili & Co".into(),
                author: "UP".into(),
                mid: 81,
                cover_url: Some("https://i0.hdslb.com/a.jpg".into()),
                duration_sec: 3723,
            }
        );
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[1].bvid, "BV1xx411c7mD");
        assert_eq!(
            page.items[1].cover_url.as_deref(),
            Some("https://i1.hdslb.com/b.jpg")
        );
        assert_eq!(page.items[1].duration_sec, 245);
    }

    #[test]
    fn uploader_search_maps_account_identity_and_honors_last_page() {
        let body = json!({"code": 0, "data": {"page": 2, "numPages": 2, "result": [
            {"type": "bili_user", "mid": 9876543210u64, "uname": "<em class='keyword'>Mili</em> &amp; Co",
             "upic": "//i0.hdslb.com/face.jpg", "usign": "I <3 &lt;music&gt; &#39;live&#39; &amp;lt;",
             "fans": 123456, "videos": 42},
            {"type": "bili_user", "mid": 9876543210u64, "uname": "same mid again"},
            {"type": "bili_user", "mid": 0, "uname": "invalid"},
            {"type": "bili_user", "mid": 12, "uname": null},
            {"type": "video", "mid": 15, "uname": "not an account result"},
            null
        ]}});
        let page = parse_uploader_search(&body, 2);
        assert_eq!(page.page, 2);
        assert!(!page.has_more(), "最后一页不再加载");
        assert_eq!(
            page.items,
            vec![BiliSearchUploader {
                mid: 9876543210,
                name: "Mili & Co".into(),
                face_url: Some("https://i0.hdslb.com/face.jpg".into()),
                sign: "I <3 <music> 'live' &lt;".into(),
                fans: Some(123456),
                videos: Some(42),
            }]
        );
    }

    #[test]
    fn empty_or_missing_results_end_paging() {
        let empty = parse_uploader_search(&json!({"code": 0, "data": {"numPages": 5}}), 3);
        assert_eq!(empty.page, 3, "缺 page 时沿用请求页");
        assert!(empty.items.is_empty());
        assert!(!empty.has_more(), "空页不能继续翻页");
        let missing = parse_video_search(&json!({"code": 0, "data": {}}), 1);
        assert_eq!(missing.num_pages, 1);
        assert!(!missing.has_more());
    }

    #[test]
    fn aid_to_bvid_matches_known_pairs() {
        assert_eq!(aid_to_bvid(170001), "BV17x411w7KC");
        assert_eq!(aid_to_bvid(111298867365120), "BV1L9Uoa9EUx");
    }
}
