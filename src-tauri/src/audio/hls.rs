use std::collections::HashMap;
use std::future::Future;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use symphonia::core::io::MediaSource;

use crate::api::transport::FallbackHttp;
use crate::api::youtube::hls::{carry_manifest_token, is_trusted_hls_url};
use crate::error::{AppError, AppResult};

use super::growing::GrowingAudioBuffer;
use super::remote::RemoteAudioCache;

#[derive(Clone, Debug, PartialEq, Eq)]
struct ByteRange {
    offset: u64,
    length: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Encryption {
    uri: String,
    iv: Option<[u8; 16]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Initialization {
    uri: String,
    range: Option<ByteRange>,
    key: Option<Encryption>,
}

#[derive(Clone, Debug)]
struct Segment {
    uri: String,
    sequence: u64,
    duration_ms: u64,
    range: Option<ByteRange>,
    key: Option<Encryption>,
    initialization: Option<Initialization>,
    discontinuity: bool,
}

#[derive(Debug)]
struct Playlist {
    segments: Vec<Segment>,
    target_duration_ms: u64,
    end_list: bool,
}

const MAX_MANIFEST_BYTES: usize = 2 * 1024 * 1024;
const MAX_SEGMENT_BYTES: usize = 32 * 1024 * 1024;
const MAX_STREAM_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_LIVE_BYTES: u64 = 1024 * 1024 * 1024;

fn invalid(message: &str) -> AppError {
    AppError::Audio(format!("HLS: {message}"))
}

fn attributes(value: &str) -> AppResult<HashMap<String, String>> {
    let mut output = HashMap::new();
    let mut rest = value.trim();
    while !rest.is_empty() {
        let (name, tail) = rest
            .split_once('=')
            .ok_or_else(|| invalid("invalid attribute list"))?;
        let (value, tail) = if let Some(quoted) = tail.strip_prefix('"') {
            let end = quoted
                .find('"')
                .ok_or_else(|| invalid("unterminated attribute"))?;
            (&quoted[..end], &quoted[end + 1..])
        } else {
            tail.split_once(',').unwrap_or((tail, ""))
        };
        if output
            .insert(name.trim().to_string(), value.to_string())
            .is_some()
        {
            return Err(invalid("duplicate attribute"));
        }
        rest = tail.strip_prefix(',').unwrap_or(tail).trim();
    }
    Ok(output)
}

fn resolve_uri(base: &str, child: &str) -> AppResult<String> {
    if !is_trusted_hls_url(base) {
        return Err(invalid("untrusted manifest URL"));
    }
    let url = url::Url::parse(base)
        .and_then(|base| base.join(child))
        .map_err(|_| invalid("invalid child URL"))?;
    if !is_trusted_hls_url(url.as_str()) {
        return Err(invalid("untrusted child URL"));
    }
    carry_manifest_token(base, url.as_str()).ok_or_else(|| invalid("untrusted child URL"))
}

fn parse_range(
    value: &str,
    previous: Option<(&str, &ByteRange)>,
    uri: &str,
) -> AppResult<ByteRange> {
    let (length, offset) = value
        .split_once('@')
        .map_or((value, None), |(len, offset)| (len, Some(offset)));
    let length = length
        .parse::<u64>()
        .ok()
        .filter(|len| *len > 0)
        .ok_or_else(|| invalid("invalid byte range length"))?;
    let offset = if let Some(offset) = offset {
        offset
            .parse::<u64>()
            .map_err(|_| invalid("invalid byte range offset"))?
    } else {
        let (_, previous) = previous
            .filter(|(last, _)| *last == uri)
            .ok_or_else(|| invalid("byte range has no previous offset"))?;
        previous
            .offset
            .checked_add(previous.length)
            .ok_or_else(|| invalid("byte range overflow"))?
    };
    offset
        .checked_add(length)
        .ok_or_else(|| invalid("byte range overflow"))?;
    Ok(ByteRange { offset, length })
}

fn parse_media_playlist(manifest: &str, base: &str) -> AppResult<Playlist> {
    if manifest.len() > MAX_MANIFEST_BYTES || !is_trusted_hls_url(base) {
        return Err(invalid("invalid manifest"));
    }
    let mut lines = manifest.trim_start_matches('\u{feff}').lines();
    if lines.next().map(str::trim) != Some("#EXTM3U") {
        return Err(invalid("missing EXTM3U header"));
    }
    let mut playlist = Playlist {
        segments: Vec::new(),
        target_duration_ms: 2000,
        end_list: false,
    };
    let mut sequence = 0u64;
    let mut duration = None;
    let mut range: Option<String> = None;
    let mut key = None;
    let mut initialization = None;
    let mut discontinuity = false;
    for line in lines.map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(value) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            if !playlist.segments.is_empty() {
                return Err(invalid("late media sequence"));
            }
            sequence = value
                .parse()
                .map_err(|_| invalid("invalid media sequence"))?;
        } else if let Some(value) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            playlist.target_duration_ms = value
                .parse::<u64>()
                .ok()
                .filter(|n| *n > 0 && *n <= 86400)
                .and_then(|n| n.checked_mul(1000))
                .ok_or_else(|| invalid("invalid target duration"))?;
        } else if let Some(value) = line.strip_prefix("#EXTINF:") {
            if duration.is_some() {
                return Err(invalid("missing segment URI"));
            }
            let seconds = value
                .split(',')
                .next()
                .unwrap_or_default()
                .parse::<f64>()
                .map_err(|_| invalid("invalid segment duration"))?;
            if !seconds.is_finite() || !(0.001..=86400.0).contains(&seconds) {
                return Err(invalid("invalid segment duration"));
            }
            duration = Some((seconds * 1000.0).round() as u64);
        } else if let Some(value) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            if range.replace(value.to_string()).is_some() {
                return Err(invalid("duplicate byte range"));
            }
        } else if let Some(value) = line.strip_prefix("#EXT-X-KEY:") {
            let values = attributes(value)?;
            key = match values.get("METHOD").map(String::as_str) {
                Some("NONE") => None,
                Some("AES-128") => {
                    if values
                        .get("KEYFORMAT")
                        .is_some_and(|format| format != "identity")
                    {
                        return Err(invalid("unsupported key format"));
                    }
                    let uri = resolve_uri(
                        base,
                        values
                            .get("URI")
                            .ok_or_else(|| invalid("missing key URI"))?,
                    )?;
                    let iv = values
                        .get("IV")
                        .map(|iv| {
                            let hex = iv
                                .strip_prefix("0x")
                                .or_else(|| iv.strip_prefix("0X"))
                                .ok_or_else(|| invalid("invalid IV"))?;
                            if hex.is_empty()
                                || hex.len() > 32
                                || !hex.bytes().all(|ch| ch.is_ascii_hexdigit())
                            {
                                return Err(invalid("invalid IV"));
                            }
                            let padded = format!("{hex:0>32}");
                            let bytes = hex::decode(padded).map_err(|_| invalid("invalid IV"))?;
                            bytes.try_into().map_err(|_| invalid("invalid IV"))
                        })
                        .transpose()?;
                    Some(Encryption { uri, iv })
                }
                _ => return Err(invalid("unsupported encryption method")),
            };
        } else if let Some(value) = line.strip_prefix("#EXT-X-MAP:") {
            let values = attributes(value)?;
            let uri = resolve_uri(
                base,
                values
                    .get("URI")
                    .ok_or_else(|| invalid("missing initialization URI"))?,
            )?;
            if key.as_ref().is_some_and(|key| key.iv.is_none()) {
                return Err(invalid("encrypted initialization requires an explicit IV"));
            }
            let map_range = values
                .get("BYTERANGE")
                .map(|value| parse_range(value, None, &uri))
                .transpose()?;
            initialization = Some(Initialization {
                uri,
                range: map_range,
                key: key.clone(),
            });
        } else if line == "#EXT-X-DISCONTINUITY" {
            discontinuity = true;
        } else if line == "#EXT-X-ENDLIST" {
            playlist.end_list = true;
        } else if line.starts_with("#EXT-X-STREAM-INF:")
            || line == "#EXT-X-GAP"
            || line.starts_with("#EXT-X-SKIP:")
        {
            return Err(invalid("unsupported media playlist tag"));
        } else if !line.starts_with('#') {
            if playlist.end_list || playlist.segments.len() >= 20000 {
                return Err(invalid("invalid segment count or placement"));
            }
            let uri = resolve_uri(base, line)?;
            let previous = playlist.segments.last().and_then(|segment| {
                segment
                    .range
                    .as_ref()
                    .map(|range| (segment.uri.as_str(), range))
            });
            let parsed_range = range
                .take()
                .map(|range| parse_range(&range, previous, &uri))
                .transpose()?;
            playlist.segments.push(Segment {
                uri,
                sequence,
                duration_ms: duration.take().ok_or_else(|| invalid("missing EXTINF"))?,
                range: parsed_range,
                key: key.clone(),
                initialization: initialization.clone(),
                discontinuity,
            });
            sequence = sequence
                .checked_add(1)
                .ok_or_else(|| invalid("media sequence overflow"))?;
            discontinuity = false;
        }
    }
    if playlist.segments.is_empty() || duration.is_some() || range.is_some() {
        return Err(invalid("empty or truncated playlist"));
    }
    Ok(playlist)
}

