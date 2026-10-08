use serde_json::{Map, Value};

use crate::error::{AppError, AppResult};

pub(super) const MAX_RECORD_BYTES: usize = 32 * 1024 * 1024;
const MAX_DECODED_OBJECTS: usize = 131_072;

pub(super) fn invalid(message: &str) -> AppError {
    AppError::Other(format!("Sync archive: {message}"))
}

pub(super) struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }
    pub fn byte(&mut self) -> AppResult<u8> {
        Ok(self.bytes(1)?[0])
    }
    pub fn bytes(&mut self, count: usize) -> AppResult<&'a [u8]> {
        if count > self.remaining() {
            return Err(invalid("truncated payload"));
        }
        let result = &self.bytes[self.position..self.position + count];
        self.position += count;
        Ok(result)
    }
    pub fn number(&mut self) -> AppResult<u64> {
        let mut result = 0;
        for index in 0..10 {
            let byte = self.byte()?;
            if index == 9 && byte > 1 {
                return Err(invalid("integer overflow"));
            }
            result |= u64::from(byte & 127) << (index * 7);
            if byte < 128 {
                if index > 0 && byte == 0 {
                    return Err(invalid("noncanonical integer"));
                }
                return Ok(result);
            }
        }
        Err(invalid("integer overflow"))
    }
    pub fn count(&mut self, maximum: usize) -> AppResult<usize> {
        let value = self.number()?;
        if value > maximum as u64 {
            return Err(invalid("size exceeds budget"));
        }
        Ok(value as usize)
    }
    pub fn part(&mut self, maximum: usize) -> AppResult<&'a [u8]> {
        let count = self.count(maximum)?;
        self.bytes(count)
    }
    pub fn finish(&self) -> AppResult<()> {
        if self.remaining() != 0 {
            return Err(invalid("unexpected trailing bytes"));
        }
        Ok(())
    }
}

pub(super) fn number(output: &mut Vec<u8>, mut value: u64) {
    while value > 127 {
        output.push(value as u8 | 128);
        value >>= 7;
    }
    output.push(value as u8);
}

pub(super) fn part(output: &mut Vec<u8>, bytes: &[u8]) {
    number(output, bytes.len() as u64);
    output.extend_from_slice(bytes);
}

#[derive(Clone)]
pub(super) struct Field {
    pub key: u64,
    pub body: Vec<u8>,
}

pub(super) fn fields(bytes: &[u8]) -> AppResult<Vec<Field>> {
    let mut input = Cursor::new(bytes);
    let mut result = Vec::new();
    while input.remaining() > 0 {
        if result.len() >= 262_144 {
            return Err(invalid("field budget exceeded"));
        }
        let key = input.number()?;
        if key >> 3 == 0 || key >> 3 > 536_870_911 {
            return Err(invalid("invalid protobuf tag"));
        }
        let body = match key & 7 {
            0 => {
                let mut body = Vec::new();
                number(&mut body, input.number()?);
                body
            }
            1 => input.bytes(8)?.to_vec(),
            2 => input.part(MAX_RECORD_BYTES)?.to_vec(),
            5 => input.bytes(4)?.to_vec(),
            _ => return Err(invalid("unsupported protobuf wire type")),
        };
        result.push(Field { key, body });
    }
    Ok(result)
}

