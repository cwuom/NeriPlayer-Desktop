// 同步数据模型：与 Android 端 SyncDataModels.kt 保持 JSON 字段兼容
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const LEGACY_SYNC_METADATA_VERSION: i32 = 0;
pub const CURRENT_SYNC_METADATA_VERSION: i32 = 1;

/// 反序列化辅助：同时接受 string 和 number 类型，统一转为 String
fn deserialize_string_or_number<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de;

    struct StringOrNumber;
    impl<'de> de::Visitor<'de> for StringOrNumber {
        type Value = String;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a string or number")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<String, E> {
            Ok(v.to_string())
        }

        fn visit_string<E: de::Error>(self, v: String) -> Result<String, E> {
            Ok(v)
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<String, E> {
            Ok(v.to_string())
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<String, E> {
            Ok(v.to_string())
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<String, E> {
            Ok(v.to_string())
        }

        fn visit_none<E: de::Error>(self) -> Result<String, E> { Ok(String::new()) }
        fn visit_unit<E: de::Error>(self) -> Result<String, E> { Ok(String::new()) }
    }

    deserializer.deserialize_any(StringOrNumber)
}

/// Option<String> 版本：接受 null / string / number
fn deserialize_opt_string_or_number<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de;

    struct OptStringOrNumber;
    impl<'de> de::Visitor<'de> for OptStringOrNumber {
        type Value = Option<String>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("null, a string, or a number")
        }

        fn visit_none<E: de::Error>(self) -> Result<Option<String>, E> { Ok(None) }
        fn visit_unit<E: de::Error>(self) -> Result<Option<String>, E> { Ok(None) }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Option<String>, E> {
            Ok(if v.is_empty() { None } else { Some(v.to_string()) })
        }
        fn visit_string<E: de::Error>(self, v: String) -> Result<Option<String>, E> {
            Ok(if v.is_empty() { None } else { Some(v) })
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Option<String>, E> {
            Ok(Some(v.to_string()))
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Option<String>, E> {
            Ok(Some(v.to_string()))
        }
        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Option<String>, E> {
            Ok(Some(v.to_string()))
        }
    }

    deserializer.deserialize_any(OptStringOrNumber)
}

fn deserialize_i64_or_default<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de;

    struct I64OrDefault;
    impl<'de> de::Visitor<'de> for I64OrDefault {
        type Value = i64;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("null, an integer, or an integer string")
        }

        fn visit_none<E: de::Error>(self) -> Result<i64, E> { Ok(0) }
        fn visit_unit<E: de::Error>(self) -> Result<i64, E> { Ok(0) }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<i64, E> { Ok(v) }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<i64, E> {
            i64::try_from(v).map_err(E::custom)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<i64, E> {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                Ok(0)
            } else {
                trimmed.parse::<i64>().map_err(E::custom)
            }
        }

        fn visit_string<E: de::Error>(self, v: String) -> Result<i64, E> {
            self.visit_str(&v)
        }
    }

    deserializer.deserialize_any(I64OrDefault)
}

pub(crate) fn sync_i64_from_string(value: &str) -> i64 {
    value.parse::<i64>().unwrap_or(0)
}

fn serialize_string_as_i64<S>(value: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_i64(sync_i64_from_string(value))
}

fn serialize_opt_string_as_i64<S>(value: &Option<String>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value.as_deref().and_then(|v| v.parse::<i64>().ok()) {
        Some(parsed) => serializer.serialize_some(&parsed),
        None => serializer.serialize_none(),
    }
}

fn option_string_is_non_numeric(value: &Option<String>) -> bool {
    value.as_deref().and_then(|v| v.parse::<i64>().ok()).is_none()
}

const SYNC_ACTIONS: &[&str] = &[
    "CREATE_PLAYLIST",
    "DELETE_PLAYLIST",
    "RENAME_PLAYLIST",
    "ADD_SONG",
    "REMOVE_SONG",
    "REORDER_SONGS",
    "PLAY_SONG",
];

fn normalize_sync_action(action: &str) -> &'static str {
    SYNC_ACTIONS
        .iter()
        .copied()
        .find(|candidate| *candidate == action)
        .unwrap_or("CREATE_PLAYLIST")
}

fn serialize_sync_action<S>(action: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(normalize_sync_action(action))
}

pub(crate) fn sync_action_to_code(action: &str) -> i32 {
    SYNC_ACTIONS
        .iter()
        .position(|candidate| *candidate == action)
        .unwrap_or(0) as i32
}

pub(crate) fn sync_action_from_code(code: i32) -> String {
    SYNC_ACTIONS
        .get(code.max(0) as usize)
        .copied()
        .unwrap_or("CREATE_PLAYLIST")
        .to_string()
}

const YOUTUBE_MUSIC_IDENTITY_ALBUM: &str = "youtube_music";
pub(crate) const DISPLAY_ORDER_SONG_ORDER_VERSION: i32 = 1;

