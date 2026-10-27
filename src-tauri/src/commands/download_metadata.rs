use crate::error::{AppError, AppResult};
use crate::lyrics::parser::{LyricLine, LyricWord};
use crate::state::{TrackInfo, TrackSource};
use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::{Picture, PictureType};
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, ItemValue, Tag, TagItem, TagType};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DownloadMetadata {
    #[serde(default)]
    pub schema_version: u32,
    pub stable_key: Option<String>,
    pub song_id: Option<i64>,
    pub identity_album: Option<String>,
    pub album: Option<String>,
    pub name: Option<String>,
    pub artist: Option<String>,
    pub cover_url: Option<String>,
    pub original_name: Option<String>,
    pub original_artist: Option<String>,
    pub original_cover_url: Option<String>,
    pub original_lyric: Option<String>,
    pub original_translated_lyric: Option<String>,
    pub original_romanized_lyric: Option<String>,
    pub matched_lyric: Option<String>,
    pub matched_translated_lyric: Option<String>,
    pub matched_romanized_lyric: Option<String>,
    pub custom_name: Option<String>,
    pub custom_artist: Option<String>,
    pub custom_cover_url: Option<String>,
    pub media_uri: Option<String>,
    pub channel_id: Option<String>,
    pub audio_id: Option<String>,
    pub sub_audio_id: Option<String>,
    pub cover_path: Option<String>,
    pub lyric_path: Option<String>,
    pub translated_lyric_path: Option<String>,
    pub romanized_lyric_path: Option<String>,
    #[serde(default)]
    pub duration_ms: u64,
    pub verified_audio_duration_ms: Option<u64>,
    pub download_time_ms: Option<u64>,
    pub download_finalized: Option<bool>,
    pub metadata_embedding_state: Option<String>,
    pub audio_file_name: Option<String>,
    pub restorable_metadata: Option<serde_json::Value>,
}

impl DownloadMetadata {
    pub fn for_track(track: &super::DownloadedTrack) -> Self {
        let source = match track.source.as_str() {
            "netease" => TrackSource::Netease,
            "qq" => TrackSource::Qq,
            "bilibili" => TrackSource::Bilibili,
            "youtube" => TrackSource::Youtube,
            _ => TrackSource::Local,
        };
        let info = TrackInfo {
            id: track.id.clone(),
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration_ms: track.duration_ms,
            source,
            url: String::new(),
            cover_url: track.cover_url.clone(),
            added_at: 0,
            sync_payload: None,
            playlist_key: None,
        };
        let song = crate::sync::manager::tracks_to_sync_songs_pub(&[info])
            .into_iter()
            .next();
        Self {
            schema_version: 6,
            stable_key: song.as_ref().map(|song| song.identity().stable_key()),
            song_id: song.as_ref().and_then(|song| song.id.parse().ok()),
            identity_album: Some(track.album.clone()),
            album: Some(normalized_album(&track.album)),
            name: Some(track.title.clone()),
            artist: Some(track.artist.clone()),
            original_name: Some(track.title.clone()),
            original_artist: Some(track.artist.clone()),
            cover_url: track.cover_url.clone(),
            original_cover_url: track.cover_url.clone(),
            media_uri: song.as_ref().map(|song| song.media_uri.clone()),
            channel_id: song.as_ref().and_then(|song| song.channel_id.clone()),
            audio_id: song.as_ref().and_then(|song| song.audio_id.clone()),
            sub_audio_id: song.as_ref().and_then(|song| song.sub_audio_id.clone()),
            duration_ms: track.duration_ms,
            verified_audio_duration_ms: Some(track.duration_ms),
            download_time_ms: Some(track.downloaded_at),
            download_finalized: Some(true),
            audio_file_name: Path::new(&track.file_path)
                .file_name()
                .map(|name| name.to_string_lossy().to_string()),
            ..Self::default()
        }
    }

