pub mod approval;
mod cache;
mod compact;
mod wire;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::io::{Read, Write};

use prost::Message;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::models::SyncData;
use crate::error::AppResult;
use wire::invalid;

pub const MANIFEST_FILE: &str = "neriplayer-sync-v3.manifest";
pub const MAX_OBJECT_BYTES: usize = 2 * 1024 * 1024;
const MAX_METADATA_BYTES: usize = 32 * 1024 * 1024;
const MAX_RECORDS: u64 = 131_072;
const LEGACY_CANDIDATES: &str = "_legacyLyricCandidates";

#[derive(Clone, PartialEq, Message, serde::Serialize, serde::Deserialize)]
struct ObjectRef {
    #[prost(string, tag = "1")]
    hash: String,
    #[prost(string, tag = "2")]
    raw_hash: String,
    #[prost(int32, tag = "3")]
    raw_bytes: i32,
    #[prost(int32, tag = "4")]
    compressed_bytes: i32,
    #[prost(bool, tag = "5")]
    index: bool,
}
#[derive(Clone, PartialEq, Message)]
struct ObjectIndex {
    #[prost(message, repeated, tag = "1")]
    children: Vec<ObjectRef>,
}
#[derive(Clone, PartialEq, Message)]
struct LegacySource {
    #[prost(message, optional, tag = "1")]
    root: Option<ObjectRef>,
    #[prost(int64, tag = "2")]
    records: i64,
    #[prost(int64, tag = "3")]
    raw_bytes: i64,
    #[prost(int64, tag = "4")]
    chunks: i64,
}
#[derive(Clone, PartialEq, Message)]
struct OriginalManifest {
    #[prost(int32, tag = "1", default = "3")]
    protocol: i32,
    #[prost(bytes = "vec", tag = "2")]
    header: Vec<u8>,
    #[prost(message, optional, tag = "3")]
    root: Option<ObjectRef>,
    #[prost(int64, tag = "4")]
    records: i64,
    #[prost(int64, tag = "5")]
    raw_bytes: i64,
    #[prost(int64, tag = "6")]
    chunks: i64,
    #[prost(message, optional, tag = "7")]
    legacy: Option<LegacySource>,
}
#[derive(Clone, PartialEq, Message)]
struct Stream {
    #[prost(message, optional, tag = "1")]
    root: Option<ObjectRef>,
    #[prost(int64, tag = "2")]
    raw_bytes: i64,
    #[prost(int64, tag = "3")]
    chunks: i64,
}
#[derive(Clone, PartialEq, Message)]
struct V4Manifest {
    #[prost(int32, tag = "1")]
    protocol: i32,
    #[prost(int32, tag = "2")]
    schema: i32,
    #[prost(message, optional, tag = "3")]
    original: Option<OriginalManifest>,
    #[prost(message, optional, tag = "4")]
    main: Option<Stream>,
    #[prost(message, optional, tag = "5")]
    legacy: Option<Stream>,
    #[prost(message, optional, tag = "6")]
    pool: Option<Stream>,
    #[prost(string, tag = "7")]
    main_raw_hash: String,
    #[prost(string, tag = "8")]
    legacy_raw_hash: String,
    #[prost(string, optional, tag = "9")]
    publication_id: Option<String>,
}

pub struct PreparedArchive {
    pub content: Vec<u8>,
    pub objects: BTreeMap<String, Vec<u8>>,
}

pub struct LoadedArchive {
    pub data: SyncData,
    pub paths: HashSet<String>,
    pub protocol: u8,
}

pub fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn compress(bytes: &[u8], window: u32) -> AppResult<Vec<u8>> {
    let mut encoder =
        zstd::stream::Encoder::new(Vec::new(), 19).map_err(|e| invalid(&e.to_string()))?;
    encoder
        .window_log(window)
        .map_err(|e| invalid(&e.to_string()))?;
    encoder
        .include_checksum(true)
        .map_err(|e| invalid(&e.to_string()))?;
    encoder
        .write_all(bytes)
        .map_err(|e| invalid(&e.to_string()))?;
    encoder.finish().map_err(|e| invalid(&e.to_string()))
}

fn decompress(bytes: &[u8], expected: usize, maximum: usize, window: u32) -> AppResult<Vec<u8>> {
    if bytes.is_empty() || bytes.len() > MAX_OBJECT_BYTES || expected == 0 || expected > maximum {
        return Err(invalid("object exceeds budget"));
    }
    let mut decoder =
        zstd::stream::read::Decoder::new(bytes).map_err(|e| invalid(&e.to_string()))?;
    decoder
        .window_log_max(window)
        .map_err(|e| invalid(&e.to_string()))?;
    let mut output = Vec::with_capacity(expected);
    decoder
        .take(expected as u64 + 1)
        .read_to_end(&mut output)
        .map_err(|e| invalid(&e.to_string()))?;
    if output.len() != expected {
        return Err(invalid("decoded object size mismatch"));
    }
    Ok(output)
}

pub fn is_manifest(bytes: &[u8]) -> bool {
    bytes.starts_with(b"NPSYNC")
}

fn envelope(content: &[u8]) -> AppResult<(u8, Vec<u8>)> {
    if content.len() < 44 || content.len() > MAX_OBJECT_BYTES {
        return Err(invalid("invalid manifest envelope"));
    }
    let protocol = match &content[..8] {
        b"NPSYNC03" => 3,
        b"NPSYNC04" => 4,
        _ => return Err(invalid("unsupported future protocol")),
    };
    let expected = u32::from_be_bytes(content[8..12].try_into().unwrap()) as usize;
    if Sha256::digest(&content[44..]).as_slice() != &content[12..44] {
        return Err(invalid("manifest checksum mismatch"));
    }
    let raw = decompress(&content[44..], expected, 1024 * 1024, 20)?;
    Ok((protocol, raw))
}

