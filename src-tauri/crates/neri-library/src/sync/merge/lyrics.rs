//! 歌词覆盖记录的规整、选取与收敛

use super::*;

pub fn converge_lyrics(
    data: &mut SyncData,
    validate_references: bool,
) -> neri_common::error::AppResult<()> {
    let mut overrides: HashMap<String, SyncSong> = HashMap::new();
    if let Some(values) = data
        .extensions
        .get("lyricOverrides")
        .and_then(serde_json::Value::as_array)
    {
        for value in values {
            let song: SyncSong = serde_json::from_value(value.clone()).map_err(|error| {
                neri_common::error::AppError::Other(format!("Invalid lyric override: {error}"))
            })?;
            select_lyric_override(&mut overrides, normalize_lyric_state(&song));
        }
    }
    if validate_references {
        for song in data
            .playlists
            .iter()
            .flat_map(|playlist| playlist.songs.iter())
            .chain(
                data.favorite_playlists
                    .iter()
                    .flat_map(|playlist| playlist.songs.iter()),
            )
            .chain(data.recent_plays.iter().map(|play| &play.song))
        {
            if song.lyric_sync_edited.is_some()
                && (song.lyric_sync_revision > 0 || song.lyric_sync_edited == Some(true))
                && overrides
                    .get(&song.identity().stable_key())
                    .is_none_or(|item| item.lyric_sync_revision < song.lyric_sync_revision)
            {
                return Err(neri_common::error::AppError::Other(
                    "Sync lyric reference has no committed override".into(),
                ));
            }
        }
    }
    for song in data
        .playlists
        .iter()
        .flat_map(|playlist| playlist.songs.iter())
        .chain(
            data.favorite_playlists
                .iter()
                .flat_map(|playlist| playlist.songs.iter()),
        )
        .chain(data.recent_plays.iter().map(|play| &play.song))
    {
        select_lyric_override(&mut overrides, normalize_lyric_state(song));
    }
    for song in data
        .playlists
        .iter_mut()
        .flat_map(|playlist| playlist.songs.iter_mut())
        .chain(
            data.favorite_playlists
                .iter_mut()
                .flat_map(|playlist| playlist.songs.iter_mut()),
        )
        .chain(data.recent_plays.iter_mut().map(|play| &mut play.song))
    {
        let mut normalized = normalize_lyric_state(song);
        if let Some(latest) = overrides.get(&song.identity().stable_key()) {
            copy_lyric_state(&mut normalized, latest);
        }
        *song = normalized;
    }
    let mut values: Vec<_> = overrides.into_iter().collect();
    values.sort_by(|left, right| left.0.cmp(&right.0));
    let values = values
        .into_iter()
        .map(|(_, song)| serde_json::to_value(lyric_override_record(&song)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            neri_common::error::AppError::Other(format!("Serialize lyric overrides: {error}"))
        })?;
    if !values.is_empty() || data.extensions.contains_key("lyricOverrides") {
        data.extensions
            .insert("lyricOverrides".into(), serde_json::Value::Array(values));
    }
    Ok(())
}

/// 把一首歌当前的歌词状态并入覆盖记录，修订号更高才替换（对齐 Android SyncLyricOverrideStore）
pub fn record_lyric_override(
    extensions: &mut serde_json::Map<String, serde_json::Value>,
    song: &SyncSong,
) -> neri_common::error::AppResult<()> {
    let mut data = SyncData {
        extensions: std::mem::take(extensions),
        playlists: vec![SyncPlaylist {
            id: String::new(),
            name: String::new(),
            songs: vec![song.clone()],
            created_at: 0,
            modified_at: 0,
            is_deleted: false,
            song_order_version: DISPLAY_ORDER_SONG_ORDER_VERSION,
        }],
        ..Default::default()
    };
    let result = converge_lyrics(&mut data, false);
    *extensions = data.extensions;
    result
}

