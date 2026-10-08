// 歌单打开记录（对齐 Android PlaylistUsageRepository.recordOpen）
//
// 记录写进同步扩展段 playlistUsageStats：首页「继续播放」按它排序，同步后其它设备也能看到。
// 每个来源的 id 必须与 Android 一致，否则两端会各记一条
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const SOURCE_LOCAL_ARTIST: &str = "localArtist";
pub const SOURCE_YOUTUBE_MUSIC: &str = "youtubeMusic";

/// 前端打开歌单时上报的信息
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistOpen {
    pub source: String,
    /// 网易歌单/专辑、本地歌单、B 站收藏夹/合集的 id；YouTube Music 与本地歌手的 id 由其它字段推导
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub cover_url: Option<String>,
    pub track_count: i64,
    #[serde(default)]
    pub fid: Option<String>,
    #[serde(default)]
    pub mid: Option<String>,
    #[serde(default)]
    pub browse_id: Option<String>,
    #[serde(default)]
    pub playlist_id: Option<String>,
    #[serde(default)]
    pub subtype: Option<String>,
    #[serde(default)]
    pub subtitle: Option<String>,
}

/// 解析成同步记录需要的字段
#[derive(Debug, Clone, PartialEq)]
pub struct PlaylistUsageOpen {
    pub key: String,
    pub source: String,
    pub id: i64,
    pub subtype: Option<String>,
    pub name: String,
    pub cover_url: Option<String>,
    pub track_count: i64,
    pub fid: i64,
    pub mid: i64,
    pub browse_id: Option<String>,
    pub playlist_id: Option<String>,
    pub subtitle: Option<String>,
}

impl PlaylistOpen {
    /// 算不出 Android 同款 id 时返回 None，不记录
    pub fn resolve(&self) -> Option<PlaylistUsageOpen> {
        let source = self.source.trim();
        let id = match source {
            SOURCE_YOUTUBE_MUSIC => {
                let value = non_blank(&self.playlist_id).or_else(|| non_blank(&self.browse_id))?;
                stable_youtube_music_id(value)
            }
            SOURCE_LOCAL_ARTIST => {
                if self.name.trim().is_empty() {
                    return None;
                }
                local_artist_stable_id(&self.name)
            }
            _ => self.id.as_deref()?.trim().parse::<i64>().ok().filter(|id| *id != 0)?,
        };
        if source.is_empty() {
            return None;
        }
        let subtype = non_blank(&self.subtype).map(|value| value.trim().to_string());
        Some(PlaylistUsageOpen {
            key: playlist_usage_key(source, id, subtype.as_deref()),
            source: source.to_string(),
            id,
            subtype,
            name: self.name.clone(),
            cover_url: self
                .cover_url
                .as_deref()
                .filter(|url| crate::sync::models::is_shareable_cover_url(url))
                .map(|url| url.trim().to_string()),
            track_count: self.track_count,
            fid: parse_id(&self.fid),
            mid: parse_id(&self.mid),
            browse_id: non_blank(&self.browse_id).map(str::to_string),
            playlist_id: non_blank(&self.playlist_id).map(str::to_string),
            subtitle: non_blank(&self.subtitle).map(str::to_string),
        })
    }
}

/// B 站收藏夹没带类型时按归属判断：自己建的是 CREATED_FAVORITE，别人的是 COLLECTED_FAVORITE
pub fn bili_favorite_kind(open: &PlaylistOpen, own_mid: Option<u64>) -> &'static str {
    let owner = open.mid.as_deref().and_then(|mid| mid.trim().parse::<u64>().ok());
    match (owner, own_mid) {
        (Some(owner), Some(own)) if owner == own => "CREATED_FAVORITE",
        _ => "COLLECTED_FAVORITE",
    }
}

/// Android playlistUsageKey：`source:id[:subtype]`
pub fn playlist_usage_key(source: &str, id: i64, subtype: Option<&str>) -> String {
    match subtype.map(str::trim).filter(|value| !value.is_empty()) {
        Some(subtype) => format!("{source}:{id}:{subtype}"),
        None => format!("{source}:{id}"),
    }
}