    pub fn write(&mut self, audio: &Path) -> AppResult<()> {
        self.restorable_metadata = Some(serde_json::json!({
            "sourceStableKey": self.stable_key,
            "baseline": {
                "title": self.original_name,
                "artist": self.original_artist,
                "album": self.album,
                "coverReference": self.original_cover_url.as_ref().or(self.cover_path.as_ref()),
                "originalLyric": self.original_lyric,
                "translatedLyric": self.original_translated_lyric,
                "romanizedLyric": self.original_romanized_lyric,
            },
            "overrides": {},
            "createdAtMs": self.download_time_ms,
            "updatedAtMs": self.download_time_ms,
        }));
        let path =
            metadata_path(audio).ok_or_else(|| AppError::Metadata("下载文件名无效".into()))?;
        crate::fsutil::atomic_write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

pub(crate) fn metadata_path(audio: &Path) -> Option<PathBuf> {
    let mut name = audio.file_name()?.to_os_string();
    name.push(".npmeta.json");
    Some(audio.with_file_name(name))
}

pub(crate) fn read_metadata(audio: &Path) -> Option<DownloadMetadata> {
    let contents = std::fs::read(metadata_path(audio)?).ok()?;
    serde_json::from_slice(&contents).ok()
}

pub(crate) fn resolve_asset(audio: &Path, reference: &str, directory: &str) -> Option<PathBuf> {
    let root = audio.parent()?;
    let path = Path::new(reference);
    // Android 的文档 URI 与迁移前绝对路径只能用文件名在当前根目录查找
    let candidate = if path.is_absolute() && path.starts_with(root) {
        path.to_path_buf()
    } else {
        let name = path.file_name()?;
        root.join(directory).join(name)
    };
    let resolved = candidate.canonicalize().ok()?;
    let root = root.canonicalize().ok()?;
    (resolved.starts_with(root) && resolved.is_file()).then_some(resolved)
}

pub(crate) fn cover_suffix(stable_key: &str) -> String {
    hex::encode(Sha256::digest(stable_key.as_bytes()))[..8].to_string()
}

pub(crate) fn normalized_album(album: &str) -> String {
    let album = album.trim();
    if album.eq_ignore_ascii_case("Bilibili")
        || album
            .get(..9)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Bilibili|"))
    {
        String::new()
    } else if album
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Netease"))
    {
        album[7..].trim().to_string()
    } else {
        album.to_string()
    }
}

fn timestamp(ms: u64) -> String {
    format!(
        "[{:02}:{:02}.{:02}]",
        ms / 60_000,
        ms / 1_000 % 60,
        ms % 1_000 / 10
    )
}

pub(crate) fn original_lyrics(lines: &[LyricLine]) -> Option<String> {
    let word_timed = lines.iter().any(|line| !line.words.is_empty());
    if !word_timed {
        return super::build_lrc_text(lines);
    }
    let mut output = String::new();
    for line in lines.iter().filter(|line| !line.text.trim().is_empty()) {
        if word_timed {
            output.push_str(&format!("[{},{}]", line.start_ms, line.duration_ms));
            if line.words.is_empty() {
                output.push_str(&line.text);
            } else {
                for LyricWord {
                    start_ms,
                    duration_ms,
                    text,
                } in &line.words
                {
                    output.push_str(&format!("({start_ms},{duration_ms},0){text}"));
                }
            }
        } else {
            output.push_str(&timestamp(line.start_ms));
            output.push_str(line.text.trim());
        }
        output.push('\n');
    }
    (!output.is_empty()).then_some(output)
}

pub(crate) fn romanized_lyrics(lines: &[LyricLine]) -> Option<String> {
    let output: String = lines
        .iter()
        .filter_map(|line| {
            let text = line.roman.as_deref()?.trim();
            (!text.is_empty()).then(|| format!("{}{text}\n", timestamp(line.start_ms)))
        })
        .collect();
    (!output.is_empty()).then_some(output)
}

fn normalize_lyrics_for_embedding(original: &str, standardized: bool) -> String {
    if !standardized {
        return original.to_string();
    }
    let mut converted = false;
    let lines: Vec<_> = original
        .lines()
        .filter_map(|raw| {
            let line = raw.trim();
            if line.starts_with('{') {
                return None;
            }
            let parsed = crate::lyrics::parser::parse_yrc(line);
            if let Some(lyric) = parsed.first() {
                converted = true;
                Some(format!(
                    "{}{}",
                    timestamp(lyric.start_ms),
                    lyric.text.trim()
                ))
            } else {
                Some(raw.to_string())
            }
        })
        .collect();
    if converted {
        lines.join("\n")
    } else {
        original.to_string()
    }
}

struct EmbeddedLine {
    raw: String,
    times: Vec<u64>,
    prefix: String,
    text: String,
}

fn parse_embedded_line(raw: &str) -> EmbeddedLine {
    static TIMESTAMP: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern =
        TIMESTAMP.get_or_init(|| regex::Regex::new(r"\[\d{1,3}:\d{2}(?:[.:]\d{1,3})?\]").unwrap());
    let prefix = pattern
        .find_iter(raw)
        .map(|matched| matched.as_str())
        .collect();
    let times = crate::lyrics::parser::parse_lrc(raw)
        .into_iter()
        .map(|line| line.start_ms)
        .collect();
    EmbeddedLine {
        raw: raw.to_string(),
        times,
        prefix,
        text: pattern.replace_all(raw, "").trim_start().to_string(),
    }
}

fn embedded_lyrics(metadata: &DownloadMetadata, standardized: bool) -> Option<String> {
    let original = normalize_lyrics_for_embedding(
        metadata.original_lyric.as_deref().unwrap_or(""),
        standardized,
    );
    let translation = metadata
        .original_translated_lyric
        .as_deref()
        .map(|text| normalize_lyrics_for_embedding(text, standardized))
        .unwrap_or_default();
    if original.trim().is_empty() {
        return (!translation.trim().is_empty()).then_some(translation);
    }
    if translation.trim().is_empty() {
        return Some(original);
    }
    let sources: Vec<_> = original.trim().lines().map(parse_embedded_line).collect();
    let mut translations: Vec<_> = translation
        .trim()
        .lines()
        .map(parse_embedded_line)
        .collect();
    if translations.iter().all(|line| line.times.is_empty()) {
        let mut timed = sources.iter().filter(|line| !line.times.is_empty());
        for line in &mut translations {
            if !line.raw.trim().is_empty() && !line.raw.trim().starts_with('[') {
                if let Some(source) = timed.next() {
                    line.times = source.times.clone();
                    line.prefix = source.prefix.clone();
                    line.raw = format!("{}{}", line.prefix, line.text);
                }
            }
        }
    }
    if sources
        .iter()
        .chain(&translations)
        .all(|line| line.times.is_empty())
    {
        let mut output = Vec::new();
        for index in 0..sources.len().max(translations.len()) {
            if let Some(line) = sources.get(index) {
                output.push(line.raw.clone());
            }
            if let Some(line) = translations.get(index) {
                output.push(line.raw.clone());
            }
        }
        return Some(output.join("\n"));
    }
    let mut output = Vec::new();
    for source in sources {
        output.push(source.raw);
        let nearest = translations
            .iter()
            .enumerate()
            .filter_map(|(index, line)| {
                let distance = source
                    .times
                    .iter()
                    .flat_map(|time| line.times.iter().map(move |other| time.abs_diff(*other)))
                    .min()?;
                (distance <= 1_500).then_some((distance, index))
            })
            .min();
        if let Some((_, index)) = nearest {
            let translation = translations.remove(index);
            output.push(format!("{}{}", source.prefix, translation.text));
        }
    }
    output.extend(translations.into_iter().map(|line| line.raw));
    Some(output.join("\n"))
}

fn custom_key(tag_type: TagType, name: &str) -> ItemKey {
    ItemKey::Unknown(if tag_type == TagType::Mp4Ilst {
        format!("----:com.apple.iTunes:{name}")
    } else {
        name.to_string()
    })
}

fn insert_custom(tag: &mut Tag, name: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        let key = custom_key(tag.tag_type(), name);
        tag.insert_unchecked(TagItem::new(key, ItemValue::Text(value.to_string())));
    }
}

fn read_cover_picture(path: &Path) -> AppResult<Picture> {
    let bytes = std::fs::read(path)?;
    let mut picture = match Picture::from_reader(&mut Cursor::new(&bytes)) {
        Ok(picture) => picture,
        Err(_) => {
            // 下载封面可能是 WebP，转为标签库和外部播放器支持的 PNG
            let mut reader = image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format()?;
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(8_192);
            limits.max_image_height = Some(8_192);
            limits.max_alloc = Some(128 * 1024 * 1024);
            reader.limits(limits);
            let image = reader
                .decode()
                .map_err(|error| AppError::Metadata(error.to_string()))?;
            let mut encoded = Cursor::new(Vec::new());
            image
                .write_to(&mut encoded, image::ImageFormat::Png)
                .map_err(|error| AppError::Metadata(error.to_string()))?;
            encoded.set_position(0);
            Picture::from_reader(&mut encoded)
                .map_err(|error| AppError::Metadata(error.to_string()))?
        }
    };
    picture.set_pic_type(PictureType::CoverFront);
    Ok(picture)
}

pub(crate) fn prepare_audio_tags(
    audio: &Path,
    metadata: &DownloadMetadata,
    standardized: bool,
) -> AppResult<tempfile::NamedTempFile> {
    let directory = audio
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".tmp");
    std::fs::create_dir_all(&directory)?;
    let prepared = tempfile::Builder::new()
        .prefix("metadata-")
        .suffix(".part")
        .tempfile_in(directory)?;
    std::fs::copy(audio, prepared.path())?;
    let mut tagged = Probe::open(prepared.path())
        .map_err(|error| AppError::Metadata(error.to_string()))?
        .guess_file_type()?
        .read()
        .map_err(|error| AppError::Metadata(error.to_string()))?;
    let tag_type = tagged.primary_tag_type();
    if !tagged.supports_tag_type(tag_type) {
        return Err(AppError::Metadata("当前音频容器不支持标签写入".into()));
    }
    if tagged.primary_tag().is_none() {
        tagged.insert_tag(Tag::new(tag_type));
    }
    let tag = tagged
        .primary_tag_mut()
        .ok_or_else(|| AppError::Metadata("无法创建音频标签".into()))?;
    if let Some(title) = metadata.name.as_ref().filter(|text| !text.trim().is_empty()) {
        tag.set_title(title.clone());
    }
    if let Some(artist) = metadata.artist.as_ref().filter(|text| !text.trim().is_empty()) {
        tag.set_artist(artist.clone());
        tag.insert_text(ItemKey::AlbumArtist, artist.clone());
    }
    if let Some(album) = metadata.album.as_ref().filter(|text| !text.trim().is_empty()) {
        tag.set_album(album.clone());
    }
    let lyrics = embedded_lyrics(metadata, standardized);
    if let Some(lyrics) = &lyrics {
        tag.insert_text(ItemKey::Lyrics, lyrics.clone());
        if tag_type == TagType::Mp4Ilst {
            tag.insert_text(ItemKey::Description, lyrics.clone());
        } else if audio
            .extension()
            .is_some_and(|extension| extension == "aac")
        {
            insert_custom(tag, "DESCRIPTION", Some(&lyrics));
        } else if audio
            .extension()
            .is_some_and(|extension| extension == "mp3")
        {
            insert_custom(tag, "UNSYNCEDLYRICS", Some(&lyrics));
        }
    }
    let original = metadata
        .original_lyric
        .as_deref()
        .map(|lyrics| normalize_lyrics_for_embedding(lyrics, standardized));
    let translated = metadata
        .original_translated_lyric
        .as_deref()
        .map(|lyrics| normalize_lyrics_for_embedding(lyrics, standardized));
    let romanized = metadata
        .original_romanized_lyric
        .as_deref()
        .map(|lyrics| normalize_lyrics_for_embedding(lyrics, standardized));
    insert_custom(tag, "NERI_LYRICS_ORIGINAL", original.as_deref());
    insert_custom(tag, "NERI_LYRICS_TRANSLATED", translated.as_deref());
    insert_custom(tag, "LYRICS_TRANSLATED", translated.as_deref());
    if tag_type != TagType::Mp4Ilst {
        insert_custom(tag, "LYRICS:TRANSLATION", translated.as_deref());
    }
    insert_custom(tag, "NERI_LYRICS_ROMANIZED", romanized.as_deref());
    insert_custom(tag, "NERI_STABLE_KEY", metadata.stable_key.as_deref());
    insert_custom(tag, "NERI_MEDIA_URI", metadata.media_uri.as_deref());
    insert_custom(tag, "NERI_SOURCE", metadata.channel_id.as_deref());
    if tag.comment().is_none() {
        tag.set_comment(serde_json::json!({"app": "NeriPlayer", "stableKey": metadata.stable_key, "mediaUri": metadata.media_uri}).to_string());
    }
    let expected_cover = metadata
        .cover_path
        .as_deref()
        .filter(|reference| !reference.trim().is_empty())
        .map(|reference| {
            let cover = resolve_asset(audio, reference, "Covers")
                .ok_or_else(|| AppError::Metadata("下载封面文件缺失或路径无效".into()))?;
            read_cover_picture(&cover)
        })
        .transpose()?;
    if expected_cover.is_none()
        && metadata.cover_url.as_deref().is_some_and(|url| !url.trim().is_empty())
    {
        return Err(AppError::Metadata("下载封面尚未获取，无法完整嵌入元信息".into()));
    }
    if let Some(picture) = &expected_cover {
        tag.remove_picture_type(PictureType::CoverFront);
        tag.push_picture(picture.clone());
    }
    tagged
        .save_to_path(prepared.path(), WriteOptions::default())
        .map_err(|error| AppError::Metadata(error.to_string()))?;
    prepared.as_file().sync_all()?;
    let verified = Probe::open(prepared.path())
        .map_err(|error| AppError::Metadata(error.to_string()))?
        .guess_file_type()?
        .read()
        .map_err(|error| AppError::Metadata(error.to_string()))?;
    let tag = verified
        .primary_tag()
        .ok_or_else(|| AppError::Metadata("写入的音频标签无法读回".into()))?;
    // 分片 MP4 的头部时长可能为零，仍需确认写标签后的音频可解码且时长可信
    let has_verified_duration = !verified.properties().duration().is_zero()
        || super::validate_download_audio(
            prepared.path(),
            None,
            None,
            metadata
                .verified_audio_duration_ms
                .unwrap_or(metadata.duration_ms),
            &AtomicBool::new(false),
        )? > 0;
    for (key, expected) in [
        (ItemKey::TrackTitle, metadata.name.as_deref()),
        (ItemKey::TrackArtist, metadata.artist.as_deref()),
        (ItemKey::AlbumArtist, metadata.artist.as_deref()),
        (ItemKey::AlbumTitle, metadata.album.as_deref()),
        (ItemKey::Lyrics, lyrics.as_deref()),
        (custom_key(tag_type, "NERI_LYRICS_ORIGINAL"), original.as_deref()),
        (custom_key(tag_type, "NERI_LYRICS_TRANSLATED"), translated.as_deref()),
        (custom_key(tag_type, "NERI_LYRICS_ROMANIZED"), romanized.as_deref()),
        (custom_key(tag_type, "NERI_STABLE_KEY"), metadata.stable_key.as_deref()),
        (custom_key(tag_type, "NERI_MEDIA_URI"), metadata.media_uri.as_deref()),
        (custom_key(tag_type, "NERI_SOURCE"), metadata.channel_id.as_deref()),
    ] {
        if let Some(expected) = expected.filter(|text| !text.trim().is_empty()) {
            if tag.get_string(&key) != Some(expected) {
                return Err(AppError::Metadata(format!("音频标签写入验证失败：{key:?}")));
            }
        }
    }
    if !has_verified_duration || expected_cover.as_ref().is_some_and(|expected| {
            !tag.pictures().iter().any(|picture| {
                // MP4 的 covr 不保存封面类型，回读时按实际图片数据校验
                (tag_type == TagType::Mp4Ilst || picture.pic_type() == PictureType::CoverFront)
                    && picture.data() == expected.data()
            })
        })
    {
        return Err(AppError::Metadata("音频标签写入验证失败".into()));
    }
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(audio: &Path) -> DownloadMetadata {
        std::fs::write(audio, include_bytes!("../audio/fixtures/hls-silence.aac")).unwrap();
        DownloadMetadata {
            schema_version: 6,
            stable_key: Some("42|Netease专辑|".into()),
            name: Some("测试歌曲".into()),
            artist: Some("测试歌手".into()),
            album: Some("测试专辑".into()),
            original_lyric: Some("[ar:测试歌手]\n[1000,2000](1000,500,0)你(1500,500,0)好".into()),
            original_translated_lyric: Some("[00:01.50]Hello".into()),
            original_romanized_lyric: Some("[00:01.00]ni hao".into()),
            ..DownloadMetadata::default()
        }
    }