fn write_envelope(raw: &[u8]) -> AppResult<Vec<u8>> {
    if raw.len() > 1024 * 1024 {
        return Err(invalid("manifest exceeds raw budget"));
    }
    let compressed = compress(raw, 20)?;
    let mut output = b"NPSYNC04".to_vec();
    output.extend_from_slice(&(raw.len() as u32).to_be_bytes());
    output.extend_from_slice(&Sha256::digest(&compressed));
    output.extend_from_slice(&compressed);
    if output.len() > MAX_OBJECT_BYTES {
        return Err(invalid("manifest exceeds wire budget"));
    }
    Ok(output)
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_ref(reference: &ObjectRef, protocol: u8) -> AppResult<()> {
    let maximum = if reference.index {
        128 * 1024
    } else if protocol == 4 {
        4 * 1024 * 1024
    } else {
        1024 * 1024
    };
    if !valid_hash(&reference.hash)
        || !valid_hash(&reference.raw_hash)
        || reference.raw_bytes <= 0
        || reference.raw_bytes as usize > maximum
        || reference.compressed_bytes <= 0
        || reference.compressed_bytes as usize > MAX_OBJECT_BYTES
    {
        return Err(invalid("invalid object reference"));
    }
    Ok(())
}

fn validate_stream(stream: &Stream) -> AppResult<()> {
    if stream.raw_bytes < 0
        || stream.chunks < 0
        || stream.chunks > stream.raw_bytes
        || (stream.root.is_none() && (stream.raw_bytes != 0 || stream.chunks != 0))
        || (stream.root.is_some() && stream.chunks == 0)
    {
        return Err(invalid("invalid stream totals"));
    }
    Ok(())
}

fn validate_original(value: &OriginalManifest) -> AppResult<()> {
    if value.protocol != 3
        || value.records < 0
        || value.raw_bytes < 0
        || value.records as u64 > MAX_RECORDS
        || value.records > value.raw_bytes / 5
    {
        return Err(invalid("invalid original manifest"));
    }
    validate_stream(&Stream {
        root: value.root.clone(),
        raw_bytes: value.raw_bytes,
        chunks: value.chunks,
    })?;
    if let Some(root) = &value.root {
        validate_ref(root, 3)?;
    }
    let mut raw_total = value.raw_bytes;
    if let Some(legacy) = &value.legacy {
        if legacy.records <= 0
            || legacy.records > legacy.raw_bytes / 5
            || legacy.records as u64 > MAX_RECORDS
            || legacy.chunks > 1024
            || legacy.root.is_none()
        {
            return Err(invalid("invalid legacy lyric source"));
        }
        validate_stream(&Stream {
            root: legacy.root.clone(),
            raw_bytes: legacy.raw_bytes,
            chunks: legacy.chunks,
        })?;
        validate_ref(legacy.root.as_ref().unwrap(), 3)?;
        raw_total = raw_total
            .checked_add(legacy.raw_bytes)
            .ok_or_else(|| invalid("original budget overflow"))?;
    }
    if raw_total as usize > MAX_METADATA_BYTES {
        return Err(invalid("retained metadata exceeds 32 MiB budget"));
    }
    let header = wire::decode("data", &value.header)?;
    if header
        .as_object()
        .unwrap()
        .values()
        .any(|value| value.as_array().is_some_and(|items| !items.is_empty()))
    {
        return Err(invalid("manifest embeds unexpected records"));
    }
    Ok(())
}

fn proto_decode<T: Message + Default>(bytes: &[u8]) -> AppResult<T> {
    T::decode(bytes).map_err(|e| invalid(&format!("protobuf decode: {e}")))
}

fn validate_archive_wire(bytes: &[u8], context: &str) -> AppResult<()> {
    let definitions: &[(u64, u64, Option<&str>)] = match context {
        "v4" => &[
            (1, 0, None),
            (2, 0, None),
            (3, 2, Some("original")),
            (4, 2, Some("stream")),
            (5, 2, Some("stream")),
            (6, 2, Some("stream")),
            (7, 2, None),
            (8, 2, None),
            (9, 2, None),
        ],
        "original" => &[
            (1, 0, None),
            (2, 2, None),
            (3, 2, Some("ref")),
            (4, 0, None),
            (5, 0, None),
            (6, 0, None),
            (7, 2, Some("legacy")),
        ],
        "ref" => &[
            (1, 2, None),
            (2, 2, None),
            (3, 0, None),
            (4, 0, None),
            (5, 0, None),
        ],
        "stream" => &[(1, 2, Some("ref")), (2, 0, None), (3, 0, None)],
        "legacy" => &[
            (1, 2, Some("ref")),
            (2, 0, None),
            (3, 0, None),
            (4, 0, None),
        ],
        "index" => &[(1, 2, Some("ref"))],
        _ => return Err(invalid("unknown archive schema")),
    };
    let mut seen = HashSet::new();
    for field in wire::fields(bytes)? {
        let tag = field.key >> 3;
        let (_, kind, child) = definitions
            .iter()
            .find(|(known, _, _)| *known == tag)
            .ok_or_else(|| invalid("archive contains unsupported future fields"))?;
        if field.key & 7 != *kind {
            return Err(invalid("archive field has invalid wire type"));
        }
        if !seen.insert(tag) && context != "index" {
            return Err(invalid("archive contains duplicate fields"));
        }
        if let Some(child) = child {
            validate_archive_wire(&field.body, child)?;
        }
    }
    let required: &[u64] = match context {
        "v4" => &[1, 2, 3, 4, 5, 6, 7, 8],
        "original" => &[2, 4, 5, 6],
        "ref" => &[1, 2, 3, 4],
        "stream" => &[2, 3],
        "legacy" => &[1, 2, 3, 4],
        "index" => &[1],
        _ => &[],
    };
    if required.iter().any(|tag| !seen.contains(tag)) {
        return Err(invalid("archive is missing Android required fields"));
    }
    Ok(())
}

async fn read_stream<F, Fut>(
    stream: &Stream,
    protocol: u8,
    fetch: &F,
    objects: &mut HashMap<String, (ObjectRef, Vec<u8>)>,
    paths: &mut HashSet<String>,
) -> AppResult<Vec<u8>>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = AppResult<Vec<u8>>>,
{
    validate_stream(stream)?;
    let mut pending = stream
        .root
        .clone()
        .map(|root| vec![(root, 0)])
        .unwrap_or_default();
    let mut output = Vec::new();
    let mut leaves = 0_i64;
    let mut visits = 0_i64;
    while let Some((reference, depth)) = pending.pop() {
        visits += 1;
        if depth > 8 || visits > stream.chunks.saturating_mul(9) {
            return Err(invalid("Merkle nesting or visit budget exceeded"));
        }
        validate_ref(&reference, protocol)?;
        let path = format!("neriplayer-sync-v{protocol}-{}.zst", reference.hash);
        let raw = if let Some((descriptor, raw)) = objects.get(&path) {
            if descriptor != &reference {
                return Err(invalid(
                    "conflicting descriptors for content addressed object",
                ));
            }
            raw.clone()
        } else {
            let compressed = fetch(path.clone()).await?;
            if compressed.len() != reference.compressed_bytes as usize
                || digest(&compressed) != reference.hash
            {
                return Err(invalid("object checksum mismatch"));
            }
            let maximum = if reference.index {
                128 * 1024
            } else if protocol == 4 {
                4 * 1024 * 1024
            } else {
                1024 * 1024
            };
            let raw = decompress(
                &compressed,
                reference.raw_bytes as usize,
                maximum,
                if protocol == 4 { 22 } else { 20 },
            )?;
            if digest(&raw) != reference.raw_hash {
                return Err(invalid("raw checksum mismatch"));
            }
            objects.insert(path.clone(), (reference.clone(), raw.clone()));
            raw
        };
        paths.insert(path);
        if reference.index {
            validate_archive_wire(&raw, "index")?;
            let index: ObjectIndex = proto_decode(&raw)?;
            if index.children.is_empty() || index.children.len() > 512 {
                return Err(invalid("invalid index fanout"));
            }
            pending.extend(
                index
                    .children
                    .into_iter()
                    .rev()
                    .map(|child| (child, depth + 1)),
            );
        } else {
            leaves += 1;
            if leaves > stream.chunks
                || raw.len() > (stream.raw_bytes as usize).saturating_sub(output.len())
            {
                return Err(invalid("stream exceeds declared budget"));
            }
            output.extend_from_slice(&raw);
        }
    }
    if leaves != stream.chunks || output.len() != stream.raw_bytes as usize {
        return Err(invalid("stream chunk or byte count mismatch"));
    }
    Ok(output)
}

pub async fn load<F, Fut>(content: &[u8], fetch: F) -> AppResult<LoadedArchive>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = AppResult<Vec<u8>>>,
{
    let (protocol, raw) = envelope(content)?;
    validate_archive_wire(&raw, if protocol == 4 { "v4" } else { "original" })?;
    let mut objects = HashMap::new();
    let mut paths = HashSet::new();
    let (original, main, legacy) = if protocol == 3 {
        let original: OriginalManifest = proto_decode(&raw)?;
        validate_original(&original)?;
        let main = read_stream(
            &Stream {
                root: original.root.clone(),
                raw_bytes: original.raw_bytes,
                chunks: original.chunks,
            },
            3,
            &fetch,
            &mut objects,
            &mut paths,
        )
        .await?;
        let legacy = if let Some(source) = &original.legacy {
            read_stream(
                &Stream {
                    root: source.root.clone(),
                    raw_bytes: source.raw_bytes,
                    chunks: source.chunks,
                },
                3,
                &fetch,
                &mut objects,
                &mut paths,
            )
            .await?
        } else {
            Vec::new()
        };
        (original, main, legacy)
    } else {
        let value: V4Manifest = proto_decode(&raw)?;
        if value.protocol != 4 || value.schema != 1 {
            return Err(invalid("unsupported or missing protocol/schema version"));
        }
        let original = value
            .original
            .ok_or_else(|| invalid("missing original manifest"))?;
        validate_original(&original)?;
        if !valid_hash(&value.main_raw_hash) || !valid_hash(&value.legacy_raw_hash) {
            return Err(invalid("invalid original stream hash"));
        }
        let main = value.main.ok_or_else(|| invalid("missing main stream"))?;
        let legacy = value
            .legacy
            .ok_or_else(|| invalid("missing legacy stream"))?;
        let pool = value.pool.ok_or_else(|| invalid("missing pool stream"))?;
        let original_legacy = original.legacy.as_ref().map_or(0, |value| value.raw_bytes);
        let budget = (original.raw_bytes + original_legacy) * 8 + 32 * 1024 * 1024;
        let compact_bytes = main
            .raw_bytes
            .checked_add(legacy.raw_bytes)
            .and_then(|size| size.checked_add(pool.raw_bytes))
            .ok_or_else(|| invalid("compact budget overflow"))?;
        if compact_bytes < 0 || compact_bytes > budget {
            return Err(invalid("compact streams exceed budget"));
        }
        let main = read_stream(&main, 4, &fetch, &mut objects, &mut paths).await?;
        let legacy = read_stream(&legacy, 4, &fetch, &mut objects, &mut paths).await?;
        let pool = read_stream(&pool, 4, &fetch, &mut objects, &mut paths).await?;
        let (main, legacy) = compact::unpack(
            &main,
            &legacy,
            &pool,
            original.raw_bytes as usize,
            original_legacy as usize,
        )?;
        if digest(&main) != value.main_raw_hash || digest(&legacy) != value.legacy_raw_hash {
            return Err(invalid("unpacked stream checksum mismatch"));
        }
        (original, main, legacy)
    };
    let mut decoded_objects = 0;
    let mut data = read_records(&original, &main, &mut decoded_objects)?;
    if let Some(source) = &original.legacy {
        let records = frames(&legacy, source.records as u64)?;
        let mut candidates = Vec::new();
        for (kind, payload) in records {
            if kind != 15 {
                return Err(invalid("unrelated legacy lyric record"));
            }
            let song = wire::decode_budgeted("song", &payload, &mut decoded_objects)?;
            if !song["lyricSyncEdited"].is_null() || song["lyricSyncRevision"].as_i64() != Some(0) {
                return Err(invalid("invalid legacy lyric candidate"));
            }
            candidates.push(song);
        }
        data.extensions
            .insert(LEGACY_CANDIDATES.into(), Value::Array(candidates));
    } else if !legacy.is_empty() {
        return Err(invalid("unexpected legacy bytes"));
    }
    super::merge::converge_lyrics(&mut data, true)?;
    Ok(LoadedArchive {
        data,
        paths,
        protocol,
    })
}

fn frames(bytes: &[u8], expected: u64) -> AppResult<Vec<(u8, Vec<u8>)>> {
    if bytes.len() > MAX_METADATA_BYTES || expected > MAX_RECORDS {
        return Err(invalid("record stream exceeds budget"));
    }
    let mut input = wire::Cursor::new(bytes);
    let mut records = Vec::new();
    while input.remaining() > 0 {
        if records.len() as u64 >= expected {
            return Err(invalid("unexpected sync record"));
        }
        let kind = input.byte()?;
        wire::context(kind)?;
        let size = u32::from_be_bytes(input.bytes(4)?.try_into().unwrap()) as usize;
        if size > wire::MAX_RECORD_BYTES {
            return Err(invalid("record exceeds budget"));
        }
        records.push((kind, input.bytes(size)?.to_vec()));
    }
    if records.len() as u64 != expected {
        return Err(invalid("record count mismatch"));
    }
    Ok(records)
}

fn read_records(
    original: &OriginalManifest,
    bytes: &[u8],
    decoded_objects: &mut usize,
) -> AppResult<SyncData> {
    let mut data = wire::decode_budgeted("data", &original.header, decoded_objects)?;
    let mut phase = 0;
    let mut current_playlist: Option<(String, Value)> = None;
    for (kind, bytes) in frames(bytes, original.records as u64)? {
        let next = match kind {
            1 | 2 => 1,
            3 | 4 => 3,
            other => other,
        };
        if next < phase {
            return Err(invalid("record sections are out of order"));
        }
        phase = next;
        let value = wire::decode_budgeted(wire::context(kind)?, &bytes, decoded_objects)?;
        if matches!(kind, 1 | 3) {
            if let Some((section, previous)) = current_playlist.take() {
                data[&section].as_array_mut().unwrap().push(previous);
            }
            if !value["songs"].as_array().unwrap().is_empty() {
                return Err(invalid("playlist header embeds songs"));
            }
            current_playlist = Some((wire::section(kind)?.into(), value));
        } else if matches!(kind, 2 | 4) {
            let (section, playlist) = current_playlist
                .as_mut()
                .ok_or_else(|| invalid("song has no playlist"))?;
            if section != wire::section(kind)? {
                return Err(invalid("song has wrong playlist kind"));
            }
            playlist["songs"].as_array_mut().unwrap().push(value);
        } else {
            if let Some((section, previous)) = current_playlist.take() {
                data[&section].as_array_mut().unwrap().push(previous);
            }
            data[wire::section(kind)?]
                .as_array_mut()
                .unwrap()
                .push(value);
        }
    }
    if let Some((section, previous)) = current_playlist {
        data[&section].as_array_mut().unwrap().push(previous);
    }
    serde_json::from_value(data).map_err(|e| invalid(&e.to_string()))
}

fn write_records(data: &SyncData) -> AppResult<(Vec<u8>, u64)> {
    let value = serde_json::to_value(data).map_err(|e| invalid(&e.to_string()))?;
    wire::validate_json("data", &value)?;
    let mut output = Vec::new();
    let mut records = 0;
    for kind in [1, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16] {
        if let Some(items) = value[wire::section(kind)?].as_array() {
            for item in items {
                let mut header = item.clone();
                if matches!(kind, 1 | 3) {
                    header["songs"] = Value::Array(Vec::new());
                }
                append_record(
                    &mut output,
                    kind,
                    &wire::encode(wire::context(kind)?, &header)?,
                    &mut records,
                )?;
                if matches!(kind, 1 | 3) {
                    for song in item["songs"].as_array().unwrap() {
                        append_record(
                            &mut output,
                            kind + 1,
                            &wire::encode("song", song)?,
                            &mut records,
                        )?;
                    }
                }
            }
        }
    }
    Ok((output, records))
}

fn append_record(output: &mut Vec<u8>, kind: u8, bytes: &[u8], count: &mut u64) -> AppResult<()> {
    if *count >= MAX_RECORDS
        || output.len().saturating_add(bytes.len()).saturating_add(5) > MAX_METADATA_BYTES
    {
        return Err(invalid("retained metadata budget exceeded"));
    }
    output.push(kind);
    output.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    output.extend_from_slice(bytes);
    *count += 1;
    Ok(())
}

fn store_object(
    raw: &[u8],
    index: bool,
    protocol: u8,
    objects: &mut BTreeMap<String, Vec<u8>>,
) -> AppResult<ObjectRef> {
    if let Some((reference, compressed)) = cache::read(raw, index, protocol) {
        objects.insert(
            format!("neriplayer-sync-v{protocol}-{}.zst", reference.hash),
            compressed,
        );
        return Ok(reference);
    }
    let compressed = compress(raw, if protocol == 4 { 22 } else { 20 })?;
    if compressed.len() > MAX_OBJECT_BYTES {
        return Err(invalid("object exceeds wire budget"));
    }
    let reference = ObjectRef {
        hash: digest(&compressed),
        raw_hash: digest(raw),
        raw_bytes: raw.len() as i32,
        compressed_bytes: compressed.len() as i32,
        index,
    };
    validate_ref(&reference, protocol)?;
    cache::write(&reference, &compressed, protocol);
    objects.insert(
        format!("neriplayer-sync-v{protocol}-{}.zst", reference.hash),
        compressed,
    );
    Ok(reference)
}

fn tree(raw: &[u8], protocol: u8, objects: &mut BTreeMap<String, Vec<u8>>) -> AppResult<Stream> {
    let mut level = Vec::new();
    for bytes in content_chunks(
        raw,
        if protocol == 4 {
            4 * 1024 * 1024
        } else {
            1024 * 1024
        },
    ) {
        append_leaf(bytes, protocol, objects, &mut level)?;
    }
    let chunks = level.len() as i64;
    while level.len() > 1 {
        let mut next = Vec::new();
        for children in level.chunks(512) {
            next.push(store_object(
                &ObjectIndex {
                    children: children.to_vec(),
                }
                .encode_to_vec(),
                true,
                protocol,
                objects,
            )?);
        }
        level = next;
    }
    Ok(Stream {
        root: level.pop(),
        raw_bytes: raw.len() as i64,
        chunks,
    })
}

fn append_leaf(
    raw: &[u8],
    protocol: u8,
    objects: &mut BTreeMap<String, Vec<u8>>,
    leaves: &mut Vec<ObjectRef>,
) -> AppResult<()> {
    match store_object(raw, false, protocol, objects) {
        Ok(reference) => {
            leaves.push(reference);
            Ok(())
        }
        Err(error) if raw.len() > 1 && error.to_string().contains("object exceeds wire budget") => {
            let middle = raw.len() / 2;
            append_leaf(&raw[..middle], protocol, objects, leaves)?;
            append_leaf(&raw[middle..], protocol, objects, leaves)
        }
        Err(error) => Err(error),
    }
}

fn content_chunks(raw: &[u8], maximum: usize) -> Vec<&[u8]> {
    let gear: [u64; 256] = std::array::from_fn(|index| {
        let mut value = (index as u64).wrapping_add(-7046029254386353131_i64 as u64);
        value = (value ^ (value >> 30)).wrapping_mul(-4658895280553007687_i64 as u64);
        value = (value ^ (value >> 27)).wrapping_mul(-7723592293110705685_i64 as u64);
        value ^ (value >> 31)
    });
    let mut rolling = 0_u64;
    let mut start = 0;
    let mut parts = Vec::new();
    for (index, byte) in raw.iter().enumerate() {
        rolling = rolling.wrapping_shl(1).wrapping_add(gear[*byte as usize]);
        let length = index + 1 - start;
        if length == maximum || (length >= 64 * 1024 && rolling & ((1 << 18) - 1) == 0) {
            parts.push(&raw[start..index + 1]);
            start = index + 1;
        }
    }
    if start < raw.len() {
        parts.push(&raw[start..]);
    }
    parts
}

pub fn prepare(data: &SyncData, publication_id: Option<String>) -> AppResult<PreparedArchive> {
    for key in data.extensions.keys() {
        if !matches!(
            key.as_str(),
            "playlistUsageStats"
                | "localPlaylistPlaybackStats"
                | "localPlaylistPlaybackBuckets"
                | "biliVideoSkipRules"
                | "lyricOverrides"
                | "playlistUsageDeletions"
                | LEGACY_CANDIDATES
        ) {
            return Err(invalid("unsupported future metadata cannot be rewritten"));
        }
    }
    let mut data = data.normalized_for_sync();
    capture_legacy_lyrics(&mut data)?;
    super::merge::converge_lyrics(&mut data, false)?;
    let (raw, records) = write_records(&data)?;
    let mut legacy = Vec::new();
    let mut legacy_records = 0;
    if let Some(candidates) = data
        .extensions
        .get(LEGACY_CANDIDATES)
        .and_then(Value::as_array)
    {
        for song in candidates {
            append_record(
                &mut legacy,
                15,
                &wire::encode("song", song)?,
                &mut legacy_records,
            )?;
        }
    }
    if raw.len().saturating_add(legacy.len()) > MAX_METADATA_BYTES {
        return Err(invalid("retained metadata exceeds 32 MiB budget"));
    }
    let mut v3_objects = BTreeMap::new();
    let main_original = tree(&raw, 3, &mut v3_objects)?;
    let legacy_original = tree(&legacy, 3, &mut v3_objects)?;
    let mut header = serde_json::to_value(&data).map_err(|e| invalid(&e.to_string()))?;
    for value in header.as_object_mut().unwrap().values_mut() {
        if value.is_array() {
            *value = Value::Array(Vec::new());
        }
    }
    let original = OriginalManifest {
        protocol: 3,
        header: wire::encode("data", &header)?,
        root: main_original.root,
        records: records as i64,
        raw_bytes: raw.len() as i64,
        chunks: main_original.chunks,
        legacy: if legacy_records == 0 {
            None
        } else {
            Some(LegacySource {
                root: legacy_original.root,
                records: legacy_records as i64,
                raw_bytes: legacy.len() as i64,
                chunks: legacy_original.chunks,
            })
        },
    };
    validate_original(&original)?;
    let (main, legacy_compact, pool) = compact::pack_literal(&raw, &legacy)?;
    let mut objects = BTreeMap::new();
    let manifest = V4Manifest {
        protocol: 4,
        schema: 1,
        original: Some(original),
        main: Some(tree(&main, 4, &mut objects)?),
        legacy: Some(tree(&legacy_compact, 4, &mut objects)?),
        pool: Some(tree(&pool, 4, &mut objects)?),
        main_raw_hash: digest(&raw),
        legacy_raw_hash: digest(&legacy),
        publication_id,
    };
    Ok(PreparedArchive {
        content: write_envelope(&encode_manifest(&manifest)?)?,
        objects,
    })
}

fn encode_manifest(manifest: &V4Manifest) -> AppResult<Vec<u8>> {
    let original = manifest
        .original
        .as_ref()
        .ok_or_else(|| invalid("missing original manifest"))?;
    let mut original_bytes = original.encode_to_vec();
    // Kotlin 将没有默认值的计数字段视为必填，零值也必须写入 protobuf
    for (tag, value) in [
        (4, original.records),
        (5, original.raw_bytes),
        (6, original.chunks),
    ] {
        if value == 0 {
            wire::number(&mut original_bytes, tag << 3);
            wire::number(&mut original_bytes, 0);
        }
    }
    let mut output = Vec::new();
    for field in wire::fields(&manifest.encode_to_vec())? {
        wire::field(
            &mut output,
            field.key,
            if field.key == 26 {
                &original_bytes
            } else {
                &field.body
            },
        );
    }
    Ok(output)
}

pub(crate) fn capture_legacy_lyrics(data: &mut SyncData) -> AppResult<()> {
    let mut candidates = data
        .extensions
        .get(LEGACY_CANDIDATES)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let songs = data
        .playlists
        .iter()
        .flat_map(|playlist| playlist.songs.iter())
        .chain(
            data.favorite_playlists
                .iter()
                .flat_map(|playlist| playlist.songs.iter()),
        )
        .chain(data.recent_plays.iter().map(|play| &play.song));
    for song in songs {
        if song.lyric_sync_edited.is_none()
            && [
                song.matched_lyric.as_ref(),
                song.matched_translated_lyric.as_ref(),
                song.matched_romanized_lyric.as_ref(),
                song.original_lyric.as_ref(),
                song.original_translated_lyric.as_ref(),
                song.original_romanized_lyric.as_ref(),
            ]
            .iter()
            .any(|value| value.is_some())
        {
            let mut value = serde_json::to_value(song).map_err(|e| invalid(&e.to_string()))?;
            value["lyricSyncRevision"] = Value::from(0);
            candidates.push(value);
        }
    }
    candidates.sort_by_key(Value::to_string);
    candidates.dedup();
    if !candidates.is_empty() {
        data.extensions
            .insert(LEGACY_CANDIDATES.into(), Value::Array(candidates));
    }
    Ok(())
}

pub fn encode_legacy_proto(data: &SyncData) -> AppResult<Vec<u8>> {
    wire::encode(
        "data",
        &serde_json::to_value(data).map_err(|e| invalid(&e.to_string()))?,
    )
}
pub fn decode_legacy_proto(bytes: &[u8]) -> AppResult<SyncData> {
    serde_json::from_value(wire::decode("data", bytes)?).map_err(|e| invalid(&e.to_string()))
}
pub fn decode_legacy_json(value: Value) -> AppResult<SyncData> {
    wire::validate_json("data", &value)?;
    serde_json::from_value(value).map_err(|e| invalid(&e.to_string()))
}

pub fn canonical_object_path(path: &str) -> bool {
    ["neriplayer-sync-v3-", "neriplayer-sync-v4-"]
        .iter()
        .any(|prefix| {
            path.strip_prefix(prefix)
                .and_then(|rest| rest.strip_suffix(".zst"))
                .is_some_and(valid_hash)
        })
}

pub fn renew_publication_id(content: &[u8]) -> AppResult<Vec<u8>> {
    let (protocol, raw) = envelope(content)?;
    if protocol != 4 {
        return Err(invalid("maintenance requires v4"));
    }
    validate_archive_wire(&raw, "v4")?;
    let mut manifest: V4Manifest = proto_decode(&raw)?;
    if manifest.protocol != 4 || manifest.schema != 1 {
        return Err(invalid("unsupported maintenance schema"));
    }
    manifest.publication_id = Some(uuid::Uuid::new_v4().to_string());
    write_envelope(&encode_manifest(&manifest)?)
}

pub fn same_closure(left: &[u8], right: &[u8]) -> AppResult<bool> {
    let (left_protocol, left) = envelope(left)?;
    let (right_protocol, right) = envelope(right)?;
    if left_protocol != 4 || right_protocol != 4 {
        return Ok(false);
    }
    validate_archive_wire(&left, "v4")?;
    validate_archive_wire(&right, "v4")?;
    let mut left: V4Manifest = proto_decode(&left)?;
    let mut right: V4Manifest = proto_decode(&right)?;
    left.publication_id = None;
    right.publication_id = None;
    Ok(left == right)
}

#[cfg(test)]
pub(super) fn test_manifest(content: &[u8]) -> (u8, Vec<u8>) {
    envelope(content).unwrap()
}
#[cfg(test)]
pub(super) fn test_object_paths(raw: &[u8]) -> Vec<String> {
    let manifest: V4Manifest = proto_decode(raw).unwrap();
    [manifest.main, manifest.legacy, manifest.pool]
        .into_iter()
        .flatten()
        .filter_map(|stream| stream.root)
        .map(|root| {
            assert!(!root.index);
            format!("neriplayer-sync-v4-{}.zst", root.hash)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::models::{SyncPlaylist, SyncSong};
    use serde_json::Map;

    #[test]
    fn export_android_interop_fixtures_when_requested() {
        let Some(destination) = std::env::var_os("NERI_CODEC_FIXTURE_DIR") else {
            return;
        };
        for (name, data) in [
            ("desktop-empty", SyncData::default()),
            (
                "desktop-nonempty",
                SyncData {
                    playlists: vec![SyncPlaylist {
                        id: "123".into(),
                        name: "互通".into(),
                        songs: vec![SyncSong {
                            id: "456".into(),
                            name: "曲目".into(),
                            matched_lyric: Some("[00:01.20]歌词\n".into()),
                            ..Default::default()
                        }],
                        created_at: 0,
                        modified_at: 0,
                        is_deleted: false,
                        song_order_version: 0,
                    }],
                    ..Default::default()
                },
            ),
        ] {
            let prepared = prepare(&data, Some("desktop-interop-test".into())).unwrap();
            let path = std::path::PathBuf::from(&destination).join(name);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join(MANIFEST_FILE), &prepared.content).unwrap();
            let (_, raw) = envelope(&prepared.content).unwrap();
            std::fs::write(path.join("manifest.pb"), raw).unwrap();
            for (filename, content) in prepared.objects {
                std::fs::write(path.join(filename), content).unwrap();
            }
        }
    }

    #[tokio::test]
    async fn duplicate_content_hash_cannot_use_conflicting_ref_descriptor() {
        let mut objects = BTreeMap::new();
        let reference = store_object(b"test", false, 4, &mut objects).unwrap();
        let mut conflicting = reference.clone();
        conflicting.raw_hash = "a".repeat(64);
        let index = ObjectIndex {
            children: vec![reference, conflicting],
        };
        let parent = store_object(&index.encode_to_vec(), true, 4, &mut objects).unwrap();
        let stream = Stream {
            root: Some(parent),
            raw_bytes: 8,
            chunks: 2,
        };
        let error = read_stream(
            &stream,
            4,
            &|path| std::future::ready(Ok(objects[&path].clone())),
            &mut HashMap::new(),
            &mut HashSet::new(),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("conflicting descriptors"));
    }

    #[test]
    fn unknown_json_fields_and_android_int_overflow_are_rejected_before_rewriting() {
        assert!(decode_legacy_json(
            serde_json::json!({"playlists":[{"id":1,"songs":[{"id":2,"futureLyricRule":1}]}]})
        )
        .is_err());
        assert!(decode_legacy_json(serde_json::json!({"playlistUsageStats":[{"playlistKey":"key","openCount":2147483648_i64}]})).is_err());
        assert!(decode_legacy_json(serde_json::json!({"playlists":[{"id":1.5}]})).is_err());
        assert!(decode_legacy_json(serde_json::json!({"localPlaylistPlaybackStats":[{"playlistId":1,"totalPlayCount":"invalid"}]})).is_err());
        assert!(
            decode_legacy_json(serde_json::json!({"_legacyLyricCandidates":"invalid"})).is_err()
        );
        let valid=decode_legacy_json(serde_json::json!({"playlists":[{"id":1,"name":"legacy","songs":[{"id":2,"lyric":"old\n"}]}]})).unwrap();
        assert_eq!(
            valid.playlists[0].songs[0].matched_lyric.as_deref(),
            Some("old\n")
        );
    }

    #[tokio::test]
    async fn unknown_or_duplicate_archive_fields_fail_before_fetch_or_maintenance() {
        let prepared = prepare(&SyncData::default(), None).unwrap();
        let (_, raw) = envelope(&prepared.content).unwrap();
        for (tag, value) in [(10, 1), (1, 4)] {
            let mut changed = raw.clone();
            wire::number(&mut changed, tag << 3);
            wire::number(&mut changed, value);
            let content = write_envelope(&changed).unwrap();
            assert!(load(&content, |_| std::future::ready(Err(invalid(
                "must not fetch"
            ))))
            .await
            .err()
            .unwrap()
            .to_string()
            .contains(if tag == 10 {
                "future fields"
            } else {
                "duplicate fields"
            }));
            assert!(renew_publication_id(&content).is_err());
            assert!(same_closure(&content, &prepared.content).is_err());
        }
        let mut malformed = Stream::default().encode_to_vec();
        assert!(validate_archive_wire(&malformed, "stream").is_err());
        for tag in [2, 3] {
            wire::number(&mut malformed, tag << 3);
            wire::number(&mut malformed, 0);
        }
        validate_archive_wire(&malformed, "stream").unwrap();
    }

    #[tokio::test]
    async fn android_frozen_v3_and_v4_archives_preserve_both_historical_variants() {
        for version in ["v3-frozen", "v4-frozen"] {
            let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/sync/archive/fixtures")
                .join(version);
            let manifest = std::fs::read(directory.join(MANIFEST_FILE)).unwrap();
            let restored = load(&manifest, |path| {
                std::future::ready(
                    std::fs::read(directory.join(path))
                        .map_err(|error| invalid(&error.to_string())),
                )
            })
            .await
            .unwrap();
            assert_eq!(
                restored.data.playback_stat_buckets[0].name,
                "different historical metadata"
            );
            let candidates = restored.data.extensions[LEGACY_CANDIDATES]
                .as_array()
                .unwrap();
            assert_eq!(candidates.len(), 2);
            assert_ne!(candidates[0]["matchedLyric"], candidates[1]["matchedLyric"]);
            let migrated = prepare(&restored.data, None).unwrap();
            let reread = load(&migrated.content, |path| {
                std::future::ready(
                    migrated
                        .objects
                        .get(&path)
                        .cloned()
                        .ok_or_else(|| invalid("missing object")),
                )
            })
            .await
            .unwrap();
            assert_eq!(
                reread.data.extensions[LEGACY_CANDIDATES]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            assert_eq!(
                reread.data.playback_stat_buckets[0].name,
                "different historical metadata"
            );
        }
    }

    #[tokio::test]
    async fn v4_round_trip_preserves_lyrics_identity_extensions_and_nullables() {
        let data = SyncData {
            playlists: vec![SyncPlaylist {
                id: "123".into(),
                name: "跨端歌单".into(),
                songs: vec![SyncSong {
                    id: "456".into(),
                    matched_lyric: Some("[00:01.20]原词".into()),
                    original_lyric: Some(String::new()),
                    matched_romanized_lyric: Some("gen shi".into()),
                    channel_id: Some("youtube_music".into()),
                    audio_id: Some("video".into()),
                    ..Default::default()
                }],
                created_at: 1,
                modified_at: 2,
                is_deleted: false,
                song_order_version: 1,
            }],
            extensions: Map::from_iter([(
                "biliVideoSkipRules".into(),
                serde_json::json!([{"bvid":"BV123","cid":9,"intervals":[{"startMs":1,"endMs":2}],"modifiedAt":3,"isDeleted":false}]),
            )]),
            ..Default::default()
        };
        let prepared = prepare(&data, None).unwrap();
        let loaded = load(&prepared.content, |path| {
            std::future::ready(
                prepared
                    .objects
                    .get(&path)
                    .cloned()
                    .ok_or_else(|| invalid("missing test object")),
            )
        })
        .await
        .unwrap();
        assert_eq!(loaded.protocol, 4);
        assert_eq!(
            loaded.data.playlists[0].songs[0].matched_lyric,
            data.playlists[0].songs[0].matched_lyric
        );
        assert_eq!(
            loaded.data.playlists[0].songs[0].original_lyric,
            Some(String::new())
        );
        assert_eq!(loaded.data.extensions["biliVideoSkipRules"][0]["cid"], 9);
        assert!(loaded.data.extensions[LEGACY_CANDIDATES]
            .as_array()
            .is_some_and(|items| !items.is_empty()));
    }

    #[tokio::test]
    async fn corrupted_or_missing_remote_object_never_applies_partial_data() {
        let prepared = prepare(&SyncData::default(), None).unwrap();
        assert!(load(&prepared.content, |_| std::future::ready(Ok(vec![0])))
            .await
            .is_err());
        assert!(load(&prepared.content, |_| std::future::ready(Err(invalid(
            "remote missing"
        ))))
        .await
        .is_err());
    }

    #[test]
    fn rejects_missing_and_future_v4_versions() {
        for (protocol, schema) in [(0, 1), (4, 0), (5, 1), (4, 2)] {
            let content = write_envelope(
                &V4Manifest {
                    protocol,
                    schema,
                    ..Default::default()
                }
                .encode_to_vec(),
            )
            .unwrap();
            let (version, raw) = envelope(&content).unwrap();
            assert_eq!(version, 4);
            let value: V4Manifest = proto_decode(&raw).unwrap();
            assert!(value.protocol != 4 || value.schema != 1);
        }
        assert!(canonical_object_path(&format!(
            "neriplayer-sync-v4-{}.zst",
            "a".repeat(64)
        )));
        assert!(!canonical_object_path("../neriplayer-sync-v4-evil.zst"));
    }
}