pub fn stable_sync_identity_id(value: &str) -> i64 {
    let digest = Sha256::digest(value.as_bytes());
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    let result = i64::from_be_bytes(bytes);
    if result == 0 { 1 } else { result }
}

pub fn build_youtube_music_media_uri(video_id: &str) -> String {
    format!("ytmusic://video/{}", urlencoding::encode(video_id))
}

fn extract_youtube_music_video_id(media_uri: &str) -> Option<String> {
    let raw = media_uri.strip_prefix("ytmusic://video/")?;
    let encoded = raw.split(['?', '#']).next().unwrap_or_default();
    let video_id = urlencoding::decode(encoded).ok()?.into_owned();
    let video_id = video_id.trim();
    if video_id.is_empty() { None } else { Some(video_id.to_string()) }
}

fn normalized_channel_id(raw_channel_id: Option<&str>, album: &str, media_uri: &str) -> Option<String> {
    let channel = raw_channel_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "youtube" | "ytmusic" | "youtubemusic" => YOUTUBE_MUSIC_IDENTITY_ALBUM.to_string(),
            other => other.to_string(),
        });
    if channel.is_some() {
        return channel;
    }

    if extract_youtube_music_video_id(media_uri).is_some() {
        return Some(YOUTUBE_MUSIC_IDENTITY_ALBUM.to_string());
    }
    if album.to_ascii_lowercase().starts_with("bilibili") {
        return Some("bilibili".to_string());
    }
    if album.to_ascii_lowercase().starts_with("netease") || media_uri.trim().is_empty() {
        return Some("netease".to_string());
    }
    None
}

fn normalized_sub_audio_id(channel: &str, raw_sub_audio_id: Option<&str>, album: &str) -> String {
    if channel != "bilibili" {
        return String::new();
    }

    raw_sub_audio_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            album
                .split_once('|')
                .map(|(_, value)| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_default()
}

fn stable_remote_identity_id(channel: &str, audio: &str, sub_audio: &str) -> i64 {
    if channel == "netease" {
        return audio.parse::<i64>().unwrap_or_else(|_| stable_sync_identity_id(&format!("{channel}|{audio}")));
    }

    stable_sync_identity_id(&format!("{channel}|{audio}|{sub_audio}"))
}

pub fn default_history_update_mode() -> String {
    "immediate".into()
}

/// 将桌面端旧值和 Android 端枚举值统一为稳定的内部值
pub fn normalize_history_update_mode(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "every_10_minutes" | "batched" | "batched_10" => "every_10_minutes".into(),
        "every_15_minutes" | "batched_15" => "every_15_minutes".into(),
        "every_30_minutes" | "batched_30" => "every_30_minutes".into(),
        "immediate" | "every_play" | "after_play" => "immediate".into(),
        _ => default_history_update_mode(),
    }
}

/// 同步数据根信封
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncData {
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub device_name: String,
    #[serde(default)]
    pub last_modified: i64,
    #[serde(default)]
    pub playlists: Vec<SyncPlaylist>,
    #[serde(default)]
    pub favorite_playlists: Vec<SyncFavoritePlaylist>,
    #[serde(default)]
    pub recent_plays: Vec<SyncRecentPlay>,
    #[serde(default)]
    pub sync_log: Vec<SyncLogEntry>,
    #[serde(default)]
    pub recent_play_deletions: Vec<SyncRecentPlayDeletion>,
    #[serde(default)]
    pub playback_stats: Vec<SyncTrackStat>,
    #[serde(default)]
    pub playback_stats_cleared_at: i64,
    #[serde(default)]
    pub playback_stat_buckets: Vec<SyncPlaybackStatBucket>,
    #[serde(default)]
    pub playlist_song_deletions: Vec<SyncPlaylistSongDeletion>,
    #[serde(default, flatten)]
    pub extensions: serde_json::Map<String, Value>,
}

fn default_version() -> String { "2.0".into() }

impl SyncData {
    pub fn normalized_for_sync(&self) -> Self {
        let mut normalized = self.clone();
        if normalized.version.is_empty() {
            normalized.version = default_version();
        }
        normalized.playlists = normalized
            .playlists
            .iter()
            .map(|playlist| {
                let mut normalized = playlist.clone();
                normalized.songs = playlist
                    .songs
                    .iter()
                    .map(SyncSong::normalized_for_sync)
                    .collect();
                normalized
            })
            .collect();
        normalized.favorite_playlists = normalized
            .favorite_playlists
            .iter()
            .map(SyncFavoritePlaylist::normalized_for_sync)
            .collect();
        normalized.recent_plays = normalized
            .recent_plays
            .iter()
            .map(|recent| {
                let mut normalized = recent.clone();
                normalized.song = recent.song.normalized_for_sync();
                normalized
            })
            .collect();
        normalized.sync_log = normalized
            .sync_log
            .iter()
            .map(|entry| {
                let mut normalized = entry.clone();
                normalized.action = normalize_sync_action(&entry.action).to_string();
                normalized
            })
            .collect();
        normalized.playlist_song_deletions = normalized
            .playlist_song_deletions
            .iter()
            .map(|deletion| {
                let mut normalized = deletion.clone();
                normalized.removed_membership_tokens = normalize_sync_causal_tokens(
                    &deletion.removed_membership_tokens,
                );
                normalized
            })
            .collect();
        normalized
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlaylist {
    #[serde(
        serialize_with = "serialize_string_as_i64",
        deserialize_with = "deserialize_string_or_number"
    )]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub songs: Vec<SyncSong>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub modified_at: i64,
    #[serde(default)]
    pub is_deleted: bool,
    #[serde(default)]
    pub song_order_version: i32,
}