    #[test]
    fn standardization_keeps_metadata_and_external_players_receive_bilingual_lrc() {
        let root = tempfile::tempdir().unwrap();
        let audio = root.path().join("Song.aac");
        let metadata = fixture(&audio);
        assert_eq!(
            normalize_lyrics_for_embedding(metadata.original_lyric.as_deref().unwrap(), true),
            "[ar:测试歌手]\n[00:01.00]你好"
        );
        assert_eq!(
            embedded_lyrics(&metadata, true).unwrap(),
            "[ar:测试歌手]\n[00:01.00]你好\n[00:01.00]Hello"
        );
        assert!(embedded_lyrics(&metadata, false)
            .unwrap()
            .contains("[1000,2000](1000,500,0)你"));
    }

    #[test]
    fn audio_tag_copy_roundtrips_metadata_cover_and_original_lyrics_without_touching_audio() {
        let root = tempfile::tempdir().unwrap();
        let audio = root.path().join("Song.aac");
        let mut metadata = fixture(&audio);
        let original = std::fs::read(&audio).unwrap();
        let cover = root.path().join("Covers").join("Song.jpg");
        std::fs::create_dir_all(cover.parent().unwrap()).unwrap();
        image::RgbImage::from_pixel(2, 2, image::Rgb([40, 80, 120]))
            .save(&cover)
            .unwrap();
        metadata.cover_path = Some(cover.to_string_lossy().to_string());
        let prepared = prepare_audio_tags(&audio, &metadata, false).unwrap();
        assert_eq!(std::fs::read(&audio).unwrap(), original);
        let tagged = Probe::open(prepared.path())
            .unwrap()
            .guess_file_type()
            .unwrap()
            .read()
            .unwrap();
        let tag = tagged.primary_tag().unwrap();
        assert_eq!(tag.title().as_deref(), metadata.name.as_deref());
        assert_eq!(
            tag.get_string(&custom_key(tag.tag_type(), "NERI_LYRICS_ORIGINAL")),
            metadata.original_lyric.as_deref()
        );
        assert_eq!(
            tag.get_string(&custom_key(tag.tag_type(), "NERI_LYRICS_TRANSLATED")),
            metadata.original_translated_lyric.as_deref()
        );
        assert_eq!(tag.pictures().len(), 1);
        assert_eq!(tag.pictures()[0].pic_type(), PictureType::CoverFront);
        prepared.persist(&audio).unwrap();
        let final_size = std::fs::metadata(&audio).unwrap().len();
        assert!(final_size > original.len() as u64);
        assert!(!super::super::has_download_size_mismatch(
            final_size,
            std::fs::metadata(&audio).unwrap().len()
        ));
    }

