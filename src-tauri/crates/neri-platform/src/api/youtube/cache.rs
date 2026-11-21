use std::collections::VecDeque;

use super::client::YtAudioStream;

const CACHE_TTL_MS: u64 = 8 * 60 * 1000;
const EXPIRY_SAFETY_MARGIN_MS: u64 = 90 * 1000;
const CACHE_MAX_ENTRIES: usize = 64;

struct Entry {
    key: String,
    inserted_at_ms: u64,
    streams: Vec<YtAudioStream>,
}

#[derive(Default)]
pub(super) struct AudioStreamCache {
    entries: VecDeque<Entry>,
}

fn stream_expires_at_ms(url: &str, cached_at_ms: u64) -> u64 {
    let default_expiry = cached_at_ms.saturating_add(CACHE_TTL_MS);
    let stream_expiry = url::Url::parse(url)
        .ok()
        .and_then(|url| {
            let query_expiry = url
                .query_pairs()
                .filter(|(key, _)| key == "expire")
                .filter_map(|(_, value)| value.parse::<u64>().ok())
                .filter(|expiry| *expiry > 0)
                .min();
            let segments = url.path_segments()?.collect::<Vec<_>>();
            let path_expiry = segments
                .windows(2)
                .filter(|pair| pair[0] == "expire")
                .filter_map(|pair| pair[1].parse::<u64>().ok())
                .filter(|expiry| *expiry > 0)
                .min();
            query_expiry.into_iter().chain(path_expiry).min()
        })
        .filter(|expiry| *expiry > 0)
        .map(|expiry| {
            expiry
                .saturating_mul(1000)
                .saturating_sub(EXPIRY_SAFETY_MARGIN_MS)
                .max(cached_at_ms)
        });
    default_expiry.min(stream_expiry.unwrap_or(default_expiry))
}

impl AudioStreamCache {
    pub(super) fn get(&mut self, key: &str, now_ms: u64) -> Option<Vec<YtAudioStream>> {
        let index = self.entries.iter().position(|entry| entry.key == key)?;
        let entry = &mut self.entries[index];
        entry.streams.retain(|stream| {
            now_ms >= entry.inserted_at_ms
                && now_ms < stream_expires_at_ms(&stream.url, entry.inserted_at_ms)
        });
        if entry.streams.is_empty() {
            self.entries.remove(index);
            None
        } else {
            Some(entry.streams.clone())
        }
    }

    pub(super) fn put(&mut self, key: String, streams: Vec<YtAudioStream>, now_ms: u64) {
        self.entries.retain(|entry| entry.key != key);
        if streams.is_empty() {
            return;
        }
        self.entries.push_back(Entry {
            key,
            inserted_at_ms: now_ms,
            streams,
        });
        while self.entries.len() > CACHE_MAX_ENTRIES {
            self.entries.pop_front();
        }
    }

    pub(super) fn remove(&mut self, key: &str) {
        self.entries.retain(|entry| entry.key != key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(url: &str) -> YtAudioStream {
        YtAudioStream {
            url: url.into(),
            bitrate: 128000,
            mime_type: "audio/mp4".into(),
            content_length: 100,
            stream_type: super::super::client::YtStreamType::Direct,
        }
    }

    #[test]
    fn expires_ninety_seconds_before_url_expiry() {
        let mut cache = AudioStreamCache::default();
        cache.put(
            "video".into(),
            vec![stream("https://rr.googlevideo.com/v?expire=200")],
            100000,
        );
        assert!(cache.get("video", 109999).is_some());
        assert!(cache.get("video", 110000).is_none());
    }

    #[test]
    fn no_expiry_uses_eight_minutes_and_rejects_clock_rollback() {
        let mut cache = AudioStreamCache::default();
        cache.put(
            "video".into(),
            vec![stream("https://rr.googlevideo.com/v")],
            1000,
        );
        assert!(cache.get("video", 480999).is_some());
        assert!(cache.get("video", 481000).is_none());
        cache.put(
            "video".into(),
            vec![stream("https://rr.googlevideo.com/v")],
            1000,
        );
        assert!(cache.get("video", 999).is_none());
    }

    #[test]
    fn hls_path_expiry_uses_earliest_valid_path_or_query_deadline() {
        for url in [
            "https://manifest.googlevideo.com/api/manifest/hls/expire/200/itag/234",
            "https://manifest.googlevideo.com/api/manifest/hls/expire/200/itag/234?expire=400",
            "https://manifest.googlevideo.com/api/manifest/hls/expire/400/itag/234?expire=200",
            "https://manifest.googlevideo.com/api/manifest/hls/expire/bad/expire/200?expire=0&expire=400",
        ] {
            let mut cache = AudioStreamCache::default();
            let mut candidate = stream(url);
            candidate.stream_type = super::super::client::YtStreamType::Hls;
            cache.put("video|hls=true".into(), vec![candidate], 100000);
            assert!(cache.get("video|hls=true", 109999).is_some(), "{url}");
            assert!(cache.get("video|hls=true", 110000).is_none(), "{url}");
        }
    }

    #[test]
    fn expired_candidate_does_not_hide_remaining_valid_candidates() {
        let mut cache = AudioStreamCache::default();
        cache.put(
            "video".into(),
            vec![
                stream("https://rr.googlevideo.com/v?expire=200"),
                stream("https://rr.googlevideo.com/v?expire=400"),
            ],
            100000,
        );
        assert_eq!(cache.get("video", 110000).unwrap().len(), 1);
    }

    #[test]
    fn fifo_capacity_is_bounded_and_replacement_is_newest() {
        let mut cache = AudioStreamCache::default();
        for index in 0..CACHE_MAX_ENTRIES {
            cache.put(
                index.to_string(),
                vec![stream("https://rr.googlevideo.com/v")],
                1000,
            );
        }
        cache.put(
            "0".into(),
            vec![stream("https://rr.googlevideo.com/new")],
            1000,
        );
        cache.put(
            "extra".into(),
            vec![stream("https://rr.googlevideo.com/v")],
            1000,
        );
        assert!(cache.get("1", 1000).is_none());
        assert_eq!(
            cache.get("0", 1000).unwrap()[0].url,
            "https://rr.googlevideo.com/new"
        );
        cache.remove("0");
        assert!(cache.get("0", 1000).is_none());
        assert_eq!(cache.entries.len(), CACHE_MAX_ENTRIES - 1);
    }
}