impl SyncPlaylist {
    pub(crate) fn normalized_for_display_order(&self) -> Self {
        let mut normalized = self.clone();
        if self.is_deleted {
            normalized.songs.clear();
            normalized.song_order_version = DISPLAY_ORDER_SONG_ORDER_VERSION;
            return normalized;
        }

        normalized.songs = if self.song_order_version >= DISPLAY_ORDER_SONG_ORDER_VERSION {
            sorted_songs_by_added_at_for_display(&self.songs)
        } else {
            migrate_legacy_songs_to_display_order(&self.songs, self.modified_at)
        };
        normalized.song_order_version = DISPLAY_ORDER_SONG_ORDER_VERSION;
        normalized
    }
}

/// 锚点必须与设备墙钟无关（对齐 Android 56489bfb 的 P1-1 修复）：
/// 只用歌单自身 modified_at（快照产生时刻）。用墙钟做锚点的话，被抬高
/// 的 added_at 恒大于任何历史 deleted_at，identity 删除墓碑永久失效并被
/// prune 裁剪——已删歌曲复活。
fn migrate_legacy_songs_to_display_order(
    songs: &[SyncSong],
    playlist_modified_at: i64,
) -> Vec<SyncSong> {
    if songs.is_empty() {
        return Vec::new();
    }

    let newest_added_at = songs
        .iter()
        .map(|song| song.added_at)
        .max()
        .unwrap_or(0)
        .max(playlist_modified_at)
        .max(1);

    songs.iter()
        .rev()
        .enumerate()
        .map(|(index, song)| {
            let mut normalized = song.clone();
            normalized.added_at = (newest_added_at - index as i64).max(1);
            normalized.legacy_added_at = normalized.legacy_added_at.or(Some(song.added_at));
            normalized
        })
        .collect()
}

fn sorted_songs_by_added_at_for_display(songs: &[SyncSong]) -> Vec<SyncSong> {
    if songs.len() < 2 {
        return songs.to_vec();
    }

    let mut indexed: Vec<(usize, SyncSong)> = songs.iter().cloned().enumerate().collect();
    indexed.sort_by(|(left_index, left), (right_index, right)| {
        right
            .added_at
            .cmp(&left.added_at)
            .then_with(|| left_index.cmp(right_index))
    });
    indexed.into_iter().map(|(_, song)| song).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncSong {
    #[serde(
        default,
        serialize_with = "serialize_string_as_i64",
        deserialize_with = "deserialize_string_or_number"
    )]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(
        default,
        serialize_with = "serialize_string_as_i64",
        deserialize_with = "deserialize_string_or_number"
    )]
    pub album_id: String,
    #[serde(default)]
    pub duration_ms: i64,
    #[serde(default, deserialize_with = "deserialize_string_or_number", skip_serializing_if = "String::is_empty")]
    pub cover_url: String,
    #[serde(default, deserialize_with = "deserialize_string_or_number", skip_serializing_if = "String::is_empty")]
    pub media_uri: String,
    #[serde(default)]
    pub added_at: i64,
    // 歌词相关
    #[serde(default, rename = "matchedLyric", alias = "lyric", skip_serializing_if = "Option::is_none")]
    pub matched_lyric: Option<String>,
    #[serde(
        default,
        rename = "matchedTranslatedLyric",
        alias = "translatedLyric",
        skip_serializing_if = "Option::is_none"
    )]
    pub matched_translated_lyric: Option<String>,
    #[serde(
        default,
        rename = "matchedLyricSource",
        alias = "lyricSource",
        skip_serializing_if = "Option::is_none"
    )]
    pub matched_lyric_source: Option<String>,
    #[serde(
        default,
        rename = "matchedSongId",
        alias = "lyricSongId",
        deserialize_with = "deserialize_opt_string_or_number",
        skip_serializing_if = "Option::is_none"
    )]
    pub matched_song_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_i64_or_default")]
    pub user_lyric_offset_ms: i64,
    // 自定义覆盖
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_cover_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_artist: Option<String>,
    // 原始元数据
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_cover_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_lyric: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_translated_lyric: Option<String>,
    // 平台相关
    #[serde(default, deserialize_with = "deserialize_opt_string_or_number", skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_string_or_number", skip_serializing_if = "Option::is_none")]
    pub audio_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_string_or_number", skip_serializing_if = "Option::is_none")]
    pub sub_audio_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_string_or_number", skip_serializing_if = "Option::is_none")]
    pub playlist_context_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sync_membership_tokens: Vec<SyncCausalToken>,
    #[serde(default)]
    pub sync_metadata_version: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_added_at: Option<i64>,
    #[serde(default)]
    pub lyric_sync_revision: i64,
    #[serde(default)]
    pub lyric_sync_edited: Option<bool>,
    #[serde(default)]
    pub matched_romanized_lyric: Option<String>,
    #[serde(default)]
    pub original_romanized_lyric: Option<String>,
}