pub(super) fn normalize_lyric_state(song: &SyncSong) -> SyncSong {
    let mut song = song.clone();
    let has_text = [
        song.matched_lyric.as_ref(),
        song.matched_translated_lyric.as_ref(),
        song.matched_romanized_lyric.as_ref(),
        song.original_lyric.as_ref(),
        song.original_translated_lyric.as_ref(),
        song.original_romanized_lyric.as_ref(),
    ]
    .iter()
    .any(|value| value.is_some());
    let channel = song.channel_id.as_deref().unwrap_or_default().trim();
    let bilibili = if channel.is_empty() {
        song.identity()
            .album
            .to_ascii_lowercase()
            .starts_with("bilibili")
    } else {
        channel.eq_ignore_ascii_case("bilibili")
    };
    if has_text
        && (song.lyric_sync_edited.is_none()
            || (song.lyric_sync_edited == Some(false) && song.lyric_sync_revision <= 0 && bilibili))
    {
        song.lyric_sync_edited = Some(true);
        song.lyric_sync_revision = 1;
        if song.matched_lyric.is_none() {
            song.matched_lyric = song.original_lyric.clone();
        }
        if song.matched_translated_lyric.is_none() {
            song.matched_translated_lyric = song.original_translated_lyric.clone();
        }
        if song.matched_romanized_lyric.is_none() {
            song.matched_romanized_lyric = song.original_romanized_lyric.clone();
        }
    } else if song.lyric_sync_edited == Some(true) {
        song.lyric_sync_revision = song.lyric_sync_revision.max(1);
    } else if song.lyric_sync_edited == Some(false) || song.lyric_sync_revision != 0 {
        if song.lyric_sync_edited.is_none() {
            song.lyric_sync_revision = 0;
        } else {
            song.lyric_sync_revision = song.lyric_sync_revision.max(0);
        }
        song.lyric_sync_edited = Some(false);
        song.matched_lyric = None;
        song.matched_translated_lyric = None;
        song.matched_romanized_lyric = None;
        song.original_lyric = None;
        song.original_translated_lyric = None;
        song.original_romanized_lyric = None;
    }
    song
}

fn lyric_payload_key(song: &SyncSong) -> Vec<u16> {
    let key: String = [
        song.matched_lyric.as_ref(),
        song.matched_translated_lyric.as_ref(),
        song.matched_romanized_lyric.as_ref(),
        song.original_lyric.as_ref(),
        song.original_translated_lyric.as_ref(),
        song.original_romanized_lyric.as_ref(),
        song.matched_lyric_source.as_ref(),
        song.matched_song_id.as_ref(),
    ]
    .iter()
    .map(|value| match value {
        Some(value) => format!("{}:{value}", value.encode_utf16().count()),
        None => "-1:".into(),
    })
    .collect();
    key.encode_utf16().collect()
}

fn lyric_override_record(song: &SyncSong) -> SyncSong {
    let mut record = SyncSong {
        id: song.id.clone(),
        album: song.album.clone(),
        media_uri: song.media_uri.clone(),
        channel_id: song.channel_id.clone(),
        audio_id: song.audio_id.clone(),
        sub_audio_id: song.sub_audio_id.clone(),
        ..Default::default()
    };
    copy_lyric_state(&mut record, song);
    record
}

pub(super) fn select_lyric_override(overrides: &mut HashMap<String, SyncSong>, song: SyncSong) {
    if song.lyric_sync_revision <= 0 {
        return;
    }
    let key = song.identity().stable_key();
    let incoming = (
        song.lyric_sync_revision,
        song.lyric_sync_edited == Some(false),
        lyric_payload_key(&song),
    );
    let replace = overrides.get(&key).is_none_or(|previous| {
        incoming
            > (
                previous.lyric_sync_revision,
                previous.lyric_sync_edited == Some(false),
                lyric_payload_key(previous),
            )
    });
    if replace {
        overrides.insert(key, song);
    }
}

fn copy_lyric_state(target: &mut SyncSong, source: &SyncSong) {
    target.matched_lyric = source.matched_lyric.clone();
    target.matched_translated_lyric = source.matched_translated_lyric.clone();
    target.matched_romanized_lyric = source.matched_romanized_lyric.clone();
    target.original_lyric = source.original_lyric.clone();
    target.original_translated_lyric = source.original_translated_lyric.clone();
    target.original_romanized_lyric = source.original_romanized_lyric.clone();
    target.matched_lyric_source = source.matched_lyric_source.clone();
    target.matched_song_id = source.matched_song_id.clone();
    target.lyric_sync_revision = source.lyric_sync_revision;
    target.lyric_sync_edited = source.lyric_sync_edited;
}
