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
use std::path::{Path, PathBuf};

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
    if let Some(title) = &metadata.name {
        tag.set_title(title.clone());
    }
    if let Some(artist) = &metadata.artist {
        tag.set_artist(artist.clone());
        tag.insert_text(ItemKey::AlbumArtist, artist.clone());
    }
    if let Some(album) = &metadata.album {
        tag.set_album(album.clone());
    }
    if let Some(lyrics) = embedded_lyrics(metadata, standardized) {
        tag.insert_text(ItemKey::Lyrics, lyrics.clone());
        if tag_type == TagType::Mp4Ilst {
            tag.insert_text(ItemKey::Description, lyrics);
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
    if let Some(cover) = metadata
        .cover_path
        .as_deref()
        .and_then(|reference| resolve_asset(audio, reference, "Covers"))
    {
        if let Ok(mut file) = std::fs::File::open(cover) {
            if let Ok(mut picture) = Picture::from_reader(&mut file) {
                picture.set_pic_type(PictureType::CoverFront);
                tag.remove_picture_type(PictureType::CoverFront);
                tag.push_picture(picture);
            }
        }
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
    if tag.title().as_deref() != metadata.name.as_deref()
        || tag.artist().as_deref() != metadata.artist.as_deref()
        || verified.properties().duration().is_zero()
        || metadata.stable_key.as_deref().is_some_and(|expected| {
            tag.get_string(&custom_key(tag_type, "NERI_STABLE_KEY")) != Some(expected)
        })
        || metadata
            .original_lyric
            .as_deref()
            .filter(|text| !text.trim().is_empty())
            .is_some_and(|_| tag.get_string(&ItemKey::Lyrics).is_none())
        || translated
            .as_deref()
            .filter(|text| !text.trim().is_empty())
            .is_some_and(|expected| {
                tag.get_string(&custom_key(tag_type, "NERI_LYRICS_TRANSLATED")) != Some(expected)
            })
        || romanized
            .as_deref()
            .filter(|text| !text.trim().is_empty())
            .is_some_and(|expected| {
                tag.get_string(&custom_key(tag_type, "NERI_LYRICS_ROMANIZED")) != Some(expected)
            })
        || metadata
            .cover_path
            .as_deref()
            .is_some_and(|reference| resolve_asset(audio, reference, "Covers").is_some())
            && tag.pictures().is_empty()
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
        prepared.persist(&audio).unwrap();
        let final_size = std::fs::metadata(&audio).unwrap().len();
        assert!(final_size > original.len() as u64);
        assert!(!super::super::has_download_size_mismatch(
            final_size,
            std::fs::metadata(&audio).unwrap().len()
        ));
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