fn decrypt_segment(mut bytes: Vec<u8>, key: &[u8], iv: [u8; 16]) -> AppResult<Vec<u8>> {
    let decryptor = cbc::Decryptor::<aes::Aes128>::new_from_slices(key, &iv)
        .map_err(|_| invalid("AES-128 key must contain 16 bytes"))?;
    let plaintext = decryptor
        .decrypt_padded_mut::<Pkcs7>(&mut bytes)
        .map_err(|_| invalid("invalid encrypted segment or padding"))?;
    let length = plaintext.len();
    bytes.truncate(length);
    Ok(bytes)
}

#[derive(Default)]
struct AdtsFramer {
    pending: Vec<u8>,
}

impl AdtsFramer {
    fn push(&mut self, bytes: &[u8]) -> AppResult<Vec<u8>> {
        self.pending.extend_from_slice(bytes);
        let mut output = Vec::new();
        let mut offset = 0;
        while offset < self.pending.len() {
            let data = &self.pending[offset..];
            if data.len() < 3 {
                break;
            }
            if data.starts_with(b"ID3") {
                if data.len() < 10 {
                    break;
                }
                if data[6..10].iter().any(|byte| byte & 0x80 != 0) {
                    return Err(invalid("invalid ID3 size"));
                }
                let size = data[6..10]
                    .iter()
                    .fold(0usize, |size, byte| (size << 7) | usize::from(*byte));
                let length = 10 + size + if data[5] & 0x10 != 0 { 10 } else { 0 };
                if length > 1024 * 1024 {
                    return Err(invalid("ID3 exceeds budget"));
                }
                if data.len() < length {
                    break;
                }
                offset += length;
                continue;
            }
            if data[0] != 0xff || data[1] & 0xf6 != 0xf0 {
                return Err(invalid("invalid AAC ADTS header"));
            }
            if data.len() < 7 {
                break;
            }
            let length = (usize::from(data[3] & 3) << 11)
                | (usize::from(data[4]) << 3)
                | usize::from(data[5] >> 5);
            let header = if data[1] & 1 == 0 { 9 } else { 7 };
            if length < header || data[6] & 3 != 0 {
                return Err(invalid("unsupported AAC ADTS frame"));
            }
            if data.len() < length {
                break;
            }
            let mut normalized = data[..7].to_vec();
            // Symphonia 的 ADTS 探测要求 MPEG-4 无 CRC 头，AAC 内容保持原样
            normalized[1] = 0xf1;
            let normalized_length = length - header + 7;
            normalized[3] = (normalized[3] & 0xfc) | ((normalized_length >> 11) as u8 & 3);
            normalized[4] = (normalized_length >> 3) as u8;
            normalized[5] = (normalized[5] & 0x1f) | ((normalized_length as u8 & 7) << 5);
            output.extend_from_slice(&normalized);
            output.extend_from_slice(&data[header..length]);
            offset += length;
        }
        self.pending.drain(..offset);
        if self.pending.len() > 1024 * 1024 {
            return Err(invalid("incomplete AAC frame exceeds budget"));
        }
        Ok(output)
    }
    fn finish(&self) -> AppResult<()> {
        if self.pending.is_empty() {
            Ok(())
        } else {
            Err(invalid("truncated AAC frame"))
        }
    }
}

#[derive(Default)]
struct TsAacDemuxer {
    pmt_pid: Option<u16>,
    audio_pid: Option<u16>,
    sections: HashMap<u16, Vec<u8>>,
    last_packet: Option<Vec<u8>>,
    pes_header: Vec<u8>,
    header_complete: bool,
    pes_remaining: Option<usize>,
    adts: AdtsFramer,
}

impl TsAacDemuxer {
    fn section(&mut self, pid: u16, payload: &[u8], start: bool) -> AppResult<Vec<Vec<u8>>> {
        let pending = self.sections.entry(pid).or_default();
        let mut payload = payload;
        if start {
            let pointer = usize::from(
                *payload
                    .first()
                    .ok_or_else(|| invalid("missing PSI pointer"))?,
            );
            if pointer >= payload.len() {
                return Err(invalid("invalid PSI pointer"));
            }
            if !pending.is_empty() {
                pending.extend_from_slice(&payload[1..1 + pointer]);
            }
            payload = &payload[1 + pointer..];
            if pending.is_empty() {
                pending.extend_from_slice(payload);
            } else {
                let mut joined = std::mem::take(pending);
                joined.extend_from_slice(payload);
                *pending = joined;
            }
        } else {
            pending.extend_from_slice(payload);
        }
        let mut output = Vec::new();
        loop {
            if pending.first() == Some(&0xff) {
                pending.clear();
                break;
            }
            if pending.len() < 3 {
                break;
            }
            let length = 3 + ((usize::from(pending[1] & 0x0f) << 8) | usize::from(pending[2]));
            if !(12..=1024).contains(&length) {
                return Err(invalid("invalid PSI section length"));
            }
            if pending.len() < length {
                break;
            }
            output.push(pending.drain(..length).collect());
        }
        Ok(output)
    }

