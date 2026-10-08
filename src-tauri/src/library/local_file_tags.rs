use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, Tag};
use crate::commands::download_cmd::metadata::metadata_path;
use crate::error::{AppError, AppResult};

static TAG_EDIT_LOCK: Mutex<()> = Mutex::new(());

fn is_edited_key(key: &ItemKey) -> bool {
    matches!(key, ItemKey::TrackTitle | ItemKey::TrackArtist | ItemKey::AlbumTitle)
}

fn read_audio(path: &Path) -> AppResult<lofty::file::TaggedFile> {
    Probe::open(path).map_err(|error| AppError::Metadata(error.to_string()))?
        .guess_file_type()?.read().map_err(|error| AppError::Metadata(error.to_string()))
}

pub fn edit_tags(root: &Path, audio: &Path, title: &str, artist: &str, album: &str) -> AppResult<()> {
    let _guard = TAG_EDIT_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = root.canonicalize()?;
    let audio = audio.canonicalize()?;
    if !root.is_dir() || !audio.is_file() || !audio.starts_with(&root) {
        return Err(AppError::Metadata("文件必须位于当前扫描目录内".into()));
    }
    let title = title.trim();
    let artist = artist.trim();
    let album = album.trim();
    if [title, artist, album].iter().any(|value| value.is_empty() || value.len() > 4096 || value.contains('\0')) {
        return Err(AppError::Metadata("标题、歌手和专辑不能为空或超过长度限制".into()));
    }
    let parent = audio.parent().ok_or_else(|| AppError::Metadata("文件目录无效".into()))?;
    let prepared = tempfile::Builder::new().prefix(".neri-tags-").suffix(".part").tempfile_in(parent)?;
    std::fs::copy(&audio, prepared.path())?;
    let mut tagged = read_audio(prepared.path())?;
    let before = tagged.tags().to_vec();
    let duration = tagged.properties().duration();
    let tag_type = tagged.primary_tag_type();
    if !tagged.supports_tag_type(tag_type) {
        return Err(AppError::Metadata("当前音频容器不支持标签写入".into()));
    }
    if tagged.primary_tag().is_none() { tagged.insert_tag(Tag::new(tag_type)); }
    let tag = tagged.primary_tag_mut().ok_or_else(|| AppError::Metadata("无法创建音频标签".into()))?;
    tag.set_title(title.to_string());
    tag.set_artist(artist.to_string());
    tag.set_album(album.to_string());
    tagged.save_to_path(prepared.path(), WriteOptions::default()).map_err(|error| AppError::Metadata(error.to_string()))?;
    prepared.as_file().sync_all()?;
    let verified = read_audio(prepared.path())?;
    let tag = verified.primary_tag().ok_or_else(|| AppError::Metadata("无法读回音频标签".into()))?;
    if tag.title().as_deref() != Some(title) || tag.artist().as_deref() != Some(artist)
        || tag.album().as_deref() != Some(album) || verified.properties().duration() != duration
    {
        return Err(AppError::Metadata("音频标签写入验证失败，原文件未修改".into()));
    }
    // 格式转换可能丢失少见标签，读回验证通过后才替换原文件
    for previous in &before {
        let current = verified.tags().iter().find(|tag| tag.tag_type() == previous.tag_type())
            .ok_or_else(|| AppError::Metadata("写入会丢失原有标签，已保留原文件".into()))?;
        if previous.pictures() != current.pictures()
            || previous.items().filter(|item| previous.tag_type() != tag_type || !is_edited_key(item.key()))
                .any(|item| !current.items().any(|candidate| candidate == item))
        {
            return Err(AppError::Metadata("写入会丢失原有歌词、封面或其他标签，已保留原文件".into()));
        }
    }

    let sidecar = metadata_path(&audio).ok_or_else(|| AppError::Metadata("元数据文件路径无效".into()))?;
    let old_sidecar = match std::fs::read(&sidecar) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let mut prepared_sidecar = None;
    if let Some(bytes) = &old_sidecar {
        let mut value: serde_json::Value = serde_json::from_slice(bytes)?;
        let object = value.as_object_mut().ok_or_else(|| AppError::Metadata("下载元数据格式无效".into()))?;
        object.insert("name".into(), title.into());
        object.insert("artist".into(), artist.into());
        object.insert("album".into(), album.into());
        object.insert("customName".into(), title.into());
        object.insert("customArtist".into(), artist.into());
        if let Some(restorable) = object.get_mut("restorableMetadata").and_then(|value| value.as_object_mut()) {
            let overrides = restorable.entry("overrides").or_insert_with(|| serde_json::json!({}));
            if let Some(overrides) = overrides.as_object_mut() {
                overrides.insert("title".into(), title.into());
                overrides.insert("artist".into(), artist.into());
            }
        }
        let mut file = tempfile::Builder::new().prefix(".neri-tags-").suffix(".json").tempfile_in(parent)?;
        file.write_all(&serde_json::to_vec_pretty(&value)?)?;
        file.as_file().sync_all()?;
        prepared_sidecar = Some(file);
    }
    // sidecar 先原子替换，音频替换失败时恢复旧 sidecar；原音频始终到最后才改动
    if let Some(file) = prepared_sidecar { file.persist(&sidecar).map_err(|error| AppError::Other(error.error.to_string()))?; }
    if let Err(error) = prepared.persist(&audio) {
        if let Some(bytes) = old_sidecar {
            crate::fsutil::atomic_write(&sidecar, bytes)
                .map_err(|restore_error| AppError::Other(format!("音频未修改，恢复元数据失败: {restore_error}; {error}")))?;
        }
        return Err(AppError::Other(error.error.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::tag::{ItemValue, TagItem, TagType};

    fn wav() -> Vec<u8> {
        let data_len = 16000u32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8000u32.to_le_bytes());
        bytes.extend_from_slice(&16000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        bytes.resize(44 + data_len as usize, 0);
        bytes
    }

    #[test]
    fn editing_retains_lyrics_picture_custom_tags_and_sidecar_fields() {
        let directory = tempfile::tempdir().unwrap();
        let audio = directory.path().join("song.wav");
        std::fs::write(&audio, wav()).unwrap();
        let mut tagged = read_audio(&audio).unwrap();
        let mut tag = Tag::new(TagType::Id3v2);
        tag.set_title("Old title".into());
        tag.set_artist("Old artist".into());
        tag.set_album("Old album".into());
        tag.set_comment("keep comment".into());
        tag.insert_text(ItemKey::AlbumArtist, "keep album artist".into());
        tag.insert_text(ItemKey::Lyrics, "[00:01.00]keep lyrics".into());
        tag.insert_unchecked(TagItem::new(ItemKey::Unknown("NERI_LYRICS_ORIGINAL".into()), ItemValue::Text("keep original".into())));
        let picture = Picture::new_unchecked(PictureType::CoverFront, Some(MimeType::Png), None, vec![137, 80, 78, 71, 13, 10, 26, 10]);
        tag.push_picture(picture);
        tagged.insert_tag(tag);
        tagged.save_to_path(&audio, WriteOptions::default()).unwrap();
        let before = read_audio(&audio).unwrap();
        let sidecar = metadata_path(&audio).unwrap();
        std::fs::write(&sidecar, br#"{"name":"Old title","artist":"Old artist","album":"Old album","lyricPath":"Lyrics/test.lrc","originalLyric":"keep original","coverPath":"Covers/test.png","futureField":{"x":1},"restorableMetadata":{"baseline":{"title":"Old title"},"overrides":{"userLyricOffsetMs":25,"coverReference":"Covers/custom.png"}}}"#).unwrap();
        edit_tags(directory.path(), &audio, "New title", "New artist", "New album").unwrap();
        let after = read_audio(&audio).unwrap();
        let tag = after.primary_tag().unwrap();
        assert_eq!(tag.title().as_deref(), Some("New title"));
        assert_eq!(tag.artist().as_deref(), Some("New artist"));
        assert_eq!(tag.album().as_deref(), Some("New album"));
        assert_eq!(tag.get_string(&ItemKey::Lyrics), Some("[00:01.00]keep lyrics"));
        assert_eq!(tag.get_string(&ItemKey::AlbumArtist), Some("keep album artist"));
        assert_eq!(tag.get_string(&ItemKey::Unknown("NERI_LYRICS_ORIGINAL".into())), Some("keep original"));
        assert_eq!(tag.comment().as_deref(), Some("keep comment"));
        assert_eq!(tag.pictures(), before.primary_tag().unwrap().pictures());
        let sidecar: serde_json::Value = serde_json::from_slice(&std::fs::read(sidecar).unwrap()).unwrap();
        assert_eq!(sidecar["customName"], "New title");
        assert_eq!(sidecar["futureField"]["x"], 1);
        assert_eq!(sidecar["originalLyric"], "keep original");
        assert_eq!(sidecar["coverPath"], "Covers/test.png");
        assert_eq!(sidecar["lyricPath"], "Lyrics/test.lrc");
        assert_eq!(sidecar["restorableMetadata"]["baseline"]["title"], "Old title");
        assert_eq!(sidecar["restorableMetadata"]["overrides"]["title"], "New title");
        assert_eq!(sidecar["restorableMetadata"]["overrides"]["artist"], "New artist");
        assert_eq!(sidecar["restorableMetadata"]["overrides"]["userLyricOffsetMs"], 25);
        assert_eq!(sidecar["restorableMetadata"]["overrides"]["coverReference"], "Covers/custom.png");
    }

    #[test]
    fn invalid_sidecar_leaves_original_audio_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let audio = directory.path().join("song.wav");
        let before = wav();
        std::fs::write(&audio, &before).unwrap();
        std::fs::write(metadata_path(&audio).unwrap(), "broken json").unwrap();
        assert!(edit_tags(directory.path(), &audio, "Title", "Artist", "Album").is_err());
        assert_eq!(std::fs::read(&audio).unwrap(), before);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[test]
    fn rejects_audio_outside_scanned_root() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let audio = other.path().join("song.wav");
        let before = wav();
        std::fs::write(&audio, &before).unwrap();
        assert!(edit_tags(root.path(), &audio, "Title", "Artist", "Album").is_err());
        assert_eq!(std::fs::read(audio).unwrap(), before);
    }
}
