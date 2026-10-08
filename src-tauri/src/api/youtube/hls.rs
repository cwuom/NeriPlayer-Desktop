use std::collections::HashSet;

use regex::Regex;

use crate::api::transport::FallbackHttp;
use crate::error::{AppError, AppResult};

use super::client::{YtAudioStream, YtStreamType};

const MAX_MANIFEST_BYTES: usize = 8 * 1024 * 1024;

pub fn is_trusted_hls_url(raw: &str) -> bool {
    let Ok(url) = url::Url::parse(raw) else {
        return false;
    };
    let host = url.host_str().unwrap_or_default();
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && (host == "googlevideo.com" || host.ends_with(".googlevideo.com"))
}

pub fn has_manifest_token(raw: &str) -> bool {
    url::Url::parse(raw).ok().is_some_and(|url| {
        url.query_pairs()
            .any(|(key, value)| key == "pot" && !value.is_empty())
            || url.path_segments().is_some_and(|segments| {
                let segments = segments.collect::<Vec<_>>();
                segments
                    .windows(2)
                    .any(|pair| pair[0] == "pot" && !pair[1].is_empty())
            })
    })
}

pub fn append_manifest_token(raw: &str, token: &str) -> Option<String> {
    if !is_trusted_hls_url(raw) {
        return None;
    }
    let mut url = url::Url::parse(raw).ok()?;
    if token.is_empty() || has_manifest_token(raw) {
        return Some(raw.into());
    }
    if url.path().contains("/api/manifest/") {
        url.path_segments_mut()
            .ok()?
            .pop_if_empty()
            .push("pot")
            .push(token);
    } else {
        url.query_pairs_mut().append_pair("pot", token);
    }
    Some(url.into())
}

pub fn carry_manifest_token(master: &str, playlist: &str) -> Option<String> {
    if !is_trusted_hls_url(master) || !is_trusted_hls_url(playlist) {
        return None;
    }
    if has_manifest_token(playlist) {
        return Some(playlist.into());
    }
    let master = url::Url::parse(master).ok()?;
    if let Some((_, token)) = master
        .query_pairs()
        .find(|(key, value)| key == "pot" && !value.is_empty())
    {
        let mut playlist = url::Url::parse(playlist).ok()?;
        playlist.query_pairs_mut().append_pair("pot", &token);
        return Some(playlist.into());
    }
    let segments = master.path_segments()?.collect::<Vec<_>>();
    let token = segments
        .windows(2)
        .find(|pair| pair[0] == "pot" && !pair[1].is_empty())
        .map(|pair| pair[1]);
    match token {
        Some(token) => append_manifest_token(playlist, &urlencoding::decode(token).ok()?),
        None => Some(playlist.into()),
    }
}