    fn push(&mut self, bytes: &[u8]) -> AppResult<Vec<u8>> {
        if !bytes.len().is_multiple_of(188) {
            return Err(invalid("truncated MPEG-TS packet"));
        }
        let mut output = Vec::new();
        for packet in bytes.chunks_exact(188) {
            if packet[0] != 0x47 || packet[1] & 0x80 != 0 || packet[3] & 0xc0 != 0 {
                return Err(invalid("invalid or scrambled MPEG-TS packet"));
            }
            let pid = (u16::from(packet[1] & 0x1f) << 8) | u16::from(packet[2]);
            let start = packet[1] & 0x40 != 0;
            let control = (packet[3] >> 4) & 3;
            if control == 0 {
                return Err(invalid("invalid MPEG-TS adaptation control"));
            }
            let mut offset = 4;
            let mut discontinuity = false;
            if control & 2 != 0 {
                let length = usize::from(packet[4]);
                offset += 1 + length;
                if offset > 188 {
                    return Err(invalid("invalid MPEG-TS adaptation length"));
                }
                discontinuity = length > 0 && packet[5] & 0x80 != 0;
            }
            if control & 1 == 0 || offset == 188 {
                continue;
            }
            let payload = &packet[offset..];
            if pid == 0 || Some(pid) == self.pmt_pid {
                for section in self.section(pid, payload, start)? {
                    if section[5] & 1 == 0 {
                        continue;
                    }
                    if pid == 0 && section[0] == 0 {
                        for entry in section[8..section.len() - 4].chunks_exact(4) {
                            if entry[0] != 0 || entry[1] != 0 {
                                self.pmt_pid =
                                    Some((u16::from(entry[2] & 0x1f) << 8) | u16::from(entry[3]));
                                break;
                            }
                        }
                    } else if section[0] == 2 {
                        let mut position = 12
                            + ((usize::from(section[10] & 0x0f) << 8) | usize::from(section[11]));
                        let end = section.len() - 4;
                        let mut found = None;
                        while position + 5 <= end {
                            let descriptor_length = (usize::from(section[position + 3] & 0x0f)
                                << 8)
                                | usize::from(section[position + 4]);
                            if position + 5 + descriptor_length > end {
                                return Err(invalid("invalid PMT descriptor"));
                            }
                            if section[position] == 0x0f && found.is_none() {
                                found = Some(
                                    (u16::from(section[position + 1] & 0x1f) << 8)
                                        | u16::from(section[position + 2]),
                                );
                            }
                            position += 5 + descriptor_length;
                        }
                        if position != end {
                            return Err(invalid("truncated PMT"));
                        }
                        let found =
                            found.ok_or_else(|| invalid("MPEG-TS contains no AAC ADTS track"))?;
                        if self.audio_pid.is_some_and(|previous| previous != found) {
                            return Err(invalid(
                                "AAC track changed without a playlist discontinuity",
                            ));
                        }
                        self.audio_pid = Some(found);
                    }
                }
            } else if Some(pid) == self.audio_pid {
                if let Some(last) = &self.last_packet {
                    if packet == last {
                        continue;
                    }
                    if !discontinuity && packet[3] & 0x0f != (last[3] + 1) & 0x0f {
                        return Err(invalid("lost MPEG-TS audio packet"));
                    }
                }
                self.last_packet = Some(packet.to_vec());
                if start {
                    if self.pes_remaining.is_some_and(|remaining| remaining != 0)
                        || (!self.header_complete && !self.pes_header.is_empty())
                    {
                        return Err(invalid("truncated AAC PES packet"));
                    }
                    self.pes_header.clear();
                    self.header_complete = false;
                    self.pes_remaining = None;
                }
                if !self.header_complete {
                    self.pes_header.extend_from_slice(payload);
                    if self.pes_header.len() < 9 {
                        continue;
                    }
                    if !self.pes_header.starts_with(&[0, 0, 1])
                        || !(0xc0..=0xdf).contains(&self.pes_header[3])
                        || self.pes_header[6] & 0xc0 != 0x80
                    {
                        return Err(invalid("invalid AAC PES header"));
                    }
                    let length = usize::from(self.pes_header[8]) + 9;
                    if self.pes_header.len() < length {
                        continue;
                    }
                    let declared =
                        (usize::from(self.pes_header[4]) << 8) | usize::from(self.pes_header[5]);
                    self.pes_remaining = if declared == 0 {
                        None
                    } else {
                        Some(
                            declared
                                .checked_sub(length - 6)
                                .ok_or_else(|| invalid("invalid PES length"))?,
                        )
                    };
                    self.header_complete = true;
                    let data = std::mem::take(&mut self.pes_header);
                    output.extend_from_slice(&self.audio_payload(&data[length..])?);
                } else {
                    output.extend_from_slice(&self.audio_payload(payload)?);
                }
            }
        }
        Ok(output)
    }

    fn audio_payload(&mut self, bytes: &[u8]) -> AppResult<Vec<u8>> {
        let length = self
            .pes_remaining
            .map_or(bytes.len(), |remaining| remaining.min(bytes.len()));
        if let Some(remaining) = &mut self.pes_remaining {
            *remaining -= length;
        }
        self.adts.push(&bytes[..length])
    }

    fn finish(&self) -> AppResult<()> {
        if self.audio_pid.is_none()
            || self.pes_remaining.is_some_and(|remaining| remaining > 0)
            || (!self.header_complete && !self.pes_header.is_empty())
        {
            return Err(invalid("missing or incomplete AAC PES"));
        }
        self.adts.finish()
    }
}

#[derive(Clone)]
struct Cancellation {
    buffer: GrowingAudioBuffer,
    generation: Option<(Arc<AtomicU64>, u64)>,
    prepare_guard: Arc<AtomicBool>,
    flag: Option<Arc<AtomicBool>>,
}

impl Cancellation {
    fn cancelled(&self) -> bool {
        self.buffer.is_aborted()
            || (self.prepare_guard.load(Ordering::Acquire)
                && self
                    .generation
                    .as_ref()
                    .is_some_and(|(generation, expected)| {
                        generation.load(Ordering::Acquire) != *expected
                    }))
            || self
                .flag
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Acquire))
    }
    fn check(&self) -> AppResult<()> {
        if self.cancelled() {
            Err(invalid("transfer cancelled"))
        } else {
            Ok(())
        }
    }
    async fn run<T>(&self, future: impl Future<Output = AppResult<T>>) -> AppResult<T> {
        self.check()?;
        tokio::pin!(future);
        let mut tick = tokio::time::interval(Duration::from_millis(50));
        loop {
            tokio::select! {
                result = &mut future => { self.check()?; return result; },
                _ = tick.tick() => self.check()?,
            }
        }
    }
}

pub(crate) fn validate_response_status(status: reqwest::StatusCode, ranged: bool) -> AppResult<()> {
    if !status.is_success() {
        return Err(invalid(&format!(
            "request returned HTTP {}",
            status.as_u16()
        )));
    }
    if ranged && status != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(invalid("server ignored byte range"));
    }
    if !ranged && status != reqwest::StatusCode::OK {
        return Err(invalid(&format!(
            "request returned HTTP {}",
            status.as_u16()
        )));
    }
    Ok(())
}