impl SyncSong {
    /// 歌曲唯一标识（与 Android SongIdentity 对齐）
    pub fn identity(&self) -> SongIdentity {
        if let Some(video_id) = extract_youtube_music_video_id(&self.media_uri) {
            return SongIdentity {
                id: stable_sync_identity_id(&video_id).to_string(),
                album: YOUTUBE_MUSIC_IDENTITY_ALBUM.to_string(),
                media_uri: build_youtube_music_media_uri(&video_id),
            };
        }

        let channel = normalized_channel_id(self.channel_id.as_deref(), &self.album, &self.media_uri);
        let audio = self
            .audio_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .or_else(|| if self.id.is_empty() || self.id == "0" { None } else { Some(self.id.clone()) });

        if let (Some(channel), Some(audio)) = (channel, audio) {
            if channel == YOUTUBE_MUSIC_IDENTITY_ALBUM {
                return SongIdentity {
                    id: stable_sync_identity_id(&audio).to_string(),
                    album: YOUTUBE_MUSIC_IDENTITY_ALBUM.to_string(),
                    media_uri: build_youtube_music_media_uri(&audio),
                };
            }

            let sub_audio = normalized_sub_audio_id(&channel, self.sub_audio_id.as_deref(), &self.album);
            return SongIdentity {
                id: stable_remote_identity_id(&channel, &audio, &sub_audio).to_string(),
                album: channel,
                media_uri: String::new(),
            };
        }

        SongIdentity {
            id: self.id.clone(),
            album: self.album.clone(),
            media_uri: self.media_uri.clone(),
        }
    }

    pub fn raw_identity(&self) -> SongIdentity {
        SongIdentity {
            id: self.id.clone(),
            album: self.album.clone(),
            media_uri: self.media_uri.clone(),
        }
    }

    pub fn identity_keys(&self) -> Vec<String> {
        let primary = self.identity().stable_key();
        let raw = self.raw_identity().stable_key();
        if primary == raw {
            vec![primary]
        } else {
            vec![primary, raw]
        }
    }

    pub fn normalized_for_sync(&self) -> Self {
        let mut normalized = self.clone();
        normalized.sync_membership_tokens = normalize_sync_causal_tokens(&self.sync_membership_tokens);
        // 歌词空值与空文本是不同的可恢复状态，不能裁剪原文或尾部换行
        normalized.matched_lyric_source =
            normalize_optional_text(self.matched_lyric_source.as_deref());
        normalized.matched_song_id = normalize_optional_text(self.matched_song_id.as_deref());
        normalized.custom_cover_url = normalize_optional_text(self.custom_cover_url.as_deref());
        normalized.custom_name = normalize_optional_text(self.custom_name.as_deref());
        normalized.custom_artist = normalize_optional_text(self.custom_artist.as_deref());
        normalized.original_name = normalize_optional_text(self.original_name.as_deref());
        normalized.original_artist = normalize_optional_text(self.original_artist.as_deref());
        normalized.original_cover_url = normalize_optional_text(self.original_cover_url.as_deref());
        normalized.channel_id = normalize_optional_text(self.channel_id.as_deref());
        normalized.audio_id = normalize_optional_text(self.audio_id.as_deref());
        normalized.sub_audio_id = normalize_optional_text(self.sub_audio_id.as_deref());
        normalized.playlist_context_id =
            normalize_optional_text(self.playlist_context_id.as_deref());
        normalized
    }
}

/// 其它设备也能打开的封面地址（对齐 Android SyncCoverUrlPolicy）：本机路径、file/content 等 URI，
/// 以及指向本机的 http 地址（含 Tauri 的 asset.localhost）都不算
pub fn is_shareable_cover_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value.trim()) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some_and(|host| {
            let host = host.to_ascii_lowercase();
            host != "localhost" && !host.ends_with(".localhost") && host != "127.0.0.1" && host != "[::1]"
        })
}