pub fn collect_audio_playlists(
    manifest: &str,
    master: &str,
    duration_ms: u64,
) -> Vec<YtAudioStream> {
    let Ok(base) = url::Url::parse(master) else {
        return Vec::new();
    };
    let attributes = Regex::new(r#"([A-Z0-9-]+)=("([^"]*)"|[^,]*)"#).expect("static regex");
    let path_itag = Regex::new(r"/itag/(\d+)").expect("static regex");
    let mut seen = HashSet::new();
    let mut streams = Vec::new();
    for line in manifest.lines().take(10000).map(str::trim) {
        let Some(raw) = line.strip_prefix("#EXT-X-MEDIA:") else {
            continue;
        };
        let attributes = attributes
            .captures_iter(raw)
            .map(|capture| {
                (
                    capture[1].to_string(),
                    capture[2].trim().trim_matches('"').to_string(),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();
        if !attributes
            .get("TYPE")
            .is_some_and(|value| value.eq_ignore_ascii_case("AUDIO"))
        {
            continue;
        }
        let Some(uri) = attributes.get("URI").and_then(|uri| base.join(uri).ok()) else {
            continue;
        };
        if !is_trusted_hls_url(uri.as_str()) {
            continue;
        }
        let Some(uri) = carry_manifest_token(master, uri.as_str()) else {
            continue;
        };
        if !seen.insert(uri.clone()) {
            continue;
        }
        let url = url::Url::parse(&uri).expect("validated URL");
        let itag = url
            .query_pairs()
            .find(|(key, _)| key == "itag")
            .and_then(|(_, value)| value.parse::<u64>().ok())
            .or_else(|| {
                path_itag
                    .captures(url.path())
                    .and_then(|capture| capture[1].parse().ok())
            });
        let content_length = url
            .query_pairs()
            .find(|(key, _)| key == "clen")
            .and_then(|(_, value)| value.parse::<u64>().ok())
            .unwrap_or(0);
        let bitrate = match itag {
            Some(139 | 233 | 249) => 48000,
            Some(140 | 234 | 250) => 128000,
            Some(141 | 251) => 256000,
            _ if duration_ms > 0 => content_length.saturating_mul(8000) / duration_ms,
            _ => 0,
        };
        streams.push(YtAudioStream {
            url: uri,
            bitrate,
            mime_type: "application/vnd.apple.mpegurl".into(),
            content_length,
            stream_type: YtStreamType::Hls,
        });
    }
    streams.sort_by(|first, second| {
        second
            .bitrate
            .cmp(&first.bitrate)
            .then(second.content_length.cmp(&first.content_length))
    });
    streams
}

pub(super) async fn fetch_audio_playlists(
    http: &FallbackHttp,
    manifest_url: &str,
    duration_ms: u64,
) -> AppResult<Vec<YtAudioStream>> {
    if !is_trusted_hls_url(manifest_url) {
        return Err(AppError::Api("Untrusted YouTube HLS manifest".into()));
    }
    let mut response = http
        .send(|client| {
            client.get(manifest_url).header(
                "User-Agent",
                super::playback::stream_user_agent_for_url(manifest_url),
            )
        })
        .await?
        .error_for_status()?;
    if !response.status().is_success() {
        return Err(AppError::Api(
            "YouTube HLS manifest redirected or returned no media".into(),
        ));
    }
    if !is_trusted_hls_url(response.url().as_str()) {
        return Err(AppError::Api(
            "YouTube HLS manifest redirected to untrusted host".into(),
        ));
    }
    let effective_url = response.url().as_str().to_owned();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > MAX_MANIFEST_BYTES {
            return Err(AppError::Api(
                "YouTube HLS manifest exceeds size limit".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let manifest = String::from_utf8(bytes)
        .map_err(|_| AppError::Api("YouTube HLS manifest is not UTF-8".into()))?;
    if !manifest.trim_start().starts_with("#EXTM3U") {
        return Err(AppError::Api(
            "YouTube HLS manifest has invalid format".into(),
        ));
    }
    Ok(collect_audio_playlists(
        &manifest,
        &effective_url,
        duration_ms,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_helpers_reject_untrusted_parent_or_child_even_with_existing_token() {
        let trusted = "https://manifest.googlevideo.com/api/manifest/hls/itag/234";
        for unsafe_url in [
            "https://evil.test/media.m3u8?pot=existing",
            "https://manifest.googlevideo.com:8443/media.m3u8",
            "https://user@manifest.googlevideo.com/media.m3u8",
        ] {
            assert!(carry_manifest_token(trusted, unsafe_url).is_none());
            assert!(carry_manifest_token(unsafe_url, trusted).is_none());
            assert!(append_manifest_token(unsafe_url, "fixture-token").is_none());
        }
    }

    #[test]
    fn selects_audio_renditions_and_carries_query_token() {
        let manifest = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aac\",NAME=\"Low, audio\",URI=\"low.m3u8?itag=233\"\n#EXT-X-MEDIA:TYPE=AUDIO,URI=\"high.m3u8?itag=234\"\n#EXT-X-MEDIA:TYPE=VIDEO,URI=\"video.m3u8\"\n#EXT-X-MEDIA:TYPE=AUDIO,URI=\"https://evil.test/audio.m3u8\"\n";
        let streams = collect_audio_playlists(
            manifest,
            "https://manifest.googlevideo.com/master.m3u8?pot=fixture",
            0,
        );
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].bitrate, 128000);
        assert_eq!(streams[1].bitrate, 48000);
        assert!(streams[0].url.contains("pot=fixture"));
        assert_eq!(streams[0].stream_type, YtStreamType::Hls);
    }

    #[test]
    fn carries_path_token_without_overwriting_existing_child_token() {
        let master = append_manifest_token(
            "https://manifest.googlevideo.com/api/manifest/hls/id/test/",
            "a/b+==",
        )
        .unwrap();
        assert!(master.ends_with("/pot/a%2Fb+=="));
        assert!(has_manifest_token(&master));
        let child = carry_manifest_token(
            &master,
            "https://manifest.googlevideo.com/api/manifest/hls/itag/234",
        )
        .unwrap();
        assert!(has_manifest_token(&child));
        let existing = "https://manifest.googlevideo.com/child?pot=existing";
        assert_eq!(carry_manifest_token(&master, existing).unwrap(), existing);
    }
}