async fn fetch(
    http: &FallbackHttp,
    uri: &str,
    range: Option<&ByteRange>,
    maximum: usize,
    cancel: &Cancellation,
) -> AppResult<Vec<u8>> {
    if !is_trusted_hls_url(uri) {
        return Err(invalid("untrusted request URL"));
    }
    if range.is_some_and(|range| range.length > maximum as u64) {
        return Err(invalid("byte range exceeds budget"));
    }
    cancel
        .run(async {
            let response = http
                .send(|http| {
                    let mut request = http
                        .get(uri)
                        .header(
                            reqwest::header::USER_AGENT,
                            crate::api::youtube::playback::stream_user_agent_for_url(uri),
                        )
                        .header(reqwest::header::ACCEPT_ENCODING, "identity")
                        .timeout(Duration::from_secs(30));
                    if let Some(range) = range {
                        request = request.header(
                            reqwest::header::RANGE,
                            format!("bytes={}-{}", range.offset, range.offset + range.length - 1),
                        );
                    }
                    request
                })
                .await
                .map_err(|error| invalid(&format!("request failed: {}", error.without_url())))?;
            if response.url().as_str() != uri || !is_trusted_hls_url(response.url().as_str()) {
                return Err(invalid("redirected response"));
            }
            validate_response_status(response.status(), range.is_some())?;
            if let Some(range) = range {
                let header = response
                    .headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| invalid("missing Content-Range"))?;
                let expected = format!(
                    "bytes {}-{}/",
                    range.offset,
                    range.offset + range.length - 1
                );
                if !header.starts_with(&expected) {
                    return Err(invalid("Content-Range mismatch"));
                }
            }
            if response
                .content_length()
                .is_some_and(|length| length > maximum as u64)
            {
                return Err(invalid("response exceeds budget"));
            }
            let mut response = response;
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|error| {
                invalid(&format!("response interrupted: {}", error.without_url()))
            })? {
                if chunk.len() > maximum.saturating_sub(bytes.len()) {
                    return Err(invalid("response exceeds budget"));
                }
                bytes.extend_from_slice(&chunk);
            }
            if range.is_some_and(|range| range.length != bytes.len() as u64) {
                return Err(invalid("truncated byte range"));
            }
            Ok(bytes)
        })
        .await
}

trait ResourceFetcher: Sync {
    fn bytes<'a>(
        &'a self,
        uri: &'a str,
        range: Option<&'a ByteRange>,
        maximum: usize,
        cancel: &'a Cancellation,
    ) -> impl Future<Output = AppResult<Vec<u8>>> + Send + 'a;
}

impl ResourceFetcher for FallbackHttp {
    fn bytes<'a>(
        &'a self,
        uri: &'a str,
        range: Option<&'a ByteRange>,
        maximum: usize,
        cancel: &'a Cancellation,
    ) -> impl Future<Output = AppResult<Vec<u8>>> + Send + 'a {
        fetch(self, uri, range, maximum, cancel)
    }
}

async fn load_playlist(
    http: &impl ResourceFetcher,
    uri: &str,
    cancel: &Cancellation,
) -> AppResult<Playlist> {
    let bytes = http.bytes(uri, None, MAX_MANIFEST_BYTES, cancel).await?;
    let manifest = std::str::from_utf8(&bytes).map_err(|_| invalid("manifest is not UTF-8"))?;
    parse_media_playlist(manifest, uri)
}