    #[test]
    fn fragmented_mp4_cover_is_embedded_and_audio_packets_remain_decodable() {
        let root = tempfile::tempdir().unwrap();
        let audio = root.path().join("Song.m4a");
        let mut metadata = fixture(&audio);
        let mut original = include_bytes!("../audio/fixtures/hls-init.mp4").to_vec();
        original.extend_from_slice(include_bytes!("../audio/fixtures/hls-segment.m4s"));
        std::fs::write(&audio, &original).unwrap();
        assert!(lofty::read_from_path(&audio)
            .unwrap()
            .properties()
            .duration()
            .is_zero());
        metadata.verified_audio_duration_ms = Some(192);
        let cover = root.path().join("Covers").join("Song.jpg");
        std::fs::create_dir_all(cover.parent().unwrap()).unwrap();
        image::RgbImage::from_pixel(2, 2, image::Rgb([40, 80, 120]))
            .save(&cover)
            .unwrap();
        let cover_bytes = std::fs::read(&cover).unwrap();
        metadata.cover_path = Some(cover.to_string_lossy().to_string());

        let prepared = prepare_audio_tags(&audio, &metadata, false).unwrap();
        assert_eq!(std::fs::read(&audio).unwrap(), original);
        let tagged = lofty::probe::Probe::open(prepared.path())
            .unwrap()
            .guess_file_type()
            .unwrap()
            .read()
            .unwrap();
        let picture = tagged.primary_tag().unwrap().pictures().first().unwrap();
        assert_eq!(picture.data(), cover_bytes);
        assert_eq!(picture.mime_type(), Some(&lofty::picture::MimeType::Jpeg));
        assert_eq!(
            super::super::validate_download_audio(
                prepared.path(),
                None,
                None,
                192,
                &std::sync::atomic::AtomicBool::new(false)
            )
            .unwrap(),
            192
        );
        let decoder = crate::audio::remote::SymphoniaAudioDecoder::new_file(prepared.path())
            .unwrap();
        assert_eq!(decoder.count(), 9_216, "写入标签后每个音频包仍可完整解码");
    }