pub(super) fn field(output: &mut Vec<u8>, key: u64, body: &[u8]) {
    number(output, key);
    if key & 7 == 2 {
        part(output, body);
    } else {
        output.extend_from_slice(body);
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Int,
    Bool,
    OptionalBool,
    Text,
    OptionalText,
    Action,
    Message(&'static str),
    List(&'static str),
}
use Kind::*;
type Schema = &'static [(&'static str, Kind)];

fn schema(context: &str) -> AppResult<Schema> {
    Ok(match context {
        "data" => &[
            ("version", Text),
            ("deviceId", Text),
            ("deviceName", Text),
            ("lastModified", Int),
            ("playlists", List("playlist")),
            ("favoritePlaylists", List("favorite")),
            ("recentPlays", List("recent")),
            ("syncLog", List("log")),
            ("recentPlayDeletions", List("recentDeletion")),
            ("playbackStats", List("stat")),
            ("playbackStatsClearedAt", Int),
            ("playbackStatBuckets", List("bucket")),
            ("playlistSongDeletions", List("songDeletion")),
            ("playlistUsageStats", List("usage")),
            ("localPlaylistPlaybackStats", List("localStat")),
            ("localPlaylistPlaybackBuckets", List("localBucket")),
            ("biliVideoSkipRules", List("skip")),
            ("lyricOverrides", List("song")),
            ("playlistUsageDeletions", List("usageDeletion")),
        ],
        "playlist" => &[
            ("id", Int),
            ("name", Text),
            ("songs", List("song")),
            ("createdAt", Int),
            ("modifiedAt", Int),
            ("isDeleted", Bool),
            ("songOrderVersion", Int),
        ],
        "favorite" => &[
            ("id", Int),
            ("name", Text),
            ("coverUrl", OptionalText),
            ("trackCount", Int),
            ("source", Text),
            ("songs", List("song")),
            ("addedTime", Int),
            ("modifiedAt", Int),
            ("isDeleted", Bool),
            ("sortOrder", Int),
            ("browseId", OptionalText),
            ("playlistId", OptionalText),
            ("subtitle", OptionalText),
        ],
        "song" => &[
            ("id", Int),
            ("name", Text),
            ("artist", Text),
            ("album", Text),
            ("albumId", Int),
            ("durationMs", Int),
            ("coverUrl", OptionalText),
            ("mediaUri", OptionalText),
            ("addedAt", Int),
            ("matchedLyric", OptionalText),
            ("matchedTranslatedLyric", OptionalText),
            ("matchedLyricSource", OptionalText),
            ("matchedSongId", OptionalText),
            ("userLyricOffsetMs", Int),
            ("customCoverUrl", OptionalText),
            ("customName", OptionalText),
            ("customArtist", OptionalText),
            ("originalName", OptionalText),
            ("originalArtist", OptionalText),
            ("originalCoverUrl", OptionalText),
            ("originalLyric", OptionalText),
            ("originalTranslatedLyric", OptionalText),
            ("channelId", OptionalText),
            ("audioId", OptionalText),
            ("subAudioId", OptionalText),
            ("playlistContextId", OptionalText),
            ("syncMembershipTokens", List("token")),
            ("syncMetadataVersion", Int),
            ("legacyAddedAt", Int),
            ("lyricSyncRevision", Int),
            ("lyricSyncEdited", OptionalBool),
            ("matchedRomanizedLyric", OptionalText),
            ("originalRomanizedLyric", OptionalText),
        ],
        "recent" => &[
            ("songId", Int),
            ("song", Message("song")),
            ("playedAt", Int),
            ("deviceId", Text),
            ("resumePositionMs", Int),
        ],
        "recentDeletion" => &[
            ("songId", Int),
            ("album", Text),
            ("mediaUri", OptionalText),
            ("deletedAt", Int),
            ("deviceId", Text),
        ],
        "log" => &[
            ("timestamp", Int),
            ("deviceId", Text),
            ("action", Action),
            ("playlistId", Int),
            ("songId", Int),
            ("details", OptionalText),
        ],
        "token" => &[("deviceId", Text), ("counter", Int)],
        "shard" => &[
            ("deviceId", Text),
            ("epochStartedAt", Int),
            ("totalListenMs", Int),
            ("playCount", Int),
            ("firstPlayedAt", Int),
            ("lastPlayedAt", Int),
        ],
        "stat" => &[
            ("identityKey", Text),
            ("name", Text),
            ("artist", Text),
            ("album", Text),
            ("totalListenMs", Int),
            ("playCount", Int),
            ("lastPlayedAt", Int),
            ("firstPlayedAt", Int),
            ("coverUrl", OptionalText),
            ("durationMs", Int),
            ("mediaUri", OptionalText),
            ("id", Int),
            ("albumId", Int),
            ("counterBaseListenMs", Int),
            ("counterBasePlayCount", Int),
            ("counterShards", List("shard")),
        ],
        "bucket" => &[
            ("dayStartAt", Int),
            ("identityKey", Text),
            ("name", Text),
            ("artist", Text),
            ("album", Text),
            ("totalListenMs", Int),
            ("playCount", Int),
            ("lastPlayedAt", Int),
            ("firstPlayedAt", Int),
            ("coverUrl", OptionalText),
            ("durationMs", Int),
            ("mediaUri", OptionalText),
            ("id", Int),
            ("albumId", Int),
            ("counterBaseListenMs", Int),
            ("counterBasePlayCount", Int),
            ("counterShards", List("shard")),
        ],
        "songDeletion" => &[
            ("playlistId", Int),
            ("songId", Int),
            ("album", Text),
            ("mediaUri", OptionalText),
            ("deletedAt", Int),
            ("deviceId", Text),
            ("removedMembershipTokens", List("token")),
        ],
        "usage" => &[
            ("playlistKey", Text),
            ("source", Text),
            ("id", Int),
            ("subtype", OptionalText),
            ("name", Text),
            ("coverUrl", OptionalText),
            ("trackCount", Int),
            ("lastOpenedAt", Int),
            ("firstOpenedAt", Int),
            ("openCount", Int),
            ("counterBaseOpenCount", Int),
            ("counterShards", List("shard")),
            ("fid", Int),
            ("mid", Int),
            ("browseId", OptionalText),
            ("playlistId", OptionalText),
            ("subtitle", OptionalText),
            ("observedDeletionTokens", List("token")),
        ],
        "localStat" => &[
            ("playlistId", Int),
            ("totalPlayCount", Int),
            ("lastPlayedAt", Int),
            ("firstPlayedAt", Int),
            ("counterBasePlayCount", Int),
            ("counterShards", List("shard")),
        ],
        "localBucket" => &[
            ("dayStartAt", Int),
            ("playlistId", Int),
            ("playCount", Int),
            ("lastPlayedAt", Int),
            ("firstPlayedAt", Int),
            ("counterBasePlayCount", Int),
            ("counterShards", List("shard")),
        ],
        "skip" => &[
            ("bvid", Text),
            ("cid", Int),
            ("intervals", List("interval")),
            ("modifiedAt", Int),
            ("isDeleted", Bool),
        ],
        "interval" => &[("startMs", Int), ("endMs", Int)],
        "usageDeletion" => &[
            ("playlistKey", Text),
            ("deletionTokens", List("token")),
            ("deletedAt", Int),
        ],
        _ => return Err(invalid("unknown record schema")),
    })
}

const ACTIONS: &[&str] = &[
    "CREATE_PLAYLIST",
    "DELETE_PLAYLIST",
    "RENAME_PLAYLIST",
    "ADD_SONG",
    "REMOVE_SONG",
    "REORDER_SONGS",
    "PLAY_SONG",
];

fn narrow_integer(context: &str, name: &str) -> bool {
    matches!(
        (context, name),
        ("playlist", "songOrderVersion")
            | ("song", "syncMetadataVersion")
            | ("favorite", "trackCount")
            | ("stat" | "bucket", "playCount" | "counterBasePlayCount")
            | ("shard", "playCount")
            | ("usage", "trackCount" | "openCount")
    )
}

pub(super) fn validate_json(context: &str, value: &Value) -> AppResult<()> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("record must be an object"))?;
    let definitions = schema(context)?;
    for (name, value) in object {
        if context == "data" && name == "_legacyLyricCandidates" {
            for candidate in value
                .as_array()
                .ok_or_else(|| invalid("legacy lyric candidates must be a list"))?
            {
                validate_json("song", candidate)?;
            }
            continue;
        }
        let alias = if context == "song" {
            match name.as_str() {
                "lyric" => "matchedLyric",
                "translatedLyric" => "matchedTranslatedLyric",
                "lyricSource" => "matchedLyricSource",
                "lyricSongId" => "matchedSongId",
                other => other,
            }
        } else {
            name.as_str()
        };
        let (_, kind) = definitions
            .iter()
            .find(|(key, _)| *key == alias)
            .ok_or_else(|| invalid("record contains unsupported future JSON fields"))?;
        match kind {
            List(child) => {
                for item in value
                    .as_array()
                    .ok_or_else(|| invalid("record list expected"))?
                {
                    validate_json(child, item)?;
                }
            }
            Message(child) => validate_json(child, value)?,
            Int if value.is_null()
                && matches!(
                    (context, alias),
                    ("song", "legacyAddedAt") | ("log", "playlistId" | "songId")
                ) => {}
            Int => {
                let integer = value
                    .as_i64()
                    .or_else(|| value.as_str()?.parse().ok())
                    .ok_or_else(|| invalid("integer expected"))?;
                if narrow_integer(context, alias) {
                    i32::try_from(integer).map_err(|_| invalid("Android integer overflow"))?;
                }
            }
            Text if !value.is_string() => return Err(invalid("text expected")),
            OptionalText
                if !(value.is_null()
                    || value.is_string()
                    || (context == "song"
                        && alias == "matchedSongId"
                        && value.as_i64().is_some())) =>
            {
                return Err(invalid("nullable text expected"))
            }
            Bool if !value.is_boolean() => return Err(invalid("boolean expected")),
            OptionalBool if !value.is_null() && !value.is_boolean() => {
                return Err(invalid("nullable boolean expected"))
            }
            Action if !ACTIONS.iter().any(|action| value.as_str() == Some(*action)) => {
                return Err(invalid("invalid action"))
            }
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn encode(context: &str, value: &Value) -> AppResult<Vec<u8>> {
    validate_json(context, value)?;
    let object = value
        .as_object()
        .ok_or_else(|| invalid("record must be an object"))?;
    let mut output = Vec::new();
    for (index, (name, kind)) in schema(context)?.iter().enumerate() {
        let Some(value) = object.get(*name).filter(|value| !value.is_null()) else {
            continue;
        };
        let key = ((index + 1) as u64) << 3;
        match kind {
            List(child) => {
                let values = value.as_array().ok_or_else(|| invalid("list expected"))?;
                for item in values {
                    field(&mut output, key | 2, &encode(child, item)?);
                }
            }
            Message(child) => field(&mut output, key | 2, &encode(child, value)?),
            Text | OptionalText => {
                let text = value.as_str().ok_or_else(|| invalid("text expected"))?;
                if !text.is_empty() || matches!(kind, OptionalText) {
                    field(&mut output, key | 2, text.as_bytes());
                }
            }
            Bool | OptionalBool => {
                let boolean = value.as_bool().ok_or_else(|| invalid("boolean expected"))?;
                if boolean || matches!(kind, OptionalBool) {
                    number(&mut output, key);
                    number(&mut output, u64::from(boolean));
                }
            }
            Int | Action => {
                let integer = if matches!(kind, Action) {
                    ACTIONS
                        .iter()
                        .position(|action| value.as_str() == Some(*action))
                        .ok_or_else(|| invalid("invalid action"))? as i64
                } else {
                    value
                        .as_i64()
                        .or_else(|| value.as_str()?.parse().ok())
                        .ok_or_else(|| invalid("integer expected"))?
                };
                if narrow_integer(context, name) {
                    i32::try_from(integer).map_err(|_| invalid("Android integer overflow"))?;
                }
                if integer != 0 || matches!(*name, "legacyAddedAt" | "playlistId" | "songId") {
                    number(&mut output, key);
                    number(&mut output, integer as u64);
                }
            }
        }
        if output.len() > MAX_RECORD_BYTES {
            return Err(invalid("record exceeds budget"));
        }
    }
    Ok(output)
}

pub(super) fn decode(context: &str, bytes: &[u8]) -> AppResult<Value> {
    decode_inner(context, bytes, &mut 0, 0)
}

pub(super) fn decode_budgeted(
    context: &str,
    bytes: &[u8],
    objects: &mut usize,
) -> AppResult<Value> {
    decode_inner(context, bytes, objects, 0)
}

fn decode_inner(
    context: &str,
    bytes: &[u8],
    objects: &mut usize,
    depth: usize,
) -> AppResult<Value> {
    *objects += 1;
    if depth > 8 || *objects > MAX_DECODED_OBJECTS {
        return Err(invalid("decoded object budget exceeded"));
    }
    let definitions = schema(context)?;
    let mut object = Map::new();
    for (name, kind) in definitions {
        let default = match kind {
            List(_) => Value::Array(Vec::new()),
            Int => Value::from(0),
            Bool => Value::Bool(false),
            Text => Value::String(String::new()),
            Action => Value::String(ACTIONS[0].into()),
            Message(child) => decode_inner(child, &[], objects, depth + 1)?,
            OptionalText | OptionalBool => Value::Null,
        };
        object.insert((*name).into(), default);
    }
    // 未出现的可空 Long 与出现的零必须区分
    if context == "song" {
        object.insert("legacyAddedAt".into(), Value::Null);
    }
    if context == "log" {
        object.insert("playlistId".into(), Value::Null);
        object.insert("songId".into(), Value::Null);
    }
    for entry in fields(bytes)? {
        let Some((name, kind)) = definitions.get((entry.key >> 3) as usize - 1) else {
            return Err(invalid("record contains unsupported future fields"));
        };
        let length_delimited = matches!(kind, Text | OptionalText | Message(_) | List(_));
        if (entry.key & 7 == 2) != length_delimited || (!length_delimited && entry.key & 7 != 0) {
            return Err(invalid("record wire type mismatch"));
        }
        let value = match kind {
            Text | OptionalText => Value::String(
                std::str::from_utf8(&entry.body)
                    .map_err(|_| invalid("invalid UTF-8"))?
                    .into(),
            ),
            Message(child) | List(child) => decode_inner(child, &entry.body, objects, depth + 1)?,
            Int | Bool | OptionalBool | Action => {
                let mut cursor = Cursor::new(&entry.body);
                let integer = cursor.number()?;
                cursor.finish()?;
                if matches!(kind, Int) && narrow_integer(context, name) {
                    i32::try_from(integer as i64)
                        .map_err(|_| invalid("Android integer overflow"))?;
                }
                match kind {
                    Bool | OptionalBool => {
                        if integer > 1 {
                            return Err(invalid("invalid boolean"));
                        }
                        Value::Bool(integer == 1)
                    }
                    Action => Value::String(
                        ACTIONS
                            .get(integer as usize)
                            .ok_or_else(|| invalid("invalid action"))?
                            .to_string(),
                    ),
                    _ => Value::from(integer as i64),
                }
            }
        };
        if matches!(kind, List(_)) {
            object
                .get_mut(*name)
                .unwrap()
                .as_array_mut()
                .unwrap()
                .push(value);
        } else {
            object.insert((*name).into(), value);
        }
    }
    Ok(Value::Object(object))
}

pub(super) fn context(kind: u8) -> AppResult<&'static str> {
    Ok(match kind {
        1 => "playlist",
        2 | 4 | 15 => "song",
        3 => "favorite",
        5 => "recent",
        6 => "log",
        7 => "recentDeletion",
        8 => "stat",
        9 => "bucket",
        10 => "songDeletion",
        11 => "usage",
        12 => "localStat",
        13 => "localBucket",
        14 => "skip",
        16 => "usageDeletion",
        _ => return Err(invalid("invalid record kind")),
    })
}

pub(super) fn section(kind: u8) -> AppResult<&'static str> {
    Ok(match kind {
        1 | 2 => "playlists",
        3 | 4 => "favoritePlaylists",
        5 => "recentPlays",
        6 => "syncLog",
        7 => "recentPlayDeletions",
        8 => "playbackStats",
        9 => "playbackStatBuckets",
        10 => "playlistSongDeletions",
        11 => "playlistUsageStats",
        12 => "localPlaylistPlaybackStats",
        13 => "localPlaylistPlaybackBuckets",
        14 => "biliVideoSkipRules",
        15 => "lyricOverrides",
        16 => "playlistUsageDeletions",
        _ => return Err(invalid("invalid record section")),
    })
}