async fn decrypt_resource(
    http: &impl ResourceFetcher,
    bytes: Vec<u8>,
    key: Option<&Encryption>,
    sequence: u64,
    keys: &mut HashMap<String, Vec<u8>>,
    cancel: &Cancellation,
) -> AppResult<Vec<u8>> {
    let Some(key) = key else {
        return Ok(bytes);
    };
    if !keys.contains_key(&key.uri) {
        let bytes = http.bytes(&key.uri, None, 16, cancel).await?;
        if bytes.len() != 16 {
            return Err(invalid("AES-128 key must contain 16 bytes"));
        }
        if keys.len() >= 32 {
            keys.clear();
        }
        keys.insert(key.uri.clone(), bytes);
    }
    let iv = key.iv.unwrap_or_else(|| u128::from(sequence).to_be_bytes());
    decrypt_segment(bytes, &keys[&key.uri], iv)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Container {
    Adts,
    TransportStream,
    FragmentedMp4,
}

struct Output {
    buffer: GrowingAudioBuffer,
    container: Option<Container>,
    initialization: Option<Initialization>,
    ts: TsAacDemuxer,
    adts: AdtsFramer,
    length: u64,
    maximum: u64,
}

impl Output {
    fn new(buffer: GrowingAudioBuffer, live: bool) -> Self {
        Self {
            buffer,
            container: None,
            initialization: None,
            ts: TsAacDemuxer::default(),
            adts: AdtsFramer::default(),
            length: 0,
            maximum: if live {
                MAX_LIVE_BYTES
            } else {
                MAX_STREAM_BYTES
            },
        }
    }
    fn append(&mut self, bytes: &[u8]) -> AppResult<()> {
        self.length = self
            .length
            .checked_add(bytes.len() as u64)
            .filter(|length| *length <= self.maximum)
            .ok_or_else(|| invalid("transfer exceeds disk budget"))?;
        self.buffer.append(bytes);
        // spool 写入失败会使 reader 长度仍为未知，错误由启动或后续 read 传回
        self.buffer
            .wait_for_buffer(0, Duration::ZERO)
            .map_err(|error| invalid(&error))?;
        Ok(())
    }
    fn push(
        &mut self,
        segment: &Segment,
        bytes: &[u8],
        initialization: Option<&[u8]>,
    ) -> AppResult<()> {
        if segment.discontinuity {
            match self.container {
                Some(Container::TransportStream) => self.ts.finish()?,
                Some(Container::Adts) => self.adts.finish()?,
                _ => {}
            }
            self.ts = TsAacDemuxer::default();
            self.adts = AdtsFramer::default();
        }
        let container = if segment.initialization.is_some() {
            Container::FragmentedMp4
        } else if bytes.first() == Some(&0x47) {
            Container::TransportStream
        } else {
            Container::Adts
        };
        if self.container.is_some_and(|previous| previous != container) {
            return Err(invalid("audio container changed"));
        }
        self.container = Some(container);
        match container {
            Container::FragmentedMp4 => {
                validate_mp4(bytes, false)?;
                if self.initialization.is_some() && self.initialization != segment.initialization {
                    return Err(invalid("fragment initialization changed"));
                }
                if self.initialization.is_none() {
                    let initialization =
                        initialization.ok_or_else(|| invalid("missing MP4 initialization"))?;
                    validate_mp4(initialization, true)?;
                    let combined = [initialization, bytes].concat();
                    self.append(&combined)?;
                    self.initialization = segment.initialization.clone();
                } else {
                    self.append(bytes)?;
                }
            }
            Container::TransportStream => {
                let audio = self.ts.push(bytes)?;
                self.append(&audio)?;
            }
            Container::Adts => {
                let audio = self.adts.push(bytes)?;
                self.append(&audio)?;
            }
        }
        Ok(())
    }
    fn finish(&self) -> AppResult<()> {
        match self.container {
            Some(Container::Adts) => self.adts.finish()?,
            Some(Container::TransportStream) => self.ts.finish()?,
            Some(Container::FragmentedMp4) => {}
            None => return Err(invalid("empty media")),
        }
        if self.length == 0 {
            return Err(invalid("empty audio stream"));
        }
        self.buffer.set_total_len(Some(self.length));
        self.buffer.finish();
        Ok(())
    }
}

fn validate_mp4(bytes: &[u8], initialization: bool) -> AppResult<()> {
    let mut offset = 0usize;
    let mut required = false;
    let mut media_data = false;
    while offset < bytes.len() {
        if bytes.len() - offset < 8 {
            return Err(invalid("truncated MP4 box"));
        }
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let kind = &bytes[offset + 4..offset + 8];
        let (length, header) = if length == 1 {
            if bytes.len() - offset < 16 {
                return Err(invalid("truncated MP4 extended box"));
            }
            (
                u64::from_be_bytes(bytes[offset + 8..offset + 16].try_into().unwrap()),
                16,
            )
        } else {
            (u64::from(length), 8)
        };
        let length = usize::try_from(length).map_err(|_| invalid("MP4 box exceeds budget"))?;
        if length < header || length > bytes.len() - offset {
            return Err(invalid("invalid MP4 box length"));
        }
        required |= if initialization {
            kind == b"moov"
        } else {
            kind == b"moof"
        };
        media_data |= kind == b"mdat";
        offset += length;
    }
    if !required || (!initialization && !media_data) {
        return Err(invalid("missing fragmented MP4 boxes"));
    }
    Ok(())
}

fn compare_reload(previous: &Playlist, next: &Playlist) -> AppResult<bool> {
    for segment in &next.segments {
        if let Ok(index) = previous
            .segments
            .binary_search_by_key(&segment.sequence, |segment| segment.sequence)
        {
            let old = &previous.segments[index];
            if old.uri != segment.uri
                || old.range != segment.range
                || old.key != segment.key
                || old.initialization != segment.initialization
            {
                return Err(invalid("reloaded playlist changed an existing segment"));
            }
        }
    }
    Ok(previous.end_list != next.end_list
        || previous.segments.first().map(|segment| segment.sequence)
            != next.segments.first().map(|segment| segment.sequence)
        || previous.segments.last().map(|segment| segment.sequence)
            != next.segments.last().map(|segment| segment.sequence))
}

fn reload_interval(target_ms: u64, changed: bool) -> Duration {
    Duration::from_millis(if changed { target_ms } else { target_ms / 2 })
}

async fn transfer(
    http: &impl ResourceFetcher,
    manifest_url: &str,
    mut playlist: Playlist,
    cancel: &Cancellation,
    cache: Option<RemoteAudioCache>,
) -> AppResult<u64> {
    let live = !playlist.end_list;
    let mut output = Output::new(cancel.buffer.clone(), live);
    let mut keys = HashMap::new();
    let mut last_sequence = None;
    let mut progress = Instant::now();
    let mut first = true;
    let mut changed = true;
    let mut loaded_at = Instant::now();
    loop {
        let begin = if first && live {
            playlist.segments.len().saturating_sub(3)
        } else {
            0
        };
        for segment in &playlist.segments[begin..] {
            if last_sequence.is_some_and(|last| segment.sequence <= last) {
                continue;
            }
            if last_sequence.is_some_and(|last| segment.sequence != last + 1) {
                return Err(invalid("live playlist skipped unread segments"));
            }
            cancel.check()?;
            let initialization = if output.initialization.is_none() {
                if let Some(map) = &segment.initialization {
                    let bytes = http
                        .bytes(&map.uri, map.range.as_ref(), 4 * 1024 * 1024, cancel)
                        .await?;
                    Some(
                        decrypt_resource(
                            http,
                            bytes,
                            map.key.as_ref(),
                            segment.sequence,
                            &mut keys,
                            cancel,
                        )
                        .await?,
                    )
                } else {
                    None
                }
            } else {
                None
            };
            let bytes = http
                .bytes(
                    &segment.uri,
                    segment.range.as_ref(),
                    MAX_SEGMENT_BYTES,
                    cancel,
                )
                .await?;
            let bytes = decrypt_resource(
                http,
                bytes,
                segment.key.as_ref(),
                segment.sequence,
                &mut keys,
                cancel,
            )
            .await?;
            output.push(segment, &bytes, initialization.as_deref())?;
            last_sequence = Some(segment.sequence);
            progress = Instant::now();
        }
        first = false;
        if playlist.end_list {
            break;
        }
        let longest_segment = playlist
            .segments
            .iter()
            .map(|segment| segment.duration_ms)
            .max()
            .unwrap_or(0);
        let stalled_for = Duration::from_millis(
            playlist
                .target_duration_ms
                .max(longest_segment)
                .saturating_mul(3),
        )
        .clamp(Duration::from_secs(30), Duration::from_secs(120));
        if progress.elapsed() > stalled_for {
            return Err(invalid("live manifest stopped advancing"));
        }
        let interval = reload_interval(playlist.target_duration_ms, changed)
            .saturating_sub(loaded_at.elapsed());
        cancel
            .run(async {
                tokio::time::sleep(interval).await;
                Ok(())
            })
            .await?;
        loaded_at = Instant::now();
        let next = load_playlist(http, manifest_url, cancel).await?;
        changed = compare_reload(&playlist, &next)?;
        playlist = next;
    }
    cancel.check()?;
    output.finish()?;
    if !live {
        if let Some(cache) = cache {
            let buffer = cancel.buffer.clone();
            let cancel = cancel.clone();
            let length = output.length;
            let result = tokio::task::spawn_blocking(move || -> AppResult<()> {
                let cache = cache.fresh_staging()?;
                let path = cache.prepare_sequential_write(length)?;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(path)?;
                copy_complete(&buffer, &mut file, &cancel)?;
                file.flush()?;
                drop(file);
                cancel.check()?;
                cache.publish_sequential_write()?;
                Ok(())
            })
            .await
            .map_err(|_| invalid("cache worker failed"))?;
            if result.is_err() {
                log::warn!(target: "hls", "completed stream cache validation or publication failed");
            }
        }
    }
    Ok(output.length)
}

fn copy_complete(
    buffer: &GrowingAudioBuffer,
    file: &mut impl Write,
    cancel: &Cancellation,
) -> AppResult<u64> {
    let mut reader = buffer.reader();
    let expected = reader
        .byte_len()
        .ok_or_else(|| invalid("stream is not complete"))?;
    let mut total = 0;
    let mut chunk = [0u8; 64 * 1024];
    loop {
        cancel.check()?;
        let length = reader.read(&mut chunk)?;
        if length == 0 {
            break;
        }
        file.write_all(&chunk[..length])?;
        total += length as u64;
    }
    if total != expected {
        return Err(invalid("spool length mismatch"));
    }
    Ok(total)
}

pub struct HlsStream {
    pub buffer: GrowingAudioBuffer,
    prepare_guard: Arc<AtomicBool>,
}

impl HlsStream {
    pub fn commit(&self) {
        self.prepare_guard.store(false, Ordering::Release);
    }
}

impl Drop for HlsStream {
    fn drop(&mut self) {
        // 准备阶段的调用方被取消时，生产者仍持有缓冲区，必须主动通知停止
        if self.prepare_guard.load(Ordering::Acquire) {
            self.buffer.abort();
        }
    }
}

pub async fn start_hls_stream(
    http: FallbackHttp,
    manifest_url: String,
    playback_generation: Arc<AtomicU64>,
    expected_generation: u64,
    cache: Option<RemoteAudioCache>,
) -> AppResult<HlsStream> {
    let buffer = GrowingAudioBuffer::new();
    let prepare_guard = Arc::new(AtomicBool::new(true));
    let cancel = Cancellation {
        buffer: buffer.clone(),
        generation: Some((playback_generation, expected_generation)),
        prepare_guard: prepare_guard.clone(),
        flag: None,
    };
    let playlist = load_playlist(&http, &manifest_url, &cancel).await?;
    tokio::spawn(async move {
        if let Err(error) = transfer(&http, &manifest_url, playlist, &cancel, cache).await {
            if cancel.cancelled() {
                cancel.buffer.abort();
            } else {
                cancel.buffer.fail(error.to_string());
            }
        }
    });
    Ok(HlsStream {
        buffer,
        prepare_guard,
    })
}

pub async fn download_hls_to_file(
    http: FallbackHttp,
    manifest_url: String,
    path: PathBuf,
    cancel_flag: Arc<AtomicBool>,
) -> AppResult<u64> {
    let buffer = GrowingAudioBuffer::new();
    let cancel = Cancellation {
        buffer: buffer.clone(),
        generation: None,
        prepare_guard: Arc::new(AtomicBool::new(true)),
        flag: Some(cancel_flag),
    };
    let playlist = load_playlist(&http, &manifest_url, &cancel).await?;
    if !playlist.end_list {
        return Err(invalid("live streams cannot be downloaded offline"));
    }
    if let Err(error) = transfer(&http, &manifest_url, playlist, &cancel, None).await {
        buffer.abort();
        return Err(error);
    }
    struct OwnedDownload {
        path: Option<PathBuf>,
        length: u64,
    }
    impl Drop for OwnedDownload {
        fn drop(&mut self) {
            if let Some(path) = &self.path {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    let mut downloaded = tokio::task::spawn_blocking(move || -> AppResult<OwnedDownload> {
        cancel.check()?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let mut owned = OwnedDownload {
            path: Some(path),
            length: 0,
        };
        let result = copy_complete(&buffer, &mut file, &cancel).and_then(|length| {
            file.flush()?;
            cancel.check()?;
            Ok(length)
        });
        drop(file);
        owned.length = result?;
        Ok(owned)
    })
    .await
    .map_err(|_| invalid("download worker failed"))??;
    // join future 被取消时，worker 返回值仍持有文件清理职责
    downloaded.path = None;
    Ok(downloaded.length)
}

pub fn detect_hls_audio_extension(path: &std::path::Path) -> AppResult<&'static str> {
    let mut header = [0u8; 8];
    std::fs::File::open(path)?.read_exact(&mut header)?;
    if header[0] == 0xff && header[1] == 0xf1 {
        Ok("aac")
    } else if &header[4..] == b"ftyp" {
        Ok("m4a")
    } else {
        Err(invalid("unrecognized completed audio container"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::pcm::PcmSource;
    use crate::audio::remote::SymphoniaAudioDecoder;

    const BASE: &str = "https://r.example.googlevideo.com/audio/index.m3u8?pot=fixture";
    const FRAME: &[u8] = &[0xff, 0xf1, 0x50, 0x80, 0x01, 0x3f, 0xfc, 0x21, 0x10];

    #[test]
    fn hls_vod_parser_resolves_ranges_initialization_and_sequence() {
        let manifest = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:7\n#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"20@0\"\n#EXTINF:3.5,\n#EXT-X-BYTERANGE:100@20\nmedia.m4s\n#EXTINF:2,\n#EXT-X-BYTERANGE:80\nmedia.m4s\n#EXT-X-ENDLIST\n";
        let playlist = parse_media_playlist(manifest, BASE).unwrap();
        assert!(playlist.end_list);
        assert_eq!(playlist.target_duration_ms, 4000);
        assert_eq!(playlist.segments[0].sequence, 7);
        assert_eq!(playlist.segments[0].duration_ms, 3500);
        assert_eq!(
            playlist.segments[0].range,
            Some(ByteRange {
                offset: 20,
                length: 100
            })
        );
        assert_eq!(
            playlist.segments[1].range,
            Some(ByteRange {
                offset: 120,
                length: 80
            })
        );
        assert_eq!(
            playlist.segments[0].uri,
            "https://r.example.googlevideo.com/audio/media.m4s?pot=fixture"
        );
        assert_eq!(
            playlist.segments[0].initialization.as_ref().unwrap().range,
            Some(ByteRange {
                offset: 0,
                length: 20
            })
        );
    }

    #[test]
    fn hls_key_rotation_and_discontinuity_are_retained() {
        let manifest = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:10\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x0000000000000000000000000000000a\n#EXTINF:2,\na.ts\n#EXT-X-DISCONTINUITY\n#EXT-X-KEY:METHOD=NONE\n#EXTINF:2,\nb.ts\n#EXT-X-ENDLIST\n";
        let playlist = parse_media_playlist(manifest, BASE).unwrap();
        assert_eq!(
            playlist.segments[0].key.as_ref().unwrap().iv.unwrap()[15],
            10
        );
        assert!(playlist.segments[1].key.is_none());
        assert!(playlist.segments[1].discontinuity);
    }

    #[test]
    fn hls_rejects_untrusted_urls_drm_and_invalid_implicit_ranges() {
        for manifest in [
            "#EXTM3U\n#EXTINF:2,\nhttps://evil.test/audio.ts\n",
            "#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"key\"\n#EXTINF:2,\na.ts\n",
            "#EXTM3U\n#EXTINF:2,\n#EXT-X-BYTERANGE:20\na.ts\n",
        ] {
            assert!(parse_media_playlist(manifest, BASE).is_err());
        }
    }

    #[test]
    fn hls_aes128_decrypts_standard_padding_and_rejects_invalid_keys() {
        use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
        let key = [7u8; 16];
        let iv = [9u8; 16];
        let mut bytes = vec![0u8; 32];
        bytes[..FRAME.len()].copy_from_slice(FRAME);
        let encrypted = cbc::Encryptor::<aes::Aes128>::new(&key.into(), &iv.into())
            .encrypt_padded_mut::<Pkcs7>(&mut bytes, FRAME.len())
            .unwrap()
            .to_vec();
        assert_eq!(decrypt_segment(encrypted.clone(), &key, iv).unwrap(), FRAME);
        assert!(decrypt_segment(encrypted, &[1; 15], iv).is_err());
    }

    #[test]
    fn hls_adts_split_frames_are_emitted_only_when_complete() {
        let mut framer = AdtsFramer::default();
        assert!(framer.push(&FRAME[..5]).unwrap().is_empty());
        assert_eq!(framer.push(&FRAME[5..]).unwrap(), FRAME);
        framer.finish().unwrap();
        framer.push(&FRAME[..5]).unwrap();
        assert!(framer.finish().is_err());
    }

    const SILENCE: &[u8] = include_bytes!("fixtures/hls-silence.aac");
    const TS_SILENCE: &[u8] = include_bytes!("fixtures/hls-silence.ts");
    const INITIALIZATION: &[u8] = include_bytes!("fixtures/hls-init.mp4");
    const FRAGMENT: &[u8] = include_bytes!("fixtures/hls-segment.m4s");

    fn decode_silence(buffer: &GrowingAudioBuffer) {
        let mut decoder = SymphoniaAudioDecoder::new(Box::new(buffer.reader()), None).unwrap();
        assert_eq!(decoder.sample_rate(), 48000);
        assert_eq!(decoder.channels(), 1);
        let pcm: Vec<f32> = decoder.by_ref().collect();
        assert_eq!(pcm.len(), 9216);
        assert!(pcm.iter().all(|sample| sample.abs() < 0.000001));
    }

    #[test]
    fn hls_real_adts_and_transport_stream_decode_with_symphonia() {
        for (file, bytes) in [("audio.aac", SILENCE), ("audio.ts", TS_SILENCE)] {
            let playlist = parse_media_playlist(
                &format!("#EXTM3U\n#EXTINF:0.192,\n{file}\n#EXT-X-ENDLIST\n"),
                BASE,
            )
            .unwrap();
            let buffer = GrowingAudioBuffer::new();
            let mut output = Output::new(buffer.clone(), false);
            output.push(&playlist.segments[0], bytes, None).unwrap();
            output.finish().unwrap();
            decode_silence(&buffer);
        }
    }

    #[test]
    fn hls_real_fragmented_mp4_decodes_progressively_with_symphonia() {
        let playlist = parse_media_playlist(
            "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:0.192,\naudio.m4s\n#EXT-X-ENDLIST\n",
            BASE,
        )
        .unwrap();
        let buffer = GrowingAudioBuffer::new();
        let mut output = Output::new(buffer.clone(), false);
        output
            .push(&playlist.segments[0], FRAGMENT, Some(INITIALIZATION))
            .unwrap();
        assert!(!buffer.reader().is_seekable());
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            output.finish().unwrap();
        });
        decode_silence(&buffer);
        writer.join().unwrap();
    }

    #[test]
    fn hls_real_aes_transport_stream_decrypts_before_demux() {
        use aes::cipher::BlockEncryptMut;
        let key = [13u8; 16];
        let iv = u128::from(23u64).to_be_bytes();
        let mut storage = vec![0u8; TS_SILENCE.len() + 16];
        storage[..TS_SILENCE.len()].copy_from_slice(TS_SILENCE);
        let encrypted = cbc::Encryptor::<aes::Aes128>::new(&key.into(), &iv.into())
            .encrypt_padded_mut::<Pkcs7>(&mut storage, TS_SILENCE.len())
            .unwrap()
            .to_vec();
        let bytes = decrypt_segment(encrypted, &key, iv).unwrap();
        let mut demux = TsAacDemuxer::default();
        let audio = demux.push(&bytes).unwrap();
        demux.finish().unwrap();
        let buffer = GrowingAudioBuffer::new();
        buffer.append(&audio);
        buffer.finish();
        decode_silence(&buffer);
    }

    #[test]
    fn hls_transport_stream_rejects_packet_loss_corruption_and_incomplete_pes() {
        let mut damaged = TS_SILENCE.to_vec();
        let first_audio = damaged
            .chunks_exact(188)
            .position(|packet| {
                let pid = (u16::from(packet[1] & 0x1f) << 8) | u16::from(packet[2]);
                pid == 0x100
            })
            .unwrap();
        damaged[(first_audio + 1) * 188 + 3] ^= 3;
        assert!(TsAacDemuxer::default().push(&damaged).is_err());
        damaged = TS_SILENCE.to_vec();
        damaged[0] = 0;
        assert!(TsAacDemuxer::default().push(&damaged).is_err());
        damaged = TS_SILENCE.to_vec();
        let last_packet = damaged.len() - 188;
        let payload = last_packet + 5 + usize::from(damaged[last_packet + 4]);
        damaged[payload + 5] += 1;
        let mut demux = TsAacDemuxer::default();
        demux.push(&damaged).unwrap();
        assert!(demux.finish().is_err());
    }

    #[test]
    fn hls_adts_normalizes_mpeg2_and_crc_without_changing_aac_payload() {
        let length = 11usize;
        let mut header = FRAME[..7].to_vec();
        header[1] = 0xf8;
        header[3] = (header[3] & 0xfc) | ((length >> 11) as u8 & 3);
        header[4] = (length >> 3) as u8;
        header[5] = (header[5] & 0x1f) | ((length as u8 & 7) << 5);
        header.extend_from_slice(&[0, 0]);
        header.extend_from_slice(&FRAME[7..]);
        assert_eq!(AdtsFramer::default().push(&header).unwrap(), FRAME);
    }

    #[test]
    fn hls_fragmented_mp4_rejects_missing_boxes_and_changed_initialization() {
        assert!(validate_mp4(&FRAGMENT[..FRAGMENT.len() - 1], false).is_err());
        assert!(validate_mp4(INITIALIZATION, false).is_err());
        let playlist = parse_media_playlist("#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:1,\na.m4s\n#EXT-X-MAP:URI=\"changed.mp4\"\n#EXTINF:1,\nb.m4s\n#EXT-X-ENDLIST\n", BASE).unwrap();
        let mut output = Output::new(GrowingAudioBuffer::new(), false);
        output
            .push(&playlist.segments[0], FRAGMENT, Some(INITIALIZATION))
            .unwrap();
        assert!(output
            .push(&playlist.segments[1], FRAGMENT, Some(INITIALIZATION))
            .is_err());
    }

    #[test]
    fn hls_live_output_enforces_disk_budget() {
        let mut output = Output::new(GrowingAudioBuffer::new(), true);
        output.length = MAX_LIVE_BYTES;
        assert!(output.append(&[0]).is_err());
    }

    #[tokio::test]
    async fn hls_cancellation_interrupts_wait_and_commit_keeps_generation_out_of_session() {
        let buffer = GrowingAudioBuffer::new();
        let generation = Arc::new(AtomicU64::new(7));
        let guard = Arc::new(AtomicBool::new(true));
        let cancel = Cancellation {
            buffer: buffer.clone(),
            generation: Some((generation.clone(), 7)),
            prepare_guard: guard.clone(),
            flag: None,
        };
        let cloned = generation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cloned.store(8, Ordering::Release);
        });
        let start = Instant::now();
        assert!(cancel
            .run(async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok(())
            })
            .await
            .is_err());
        assert!(start.elapsed() < Duration::from_millis(250));
        HlsStream {
            buffer: buffer.clone(),
            prepare_guard: guard,
        }
        .commit();
        assert!(cancel.check().is_ok());
        buffer.abort();
        assert!(cancel.check().is_err());
    }

    #[test]
    fn hls_uncommitted_handle_drop_aborts_only_its_preparing_transfer() {
        let buffer = GrowingAudioBuffer::new();
        let guard = Arc::new(AtomicBool::new(true));
        let cancel = Cancellation {
            buffer: buffer.clone(),
            generation: None,
            prepare_guard: guard.clone(),
            flag: None,
        };
        drop(HlsStream {
            buffer: buffer.clone(),
            prepare_guard: guard,
        });
        assert!(buffer.is_aborted());
        assert!(cancel.check().is_err());

        let buffer = GrowingAudioBuffer::new();
        let stream = HlsStream {
            buffer: buffer.clone(),
            prepare_guard: Arc::new(AtomicBool::new(true)),
        };
        stream.commit();
        drop(stream);
        assert!(!buffer.is_aborted());
        buffer.append(FRAME);
        assert!(buffer.wait_for_buffer(1, Duration::ZERO).is_ok());
        buffer.abort();
    }

    #[test]
    fn hls_completed_file_extension_and_spool_copy_are_verified() {
        let directory = tempfile::tempdir().unwrap();
        for (name, bytes, extension) in [
            ("audio.aac", SILENCE.to_vec(), "aac"),
            ("audio.m4a", [INITIALIZATION, FRAGMENT].concat(), "m4a"),
        ] {
            let buffer = GrowingAudioBuffer::new();
            buffer.append(&bytes);
            buffer.finish();
            let cancel = Cancellation {
                buffer: buffer.clone(),
                generation: None,
                prepare_guard: Arc::new(AtomicBool::new(true)),
                flag: None,
            };
            let path = directory.path().join(name);
            let mut file = std::fs::File::create(&path).unwrap();
            assert_eq!(
                copy_complete(&buffer, &mut file, &cancel).unwrap(),
                bytes.len() as u64
            );
            drop(file);
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            assert_eq!(detect_hls_audio_extension(&path).unwrap(), extension);
        }
    }

    struct FixtureFetcher {
        resources: HashMap<String, Vec<u8>>,
        manifests: std::sync::Mutex<std::collections::VecDeque<String>>,
        requests: std::sync::Mutex<Vec<String>>,
    }

    impl ResourceFetcher for FixtureFetcher {
        async fn bytes<'a>(
            &'a self,
            uri: &'a str,
            range: Option<&'a ByteRange>,
            maximum: usize,
            cancel: &'a Cancellation,
        ) -> AppResult<Vec<u8>> {
            cancel.check()?;
            let url = url::Url::parse(uri).unwrap();
            assert!(url
                .query_pairs()
                .any(|(name, value)| name == "pot" && value == "fixture"));
            self.requests.lock().unwrap().push(url.path().to_string());
            let mut bytes = if url.path().ends_with("index.m3u8") {
                self.manifests
                    .lock()
                    .unwrap()
                    .pop_front()
                    .ok_or_else(|| invalid("missing fixture manifest"))?
                    .into_bytes()
            } else {
                self.resources
                    .get(url.path())
                    .cloned()
                    .ok_or_else(|| invalid("missing fixture resource"))?
            };
            if let Some(range) = range {
                let begin = range.offset as usize;
                let end = begin + range.length as usize;
                bytes = bytes
                    .get(begin..end)
                    .ok_or_else(|| invalid("invalid fixture range"))?
                    .to_vec();
            }
            if bytes.len() > maximum {
                return Err(invalid("fixture response exceeds budget"));
            }
            Ok(bytes)
        }
    }

    fn fixture_fetcher(resources: &[(&str, Vec<u8>)], manifests: &[&str]) -> FixtureFetcher {
        FixtureFetcher {
            resources: resources
                .iter()
                .map(|(path, bytes)| (format!("/audio/{path}"), bytes.clone()))
                .collect(),
            manifests: std::sync::Mutex::new(
                manifests
                    .iter()
                    .map(|manifest| manifest.to_string())
                    .collect(),
            ),
            requests: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn fixture_cancellation() -> Cancellation {
        Cancellation {
            buffer: GrowingAudioBuffer::new(),
            generation: None,
            prepare_guard: Arc::new(AtomicBool::new(true)),
            flag: None,
        }
    }

    #[tokio::test]
    async fn hls_executor_decrypts_vod_with_sequence_iv_and_publishes_complete_audio() {
        use aes::cipher::BlockEncryptMut;
        let key = [6u8; 16];
        let encrypted = |sequence: u64| {
            let mut storage = vec![0u8; SILENCE.len() + 16];
            storage[..SILENCE.len()].copy_from_slice(SILENCE);
            cbc::Encryptor::<aes::Aes128>::new(
                &key.into(),
                &u128::from(sequence).to_be_bytes().into(),
            )
            .encrypt_padded_mut::<Pkcs7>(&mut storage, SILENCE.len())
            .unwrap()
            .to_vec()
        };
        let manifest = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:23\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n#EXTINF:0.192,\na.aac\n#EXTINF:0.192,\nb.aac\n#EXT-X-ENDLIST\n";
        let fetcher = fixture_fetcher(
            &[
                ("key.bin", key.to_vec()),
                ("a.aac", encrypted(23)),
                ("b.aac", encrypted(24)),
            ],
            &[manifest],
        );
        let cancel = fixture_cancellation();
        let directory = tempfile::tempdir().unwrap();
        let cache = RemoteAudioCache::new(
            directory.path().to_path_buf(),
            "vod-hls",
            1024 * 1024,
            None,
            384,
        )
        .unwrap();
        let playlist = load_playlist(&fetcher, BASE, &cancel).await.unwrap();
        assert_eq!(
            transfer(&fetcher, BASE, playlist, &cancel, Some(cache.clone()))
                .await
                .unwrap(),
            2 * SILENCE.len() as u64
        );
        assert_eq!(
            std::fs::read(cache.ready_path().unwrap()).unwrap(),
            [SILENCE, SILENCE].concat()
        );
        assert_eq!(
            fetcher
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|path| path.ends_with("key.bin"))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn hls_executor_reloads_live_from_edge_without_publishing_partial_recording() {
        let first = "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:4\n#EXTINF:1,\n4.aac\n#EXTINF:1,\n5.aac\n#EXTINF:1,\n6.aac\n#EXTINF:1,\n7.aac\n#EXTINF:1,\n8.aac\n";
        let next = "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:6\n#EXTINF:1,\n6.aac\n#EXTINF:1,\n7.aac\n#EXTINF:1,\n8.aac\n#EXTINF:1,\n9.aac\n#EXT-X-ENDLIST\n";
        let fetcher = fixture_fetcher(
            &[
                ("6.aac", SILENCE.to_vec()),
                ("7.aac", SILENCE.to_vec()),
                ("8.aac", SILENCE.to_vec()),
                ("9.aac", SILENCE.to_vec()),
            ],
            &[first, next],
        );
        let directory = tempfile::tempdir().unwrap();
        let cache = RemoteAudioCache::new(
            directory.path().to_path_buf(),
            "live-hls",
            1024 * 1024,
            None,
            0,
        )
        .unwrap();
        let cancel = fixture_cancellation();
        let playlist = load_playlist(&fetcher, BASE, &cancel).await.unwrap();
        assert_eq!(
            transfer(&fetcher, BASE, playlist, &cancel, Some(cache.clone()))
                .await
                .unwrap(),
            4 * SILENCE.len() as u64
        );
        assert!(cache.ready_path().is_none());
        let requests = fetcher.requests.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|path| path.ends_with("index.m3u8"))
                .count(),
            2
        );
        for sequence in 6..=9 {
            assert_eq!(
                requests
                    .iter()
                    .filter(|path| *path == &format!("/audio/{sequence}.aac"))
                    .count(),
                1
            );
        }
        assert_eq!(reload_interval(6000, true), Duration::from_secs(6));
        assert_eq!(reload_interval(6000, false), Duration::from_secs(3));
    }

    #[tokio::test]
    async fn hls_executor_failure_and_cancel_never_publish_incomplete_cache() {
        let manifest =
            "#EXTM3U\n#EXTINF:0.192,\ngood.aac\n#EXTINF:0.192,\nbad.aac\n#EXT-X-ENDLIST\n";
        let fetcher = fixture_fetcher(
            &[
                ("good.aac", SILENCE.to_vec()),
                ("bad.aac", b"invalid".to_vec()),
            ],
            &[manifest],
        );
        let cancel = fixture_cancellation();
        let directory = tempfile::tempdir().unwrap();
        let cache = RemoteAudioCache::new(
            directory.path().to_path_buf(),
            "failed-hls",
            1024 * 1024,
            None,
            0,
        )
        .unwrap();
        let playlist = load_playlist(&fetcher, BASE, &cancel).await.unwrap();
        assert!(
            transfer(&fetcher, BASE, playlist, &cancel, Some(cache.clone()))
                .await
                .is_err()
        );
        assert!(cache.ready_path().is_none());
        assert!(cancel.buffer.reader().byte_len().is_none());
        cancel.buffer.abort();
        let playlist = parse_media_playlist(manifest, BASE).unwrap();
        assert!(
            transfer(&fetcher, BASE, playlist, &cancel, Some(cache.clone()))
                .await
                .is_err()
        );
        assert!(cache.ready_path().is_none());
    }

    #[test]
    fn hls_reload_rejects_changed_media_or_ranges_for_existing_sequence() {
        let original = parse_media_playlist(
            "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:2\n#EXTINF:1,\na.aac\n",
            BASE,
        )
        .unwrap();
        let changed = parse_media_playlist(
            "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:2\n#EXTINF:1,\nb.aac\n",
            BASE,
        )
        .unwrap();
        assert!(compare_reload(&original, &changed).is_err());
        assert!(!compare_reload(&original, &original).unwrap());
    }
}