/// Android stableYouTubeMusicId：SHA-256 前 8 字节按大端读成有符号 64 位，0 改为 1
pub fn stable_youtube_music_id(value: &str) -> i64 {
    let digest = Sha256::digest(value.as_bytes());
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    match i64::from_be_bytes(bytes) {
        0 => 1,
        id => id,
    }
}

/// Android localArtistStableId：对去空白、转小写后的名字逐个 UTF-16 码元做 FNV-1a 64，取非负
pub fn local_artist_stable_id(name: &str) -> i64 {
    const OFFSET_BASIS: i64 = -3_750_763_034_362_895_579;
    const PRIME: i64 = 1_099_511_628_211;
    name.trim()
        .to_lowercase()
        .encode_utf16()
        .fold(OFFSET_BASIS, |hash, unit| (hash ^ i64::from(unit)).wrapping_mul(PRIME))
        & i64::MAX
}

fn non_blank(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|value| !value.trim().is_empty())
}

fn parse_id(value: &Option<String>) -> i64 {
    value.as_deref().and_then(|value| value.trim().parse().ok()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(source: &str) -> PlaylistOpen {
        PlaylistOpen {
            source: source.into(),
            id: None,
            name: String::new(),
            cover_url: None,
            track_count: 3,
            fid: None,
            mid: None,
            browse_id: None,
            playlist_id: None,
            subtype: None,
            subtitle: None,
        }
    }

    #[test]
    fn derived_ids_match_android() {
        assert_eq!(local_artist_stable_id(" Taylor Swift "), local_artist_stable_id("taylor swift"));
        assert_eq!(local_artist_stable_id(""), 0x4bf2_9ce4_8422_2325, "the FNV offset basis with the sign bit cleared");
        // Android 的常量就是标准 FNV-1a 64 的偏移基与质数（按有符号写出）
        let fnv = |text: &str| {
            text.encode_utf16().fold(0xcbf2_9ce4_8422_2325_u64, |hash, unit| (hash ^ u64::from(unit)).wrapping_mul(0x0000_0100_0000_01b3))
                as i64
                & i64::MAX
        };
        assert_eq!(local_artist_stable_id("周杰伦"), fnv("周杰伦"));
        assert_eq!(stable_youtube_music_id("PLdemo"), {
            let digest = Sha256::digest(b"PLdemo");
            i64::from_be_bytes(digest[..8].try_into().unwrap())
        });
    }

    #[test]
    fn each_source_resolves_the_android_key() {
        let mut netease = open("netease");
        netease.id = Some("123".into());
        assert_eq!(netease.resolve().unwrap().key, "netease:123");

        let mut youtube = open(SOURCE_YOUTUBE_MUSIC);
        youtube.browse_id = Some("VLPLdemo".into());
        youtube.playlist_id = Some(" ".into());
        let resolved = youtube.resolve().unwrap();
        assert_eq!(resolved.id, stable_youtube_music_id("VLPLdemo"), "a blank playlistId falls back to browseId");
        youtube.playlist_id = Some("PLdemo".into());
        assert_eq!(youtube.resolve().unwrap().id, stable_youtube_music_id("PLdemo"));

        let mut artist = open(SOURCE_LOCAL_ARTIST);
        artist.name = "周杰伦".into();
        assert_eq!(artist.resolve().unwrap().key, format!("localArtist:{}", local_artist_stable_id("周杰伦")));

        let mut bili = open("bili");
        bili.id = Some("456".into());
        bili.mid = Some("789".into());
        bili.subtype = Some("COLLECTION".into());
        bili.cover_url = Some("C:\\cover.jpg".into());
        let resolved = bili.resolve().unwrap();
        assert_eq!((resolved.key.as_str(), resolved.mid, resolved.cover_url), ("bili:456:COLLECTION", 789, None));

        let mut folder = open("bili");
        folder.mid = Some("42".into());
        assert_eq!(bili_favorite_kind(&folder, Some(42)), "CREATED_FAVORITE");
        assert_eq!(bili_favorite_kind(&folder, Some(7)), "COLLECTED_FAVORITE");
        assert_eq!(bili_favorite_kind(&folder, None), "COLLECTED_FAVORITE");

        assert!(open("netease").resolve().is_none(), "no id, nothing to record");
        let mut zero = open("local");
        zero.id = Some("0".into());
        assert!(zero.resolve().is_none());
    }
}