impl SyncSong {
    /// 上传用的副本：去掉只在本机有效的封面
    pub fn with_shareable_covers(&self) -> Self {
        let keep = |value: &Option<String>| value.clone().filter(|url| is_shareable_cover_url(url));
        Self {
            cover_url: if is_shareable_cover_url(&self.cover_url) {
                self.cover_url.clone()
            } else {
                String::new()
            },
            custom_cover_url: keep(&self.custom_cover_url),
            original_cover_url: keep(&self.original_cover_url),
            ..self.clone()
        }
    }
}

impl SyncData {
    /// 上传用的副本：歌单、收藏歌单和最近播放里的歌曲都去掉只在本机有效的封面
    pub fn with_shareable_covers(&self) -> Self {
        let mut data = self.clone();
        for song in data
            .playlists
            .iter_mut()
            .flat_map(|playlist| playlist.songs.iter_mut())
            .chain(data.favorite_playlists.iter_mut().flat_map(|playlist| playlist.songs.iter_mut()))
            .chain(data.recent_plays.iter_mut().map(|play| &mut play.song))
        {
            *song = song.with_shareable_covers();
        }
        data
    }
}

/// 空白 optional 文本 -> None (与 Android nonBlank 语义对齐)
fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

/// 歌曲身份标识，用于去重
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct SongIdentity {
    pub id: String,
    pub album: String,
    pub media_uri: String,
}

impl SongIdentity {
    pub fn stable_key(&self) -> String {
        format!("{}|{}|{}", self.id, self.album, self.media_uri)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRecentPlay {
    #[serde(
        default,
        serialize_with = "serialize_string_as_i64",
        deserialize_with = "deserialize_string_or_number"
    )]
    pub song_id: String,
    #[serde(default)]
    pub song: SyncSong,
    #[serde(default)]
    pub played_at: i64,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub resume_position_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRecentPlayDeletion {
    #[serde(
        default,
        serialize_with = "serialize_string_as_i64",
        deserialize_with = "deserialize_string_or_number"
    )]
    pub song_id: String,
    #[serde(default)]
    pub album: String,
    #[serde(default, deserialize_with = "deserialize_string_or_number", skip_serializing_if = "String::is_empty")]
    pub media_uri: String,
    #[serde(default)]
    pub deleted_at: i64,
    #[serde(default)]
    pub device_id: String,
}

impl SyncRecentPlayDeletion {
    pub fn identity(&self) -> SongIdentity {
        SongIdentity {
            id: self.song_id.clone(),
            album: self.album.clone(),
            media_uri: self.media_uri.clone(),
        }
    }

    pub fn identity_keys(&self) -> Vec<String> {
        vec![self.identity().stable_key()]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncFavoritePlaylist {
    #[serde(
        default,
        serialize_with = "serialize_string_as_i64",
        deserialize_with = "deserialize_string_or_number"
    )]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_string_or_number", skip_serializing_if = "String::is_empty")]
    pub cover_url: String,
    #[serde(default)]
    pub track_count: i32,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub songs: Vec<SyncSong>,
    #[serde(default)]
    pub added_time: i64,
    #[serde(default)]
    pub modified_at: i64,
    #[serde(default)]
    pub is_deleted: bool,
    #[serde(default)]
    pub sort_order: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browse_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playlist_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
}

impl SyncFavoritePlaylist {
    /// 分组键
    pub fn group_key(&self) -> String {
        format!("{}_{}", self.id, self.source)
    }

    pub fn normalized_for_sync(&self) -> Self {
        let mut normalized = self.clone();
        if normalized.modified_at <= 0 {
            normalized.modified_at = normalized.added_time.max(0);
        }
        if normalized.sort_order == 0 {
            normalized.sort_order = normalized.added_time.max(0);
        }
        normalized.songs = normalized
            .songs
            .iter()
            .map(SyncSong::normalized_for_sync)
            .collect();
        if normalized.is_deleted {
            normalized.songs.clear();
            normalized.track_count = 0;
        } else {
            normalized.track_count = normalized
                .track_count
                .max(normalized.songs.len().try_into().unwrap_or(i32::MAX));
            if normalized.cover_url.is_empty() {
                normalized.cover_url = normalized
                    .songs
                    .iter()
                    .map(|song| song.cover_url.as_str())
                    .find(|cover| !cover.is_empty())
                    .unwrap_or_default()
                    .to_string();
            }
        }
        normalized
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncLogEntry {
    #[serde(default)]
    pub timestamp: i64,
    #[serde(default)]
    pub device_id: String,
    #[serde(default, serialize_with = "serialize_sync_action")]
    pub action: String,
    #[serde(
        default,
        skip_serializing_if = "option_string_is_non_numeric",
        serialize_with = "serialize_opt_string_as_i64",
        deserialize_with = "deserialize_opt_string_or_number"
    )]
    pub playlist_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "option_string_is_non_numeric",
        serialize_with = "serialize_opt_string_as_i64",
        deserialize_with = "deserialize_opt_string_or_number"
    )]
    pub song_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

/// 因果一致性令牌
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct SyncCausalToken {
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub counter: i64,
}