    #[test]
    fn webp_download_cover_is_embedded_in_a_supported_picture_format() {
        let root = tempfile::tempdir().unwrap();
        let audio = root.path().join("Song.aac");
        let mut metadata = fixture(&audio);
        let cover = root.path().join("Covers").join("Song.webp");
        std::fs::create_dir_all(cover.parent().unwrap()).unwrap();
        image::RgbImage::from_pixel(2, 2, image::Rgb([40, 80, 120]))
            .save(&cover)
            .unwrap();
        metadata.cover_path = Some(cover.to_string_lossy().to_string());
        let prepared = prepare_audio_tags(&audio, &metadata, false).unwrap();
        let tagged = lofty::probe::Probe::open(prepared.path())
            .unwrap()
            .guess_file_type()
            .unwrap()
            .read()
            .unwrap();
        let picture = tagged.primary_tag().unwrap().pictures().first().unwrap();
        assert!(matches!(
            picture.mime_type(),
            Some(lofty::picture::MimeType::Png | lofty::picture::MimeType::Jpeg)
        ));
        let decoded = image::load_from_memory(picture.data()).unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), (2, 2));
        assert_eq!(decoded.get_pixel(0, 0).0, [40, 80, 120]);
    }

    #[test]
    fn standardized_tag_is_lrc_but_raw_yrc_sidecar_metadata_is_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let audio = root.path().join("Song.aac");
        let mut metadata = fixture(&audio);
        metadata.write(&audio).unwrap();
        let prepared = prepare_audio_tags(&audio, &metadata, true).unwrap();
        prepared.persist(&audio).unwrap();
        let tagged = lofty::read_from_path(&audio).unwrap();
        let tag = tagged.primary_tag().unwrap();
        assert_eq!(
            tag.get_string(&ItemKey::Lyrics),
            Some("[ar:测试歌手]\n[00:01.00]你好\n[00:01.00]Hello")
        );
        assert_eq!(
            read_metadata(&audio).unwrap().original_lyric,
            metadata.original_lyric
        );
    }

    #[test]
    fn blank_optional_lyrics_do_not_fail_tag_write_verification() {
        let root = tempfile::tempdir().unwrap();
        let audio = root.path().join("Song.aac");
        let mut metadata = fixture(&audio);
        metadata.original_translated_lyric = Some("  ".into());
        metadata.original_romanized_lyric = Some(String::new());
        let prepared = prepare_audio_tags(&audio, &metadata, false).unwrap();
        let tagged = Probe::open(prepared.path())
            .unwrap()
            .guess_file_type()
            .unwrap()
            .read()
            .unwrap();
        let tag = tagged.primary_tag().unwrap();
        assert!(tag
            .get_string(&custom_key(tag.tag_type(), "NERI_LYRICS_TRANSLATED"))
            .is_none());
        assert!(tag
            .get_string(&custom_key(tag.tag_type(), "NERI_LYRICS_ROMANIZED"))
            .is_none());
    }

    #[test]
    fn missing_cover_asset_cannot_be_reported_as_verified_embedding() {
        let root = tempfile::tempdir().unwrap();
        let audio = root.path().join("Song.aac");
        let mut metadata = fixture(&audio);
        let original = std::fs::read(&audio).unwrap();
        metadata.cover_path = Some("Covers/missing.jpg".into());
        assert!(prepare_audio_tags(&audio, &metadata, false).is_err());
        assert_eq!(std::fs::read(&audio).unwrap(), original);
    }

    #[test]
    fn advertised_cover_must_be_downloaded_before_embedding_is_verified() {
        let root = tempfile::tempdir().unwrap();
        let audio = root.path().join("Song.aac");
        let mut metadata = fixture(&audio);
        metadata.cover_url = Some("https://example.test/cover.jpg".into());
        assert!(prepare_audio_tags(&audio, &metadata, false).is_err());
    }

    #[test]
    fn absent_optional_values_preserve_existing_embedded_metadata() {
        let root = tempfile::tempdir().unwrap();
        let audio = root.path().join("Song.aac");
        let mut metadata = fixture(&audio);
        let prepared = prepare_audio_tags(&audio, &metadata, false).unwrap();
        prepared.persist(&audio).unwrap();
        metadata.name = None;
        metadata.artist = None;
        metadata.album = Some(String::new());
        let prepared = prepare_audio_tags(&audio, &metadata, false).unwrap();
        let tagged = Probe::open(prepared.path()).unwrap().guess_file_type().unwrap().read().unwrap();
        let tag = tagged.primary_tag().unwrap();
        assert_eq!(tag.title().as_deref(), Some("测试歌曲"));
        assert_eq!(tag.artist().as_deref(), Some("测试歌手"));
        assert_eq!(tag.album().as_deref(), Some("测试专辑"));
    }

    #[test]
    fn supported_download_containers_roundtrip_all_available_metadata_and_audio() {
        let root = tempfile::tempdir().unwrap();
        for (extension, bytes) in [
            ("aac", include_bytes!("../audio/fixtures/hls-silence.aac").as_slice()),
            ("mp3", include_bytes!("../audio/fixtures/ffmpeg/mp3-stereo-1s.mp3").as_slice()),
            ("flac", include_bytes!("../audio/fixtures/ffmpeg/flac-s16-stereo-0.5s.flac").as_slice()),
            ("m4a", include_bytes!("../audio/fixtures/ffmpeg/aac-stereo-1s.m4a").as_slice()),
            ("wav", include_bytes!("../audio/fixtures/ffmpeg/pcm-s16-stereo-0.5s.wav").as_slice()),
        ] {
            let audio = root.path().join(format!("Song.{extension}"));
            let mut metadata = fixture(&audio);
            // 极短 ADTS 的标签大于音频时会触发 Symphonia 0.5 的帧数估算溢出
            let bytes = if extension == "aac" { bytes.repeat(64) } else { bytes.to_vec() };
            std::fs::write(&audio, &bytes).unwrap();
            metadata.media_uri = Some("https://example.test/track/42".into());
            metadata.channel_id = Some("netease".into());
            let cover = root.path().join("Covers/cover.png");
            std::fs::create_dir_all(cover.parent().unwrap()).unwrap();
            image::RgbImage::from_pixel(2, 2, image::Rgb([40, 80, 120])).save(&cover).unwrap();
            metadata.cover_path = Some(cover.to_string_lossy().into());
            let before: Vec<_> = crate::audio::remote::SymphoniaAudioDecoder::new_file(&audio).unwrap().collect();
            let prepared = prepare_audio_tags(&audio, &metadata, true)
                .unwrap_or_else(|error| panic!("{extension}: {error}"));
            let tagged = Probe::open(prepared.path()).unwrap().guess_file_type().unwrap().read().unwrap();
            let tag = tagged.primary_tag().unwrap();
            assert_eq!(tag.title().as_deref(), Some("测试歌曲"), "{extension}");
            assert_eq!(tag.artist().as_deref(), Some("测试歌手"), "{extension}");
            assert_eq!(tag.album().as_deref(), Some("测试专辑"), "{extension}");
            assert_eq!(tag.get_string(&ItemKey::AlbumArtist), Some("测试歌手"), "{extension}");
            assert_eq!(tag.get_string(&ItemKey::Lyrics), Some("[ar:测试歌手]\n[00:01.00]你好\n[00:01.00]Hello"), "{extension}");
            for (name, expected) in [
                ("NERI_LYRICS_ORIGINAL", "[ar:测试歌手]\n[00:01.00]你好"),
                ("NERI_LYRICS_TRANSLATED", "[00:01.50]Hello"),
                ("NERI_LYRICS_ROMANIZED", "[00:01.00]ni hao"),
                ("NERI_STABLE_KEY", "42|Netease专辑|"),
                ("NERI_MEDIA_URI", "https://example.test/track/42"),
                ("NERI_SOURCE", "netease"),
            ] {
                assert_eq!(tag.get_string(&custom_key(tag.tag_type(), name)), Some(expected), "{extension}: {name}");
            }
            assert_eq!(tag.pictures().len(), 1, "{extension}");
            assert_eq!(tag.pictures()[0].data(), std::fs::read(&cover).unwrap(), "{extension}");
            assert_eq!(crate::audio::remote::SymphoniaAudioDecoder::new_file(prepared.path()).unwrap().collect::<Vec<_>>(), before, "{extension}");
            assert_eq!(std::fs::read(&audio).unwrap(), bytes, "{extension}");
        }
    }

    #[test]
    fn corrupt_tag_copy_does_not_change_original_and_temporary_file_is_removed() {
        let root = tempfile::tempdir().unwrap();
        let audio = root.path().join("Song.mp3");
        std::fs::write(&audio, b"not audio").unwrap();
        assert!(prepare_audio_tags(&audio, &DownloadMetadata::default(), true).is_err());
        assert_eq!(std::fs::read(&audio).unwrap(), b"not audio");
        assert_eq!(
            std::fs::read_dir(root.path().join(".tmp")).unwrap().count(),
            0
        );
    }

    #[test]
    fn bilibili_download_metadata_keeps_resolved_identity_from_album() {
        let track = super::super::DownloadedTrack {
            id: "bilibili:12529502280002".into(),
            title: "Video".into(),
            artist: "Owner".into(),
            album: "Bilibili|987654|BV1actual".into(),
            duration_ms: 30000,
            cover_url: None,
            source: "bilibili".into(),
            file_path: "Video.m4a".into(),
            file_size: 1000,
            downloaded_at: 100,
        };
        let metadata = DownloadMetadata::for_track(&track);
        assert_eq!(metadata.identity_album.as_deref(), Some("Bilibili|987654|BV1actual"));
        assert_eq!(metadata.audio_id.as_deref(), Some("BV1actual"));
        assert_eq!(metadata.sub_audio_id.as_deref(), Some("987654"));
        assert_eq!(metadata.channel_id.as_deref(), Some("bilibili"));
        assert_eq!(metadata.album.as_deref(), Some(""));
    }

    #[test]
    fn asset_references_cannot_escape_the_managed_root() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let audio = root.path().join("Song.aac");
        std::fs::write(outside.path().join("unrelated.jpg"), b"image").unwrap();
        assert!(resolve_asset(
            &audio,
            outside.path().join("unrelated.jpg").to_str().unwrap(),
            "Covers"
        )
        .is_none());
        assert_eq!(normalized_album("Bilibili|123"), "");
        assert_eq!(normalized_album("Netease真正的专辑"), "真正的专辑");
    }
}
