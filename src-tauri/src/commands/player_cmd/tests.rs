#[test]
fn playback_ratios_reject_non_finite_values() {
    for name in ["speed", "pitch"] {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(super::require_finite_ratio(value, name).is_err());
        }
        assert!(super::require_finite_ratio(1.05, name).is_ok());
    }
}

use super::{
    balance_centi, claim_generation, is_generation_current, playback_referer, playback_trace_field,
    stream_length_matches, CachedAudioPlaybackRequest, PlaybackUiTraceRequest,
};
use std::sync::atomic::AtomicU64;

#[test]
fn balance_is_rounded_to_hundredths_and_rejects_non_finite_values() {
    assert_eq!(balance_centi(0.0).unwrap(), 0);
    assert_eq!(balance_centi(0.354).unwrap(), 35);
    assert_eq!(balance_centi(-0.35).unwrap(), -35);
    assert_eq!(balance_centi(2.5).unwrap(), 100);
    assert_eq!(balance_centi(-7.0).unwrap(), -100);
    assert!(balance_centi(f32::NAN).is_err());
    assert!(balance_centi(f32::INFINITY).is_err());
}

#[test]
fn bilibili_edge_hosts_get_the_bilibili_referer() {
    for url in [
        "https://b-demo.edge.mountaintoys.cn/upgcxcode/demo.m4s",
        "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/a.m4s",
        "https://xy1x2x3x4xy.mcdn.bilivideo.cn:4483/upgcxcode/a.m4s",
    ] {
        assert_eq!(playback_referer(url), "https://www.bilibili.com", "{url}");
    }
    // 只看主机名：路径里出现 mountaintoys 的其他站点不算
    assert_eq!(
        playback_referer("https://m701.music.126.net/x.mountaintoys.cn.flac"),
        "https://music.163.com"
    );
    assert_eq!(
        playback_referer("https://rr1.googlevideo.com/videoplayback"),
        "https://music.youtube.com"
    );
}

#[test]
fn playback_generation_never_moves_backwards() {
    let generation = AtomicU64::new(10);

    assert!(!claim_generation(&generation, 9));
    assert!(is_generation_current(&generation, 10));
    assert!(claim_generation(&generation, 10));
    assert!(claim_generation(&generation, 11));
    assert!(is_generation_current(&generation, 11));
    assert!(!is_generation_current(&generation, 10));
}

#[test]
fn sequential_stream_length_guard_rejects_truncated_content() {
    assert!(stream_length_matches(Some(128), 128));
    assert!(!stream_length_matches(Some(128), 127));
    assert!(stream_length_matches(None, 127));
}

#[test]
fn cached_audio_request_accepts_frontend_camel_case_fields() {
    let request: CachedAudioPlaybackRequest = serde_json::from_value(serde_json::json!({
        "cacheKey": "bili-BV1-high",
        "durationHintMs": 272_000,
        "startPositionMs": 22_720,
        "useCrossfade": false,
        "fadeOutMs": 0,
        "fadeInMs": 0,
        "cacheLimitBytes": 1024_u64 * 1024 * 1024,
        "requestGeneration": 42,
    }))
    .expect("cached playback request");

    assert_eq!(request.cache_key, "bili-BV1-high");
    assert_eq!(request.start_position_ms, Some(22_720));
    assert_eq!(request.request_generation, 42);
}

#[test]
fn playback_ui_trace_accepts_camel_case_and_sanitizes_lines() {
    let request: PlaybackUiTraceRequest = serde_json::from_value(serde_json::json!({
        "stage": "store_play_enter",
        "trackId": "netease:42",
        "source": "netease",
        "detail": "first\nsecond",
        "requestGeneration": 9,
    }))
    .expect("playback UI trace request");

    assert_eq!(request.track_id.as_deref(), Some("netease:42"));
    assert_eq!(request.request_generation, Some(9));
    assert_eq!(
        playback_trace_field(request.detail.as_deref()),
        "first second"
    );
    assert_eq!(
        playback_trace_field(Some("failed https://media.example/audio?id=secret")),
        "failed [url]",
    );
}