/// 清理无效令牌并固定输出顺序，避免双端仅因输入顺序不同反复产生变更
pub fn normalize_sync_causal_tokens(tokens: &[SyncCausalToken]) -> Vec<SyncCausalToken> {
    let mut normalized: Vec<SyncCausalToken> = tokens
        .iter()
        .filter(|token| !token.device_id.trim().is_empty() && token.counter > 0)
        .cloned()
        .collect();
    normalized.sort_by(|left, right| {
        left.device_id
            .cmp(&right.device_id)
            .then_with(|| left.counter.cmp(&right.counter))
    });
    normalized.dedup();
    normalized
}

/// 单曲播放统计
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncTrackStat {
    #[serde(default)]
    pub identity_key: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub total_listen_ms: i64,
    #[serde(default)]
    pub play_count: i32,
    #[serde(default)]
    pub last_played_at: i64,
    #[serde(default)]
    pub first_played_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(default)]
    pub duration_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_uri: Option<String>,
    #[serde(
        default,
        serialize_with = "serialize_string_as_i64",
        deserialize_with = "deserialize_string_or_number"
    )]
    pub id: String,
    #[serde(
        default,
        serialize_with = "serialize_string_as_i64",
        deserialize_with = "deserialize_string_or_number"
    )]
    pub album_id: String,
    #[serde(default)]
    pub counter_base_listen_ms: i64,
    #[serde(default)]
    pub counter_base_play_count: i32,
    #[serde(default)]
    pub counter_shards: Vec<SyncPlaybackCounterShard>,
}

/// 播放计数分片
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlaybackCounterShard {
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub epoch_started_at: i64,
    #[serde(default)]
    pub total_listen_ms: i64,
    #[serde(default)]
    pub play_count: i32,
    #[serde(default)]
    pub first_played_at: i64,
    #[serde(default)]
    pub last_played_at: i64,
}

/// 按天分桶的播放统计
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlaybackStatBucket {
    #[serde(default)]
    pub day_start_at: i64,
    #[serde(default)]
    pub identity_key: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub total_listen_ms: i64,
    #[serde(default)]
    pub play_count: i32,
    #[serde(default)]
    pub last_played_at: i64,
    #[serde(default)]
    pub first_played_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(default)]
    pub duration_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_uri: Option<String>,
    #[serde(
        default,
        serialize_with = "serialize_string_as_i64",
        deserialize_with = "deserialize_string_or_number"
    )]
    pub id: String,
    #[serde(
        default,
        serialize_with = "serialize_string_as_i64",
        deserialize_with = "deserialize_string_or_number"
    )]
    pub album_id: String,
    #[serde(default)]
    pub counter_base_listen_ms: i64,
    #[serde(default)]
    pub counter_base_play_count: i32,
    #[serde(default)]
    pub counter_shards: Vec<SyncPlaybackCounterShard>,
}

/// 歌单内歌曲删除记录
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlaylistSongDeletion {
    #[serde(
        default,
        serialize_with = "serialize_string_as_i64",
        deserialize_with = "deserialize_string_or_number"
    )]
    pub playlist_id: String,
    #[serde(
        default,
        serialize_with = "serialize_string_as_i64",
        deserialize_with = "deserialize_string_or_number"
    )]
    pub song_id: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub media_uri: Option<String>,
    #[serde(default)]
    pub deleted_at: i64,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub removed_membership_tokens: Vec<SyncCausalToken>,
}

impl SyncPlaylistSongDeletion {
    pub fn song_identity_key(&self) -> String {
        let song_identity = SongIdentity {
            id: self.song_id.clone(),
            album: self.album.clone(),
            media_uri: self.media_uri.clone().unwrap_or_default(),
        };
        song_identity.stable_key()
    }

    pub fn identity(&self) -> String {
        format!("{}|{}", self.playlist_id, self.song_identity_key())
    }

    pub fn matches_song(&self, playlist_id: &str, song: &SyncSong) -> bool {
        if self.playlist_id != playlist_id {
            return false;
        }
        let deletion_key = self.song_identity_key();
        song.identity_keys().iter().any(|key| key == &deletion_key)
    }
}

/// 同步结果
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub success: bool,
    pub message: String,
    #[serde(default)]
    pub playlists_added: i32,
    #[serde(default)]
    pub playlists_updated: i32,
    #[serde(default)]
    pub playlists_deleted: i32,
    #[serde(default)]
    pub songs_added: i32,
    #[serde(default)]
    pub songs_removed: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<Value>,
    /// 合并结果校正了本地逐曲歌词偏移时，给出新的完整映射
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lyric_offsets: Option<std::collections::BTreeMap<String, i64>>,
    /// 同步期间本地数据有变化，这一轮没有写回任何东西，前端应稍后再同步一次
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deferred: bool,
}

/// 同步配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GitHubSyncConfig {
    #[serde(skip_serializing, default)]
    pub token: String,
    pub owner: String,
    pub repo: String,
    #[serde(default)]
    pub last_remote_sha: String,
    #[serde(default)]
    pub last_sync_time: i64,
    #[serde(default)]
    pub auto_sync: bool,
    #[serde(default = "default_true")]
    pub data_saver: bool,
    #[serde(default)]
    pub silent_failures: bool,
    #[serde(default = "default_history_update_mode")]
    pub history_update_mode: String,
}

fn default_true() -> bool { true }

/// 不绑定具体同步提供商的同步偏好
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPreferencesConfig {
    #[serde(default = "default_history_update_mode")]
    pub history_update_mode: String,
}

impl Default for SyncPreferencesConfig {
    fn default() -> Self {
        Self {
            history_update_mode: default_history_update_mode(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WebDavSyncConfig {
    pub server_url: String,
    pub username: String,
    #[serde(skip_serializing, default)]
    pub password: String,
    #[serde(default)]
    pub base_path: String,
    #[serde(default)]
    pub last_remote_fingerprint: String,
    #[serde(default)]
    pub last_sync_time: i64,
    #[serde(default)]
    pub auto_sync: bool,
    /// 省流模式：与 Android 的全局开关对齐，开启后 WebDAV 也传 GZIP(ProtoBuf)
    #[serde(default = "default_true")]
    pub data_saver: bool,
}

#[cfg(test)]
mod legacy_json_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_covers_other_devices_can_open_are_uploaded() {
        for local in [
            "C:\\Music\\cover.jpg",
            "\\\\nas\\share\\cover.jpg",
            "/storage/emulated/0/cover.jpg",
            "file:///C:/cover.jpg",
            "content://media/external/images/1",
            "http://asset.localhost/C%3A%5Ccover.jpg",
            "http://127.0.0.1:8080/cover.jpg",
            "",
        ] {
            assert!(!is_shareable_cover_url(local), "{local} must stay on this device");
        }
        assert!(is_shareable_cover_url("https://p1.music.126.net/cover.jpg"));
        assert!(is_shareable_cover_url(" http://i0.hdslb.com/cover.jpg "));

        let song = SyncSong {
            id: "1".into(),
            cover_url: "C:\\Music\\cover.jpg".into(),
            custom_cover_url: Some("D:\\custom.png".into()),
            original_cover_url: Some("https://p1.music.126.net/original.jpg".into()),
            ..Default::default()
        };
        let shared = song.with_shareable_covers();
        assert_eq!(shared.cover_url, "");
        assert_eq!(shared.custom_cover_url, None);
        assert_eq!(shared.original_cover_url.as_deref(), Some("https://p1.music.126.net/original.jpg"));
    }

    #[test]
    fn sync_song_json_uses_android_field_contract() {
        let song = SyncSong {
            id: "yt-video".into(),
            album_id: "123".into(),
            legacy_added_at: Some(456),
            matched_lyric: Some("matched".into()),
            matched_translated_lyric: Some("translated".into()),
            matched_lyric_source: Some("NETEASE".into()),
            matched_song_id: Some("song-key".into()),
            original_lyric: Some("original".into()),
            original_translated_lyric: Some("original translated".into()),
            ..Default::default()
        };

        let value = serde_json::to_value(&song).unwrap();
        assert_eq!(value["id"], 0);
        assert_eq!(value["albumId"], 123);
        assert_eq!(value["matchedLyric"], "matched");
        assert_eq!(value["matchedTranslatedLyric"], "translated");
        assert_eq!(value["matchedLyricSource"], "NETEASE");
        assert_eq!(value["matchedSongId"], "song-key");
        assert_eq!(value["originalLyric"], "original");
        assert_eq!(value["originalTranslatedLyric"], "original translated");
        assert_eq!(value["userLyricOffsetMs"], 0);
        assert_eq!(value["syncMetadataVersion"], LEGACY_SYNC_METADATA_VERSION);
        assert_eq!(value["legacyAddedAt"], 456);
        assert!(value.get("lyric").is_none());
        assert!(value.get("coverUrl").is_none());

        let decoded: SyncSong = serde_json::from_value(json!({
            "id": 12,
            "albumId": 34,
            "coverUrl": null,
            "mediaUri": null,
            "lyric": "legacy lyric",
            "translatedLyric": "legacy translated",
            "lyricSource": "QQ_MUSIC",
            "lyricSongId": 56,
            "userLyricOffsetMs": null,
            "legacyAddedAt": 78
        })).unwrap();
        assert_eq!(decoded.id, "12");
        assert_eq!(decoded.album_id, "34");
        assert_eq!(decoded.cover_url, "");
        assert_eq!(decoded.media_uri, "");
        assert_eq!(decoded.matched_lyric.as_deref(), Some("legacy lyric"));
        assert_eq!(decoded.matched_translated_lyric.as_deref(), Some("legacy translated"));
        assert_eq!(decoded.matched_lyric_source.as_deref(), Some("QQ_MUSIC"));
        assert_eq!(decoded.matched_song_id.as_deref(), Some("56"));
        assert_eq!(decoded.user_lyric_offset_ms, 0);
        assert_eq!(decoded.sync_metadata_version, LEGACY_SYNC_METADATA_VERSION);
        assert_eq!(decoded.legacy_added_at, Some(78));
    }

    #[test]
    fn sync_log_json_uses_android_action_and_numeric_ids() {
        let entry = SyncLogEntry {
            timestamp: 100,
            device_id: "desktop".into(),
            action: "REMOVE_SONG".into(),
            playlist_id: Some("7".into()),
            song_id: Some("not-numeric".into()),
            details: None,
        };

        let value = serde_json::to_value(&entry).unwrap();
        assert_eq!(value["action"], "REMOVE_SONG");
        assert_eq!(value["playlistId"], 7);
        assert!(value.get("songId").is_none());
    }

    #[test]
    fn legacy_playlist_order_migrates_like_android_display_order() {
        let playlist = SyncPlaylist {
            id: "1".into(),
            name: "Legacy".into(),
            songs: vec![
                SyncSong { id: "1".into(), added_at: 1, ..Default::default() },
                SyncSong { id: "2".into(), added_at: 2, ..Default::default() },
                SyncSong { id: "3".into(), added_at: 3, ..Default::default() },
            ],
            created_at: 10,
            modified_at: 20,
            is_deleted: false,
            song_order_version: 0,
        };

        let normalized = playlist.normalized_for_display_order();

        assert_eq!(normalized.song_order_version, DISPLAY_ORDER_SONG_ORDER_VERSION);
        assert_eq!(
            normalized.songs.iter().map(|song| song.id.as_str()).collect::<Vec<_>>(),
            vec!["3", "2", "1"]
        );
        // 锚点是歌单自身 modified_at（20），与墙钟无关：
        // 墙钟锚点会让抬高的 added_at 恒大于历史 deleted_at，删除墓碑失效
        assert_eq!(
            normalized.songs.iter().map(|song| song.added_at).collect::<Vec<_>>(),
            vec![20, 19, 18]
        );
        assert_eq!(
            normalized
                .songs
                .iter()
                .map(|song| song.legacy_added_at)
                .collect::<Vec<_>>(),
            vec![Some(3), Some(2), Some(1)]
        );
    }

    #[test]
    fn current_playlist_order_sorts_by_added_at_stably() {
        let playlist = SyncPlaylist {
            id: "1".into(),
            name: "Current".into(),
            songs: vec![
                SyncSong { id: "1".into(), added_at: 20, ..Default::default() },
                SyncSong { id: "2".into(), added_at: 30, ..Default::default() },
                SyncSong { id: "3".into(), added_at: 30, ..Default::default() },
                SyncSong { id: "4".into(), added_at: 10, ..Default::default() },
            ],
            created_at: 10,
            modified_at: 20,
            is_deleted: false,
            song_order_version: DISPLAY_ORDER_SONG_ORDER_VERSION,
        };

        let normalized = playlist.normalized_for_display_order();

        assert_eq!(
            normalized.songs.iter().map(|song| song.id.as_str()).collect::<Vec<_>>(),
            vec!["2", "3", "1", "4"]
        );
    }

    #[test]
    fn history_update_mode_accepts_android_and_legacy_values() {
        assert_eq!(normalize_history_update_mode("IMMEDIATE"), "immediate");
        assert_eq!(normalize_history_update_mode("EVERY_10_MINUTES"), "every_10_minutes");
        assert_eq!(normalize_history_update_mode("EVERY_15_MINUTES"), "every_15_minutes");
        assert_eq!(normalize_history_update_mode("EVERY_30_MINUTES"), "every_30_minutes");
        assert_eq!(normalize_history_update_mode("BATCHED"), "every_10_minutes");
        assert_eq!(normalize_history_update_mode("unknown"), "immediate");
    }

    #[test]
    fn normalized_for_sync_preserves_lyric_text_and_clears_blank_metadata() {
        let song = SyncSong {
            id: "1".into(),
            matched_lyric: Some("   ".into()),
            matched_translated_lyric: Some("".into()),
            original_lyric: Some(" keep ".into()),
            matched_lyric_source: Some("\t".into()),
            custom_name: Some("".into()),
            channel_id: Some(" netease ".into()),
            ..Default::default()
        };

        let normalized = song.normalized_for_sync();
        assert_eq!(normalized.matched_lyric.as_deref(),Some("   "));
        assert_eq!(normalized.matched_translated_lyric.as_deref(),Some(""));
        assert_eq!(normalized.original_lyric.as_deref(), Some(" keep "));
        assert!(normalized.matched_lyric_source.is_none());
        assert!(normalized.custom_name.is_none());
        assert_eq!(normalized.channel_id.as_deref(), Some("netease"));
    }
}
