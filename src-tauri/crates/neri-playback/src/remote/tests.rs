use crate::pcm::PcmSource;

#[test]
fn android_alignment_completed_aac_reaches_eof_without_virtual_body_waits() {
    let started = std::time::Instant::now();
    let decoder = super::SymphoniaAudioDecoder::new(
        Box::new(std::io::Cursor::new(
            include_bytes!("../fixtures/hls-silence.aac").as_slice(),
        )),
        Some("aac"),
    )
    .unwrap();
    assert_eq!(decoder.count(), 9216);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "normal EOF must not retry for 5.12 seconds"
    );
}

use super::decoder::{seek_target_time, track_duration};
use super::{
    adaptive_block_bytes, cache_duration_is_suspicious, can_return_partial_read,
    claim_prefetch_sequence_start, ewma_bps_after_failure, ewma_bps_after_success,
    format_cache_marker, halved_fetch_len, is_fragmented_mp4_url, parse_cache_marker,
    parse_content_range, prefetch_lane_count, prefetch_plan, prefetch_window, ranges_cover_source,
    release_prefetch_start, remember_validated_audio_cache, replenish_prefetch_window,
    request_timeout_for, reuse_validated_audio_cache, splice_skip_retryable, throughput_sample_bps,
    CacheMarker, CachedRange, CachedSegment, HttpByteRange, InflightFetch, PrefetchPlan,
    PrefetchWindow, RemoteAccessMode, RemoteAudioCache, RemoteAudioInner, RemoteAudioSource,
    RemoteReadCancellation, RemoteSeekIndex, SeekIndexEntry, SymphoniaAudioDecoder,
    ValidatedCacheFile, REMOTE_PREFETCH_CONCURRENCY, REMOTE_PREFETCH_FALLBACK_LOW_WATER_BYTES,
    REMOTE_PREFETCH_FALLBACK_TARGET_BYTES,
};
use reqwest::header::HeaderValue;
use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use symphonia::core::units::{Time, TimeBase};

fn silent_flac_24bit_stereo() -> Vec<u8> {
    use symphonia::core::checksum::{Crc16Ansi, Crc8Ccitt};
    use symphonia::core::io::Monitor;
    // 单帧常量零 PCM，素材在测试中生成，不读取外部歌曲
    let mut data = b"fLaC\x80\x00\x00\x22".to_vec();
    data.extend_from_slice(&16u16.to_be_bytes());
    data.extend_from_slice(&16u16.to_be_bytes());
    data.extend_from_slice(&[0; 6]);
    let stream_info = (96_000u64 << 44) | (1u64 << 41) | (23u64 << 36) | 16;
    data.extend_from_slice(&stream_info.to_be_bytes());
    data.extend_from_slice(&[0; 16]);
    let mut frame = vec![0xff, 0xf8, 0x60, 0x10, 0x00, 0x0f];
    let mut crc8 = Crc8Ccitt::new(0);
    crc8.process_buf_bytes(&frame);
    frame.push(crc8.crc());
    frame.extend_from_slice(&[0; 8]);
    let mut crc16 = Crc16Ansi::new(0);
    crc16.process_buf_bytes(&frame);
    frame.extend_from_slice(&crc16.crc().to_be_bytes());
    data.extend(frame);
    data
}

fn assert_flac_source_info(mut decoder: SymphoniaAudioDecoder) {
    let info = serde_json::to_value(decoder.source_audio_info()).unwrap();
    assert_eq!(info["sampleRateHz"], 96_000);
    assert_eq!(info["channelCount"], 2);
    assert_eq!(info["bitDepth"], 24);
    assert_eq!(info["codec"], "flac");
    assert!(!info.as_object().unwrap().contains_key("bitrate"));
    let samples: Vec<f32> = decoder.by_ref().collect();
    assert_eq!(samples.len(), 32);
    assert!(samples.iter().all(|value| *value == 0.0));
}

#[test]
fn stream_audio_info_flac_bytes_preserves_source_bits_before_f32_conversion() {
    let decoder =
        SymphoniaAudioDecoder::new(Box::new(Cursor::new(silent_flac_24bit_stereo())), None)
            .unwrap();
    assert_flac_source_info(decoder);
}

#[test]
fn stream_audio_info_file_and_growing_flac_match_the_source_header() {
    let bytes = silent_flac_24bit_stereo();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cache.audio");
    std::fs::write(&path, &bytes).unwrap();
    let decoder = SymphoniaAudioDecoder::new_file(&path).unwrap();
    let cache_info = decoder.source_audio_info().with_encoded_bitrate(
        Some(std::fs::metadata(&path).unwrap().len()),
        decoder.total_duration(),
    );
    assert_eq!(serde_json::to_value(cache_info).unwrap()["bitrate"], 2_832);
    assert_flac_source_info(decoder);
    let growing = crate::growing::GrowingAudioBuffer::new();
    for chunk in bytes.chunks(7) {
        growing.append(chunk);
    }
    growing.finish();
    assert_flac_source_info(SymphoniaAudioDecoder::new(Box::new(growing.reader()), None).unwrap());
}

#[test]
fn stream_audio_info_aac_reports_channels_and_rate_without_inventing_bit_depth() {
    let growing = crate::growing::GrowingAudioBuffer::new();
    growing.append(include_bytes!("../fixtures/hls-silence.aac"));
    growing.finish();
    let decoder = SymphoniaAudioDecoder::new(Box::new(growing.reader()), None).unwrap();
    let info = serde_json::to_value(decoder.source_audio_info()).unwrap();
    assert_eq!(info["sampleRateHz"], 48_000);
    assert_eq!(info["channelCount"], 1);
    assert_eq!(info["codec"], "aac");
    assert!(!info.as_object().unwrap().contains_key("bitDepth"));
    assert!(!info.as_object().unwrap().contains_key("bitrate"));
    assert_eq!(decoder.count(), 9_216);
}

#[test]
fn stream_audio_info_pcm_bitrate_uses_encoded_width_instead_of_f32_output() {
    let decoder = SymphoniaAudioDecoder::new(Box::new(Cursor::new(pcm_wav(8))), None).unwrap();
    let info = serde_json::to_value(decoder.source_audio_info()).unwrap();
    assert_eq!(info["sampleRateHz"], 8_000);
    assert_eq!(info["channelCount"], 1);
    assert_eq!(info["bitDepth"], 16);
    assert_eq!(info["bitrate"], 128);
    assert_eq!(info["codec"], "pcm_s16le");
}

#[test]
fn stream_audio_info_average_bitrate_requires_known_length_and_actual_duration() {
    let decoder =
        SymphoniaAudioDecoder::new(Box::new(Cursor::new(silent_flac_24bit_stereo())), None)
            .unwrap();
    let source_info = decoder.source_audio_info();
    for (length, duration) in [
        (Some(59), None),
        (None, Some(Duration::from_secs(1))),
        (Some(59), Some(Duration::ZERO)),
        (Some(0), Some(Duration::from_secs(1))),
    ] {
        let info = serde_json::to_value(source_info.clone().with_encoded_bitrate(length, duration))
            .unwrap();
        assert!(!info.as_object().unwrap().contains_key("bitrate"));
    }
    let info = source_info.with_encoded_bitrate(Some(59), decoder.total_duration());
    assert_eq!(serde_json::to_value(info).unwrap()["bitrate"], 2_832);
    let pcm = SymphoniaAudioDecoder::new(Box::new(Cursor::new(pcm_wav(8))), None).unwrap();
    let info = pcm
        .source_audio_info()
        .with_encoded_bitrate(Some(1_000_000), Some(Duration::from_secs(1)));
    assert_eq!(serde_json::to_value(info).unwrap()["bitrate"], 128);
}

#[test]
fn stream_audio_info_http_range_flac_uses_the_decoded_source() {
    let body = silent_flac_24bit_stereo();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/source.flac", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => {
                    panic!("loopback range server did not receive a request: {error}")
                }
            }
        };
        // Windows 上 accept 出来的连接继承监听端的非阻塞模式，读超时只对阻塞套接字生效
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&chunk[..count]);
        }
        assert!(String::from_utf8_lossy(&request)
            .to_ascii_lowercase()
            .contains("range: bytes=0-"));
        let header = format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len() - 1, body.len(), body.len(),
        );
        stream.write_all(header.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let source = runtime.block_on(RemoteAudioSource::open(
        reqwest::Client::builder().no_proxy().build().unwrap(),
        url,
        String::new(),
        None,
        0,
        Arc::new(AtomicU64::new(1)),
        1,
    ));
    server.join().unwrap();
    assert_flac_source_info(SymphoniaAudioDecoder::new_remote(source.unwrap()).unwrap());
}

/// 首包收够文件头就开始解码，剩下的部分沿用同一个响应在后台收完：
/// 打开不等首包传完，整段读出来逐字节一致，服务端只收到一次请求
#[test]
fn open_returns_before_the_first_block_finishes_streaming() {
    let mut body = b"fLaC".to_vec();
    body.extend(
        (0..(super::REMOTE_INITIAL_BLOCK_BYTES as usize - 4)).map(|index| (index % 251) as u8),
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/streaming.flac", listener.local_addr().unwrap());
    let served = body.clone();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&chunk[..count]);
        }
        let header = format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            served.len() - 1, served.len(), served.len(),
        );
        stream.write_all(header.as_bytes()).unwrap();
        stream.write_all(&served[..40 * 1024]).unwrap();
        stream.flush().unwrap();
        thread::sleep(Duration::from_millis(400));
        stream.write_all(&served[40 * 1024..]).unwrap();
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    // 客户端构建在 Linux 上要加载系统根证书，并行测试时可能耗时上百毫秒；
    // 计时只应覆盖 open 本身
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let started = Instant::now();
    let mut source = runtime
        .block_on(RemoteAudioSource::open(
            client,
            url,
            String::new(),
            None,
            0,
            Arc::new(AtomicU64::new(1)),
            1,
        ))
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(350),
        "打开要在首包传完之前返回: {:?}",
        started.elapsed()
    );

    let mut read_back = Vec::new();
    source.read_to_end(&mut read_back).unwrap();
    server.join().unwrap();
    assert!(read_back == body, "流式收完的首包要与原文件一致");
}

/// 首包剩下的部分传到一半断了：撤销在途登记，读的人自己按 Range 补拉，数据照样完整
#[test]
fn a_broken_probe_tail_is_refetched_by_the_reader() {
    let mut body = b"fLaC".to_vec();
    body.extend(
        (0..(super::REMOTE_INITIAL_BLOCK_BYTES as usize - 4)).map(|index| (index % 241) as u8),
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/broken.flac", listener.local_addr().unwrap());
    let served = body.clone();
    let server = thread::spawn(move || {
        let read_request = |stream: &mut std::net::TcpStream| {
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let count = stream.read(&mut chunk).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..count]);
            }
            String::from_utf8_lossy(&request).to_ascii_lowercase()
        };
        let (mut first, _) = listener.accept().unwrap();
        read_request(&mut first);
        let header = format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            served.len() - 1, served.len(), served.len(),
        );
        first.write_all(header.as_bytes()).unwrap();
        first.write_all(&served[..40 * 1024]).unwrap();
        first.flush().unwrap();
        thread::sleep(Duration::from_millis(100));
        drop(first);
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let Ok((mut stream, _)) = listener.accept() else {
                thread::sleep(Duration::from_millis(5));
                continue;
            };
            stream.set_nonblocking(false).unwrap();
            let request = read_request(&mut stream);
            let range = request
                .split("range: bytes=")
                .nth(1)
                .and_then(|rest| rest.lines().next())
                .unwrap_or("0-");
            let (start, end) = range.split_once('-').unwrap();
            let start: usize = start.trim().parse().unwrap();
            let end: usize = end
                .trim()
                .parse::<usize>()
                .unwrap_or(served.len() - 1)
                .min(served.len() - 1);
            let header = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                served.len(), end - start + 1,
            );
            stream.write_all(header.as_bytes()).unwrap();
            stream.write_all(&served[start..=end]).unwrap();
            if end == served.len() - 1 {
                break;
            }
        }
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut source = runtime
        .block_on(RemoteAudioSource::open(
            reqwest::Client::builder().no_proxy().build().unwrap(),
            url,
            String::new(),
            None,
            0,
            Arc::new(AtomicU64::new(1)),
            1,
        ))
        .unwrap();

    let mut read_back = Vec::new();
    source.read_to_end(&mut read_back).unwrap();
    assert!(read_back == body, "断掉的部分要由读的人补拉回来");
    drop(source);
    server.join().unwrap();
}

/// 点下去时预开还在路上：正式打开等它出结果，不对同一主机再发一次首包请求
#[test]
fn open_joins_a_prewarm_that_is_still_in_flight() {
    let body = silent_flac_24bit_stereo();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/joining.flac", listener.local_addr().unwrap());
    let served = body.clone();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&chunk[..count]);
        }
        thread::sleep(Duration::from_millis(300));
        let header = format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            served.len() - 1, served.len(), served.len(),
        );
        stream.write_all(header.as_bytes()).unwrap();
        stream.write_all(&served).unwrap();
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let warming = {
        let (client, url) = (client.clone(), url.clone());
        runtime.spawn(async move { super::prewarm(&client, &url, "", 0).await })
    };
    thread::sleep(Duration::from_millis(50));

    let source = runtime
        .block_on(RemoteAudioSource::open(
            client,
            url,
            String::new(),
            None,
            0,
            Arc::new(AtomicU64::new(1)),
            1,
        ))
        .expect("等到预开的首包就能打开，不该再发请求");
    assert!(runtime.block_on(warming).unwrap().unwrap());
    server.join().unwrap();
    assert_eq!(source.inner.total_len, body.len() as u64);
}

/// 预开过的直链正式打开时不再请求首包：本地服务只应答一次，之后再连就被拒
#[test]
fn prewarmed_url_opens_without_another_probe() {
    let body = silent_flac_24bit_stereo();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/prewarmed.flac", listener.local_addr().unwrap());
    let served = body.clone();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&chunk[..count]);
        }
        let header = format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            served.len() - 1, served.len(), served.len(),
        );
        stream.write_all(header.as_bytes()).unwrap();
        stream.write_all(&served).unwrap();
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    assert!(runtime
        .block_on(super::prewarm(&client, &url, "", 0))
        .unwrap());
    server.join().unwrap();
    assert!(
        !runtime
            .block_on(super::prewarm(&client, &url, "", 0))
            .unwrap(),
        "同一地址不重复预开"
    );

    let source = runtime
        .block_on(RemoteAudioSource::open(
            client,
            url.clone(),
            String::new(),
            None,
            0,
            Arc::new(AtomicU64::new(1)),
            1,
        ))
        .expect("预开的首包足够打开，不能再发请求");
    assert_eq!(source.inner.total_len, body.len() as u64);
    assert!(
        super::take_warm_probe(&url).is_none(),
        "打开后首包从预开缓存里取走"
    );
}

fn pcm_wav(sample_count: u32) -> Vec<u8> {
    let data_len = sample_count * 2;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&8_000u32.to_le_bytes());
    wav.extend_from_slice(&16_000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.resize(44 + data_len as usize, 0);
    wav
}

#[test]
fn permanent_range_error_is_not_folded_into_partial_eof() {
    let permanent = std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "remote range no longer satisfiable: 416",
    );
    let transient = std::io::Error::new(
        std::io::ErrorKind::ConnectionReset,
        "connection reset while fetching range",
    );

    assert!(!can_return_partial_read(&permanent));
    assert!(can_return_partial_read(&transient));
}

fn pcm_wav_24(samples: &[i32]) -> Vec<u8> {
    let data_len = samples.len() as u32 * 3;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&8_000u32.to_le_bytes());
    wav.extend_from_slice(&24_000u32.to_le_bytes());
    wav.extend_from_slice(&3u16.to_le_bytes());
    wav.extend_from_slice(&24u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        let bytes = sample.to_le_bytes();
        wav.extend_from_slice(&bytes[..3]);
    }
    wav
}

#[test]
fn decoder_preserves_pcm_below_sixteen_bit_resolution() {
    let wav = pcm_wav_24(&[1, 256]);
    let mut decoder = SymphoniaAudioDecoder::new(Box::new(Cursor::new(wav)), Some("wav"))
        .expect("24-bit wav decoder");

    let least_significant_sample = decoder.next().expect("first PCM sample");

    assert!(least_significant_sample > 0.0);
    assert!(least_significant_sample < 1.0 / 32_768.0);
}

#[test]
fn prefetch_window_uses_duration_hint_for_time_based_buffering() {
    // 9MB / 180s ≈ 50KB/s → 5s=250KB、15s=750KB，均被抬到 min
    let window = prefetch_window(9_000_000, 180_000);
    assert_eq!(window.low_water_bytes, 256 * 1024);
    assert_eq!(window.target_bytes, 1024 * 1024);

    // 更高码率时按时间窗口缩放
    let hi = prefetch_window(36_000_000, 180_000);
    assert_eq!(hi.low_water_bytes, 1_000_000);
    assert_eq!(hi.target_bytes, 3_000_000);
}

#[test]
fn prefetch_window_falls_back_when_duration_is_unknown() {
    let window = prefetch_window(10_000_000, 0);

    assert_eq!(
        window,
        PrefetchWindow {
            low_water_bytes: REMOTE_PREFETCH_FALLBACK_LOW_WATER_BYTES,
            target_bytes: REMOTE_PREFETCH_FALLBACK_TARGET_BYTES,
        }
    );
}

#[test]
fn prefetch_plan_refills_only_after_reaching_low_watermark() {
    let window = PrefetchWindow {
        low_water_bytes: 250_000,
        target_bytes: 750_000,
    };

    assert_eq!(prefetch_plan(1_000_000, 1_250_001, 5_000_000, window), None);
    assert_eq!(
        prefetch_plan(1_000_000, 1_250_000, 5_000_000, window),
        Some(PrefetchPlan {
            start: 1_250_000,
            target_end: 1_750_000,
        })
    );
}

#[test]
fn prefetch_plan_never_reads_past_source_end() {
    let window = PrefetchWindow {
        low_water_bytes: 250_000,
        target_bytes: 750_000,
    };

    assert_eq!(
        prefetch_plan(4_800_000, 4_900_000, 5_000_000, window),
        Some(PrefetchPlan {
            start: 4_900_000,
            target_end: 5_000_000,
        })
    );
}

#[test]
fn duplicate_prefetch_sequence_is_rejected_before_task_submission() {
    let inner = RemoteAudioInner {
        client: reqwest::Client::new(),
        url: "http://127.0.0.1:9/should-not-be-requested".into(),
        referer: "https://example.com".into(),
        total_len: 4,
        header_end: AtomicU64::new(0),
        prefetch_window: PrefetchWindow {
            low_water_bytes: 1,
            target_bytes: 4,
        },
        cache: Mutex::new(Vec::new()),
        disk_ranges: Mutex::new(Vec::new()),
        in_flight: Mutex::new(HashMap::new()),
        range_available: Condvar::new(),
        demand_fetches: AtomicUsize::new(0),
        last_read_pos: Mutex::new(0),
        protected_ranges: Mutex::new(Vec::new()),
        disk_cache: None,
        cache_finalize_started: AtomicBool::new(false),
        prefetch_epoch: AtomicU64::new(0),
        remote_forbidden: AtomicBool::new(false),
        demuxer_open_sequential: AtomicBool::new(false),
        virtual_body_origin: AtomicU64::new(0),
        seek_index: Mutex::new(None),
        throughput_ewma_bps: AtomicU64::new(0),
        bitrate_bps: 0,
        playback_generation: Arc::new(AtomicU64::new(1)),
        expected_generation: 1,
    };
    let plan = PrefetchPlan {
        start: 0,
        target_end: 4,
    };

    let (start, _end, entry) =
        claim_prefetch_sequence_start(&inner, plan).expect("first sequence claim");
    assert_eq!(start, 0);
    assert!(claim_prefetch_sequence_start(&inner, plan).is_none());
    assert_eq!(inner.in_flight.lock().expect("in-flight lock").len(), 1);

    release_prefetch_start(&inner, 0, &entry);
    let (start, _end, entry) =
        claim_prefetch_sequence_start(&inner, plan).expect("reclaim after release");
    assert_eq!(start, 0);
    release_prefetch_start(&inner, 0, &entry);
}

#[test]
fn demand_read_waits_for_matching_prefetch_instead_of_downloading_twice() {
    let playback_generation = Arc::new(AtomicU64::new(1));
    let inflight_entry = Arc::new(InflightFetch {
        end: AtomicU64::new(3),
        done: AtomicBool::new(false),
    });
    let inner = Arc::new(RemoteAudioInner {
        client: reqwest::Client::new(),
        url: "http://127.0.0.1:9/should-not-be-requested".into(),
        referer: "https://example.com".into(),
        total_len: 4,
        header_end: AtomicU64::new(0),
        prefetch_window: PrefetchWindow {
            low_water_bytes: 1,
            target_bytes: 1,
        },
        cache: Mutex::new(Vec::new()),
        disk_ranges: Mutex::new(Vec::new()),
        in_flight: Mutex::new(HashMap::from([(0, Arc::clone(&inflight_entry))])),
        range_available: Condvar::new(),
        demand_fetches: AtomicUsize::new(0),
        last_read_pos: Mutex::new(0),
        protected_ranges: Mutex::new(Vec::new()),
        disk_cache: None,
        cache_finalize_started: AtomicBool::new(false),
        prefetch_epoch: AtomicU64::new(0),
        remote_forbidden: AtomicBool::new(false),
        demuxer_open_sequential: AtomicBool::new(false),
        virtual_body_origin: AtomicU64::new(0),
        seek_index: Mutex::new(None),
        throughput_ewma_bps: AtomicU64::new(0),
        bitrate_bps: 0,
        playback_generation,
        expected_generation: 1,
    });
    let producer_inner = Arc::clone(&inner);
    let producer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        producer_inner
            .cache
            .lock()
            .expect("cache lock")
            .push(CachedSegment {
                start: 0,
                data: vec![1, 2, 3, 4],
            });
        release_prefetch_start(&producer_inner, 0, &inflight_entry);
    });
    let mut source = RemoteAudioSource {
        inner,
        pos: 0,
        access_mode: RemoteAccessMode::StandardSeekable,
        read_cancellation: None,
    };
    let mut bytes = [0; 4];

    source.read_exact(&mut bytes).expect("prefetched read");

    assert_eq!(bytes, [1, 2, 3, 4]);
    assert!(producer.join().is_ok());
}

/// 需求读落在在途区间中段（起点不同）也必须共享结果而非重复拉取
#[test]
fn demand_read_shares_inflight_range_covering_its_offset() {
    let inflight_entry = Arc::new(InflightFetch {
        end: AtomicU64::new(1023),
        done: AtomicBool::new(false),
    });
    let inner = Arc::new(RemoteAudioInner {
        client: reqwest::Client::new(),
        url: "http://127.0.0.1:9/should-not-be-requested".into(),
        referer: "https://example.com".into(),
        total_len: 1024,
        header_end: AtomicU64::new(0),
        prefetch_window: PrefetchWindow {
            low_water_bytes: 1,
            target_bytes: 1,
        },
        cache: Mutex::new(Vec::new()),
        disk_ranges: Mutex::new(Vec::new()),
        in_flight: Mutex::new(HashMap::from([(0, Arc::clone(&inflight_entry))])),
        range_available: Condvar::new(),
        demand_fetches: AtomicUsize::new(0),
        last_read_pos: Mutex::new(0),
        protected_ranges: Mutex::new(Vec::new()),
        disk_cache: None,
        cache_finalize_started: AtomicBool::new(false),
        prefetch_epoch: AtomicU64::new(0),
        remote_forbidden: AtomicBool::new(false),
        demuxer_open_sequential: AtomicBool::new(false),
        virtual_body_origin: AtomicU64::new(0),
        seek_index: Mutex::new(None),
        throughput_ewma_bps: AtomicU64::new(0),
        bitrate_bps: 0,
        playback_generation: Arc::new(AtomicU64::new(1)),
        expected_generation: 1,
    });
    let producer_inner = Arc::clone(&inner);
    let producer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        producer_inner
            .cache
            .lock()
            .expect("cache lock")
            .push(CachedSegment {
                start: 0,
                data: vec![7; 1024],
            });
        release_prefetch_start(&producer_inner, 0, &inflight_entry);
    });
    let mut source = RemoteAudioSource {
        inner,
        pos: 512,
        access_mode: RemoteAccessMode::StandardSeekable,
        read_cancellation: None,
    };
    let mut bytes = [0; 4];

    source.read_exact(&mut bytes).expect("mid-range read");

    assert_eq!(bytes, [7, 7, 7, 7]);
    assert!(producer.join().is_ok());
}

/// 代际作废必须立刻解除共享等待方，不允许陪旧预取等满超时预算
#[test]
fn epoch_invalidation_releases_inflight_waiters_immediately() {
    let inner = Arc::new(RemoteAudioInner {
        client: reqwest::Client::new(),
        url: "http://127.0.0.1:9/should-not-be-requested".into(),
        referer: "https://example.com".into(),
        total_len: 4,
        header_end: AtomicU64::new(0),
        prefetch_window: PrefetchWindow {
            low_water_bytes: 1,
            target_bytes: 1,
        },
        cache: Mutex::new(Vec::new()),
        disk_ranges: Mutex::new(Vec::new()),
        in_flight: Mutex::new(HashMap::from([(
            0,
            Arc::new(InflightFetch {
                end: AtomicU64::new(3),
                done: AtomicBool::new(false),
            }),
        )])),
        range_available: Condvar::new(),
        demand_fetches: AtomicUsize::new(0),
        last_read_pos: Mutex::new(0),
        protected_ranges: Mutex::new(Vec::new()),
        disk_cache: None,
        cache_finalize_started: AtomicBool::new(false),
        prefetch_epoch: AtomicU64::new(0),
        remote_forbidden: AtomicBool::new(false),
        demuxer_open_sequential: AtomicBool::new(false),
        virtual_body_origin: AtomicU64::new(0),
        seek_index: Mutex::new(None),
        throughput_ewma_bps: AtomicU64::new(0),
        bitrate_bps: 0,
        playback_generation: Arc::new(AtomicU64::new(1)),
        expected_generation: 1,
    });
    let invalidator_inner = Arc::clone(&inner);
    let invalidator = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        // seek 语义：数据入缓存后作废旧代，等待方应立即醒来并命中缓存
        invalidator_inner
            .cache
            .lock()
            .expect("cache lock")
            .push(CachedSegment {
                start: 0,
                data: vec![5, 6, 7, 8],
            });
        super::invalidate_prefetch_epoch(&invalidator_inner);
    });
    let mut source = RemoteAudioSource {
        inner,
        pos: 0,
        access_mode: RemoteAccessMode::StandardSeekable,
        read_cancellation: None,
    };
    let mut bytes = [0; 4];
    let started = std::time::Instant::now();

    source
        .read_exact(&mut bytes)
        .expect("read after invalidate");

    assert_eq!(bytes, [5, 6, 7, 8]);
    // 远小于 60s+5s 的防御预算，证明等待方是被 invalidate 主动唤醒的
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(invalidator.join().is_ok());
}

/// 起点被在途区间覆盖时不得再登记新请求（转入共享等待）
#[test]
fn claim_rejects_offset_covered_by_inflight_range() {
    let inner = RemoteAudioInner {
        client: reqwest::Client::new(),
        url: "http://127.0.0.1:9/should-not-be-requested".into(),
        referer: "https://example.com".into(),
        total_len: 1024,
        header_end: AtomicU64::new(0),
        prefetch_window: PrefetchWindow {
            low_water_bytes: 1,
            target_bytes: 1,
        },
        cache: Mutex::new(Vec::new()),
        disk_ranges: Mutex::new(Vec::new()),
        in_flight: Mutex::new(HashMap::from([(
            0,
            Arc::new(InflightFetch {
                end: AtomicU64::new(255),
                done: AtomicBool::new(false),
            }),
        )])),
        range_available: Condvar::new(),
        demand_fetches: AtomicUsize::new(0),
        last_read_pos: Mutex::new(0),
        protected_ranges: Mutex::new(Vec::new()),
        disk_cache: None,
        cache_finalize_started: AtomicBool::new(false),
        prefetch_epoch: AtomicU64::new(0),
        remote_forbidden: AtomicBool::new(false),
        demuxer_open_sequential: AtomicBool::new(false),
        virtual_body_origin: AtomicU64::new(0),
        seek_index: Mutex::new(None),
        throughput_ewma_bps: AtomicU64::new(0),
        bitrate_bps: 0,
        playback_generation: Arc::new(AtomicU64::new(1)),
        expected_generation: 1,
    };

    // 中段偏移已被 [0, 255] 在途请求覆盖 → 拒绝重复登记
    assert!(super::claim_prefetch_start(&inner, 128, 383).is_none());
    // 在途区间之外可以正常登记
    let entry = super::claim_prefetch_start(&inner, 256, 511).expect("claim beyond range");
    super::release_prefetch_start(&inner, 256, &entry);
}

fn assert_cancelled_read_is_not_retried(source: RemoteAudioSource, use_symphonia: bool) {
    struct CountedSource {
        source: RemoteAudioSource,
        reads: Arc<AtomicUsize>,
    }

    impl Read for CountedSource {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let reads = self.reads.fetch_add(1, Ordering::Relaxed);
            if reads >= 32 {
                return Err(std::io::Error::other("cancelled read retry limit"));
            }
            self.source.read(out)
        }
    }

    impl std::io::Seek for CountedSource {
        fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
            std::io::Seek::seek(&mut self.source, position)
        }
    }

    impl symphonia::core::io::MediaSource for CountedSource {
        fn is_seekable(&self) -> bool {
            true
        }

        fn byte_len(&self) -> Option<u64> {
            Some(self.source.inner.total_len)
        }
    }

    assert!(crate::ffmpeg::ByteInput::interrupted(&source));
    let seek_error = std::io::Seek::seek(&mut source.clone(), std::io::SeekFrom::Start(0))
        .expect_err("cancelled seek");
    assert_eq!(seek_error.kind(), std::io::ErrorKind::Other);
    let fetch_error = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(super::fetch_range_block_async(
            &source.inner,
            0,
            3,
            source.read_cancellation.as_ref(),
        ))
        .expect_err("cancelled fetch must stop before network I/O");
    assert_eq!(fetch_error.kind(), std::io::ErrorKind::Other);

    let reads = Arc::new(AtomicUsize::new(0));
    let mut counted = CountedSource {
        source,
        reads: Arc::clone(&reads),
    };
    let error = if use_symphonia {
        let mut stream = symphonia::core::io::MediaSourceStream::new(
            Box::new(counted),
            symphonia::core::io::MediaSourceStreamOptions::default(),
        );
        symphonia::core::io::ReadBytes::read_buf_exact(&mut stream, &mut [0; 4])
            .expect_err("cancelled symphonia read")
    } else {
        counted
            .read_exact(&mut [0; 4])
            .expect_err("cancelled exact read")
    };
    assert_eq!(
        reads.load(Ordering::Relaxed),
        1,
        "permanent cancellation must escape exact reads without retrying"
    );
    assert!(!super::is_transient_fetch_error(&error));
    assert!(!splice_skip_retryable(
        &symphonia::core::errors::Error::IoError(error)
    ));
}

#[tokio::test]
async fn cancelled_inflight_range_read_is_permanent() {
    use tokio::io::AsyncReadExt;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let inner = forbidden_test_inner(
        format!("http://{}/cancel", listener.local_addr().unwrap()),
        4,
    );
    let stopped = Arc::new(AtomicBool::new(false));
    let cancellation = RemoteReadCancellation::new(Arc::clone(&stopped), None);
    let (released, release_wait) = tokio::sync::oneshot::channel();
    let request = async {
        let result = super::fetch_range_block_async(&inner, 0, 3, Some(&cancellation)).await;
        let _ = released.send(());
        result
    };
    let server = async {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut chunk = [0u8; 512];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut chunk).await.unwrap();
            assert!(count > 0);
            request.extend_from_slice(&chunk[..count]);
        }
        stopped.store(true, Ordering::Release);
        let _ = release_wait.await;
        drop(stream);
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(request, server)
    })
    .await
    .expect("in-flight cancellation must finish without the request timeout");
    let error = result.expect_err("cancelled in-flight fetch");
    assert_eq!(error.kind(), std::io::ErrorKind::Other);
    assert!(!super::is_transient_fetch_error(&error));
    assert!(!splice_skip_retryable(
        &symphonia::core::errors::Error::IoError(error)
    ));
}

#[test]
fn superseded_remote_source_cancels_before_network_or_cache_io() {
    let playback_generation = Arc::new(AtomicU64::new(1));
    let inner = Arc::new(RemoteAudioInner {
        client: reqwest::Client::new(),
        url: "http://127.0.0.1:9/should-not-be-requested".into(),
        referer: "https://example.com".into(),
        total_len: 4,
        header_end: AtomicU64::new(0),
        prefetch_window: PrefetchWindow {
            low_water_bytes: 1,
            target_bytes: 1,
        },
        cache: Mutex::new(Vec::new()),
        disk_ranges: Mutex::new(Vec::new()),
        in_flight: Mutex::new(HashMap::new()),
        range_available: Condvar::new(),
        demand_fetches: AtomicUsize::new(0),
        last_read_pos: Mutex::new(0),
        protected_ranges: Mutex::new(Vec::new()),
        disk_cache: None,
        cache_finalize_started: AtomicBool::new(false),
        prefetch_epoch: AtomicU64::new(0),
        remote_forbidden: AtomicBool::new(false),
        demuxer_open_sequential: AtomicBool::new(false),
        virtual_body_origin: AtomicU64::new(0),
        seek_index: Mutex::new(None),
        throughput_ewma_bps: AtomicU64::new(0),
        bitrate_bps: 0,
        playback_generation: Arc::clone(&playback_generation),
        expected_generation: 1,
    });
    playback_generation.store(2, std::sync::atomic::Ordering::Release);
    let mut source = RemoteAudioSource {
        inner,
        pos: 0,
        access_mode: RemoteAccessMode::StandardSeekable,
        read_cancellation: None,
    };
    let mut byte = [0];

    let error = source.read(&mut byte).expect_err("superseded read");

    assert_eq!(error.kind(), std::io::ErrorKind::Other);
    assert_cancelled_read_is_not_retried(source, false);
}

#[test]
fn stopped_session_cancels_remote_read_without_changing_track_generation() {
    let playback_generation = Arc::new(AtomicU64::new(1));
    let session_cancelled = Arc::new(AtomicBool::new(false));
    let inner = Arc::new(RemoteAudioInner {
        client: reqwest::Client::new(),
        url: "http://127.0.0.1:9/should-not-be-requested".into(),
        referer: "https://example.com".into(),
        total_len: 4,
        header_end: AtomicU64::new(0),
        prefetch_window: PrefetchWindow {
            low_water_bytes: 1,
            target_bytes: 1,
        },
        cache: Mutex::new(Vec::new()),
        disk_ranges: Mutex::new(Vec::new()),
        in_flight: Mutex::new(HashMap::new()),
        range_available: Condvar::new(),
        demand_fetches: AtomicUsize::new(0),
        last_read_pos: Mutex::new(0),
        protected_ranges: Mutex::new(Vec::new()),
        disk_cache: None,
        cache_finalize_started: AtomicBool::new(false),
        prefetch_epoch: AtomicU64::new(0),
        remote_forbidden: AtomicBool::new(false),
        demuxer_open_sequential: AtomicBool::new(false),
        virtual_body_origin: AtomicU64::new(0),
        seek_index: Mutex::new(None),
        throughput_ewma_bps: AtomicU64::new(0),
        bitrate_bps: 0,
        playback_generation,
        expected_generation: 1,
    });
    let mut source = RemoteAudioSource {
        inner,
        pos: 0,
        access_mode: RemoteAccessMode::StandardSeekable,
        read_cancellation: Some(RemoteReadCancellation::new(
            Arc::clone(&session_cancelled),
            None,
        )),
    };
    session_cancelled.store(true, Ordering::Release);

    let error = source.read(&mut [0]).expect_err("stopped session read");

    assert_eq!(error.kind(), std::io::ErrorKind::Other);
    assert_cancelled_read_is_not_retried(source, true);
}

#[test]
fn fragmented_mp4_starts_progressively_and_promotes_for_seek() {
    assert!(is_fragmented_mp4_url(
        "https://cdn.example/audio/track.M4S?deadline=123"
    ));
    assert!(!is_fragmented_mp4_url(
        "https://cdn.example/audio/track.flac?deadline=123"
    ));

    let source = RemoteAudioSource {
        inner: Arc::new(RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "https://cdn.example/audio/track.m4s".into(),
            referer: "https://example.com".into(),
            total_len: 4,
            header_end: AtomicU64::new(0),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 1,
                target_bytes: 1,
            },
            cache: Mutex::new(Vec::new()),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::new()),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(0),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::new(AtomicU64::new(1)),
            expected_generation: 1,
        }),
        pos: 0,
        access_mode: RemoteAccessMode::FragmentedProgressive,
        read_cancellation: None,
    };

    assert!(!symphonia::core::io::MediaSource::is_seekable(&source));
    assert!(symphonia::core::io::MediaSource::byte_len(&source).is_none());
    let seekable = source.seekable_clone();
    // demuxer open 阶段隐藏 seekable，避免扫 mdat
    assert!(!symphonia::core::io::MediaSource::is_seekable(&seekable));
    assert!(source
        .inner
        .demuxer_open_sequential
        .load(std::sync::atomic::Ordering::Acquire));
    seekable.finish_demuxer_open();
    assert!(symphonia::core::io::MediaSource::is_seekable(&seekable));
    assert_eq!(
        seekable.access_mode.demand_fetch_block_bytes(),
        super::REMOTE_FRAGMENTED_SEEK_BLOCK_BYTES
    );
}

#[test]
fn long_form_progressive_promotes_to_standard_seekable() {
    let source = RemoteAudioSource {
        inner: Arc::new(RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "https://cdn.example/audio/long.m4a".into(),
            referer: "https://example.com".into(),
            total_len: 135_000_000,
            header_end: AtomicU64::new(0),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 256 * 1024,
                target_bytes: 1024 * 1024,
            },
            cache: Mutex::new(Vec::new()),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::new()),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(0),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::new(AtomicU64::new(1)),
            expected_generation: 1,
        }),
        pos: 0,
        access_mode: RemoteAccessMode::LongFormProgressive,
        read_cancellation: None,
    };

    assert!(!symphonia::core::io::MediaSource::is_seekable(&source));
    assert!(symphonia::core::io::MediaSource::byte_len(&source).is_none());

    let seekable = source.seekable_clone();
    assert_eq!(seekable.access_mode, RemoteAccessMode::StandardSeekable);
    // demuxer open 阶段隐藏 seekable，finish 后才对 demuxer 暴露
    assert!(!symphonia::core::io::MediaSource::is_seekable(&seekable));
    assert!(source
        .inner
        .demuxer_open_sequential
        .load(std::sync::atomic::Ordering::Acquire));
    seekable.finish_demuxer_open();
    assert!(symphonia::core::io::MediaSource::is_seekable(&seekable));
    // 绝不能走分片 8MB 按需块，否则 seek 会卡死
    assert_eq!(
        seekable.access_mode.demand_fetch_block_bytes(),
        super::REMOTE_FETCH_BLOCK_BYTES
    );
    // seekable_clone 必须作废旧预取 epoch，否则 seek 会撞 in_flight
    assert_eq!(
        source
            .inner
            .prefetch_epoch
            .load(std::sync::atomic::Ordering::Acquire),
        1
    );
    assert!(source.inner.in_flight.lock().expect("lock").is_empty());
}

#[test]
fn long_form_prefers_virtual_body_before_and_after_promote() {
    let index = super::RemoteSeekIndex {
        duration_ms: 100_000,
        entries: vec![
            super::SeekIndexEntry {
                time_ms: 0,
                byte_offset: 1_000,
            },
            super::SeekIndexEntry {
                time_ms: 50_000,
                byte_offset: 50_000_000,
            },
            super::SeekIndexEntry {
                time_ms: 100_000,
                byte_offset: 100_000_000,
            },
        ],
        exact_times: true,
    };
    let source = RemoteAudioSource {
        inner: Arc::new(RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "https://cdn.example/audio/long.m4a".into(),
            referer: "https://example.com".into(),
            total_len: 135_000_000,
            header_end: AtomicU64::new(10_799),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 256 * 1024,
                target_bytes: 1024 * 1024,
            },
            cache: Mutex::new(Vec::new()),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::new()),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(0),
            seek_index: Mutex::new(Some(index)),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::new(AtomicU64::new(1)),
            expected_generation: 1,
        }),
        pos: 0,
        access_mode: RemoteAccessMode::LongFormProgressive,
        read_cancellation: None,
    };

    // 关键：clone 前 LongForm 也必须 prefers，否则 seek 会先选 format.seek
    assert!(source.prefers_virtual_body_seek());
    let seekable = source.seekable_clone();
    assert!(seekable.prefers_virtual_body_seek());
    assert_eq!(seekable.access_mode, RemoteAccessMode::StandardSeekable);
}

#[test]
fn virtual_body_media_source_seekable_after_open_only() {
    let source = RemoteAudioSource {
        inner: Arc::new(RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "https://cdn.example/audio/long.m4a".into(),
            referer: "https://example.com".into(),
            total_len: 10_000,
            header_end: AtomicU64::new(100),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 1,
                target_bytes: 1,
            },
            cache: Mutex::new(Vec::new()),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::new()),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(true),
            virtual_body_origin: AtomicU64::new(5_000),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::new(AtomicU64::new(1)),
            expected_generation: 1,
        }),
        pos: 0,
        access_mode: RemoteAccessMode::StandardSeekable,
        read_cancellation: None,
    };

    assert!(!symphonia::core::io::MediaSource::is_seekable(&source));
    assert!(symphonia::core::io::MediaSource::byte_len(&source).is_none());

    source.finish_demuxer_open();
    assert!(symphonia::core::io::MediaSource::is_seekable(&source));
    assert_eq!(
        symphonia::core::io::MediaSource::byte_len(&source),
        Some(source.logical_len())
    );
    // 纯逻辑连续：header + (total - origin)
    assert_eq!(source.logical_len(), 100 + (10_000 - 5_000));
}

fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

#[test]
fn fragmented_mp4_is_sniffed_from_sidx_moof_and_mvex() {
    // ftyp + sidx（YouTube itag140 的典型头部布局）
    let mut sidx_head = mp4_box(b"ftyp", &[0u8; 8]);
    sidx_head.extend_from_slice(&mp4_box(b"sidx", &[0u8; 16]));
    assert!(super::detect_fragmented_mp4(&sidx_head));

    // ftyp + moov(含 mvex)
    let moov_body = mp4_box(b"mvex", &[0u8; 8]);
    let mut mvex_head = mp4_box(b"ftyp", &[0u8; 8]);
    mvex_head.extend_from_slice(&mp4_box(b"moov", &moov_body));
    assert!(super::detect_fragmented_mp4(&mvex_head));

    // styp 开头的独立分片
    assert!(super::detect_fragmented_mp4(&mp4_box(b"styp", &[0u8; 8])));
}

/// 回归：嗅探到分片只能影响「打开阶段顺序读」，绝不能启用虚拟 body
///
/// 曾经因为给这类流建了 seek 索引而走上 virtual-body 拼接，
/// 样本偏移对不上导致 seek 与回滚双双 end of stream，播放直接死掉。
#[test]
fn sniffed_fragments_must_not_enable_virtual_body_seek() {
    let source = RemoteAudioSource {
        inner: Arc::new(RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "https://rr1---sn-x.googlevideo.com/videoplayback?itag=140".into(),
            referer: "https://www.youtube.com".into(),
            total_len: 4_842_215,
            header_end: AtomicU64::new(1151),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 262_144,
                target_bytes: 1_048_576,
            },
            cache: Mutex::new(Vec::new()),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::new()),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            // open 阶段顺序读，避免 symphonia 顺着 moof 链扫完整个文件
            demuxer_open_sequential: AtomicBool::new(true),
            virtual_body_origin: AtomicU64::new(0),
            // 嗅探路径不建索引，交给 symphonia 原生 sidx 定位
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::new(AtomicU64::new(1)),
            expected_generation: 1,
        }),
        pos: 0,
        access_mode: RemoteAccessMode::StandardSeekable,
        read_cancellation: None,
    };

    // 打开阶段对 demuxer 隐藏 seekable / byte_len，否则会扫全文件
    assert!(!symphonia::core::io::MediaSource::is_seekable(&source));
    assert!(symphonia::core::io::MediaSource::byte_len(&source).is_none());

    source.finish_demuxer_open();
    // 打开完成后恢复可 seek，让 symphonia 自己按 sidx 跳转
    assert!(symphonia::core::io::MediaSource::is_seekable(&source));
    assert!(
        !source.prefers_virtual_body_seek(),
        "虚拟 body 不适用于自带 sidx 的流"
    );

    let seekable = source.seekable_clone();
    assert!(!seekable.prefers_virtual_body_seek());
    // 按需块必须保持 256KB：8MB 是给 Bilibili 独立 .m4s 分片调的
    assert_eq!(
        seekable.access_mode.demand_fetch_block_bytes(),
        super::REMOTE_FETCH_BLOCK_BYTES
    );
}

/// 回归：分片流必须走虚拟 body，且两种模式都不许退化成 8MB 块
///
/// symphonia 0.5.5 解析 sidx 后直接丢弃，format.seek 只能逐个 moof 爬，
/// 代价 O(文件大小)——实测 68MB 音频跳到中段要顺序读满 30MB。
/// 所以只要建得出 sidx 索引就必须改走虚拟 body 的字节跳转；
/// 同时按需块要维持 256KB，8MB 会让一次 seek 卡在整块下完之前。
#[test]
fn fragmented_streams_seek_through_the_virtual_body() {
    for mode in [
        RemoteAccessMode::StandardSeekable,
        RemoteAccessMode::LongFormProgressive,
    ] {
        let source = RemoteAudioSource {
            inner: Arc::new(RemoteAudioInner {
                client: reqwest::Client::new(),
                url: "https://rr5---sn-x.googlevideo.com/videoplayback?itag=140".into(),
                referer: "https://www.youtube.com".into(),
                total_len: 135_169_876,
                header_end: AtomicU64::new(10_799),
                prefetch_window: PrefetchWindow {
                    low_water_bytes: 262_144,
                    target_bytes: 1_048_576,
                },
                cache: Mutex::new(Vec::new()),
                disk_ranges: Mutex::new(Vec::new()),
                in_flight: Mutex::new(HashMap::new()),
                range_available: Condvar::new(),
                demand_fetches: AtomicUsize::new(0),
                last_read_pos: Mutex::new(0),
                protected_ranges: Mutex::new(Vec::new()),
                disk_cache: None,
                cache_finalize_started: AtomicBool::new(false),
                prefetch_epoch: AtomicU64::new(0),
                remote_forbidden: AtomicBool::new(false),
                demuxer_open_sequential: AtomicBool::new(true),
                virtual_body_origin: AtomicU64::new(0),
                seek_index: Mutex::new(Some(RemoteSeekIndex {
                    duration_ms: 8_352_000,
                    entries: vec![
                        SeekIndexEntry {
                            time_ms: 0,
                            byte_offset: 10_799,
                        },
                        SeekIndexEntry {
                            time_ms: 10_000,
                            byte_offset: 172_000,
                        },
                    ],
                    exact_times: true,
                })),
                throughput_ewma_bps: AtomicU64::new(0),
                bitrate_bps: 0,
                playback_generation: Arc::new(AtomicU64::new(1)),
                expected_generation: 1,
            }),
            pos: 0,
            access_mode: mode,
            read_cancellation: None,
        };

        let seekable = source.seekable_clone();
        assert!(
            seekable.prefers_virtual_body_seek(),
            "{mode:?} 有 sidx 索引就必须走虚拟 body，绝不能退回 format.seek",
        );
        assert_eq!(
            seekable.access_mode.seek_fetch_block_bytes(),
            super::REMOTE_FETCH_BLOCK_BYTES,
            "{mode:?} 的 seek 块不得退化成 8MB",
        );
        seekable.finish_demuxer_open();
        // 打开后必须对 demuxer 可 seek，样本才能在拼接视图里回跳
        assert!(symphonia::core::io::MediaSource::is_seekable(&seekable));
    }
}

/// 回归：代际守卫只许在 prepare 阶段杀读，提交后的会话必须免疫
///
/// seek 代际在发送命令的瞬间递增。旧实现的令牌终生生效——用户下一次
/// seek 会先把还在正常播放的会话的按需读掐死（实机：seek 后 6 秒
/// next_packet fatal: remote read superseded，随后一次可听见的重建）。
#[test]
fn operation_guard_stops_killing_reads_after_disarm() {
    let generation = Arc::new(AtomicU64::new(7));
    let session_cancelled = Arc::new(AtomicBool::new(false));
    let cancellation = RemoteReadCancellation::new(
        Arc::clone(&session_cancelled),
        Some((Arc::clone(&generation), 7)),
    );
    // 克隆共享同一守卫：prepare 把令牌分发给解码/预取各处后，
    // 在任何一份上解除都要对全体生效
    let reader_copy = cancellation.clone();

    // prepare 阶段：代际一致不取消，被更新的 seek 抢先则必须取消
    assert!(!reader_copy.is_cancelled());
    generation.store(8, Ordering::Release);
    assert!(
        reader_copy.is_cancelled(),
        "prepare 输给新 seek 时必须可被打断"
    );

    // 会话提交：解除代际守卫，之后代际再怎么涨都不关本会话的事
    generation.store(7, Ordering::Release);
    cancellation.disarm_operation_guard();
    generation.store(99, Ordering::Release);
    assert!(
        !reader_copy.is_cancelled(),
        "已提交会话不得被后续 seek 的代际递增杀死",
    );

    // 会话自身的收尾通道必须仍然有效
    session_cancelled.store(true, Ordering::Release);
    assert!(reader_copy.is_cancelled(), "previous.stop() 仍要能停掉读");
}

/// 回归：迟到的 prepare 超时旗标不得杀死已提交的会话
///
/// 回包超时与提交存在竞态窗口：命令侧 recv 超时置位旗标的一瞬，
/// 音频线程可能刚提交完会话。旧实现里 external_cancel 终生生效，
/// 这一下就把好端端的会话无声杀死——偶发「突然不播了」。
#[test]
fn late_prepare_timeout_cannot_kill_a_committed_session() {
    let external = Arc::new(AtomicBool::new(false));
    let cancellation = RemoteReadCancellation::new(Arc::new(AtomicBool::new(false)), None)
        .with_external_cancel(Arc::clone(&external));

    // prepare 期内超时旗标必须能打断
    external.store(true, Ordering::Release);
    assert!(
        cancellation.is_cancelled(),
        "prepare 卡死时必须可被超时解卡"
    );
    external.store(false, Ordering::Release);

    // 提交后迟到的置位不得再影响
    cancellation.disarm_operation_guard();
    external.store(true, Ordering::Release);
    assert!(
        !cancellation.is_cancelled(),
        "迟到的 prepare 超时不得杀死已提交的会话",
    );
}

/// 端到端复现：真实 YouTube fMP4 字节走完整虚拟 body 打开 + 解码
///
/// 需要实机字节（含版权内容，不入库）。运行方式：
/// NERI_YT_REPRO_DIR=<dir> cargo test -- --ignored yt_virtual_body
/// dir 下需有 yt-head.bin（物理 0..512KB）与 yt-mid.bin（物理 25212301 起 ~1MB）。
/// 修复前该流在 ~12k 帧处死于 "unexpected end of bitstream"；
/// 修复后应能稳定解出 5 秒以上的 PCM。
#[test]
#[ignore]
fn yt_virtual_body_end_to_end_decodes_real_bytes() {
    let Ok(dir) = std::env::var("NERI_YT_REPRO_DIR") else {
        eprintln!("NERI_YT_REPRO_DIR 未设置，跳过");
        return;
    };
    let head = std::fs::read(format!("{dir}/yt-head.bin")).expect("yt-head.bin");
    let mid = std::fs::read(format!("{dir}/yt-mid.bin")).expect("yt-mid.bin");

    let source = RemoteAudioSource {
        inner: Arc::new(RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "https://example.invalid/videoplayback".into(),
            referer: String::new(),
            total_len: 68_201_223,
            header_end: AtomicU64::new(5_831),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 262_144,
                target_bytes: 1_048_576,
            },
            cache: Mutex::new(vec![
                CachedSegment {
                    start: 0,
                    data: head,
                },
                CachedSegment {
                    start: 25_212_301,
                    data: mid,
                },
            ]),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::new()),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(true),
            virtual_body_origin: AtomicU64::new(25_212_301),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::new(AtomicU64::new(1)),
            expected_generation: 1,
        }),
        pos: 0,
        access_mode: RemoteAccessMode::StandardSeekable,
        read_cancellation: None,
    };

    let decoder = SymphoniaAudioDecoder::new_remote_virtual(source, true)
        .expect("virtual body open should succeed");
    // 5 秒 PCM（44.1kHz 立体声）；坏字节流在 0.25 秒左右就会枯竭
    let want = 44_100 * 2 * 5;
    let got = decoder.take(want).count();
    assert_eq!(got, want, "解码在 {got} 采样处枯竭（应 >= {want}）");
}

/// 回归：单次 read 绝不许跨过 header_end 的拼接边界
///
/// 实机症状：seek 后 AAC 全是真假混杂的垃圾帧，0.25 秒后
/// "unexpected end of bitstream" 杀死解码器。根因是头部缓存段在物理
/// 空间连续（0..512KB），从 logical<header_end 起读时一路读进
/// 「文件开头的第一个 moof」——它也是合法 moof，demuxer 便按它的
/// trun 取样本。ts=5120 正是文件第一段的时间戳，铁证。
#[test]
fn reads_never_cross_the_virtual_body_boundary() {
    const HEADER_END: u64 = 5_831;
    const ORIGIN: u64 = 25_212_301;
    // 物理头部缓存（模拟启动时拉的 512KB 首包）：全部填 'H'
    let head_segment = CachedSegment {
        start: 0,
        data: vec![b'H'; 524_288],
    };
    // 目标 moof 处的缓存：全部填 'B'
    let body_segment = CachedSegment {
        start: ORIGIN,
        data: vec![b'B'; 262_144],
    };
    let mut source = RemoteAudioSource {
        inner: Arc::new(RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "https://example.invalid/audio.m4a".into(),
            referer: String::new(),
            total_len: 68_201_223,
            header_end: AtomicU64::new(HEADER_END),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 262_144,
                target_bytes: 1_048_576,
            },
            cache: Mutex::new(vec![head_segment, body_segment]),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::new()),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(true),
            virtual_body_origin: AtomicU64::new(ORIGIN),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::new(AtomicU64::new(1)),
            expected_generation: 1,
        }),
        pos: 0,
        access_mode: RemoteAccessMode::StandardSeekable,
        read_cancellation: None,
    };

    // 一次性请求 8KB，跨过 header_end：前 5831 字节必须是 'H'，之后必须是 'B'
    let mut out = vec![0_u8; 8_192];
    let mut filled = 0;
    while filled < out.len() {
        let read = std::io::Read::read(&mut source, &mut out[filled..]).unwrap();
        assert!(read > 0, "boundary read stalled at {filled}");
        filled += read;
    }
    assert!(out[..HEADER_END as usize].iter().all(|byte| *byte == b'H'));
    assert!(
        out[HEADER_END as usize..].iter().all(|byte| *byte == b'B'),
        "读取跨过了拼接边界：header 之后必须切到 body 物理段",
    );
}

/// 虚拟 body 的逻辑坐标必须连续，且长度不得被截断成已拉取的那一段
///
/// 旧实现先把 2MB body 拉进内存再交给 Cursor，demuxer 于是在 2MB 处
/// 撞上一个凭空多出来的 EOF，probe 直接 end of stream。
#[test]
fn virtual_body_reports_the_full_remaining_length() {
    let source = RemoteAudioSource {
        inner: Arc::new(RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "https://rr5---sn-x.googlevideo.com/videoplayback".into(),
            referer: String::new(),
            total_len: 135_169_876,
            header_end: AtomicU64::new(10_799),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 262_144,
                target_bytes: 1_048_576,
            },
            cache: Mutex::new(Vec::new()),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::new()),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(57_047_830),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::new(AtomicU64::new(1)),
            expected_generation: 1,
        }),
        pos: 0,
        access_mode: RemoteAccessMode::StandardSeekable,
        read_cancellation: None,
    };

    // 逻辑长度 = 头部 + 从落点到文件尾，而不是「已经拉下来的那一段」
    let expected = 10_799 + (135_169_876 - 57_047_830);
    assert_eq!(
        symphonia::core::io::MediaSource::byte_len(&source),
        Some(expected),
    );

    // [0, header_end) 映射到物理头部，之后按落点平移，坐标必须连续
    assert_eq!(source.map_logical_to_physical(0), 0);
    assert_eq!(source.map_logical_to_physical(10_798), 10_798);
    assert_eq!(source.map_logical_to_physical(10_799), 57_047_830);
    assert_eq!(source.map_logical_to_physical(10_799 + 4_096), 57_051_926);

    // demuxer 给的绝对 base_data_offset 要能折回逻辑坐标
    assert_eq!(source.normalize_seek_offset(57_047_830), 10_799);
    assert_eq!(source.normalize_seek_offset(57_051_926), 10_799 + 4_096);
}

#[test]
fn progressive_mp4_is_not_sniffed_as_fragmented() {
    // ftyp + moov(仅 mvhd) + mdat：普通渐进式 MP4
    let moov_body = mp4_box(b"mvhd", &[0u8; 16]);
    let mut head = mp4_box(b"ftyp", &[0u8; 8]);
    head.extend_from_slice(&mp4_box(b"moov", &moov_body));
    head.extend_from_slice(&mp4_box(b"mdat", &[0u8; 32]));

    assert!(!super::detect_fragmented_mp4(&head));
    assert!(!super::detect_fragmented_mp4(&[]));
    assert!(!super::detect_fragmented_mp4(b"OggS\x00\x02"));
}

#[test]
fn youtube_style_url_needs_payload_sniffing() {
    // googlevideo 直链没有扩展名，仅靠 URL 判不出分片
    let url = "https://rr5---sn-abc.googlevideo.com/videoplayback?expire=1&itag=140";
    assert!(!super::is_fragmented_mp4_url(url));
    assert!(super::is_fragmented_mp4_url(
        "https://cdn.example/audio/seg-1.m4s"
    ));
}

#[test]
fn detect_mp4_header_end_finds_mdat() {
    // ftyp(16) + free(8) + mdat(size=100)
    let mut data = Vec::new();
    data.extend_from_slice(&16u32.to_be_bytes());
    data.extend_from_slice(b"ftyp");
    data.extend_from_slice(&[0u8; 8]);
    data.extend_from_slice(&8u32.to_be_bytes());
    data.extend_from_slice(b"free");
    data.extend_from_slice(&100u32.to_be_bytes());
    data.extend_from_slice(b"mdat");
    data.extend_from_slice(&[0u8; 20]);
    assert_eq!(super::detect_mp4_header_end(&data), Some(24));
}

#[test]
fn find_moof_offset_near_snaps_past_garbage_prefix() {
    // 12 字节垃圾 + moof(size=32)
    let mut data = vec![0u8; 12];
    data.extend_from_slice(&32u32.to_be_bytes());
    data.extend_from_slice(b"moof");
    data.extend_from_slice(&[0u8; 24]);
    assert_eq!(super::find_moof_offset_near(&data, 0, 64), Some(12));
    // 已对齐
    assert_eq!(super::find_moof_offset_near(&data[12..], 0, 64), Some(0));
}

#[test]
fn virtual_body_maps_logical_header_and_body() {
    let source = RemoteAudioSource {
        inner: Arc::new(RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "https://cdn.example/audio/long.m4a".into(),
            referer: "https://example.com".into(),
            total_len: 10_000,
            header_end: AtomicU64::new(100),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 1,
                target_bytes: 1,
            },
            cache: Mutex::new(Vec::new()),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::new()),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(5_000),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::new(AtomicU64::new(1)),
            expected_generation: 1,
        }),
        pos: 0,
        access_mode: RemoteAccessMode::StandardSeekable,
        read_cancellation: None,
    };

    assert_eq!(source.map_logical_to_physical(0), 0);
    assert_eq!(source.map_logical_to_physical(99), 99);
    // 相对 moof：逻辑 body 映射到 physical origin+
    assert_eq!(source.map_logical_to_physical(100), 5_000);
    assert_eq!(source.map_logical_to_physical(150), 5_050);
    // 纯逻辑：pos 绝不能用物理恒等，否则 MSS/AtomIterator 失步
    // 5_000 在逻辑里是 body 中点：physical = 5000 + (5000-100) = 9900
    assert_eq!(source.map_logical_to_physical(5_000), 9_900);
    assert_eq!(source.logical_len(), 100 + (10_000 - 5_000));
}

#[test]
fn virtual_body_normalizes_absolute_sample_seeks() {
    use std::io::{Seek, SeekFrom};
    // 对齐真实长视频：header=10799, body_origin≈55MB, total≈135MB
    let mut source = RemoteAudioSource {
        inner: Arc::new(RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "https://cdn.example/audio/long.m4a".into(),
            referer: "https://example.com".into(),
            total_len: 135_169_876,
            header_end: AtomicU64::new(10_799),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 1,
                target_bytes: 1,
            },
            cache: Mutex::new(Vec::new()),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::new()),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(54_947_352),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::new(AtomicU64::new(1)),
            expected_generation: 1,
        }),
        pos: 0,
        access_mode: RemoteAccessMode::StandardSeekable,
        read_cancellation: None,
    };

    // 相对样本：逻辑 header_end → 物理 origin
    assert_eq!(source.map_logical_to_physical(10_799), 54_947_352);
    assert_eq!(source.map_logical_to_physical(11_299), 54_947_852);
    // 绝对 base_data_offset：Seek(物理) 必须折成逻辑并返回逻辑 pos
    let logical = source.seek(SeekFrom::Start(54_947_852)).unwrap();
    assert_eq!(logical, 10_799 + 500);
    assert_eq!(source.pos, 10_799 + 500);
    assert_eq!(source.map_logical_to_physical(source.pos), 54_947_852);
    // logical_len = header + (total - origin)，不能被拉成 total
    assert_eq!(source.logical_len(), 10_799 + (135_169_876 - 54_947_352));
}

#[test]
fn estimate_segment_start_snaps_to_entry_boundary() {
    let index = super::RemoteSeekIndex {
        duration_ms: 100_000,
        entries: vec![
            super::SeekIndexEntry {
                time_ms: 0,
                byte_offset: 1_000,
            },
            super::SeekIndexEntry {
                time_ms: 50_000,
                byte_offset: 50_000,
            },
            super::SeekIndexEntry {
                time_ms: 100_000,
                byte_offset: 100_000,
            },
        ],
        exact_times: true,
    };
    assert_eq!(index.estimate_segment_start(0), 1_000);
    assert_eq!(index.estimate_segment_start(49_999), 1_000);
    assert_eq!(index.estimate_segment_start(50_000), 50_000);
    assert_eq!(index.estimate_segment_start(90_000), 50_000);
    assert_eq!(index.estimate_segment_start(100_000), 50_000);
    assert_eq!(index.estimate_segment_start(120_000), 50_000);
}

#[test]
fn virtual_body_seek_uses_logical_sample_offsets() {
    use std::io::{Seek, SeekFrom};
    let mut source = RemoteAudioSource {
        inner: Arc::new(RemoteAudioInner {
            client: reqwest::Client::new(),
            url: "https://cdn.example/audio/long.m4a".into(),
            referer: "https://example.com".into(),
            total_len: 10_000,
            header_end: AtomicU64::new(100),
            prefetch_window: PrefetchWindow {
                low_water_bytes: 1,
                target_bytes: 1,
            },
            cache: Mutex::new(Vec::new()),
            disk_ranges: Mutex::new(Vec::new()),
            in_flight: Mutex::new(HashMap::new()),
            range_available: Condvar::new(),
            demand_fetches: AtomicUsize::new(0),
            last_read_pos: Mutex::new(0),
            protected_ranges: Mutex::new(Vec::new()),
            disk_cache: None,
            cache_finalize_started: AtomicBool::new(false),
            prefetch_epoch: AtomicU64::new(0),
            remote_forbidden: AtomicBool::new(false),
            demuxer_open_sequential: AtomicBool::new(false),
            virtual_body_origin: AtomicU64::new(5_000),
            seek_index: Mutex::new(None),
            throughput_ewma_bps: AtomicU64::new(0),
            bitrate_bps: 0,
            playback_generation: Arc::new(AtomicU64::new(1)),
            expected_generation: 1,
        }),
        pos: 0,
        access_mode: RemoteAccessMode::StandardSeekable,
        read_cancellation: None,
    };

    // sample Seek 用逻辑坐标；映射到物理 body
    assert_eq!(source.seek(SeekFrom::Start(100)).unwrap(), 100);
    assert_eq!(source.map_logical_to_physical(source.pos), 5_000);
    assert_eq!(source.seek(SeekFrom::Start(150)).unwrap(), 150);
    assert_eq!(source.map_logical_to_physical(source.pos), 5_050);
    assert_eq!(source.seek(SeekFrom::Start(50)).unwrap(), 50);
    assert_eq!(source.map_logical_to_physical(source.pos), 50);
}

#[test]
fn linear_seek_index_estimates_midpoint_bytes() {
    let index = super::RemoteSeekIndex {
        duration_ms: 100_000,
        entries: vec![
            super::SeekIndexEntry {
                time_ms: 0,
                byte_offset: 0,
            },
            super::SeekIndexEntry {
                time_ms: 50_000,
                byte_offset: 500,
            },
            super::SeekIndexEntry {
                time_ms: 100_000,
                byte_offset: 1_000,
            },
        ],
        exact_times: false,
    };
    assert_eq!(index.estimate_byte(0), 0);
    assert_eq!(index.estimate_byte(25_000), 250);
    assert_eq!(index.estimate_byte(75_000), 750);
    assert_eq!(index.estimate_byte(100_000), 1_000);
}

#[test]
fn parse_sidx_body_builds_time_byte_entries() {
    // version0 sidx: timescale=1000, earliest=0, first_offset=0, 2 refs size=100 dur=1000
    let mut body = Vec::new();
    body.push(0); // version
    body.extend_from_slice(&[0, 0, 0]); // flags
    body.extend_from_slice(&1u32.to_be_bytes()); // reference_id
    body.extend_from_slice(&1000u32.to_be_bytes()); // timescale
    body.extend_from_slice(&0u32.to_be_bytes()); // earliest_pts
    body.extend_from_slice(&0u32.to_be_bytes()); // first_offset
    body.extend_from_slice(&0u16.to_be_bytes()); // reserved
    body.extend_from_slice(&2u16.to_be_bytes()); // reference_count
    for _ in 0..2 {
        body.extend_from_slice(&100u32.to_be_bytes()); // size, type=media
        body.extend_from_slice(&1000u32.to_be_bytes()); // duration ticks
        body.extend_from_slice(&0u32.to_be_bytes()); // sap
    }
    let index = super::parse_sidx_body(&body, 0, 10_000, 10_000).expect("sidx");
    assert!(index.entries.len() >= 3);
    assert_eq!(index.entries[0].byte_offset, 0);
    assert_eq!(index.entries[1].byte_offset, 100);
    assert_eq!(index.entries[1].time_ms, 1000);
}

#[test]
fn segment_seek_excludes_the_terminal_anchor_at_and_after_duration() {
    let atom = sidx_atom_two_segments();
    let media_end = atom.len() as u64 + 2 * 1024 * 1024;
    let index = super::parse_sidx_from_bytes(&atom, 0, 20_000, media_end + 256)
        .expect("sidx index with trailing metadata");
    let last_segment = (atom.len() as u64 + 1024 * 1024, 10_000);
    assert_eq!(index.estimate_segment(0), (atom.len() as u64, 0));
    assert_eq!(index.estimate_segment(9_999), (atom.len() as u64, 0));
    assert_eq!(index.estimate_segment(10_000), last_segment);
    assert_eq!(index.estimate_segment(19_999), last_segment);
    assert_eq!(index.estimate_segment(20_000), last_segment);
    assert_eq!(index.estimate_segment(25_000), last_segment);
    assert_eq!(
        index.entries.last().expect("end anchor").byte_offset,
        media_end
    );
}

#[test]
fn virtual_body_seek_to_end_decodes_the_last_fragment_to_eof() {
    let init = include_bytes!("../fixtures/hls-init.mp4");
    let segment = include_bytes!("../fixtures/hls-segment.m4s");
    let duration_ms = 200u64;
    let mut sidx = Vec::new();
    sidx.extend_from_slice(&44u32.to_be_bytes());
    sidx.extend_from_slice(b"sidx");
    sidx.extend_from_slice(&[0; 4]);
    sidx.extend_from_slice(&1u32.to_be_bytes());
    sidx.extend_from_slice(&1_000u32.to_be_bytes());
    sidx.extend_from_slice(&0u32.to_be_bytes());
    sidx.extend_from_slice(&0u32.to_be_bytes());
    sidx.extend_from_slice(&0u16.to_be_bytes());
    sidx.extend_from_slice(&1u16.to_be_bytes());
    sidx.extend_from_slice(&(segment.len() as u32).to_be_bytes());
    sidx.extend_from_slice(&(duration_ms as u32).to_be_bytes());
    sidx.extend_from_slice(&0u32.to_be_bytes());
    let bytes = [init.as_slice(), sidx.as_slice(), segment.as_slice()].concat();
    let inner = forbidden_test_inner("http://127.0.0.1:1/end.m4s".into(), bytes.len() as u64);
    let header_end = super::detect_mp4_header_end(&bytes).expect("fragment header");
    inner.header_end.store(header_end, Ordering::Release);
    inner.cache.lock().expect("cache lock").push(CachedSegment {
        start: 0,
        data: bytes,
    });
    assert!(super::build_seek_index_from_head(&inner, duration_ms));
    let source = RemoteAudioSource {
        inner,
        pos: 0,
        access_mode: RemoteAccessMode::StandardSeekable,
        read_cancellation: None,
    };
    assert_eq!(
        source
            .configure_virtual_body_for_time(duration_ms)
            .expect("end seek"),
        Some(0)
    );
    let mut decoder =
        SymphoniaAudioDecoder::new_remote_virtual(source, true).expect("last fragment decoder");
    let discard =
        duration_ms * u64::from(decoder.sample_rate()) / 1_000 * u64::from(decoder.channels());
    let started = Instant::now();
    assert_eq!(decoder.by_ref().take(discard as usize).count(), 9_216);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "normal EOF must not wait through network retry backoff"
    );
    decoder.format = Box::new(ExhaustedFormat);
    let exhausted = Instant::now();
    for _ in 0..3 {
        assert!(
            decoder.next().is_none(),
            "a decoder consumed to EOF must remain exhausted"
        );
    }
    assert!(
        exhausted.elapsed() < Duration::from_secs(1),
        "the worker must not retry an EOF already reached while seeking"
    );
}

struct ExhaustedFormat;

impl symphonia::core::formats::FormatReader for ExhaustedFormat {
    fn try_new(
        _: symphonia::core::io::MediaSourceStream,
        _: &symphonia::core::formats::FormatOptions,
    ) -> symphonia::core::errors::Result<Self> {
        unreachable!("only used after decoding has finished")
    }
    fn cues(&self) -> &[symphonia::core::formats::Cue] {
        &[]
    }
    fn metadata(&mut self) -> symphonia::core::meta::Metadata<'_> {
        unreachable!("finished decoder metadata")
    }
    fn seek(
        &mut self,
        _: symphonia::core::formats::SeekMode,
        _: symphonia::core::formats::SeekTo,
    ) -> symphonia::core::errors::Result<symphonia::core::formats::SeekedTo> {
        unreachable!("finished decoder seek")
    }
    fn tracks(&self) -> &[symphonia::core::formats::Track] {
        &[]
    }
    fn next_packet(&mut self) -> symphonia::core::errors::Result<symphonia::core::formats::Packet> {
        panic!("a finished decoder must not poll its demuxer again")
    }
    fn into_inner(self: Box<Self>) -> symphonia::core::io::MediaSourceStream {
        unreachable!("finished decoder source")
    }
}

#[test]
fn remote_decoder_source_marks_only_an_actual_eof_and_clears_it_on_seek() {
    use std::io::{Read, Seek, SeekFrom};
    let inner = forbidden_test_inner("http://127.0.0.1:1/eof.bin".into(), 1);
    inner.cache.lock().unwrap().push(CachedSegment {
        start: 0,
        data: vec![42],
    });
    let eof = Arc::new(AtomicBool::new(false));
    let source = RemoteAudioSource {
        inner,
        pos: 0,
        access_mode: RemoteAccessMode::StandardSeekable,
        read_cancellation: None,
    };
    let mut tracked = super::decoder::RemoteDecoderSource {
        source,
        reached_eof: Arc::clone(&eof),
    };
    let mut out = [0; 2];
    assert_eq!(tracked.read(&mut out).unwrap(), 1);
    assert!(
        !eof.load(Ordering::Acquire),
        "read-ahead to the final byte is not EOF"
    );
    assert_eq!(tracked.read(&mut []).unwrap(), 0);
    assert!(
        !eof.load(Ordering::Acquire),
        "an empty read cannot establish EOF"
    );
    assert_eq!(tracked.read(&mut out).unwrap(), 0);
    assert!(eof.load(Ordering::Acquire));
    tracked.seek(SeekFrom::Start(0)).unwrap();
    assert!(!eof.load(Ordering::Acquire));
    assert_eq!(tracked.read(&mut out).unwrap(), 1);
    assert_eq!(tracked.read(&mut out).unwrap(), 0);
    assert!(eof.load(Ordering::Acquire));
    tracked
        .source
        .inner
        .playback_generation
        .store(2, Ordering::Release);
    assert!(
        tracked.read(&mut out).is_err(),
        "cancellation remains an error at the end"
    );
    assert!(
        !eof.load(Ordering::Acquire),
        "cancellation cannot be mistaken for EOF"
    );
}

#[test]
fn decoder_can_seek_back_after_it_has_been_exhausted() {
    let mut decoder =
        SymphoniaAudioDecoder::new(Box::new(std::io::Cursor::new(pcm_wav(8))), Some("wav"))
            .unwrap();
    assert!(decoder.by_ref().count() > 0);
    assert!(decoder.next().is_none());
    decoder.try_seek(Duration::ZERO).unwrap();
    assert!(
        decoder.next().is_some(),
        "a successful seek must reset the exhausted state"
    );
}

#[test]
fn invalidate_prefetch_epoch_clears_in_flight_and_allows_reclaim() {
    let inner = Arc::new(RemoteAudioInner {
        client: reqwest::Client::new(),
        url: "https://cdn.example/audio/long.m4a".into(),
        referer: "https://example.com".into(),
        total_len: 1024,
        header_end: AtomicU64::new(0),
        prefetch_window: PrefetchWindow {
            low_water_bytes: 1,
            target_bytes: 1,
        },
        cache: Mutex::new(Vec::new()),
        disk_ranges: Mutex::new(Vec::new()),
        in_flight: Mutex::new(HashMap::from([
            (
                0,
                Arc::new(InflightFetch {
                    end: AtomicU64::new(255),
                    done: AtomicBool::new(false),
                }),
            ),
            (
                256,
                Arc::new(InflightFetch {
                    end: AtomicU64::new(511),
                    done: AtomicBool::new(false),
                }),
            ),
        ])),
        range_available: Condvar::new(),
        demand_fetches: AtomicUsize::new(0),
        last_read_pos: Mutex::new(0),
        protected_ranges: Mutex::new(Vec::new()),
        disk_cache: None,
        cache_finalize_started: AtomicBool::new(false),
        prefetch_epoch: AtomicU64::new(0),
        remote_forbidden: AtomicBool::new(false),
        demuxer_open_sequential: AtomicBool::new(false),
        virtual_body_origin: AtomicU64::new(0),
        seek_index: Mutex::new(None),
        throughput_ewma_bps: AtomicU64::new(0),
        bitrate_bps: 0,
        playback_generation: Arc::new(AtomicU64::new(1)),
        expected_generation: 1,
    });
    assert!(super::claim_prefetch_start(&inner, 0, 255).is_none());
    super::invalidate_prefetch_epoch(&inner);
    assert_eq!(
        inner
            .prefetch_epoch
            .load(std::sync::atomic::Ordering::Acquire),
        1
    );
    assert!(inner.in_flight.lock().expect("lock").is_empty());
    let entry = super::claim_prefetch_start(&inner, 0, 255).expect("reclaim after invalidate");
    super::release_prefetch_start(&inner, 0, &entry);
}

#[test]
fn trim_cache_keeps_protected_tail_segments() {
    let protected = CachedSegment {
        start: 900,
        data: vec![9; 100],
    };
    let near = CachedSegment {
        start: 0,
        data: vec![1; super::REMOTE_MAX_CACHE_BYTES],
    };
    let inner = RemoteAudioInner {
        client: reqwest::Client::new(),
        url: "https://cdn.example/audio/long.m4a".into(),
        referer: "https://example.com".into(),
        total_len: 1000,
        header_end: AtomicU64::new(0),
        prefetch_window: PrefetchWindow {
            low_water_bytes: 1,
            target_bytes: 1,
        },
        cache: Mutex::new(vec![near, protected]),
        disk_ranges: Mutex::new(Vec::new()),
        in_flight: Mutex::new(HashMap::new()),
        range_available: Condvar::new(),
        demand_fetches: AtomicUsize::new(0),
        last_read_pos: Mutex::new(0),
        protected_ranges: Mutex::new(vec![CachedRange {
            start: 900,
            end: 1000,
        }]),
        disk_cache: None,
        cache_finalize_started: AtomicBool::new(false),
        prefetch_epoch: AtomicU64::new(0),
        remote_forbidden: AtomicBool::new(false),
        demuxer_open_sequential: AtomicBool::new(false),
        virtual_body_origin: AtomicU64::new(0),
        seek_index: Mutex::new(None),
        throughput_ewma_bps: AtomicU64::new(0),
        bitrate_bps: 0,
        playback_generation: Arc::new(AtomicU64::new(1)),
        expected_generation: 1,
    };
    let mut cache = {
        let guard = inner.cache.lock().expect("cache");
        guard
            .iter()
            .map(|segment| CachedSegment {
                start: segment.start,
                data: segment.data.clone(),
            })
            .collect::<Vec<_>>()
    };
    super::trim_cache(&inner, &mut cache, 0);
    assert!(
        cache.iter().any(|segment| segment.start == 900),
        "protected tail must survive trim"
    );
}

#[test]
fn disk_cache_is_ready_only_when_ranges_cover_entire_source() {
    assert!(!ranges_cover_source(
        &[
            CachedRange { start: 0, end: 512 },
            CachedRange {
                start: 768,
                end: 1_024,
            },
        ],
        1_024,
    ));
    assert!(ranges_cover_source(
        &[
            CachedRange { start: 0, end: 512 },
            CachedRange {
                start: 512,
                end: 1_024,
            },
        ],
        1_024,
    ));
}

#[test]
fn cache_marker_requires_hash_length_and_file_name() {
    let hash = "ab".repeat(32);
    let marker = format_cache_marker(2_048, &hash, "digest.audio");

    assert_eq!(
        parse_cache_marker(&marker),
        Some(CacheMarker::Validated {
            content_length: 2_048,
            sha256: hash,
            file_name: "digest.audio".into(),
        })
    );
    assert_eq!(
        parse_cache_marker("v1:1024"),
        Some(CacheMarker::Legacy {
            content_length: 1_024,
        })
    );
    assert_eq!(parse_cache_marker("v1:0"), None);
    assert_eq!(parse_cache_marker("v2:1024:bad:digest.audio"), None);
}

#[test]
fn content_range_parser_rejects_wildcards_and_invalid_bounds() {
    assert_eq!(
        parse_content_range(&HeaderValue::from_static("bytes 0-511/1024")),
        Some(HttpByteRange {
            start: 0,
            end: 511,
            total: 1_024,
        })
    );
    assert_eq!(
        parse_content_range(&HeaderValue::from_static("bytes */1024")),
        None
    );
    assert_eq!(
        parse_content_range(&HeaderValue::from_static("bytes 512-511/1024")),
        None
    );
}

#[test]
fn duration_guard_only_rejects_materially_short_media() {
    assert!(!cache_duration_is_suspicious(Some(272_000), 271_530));
    assert!(!cache_duration_is_suspicious(Some(272_000), 0));
    assert!(cache_duration_is_suspicious(Some(272_000), 30_000));
}

#[test]
fn zero_frame_track_duration_is_treated_as_unknown() {
    assert_eq!(
        track_duration(Some(TimeBase::new(1, 96_000)), Some(0)),
        None
    );
}

#[test]
fn unknown_zero_duration_does_not_clamp_seek_to_track_start() {
    let resolved = seek_target_time(Some(Time::new(0, 0.0)), Duration::from_millis(22_720));

    assert_eq!(resolved.seconds, 22);
    assert!((resolved.frac - 0.72).abs() < 0.000_001);
}

#[test]
fn known_duration_still_clamps_seek_just_before_track_end() {
    let resolved = seek_target_time(Some(Time::new(272, 0.0)), Duration::from_millis(300_000));

    assert_eq!(resolved.seconds, 271);
    assert!((resolved.frac - 0.9999).abs() < 0.000_001);
}

#[test]
fn cache_publish_requires_decodable_complete_staging_file() {
    let root = tempfile::tempdir().expect("temp cache root");
    let wav = pcm_wav(800);
    let cache = RemoteAudioCache::new(
        root.path().to_path_buf(),
        "valid-audio",
        1024 * 1024,
        Some(wav.len() as u64),
        100,
    )
    .expect("cache");

    cache.prepare(wav.len() as u64).expect("prepare");
    cache.write_range(0, &wav).expect("write staging");
    let staging_path = cache.staging_path().expect("staging path");
    assert!(!cache.ready_path.exists());

    cache.mark_ready().expect("publish validated cache");
    let published = cache.ready_path().expect("ready cache");
    assert_eq!(std::fs::read(published).expect("published bytes"), wav);
    assert!(!staging_path.exists());
}

#[test]
fn invalid_media_never_creates_ready_marker() {
    let root = tempfile::tempdir().expect("temp cache root");
    let invalid = b"<html>not audio</html>";
    let cache = RemoteAudioCache::new(
        root.path().to_path_buf(),
        "invalid-audio",
        1024 * 1024,
        Some(invalid.len() as u64),
        0,
    )
    .expect("cache");

    cache.prepare(invalid.len() as u64).expect("prepare");
    cache.write_range(0, invalid).expect("write staging");

    assert!(cache.mark_ready().is_err());
    assert!(!cache.ready_path.exists());
    assert!(cache.ready_path().is_none());
}

#[test]
fn concurrent_cache_instances_never_share_staging_files() {
    let root = tempfile::tempdir().expect("temp cache root");
    let first = RemoteAudioCache::new(root.path().to_path_buf(), "same-key", 1024 * 1024, None, 0)
        .expect("first cache");
    let second = RemoteAudioCache::new(root.path().to_path_buf(), "same-key", 1024 * 1024, None, 0)
        .expect("second cache");

    assert_ne!(
        first.staging_path().expect("first staging"),
        second.staging_path().expect("second staging")
    );
}

#[test]
fn complete_fallback_download_publishes_through_fresh_staging() {
    let root = tempfile::tempdir().expect("temp cache root");
    let wav = pcm_wav(800);
    let initial = RemoteAudioCache::new(
        root.path().to_path_buf(),
        "fallback-audio",
        1024 * 1024,
        Some(wav.len() as u64),
        100,
    )
    .expect("initial cache");
    let fallback = initial.fresh_staging().expect("fresh staging");
    let initial_staging = initial.staging_path().expect("initial staging");

    let published = fallback
        .publish_complete_bytes(&wav)
        .expect("publish fallback bytes");
    let fallback_staging = fallback.staging_path().expect("fallback staging");

    assert_ne!(initial_staging, fallback_staging);
    assert_eq!(std::fs::read(published).expect("published bytes"), wav);
    assert!(fallback.ready_path().is_none());
    fallback
        .bypass_ready
        .store(false, std::sync::atomic::Ordering::Release);
    assert!(fallback.ready_path().is_some());
}

#[test]
fn sequential_stream_publish_keeps_cache_unready_until_download_finishes() {
    let root = tempfile::tempdir().expect("temp cache root");
    let wav = pcm_wav(800);
    let cache = RemoteAudioCache::new(
        root.path().to_path_buf(),
        "sequential-audio",
        1024 * 1024,
        Some(wav.len() as u64),
        100,
    )
    .expect("cache");

    let staging = cache
        .prepare_sequential_write(wav.len() as u64)
        .expect("prepare sequential cache");
    std::fs::write(&staging, &wav).expect("write sequential cache");
    assert!(cache.ready_path().is_none());

    let published = cache
        .publish_sequential_write()
        .expect("publish sequential cache");
    assert_eq!(std::fs::read(published).expect("published bytes"), wav);
}

#[test]
fn cache_hit_does_not_create_unused_staging_file() {
    let root = tempfile::tempdir().expect("temp cache root");
    let wav = pcm_wav(800);
    let writer = RemoteAudioCache::new(
        root.path().to_path_buf(),
        "cache-hit",
        1024 * 1024,
        Some(wav.len() as u64),
        100,
    )
    .expect("writer cache");
    writer
        .publish_complete_bytes(&wav)
        .expect("publish cached audio");
    drop(writer);

    let hit = RemoteAudioCache::new(
        root.path().to_path_buf(),
        "cache-hit",
        1024 * 1024,
        Some(wav.len() as u64),
        100,
    )
    .expect("cache hit");

    assert!(hit.ready_path().is_some());
    assert!(hit.staging.lock().expect("staging lock").path.is_none());
}

#[test]
fn published_entry_check_needs_no_validation_or_staging() {
    let root = tempfile::tempdir().expect("temp cache root");
    let wav = pcm_wav(800);
    let probe = || {
        RemoteAudioCache::new(root.path().to_path_buf(), "quick-check", 0, None, 0).expect("probe")
    };
    assert!(!probe().has_published_entry());

    let writer = RemoteAudioCache::new(
        root.path().to_path_buf(),
        "quick-check",
        1024 * 1024,
        Some(wav.len() as u64),
        100,
    )
    .expect("writer cache");
    writer
        .publish_complete_bytes(&wav)
        .expect("publish cached audio");
    drop(writer);

    let published = probe();
    assert!(published.has_published_entry());
    assert!(published
        .staging
        .lock()
        .expect("staging lock")
        .path
        .is_none());

    // 文件被截断后不再视为完整缓存
    let audio = std::fs::read_dir(root.path().join(&published.digest[..2]))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.extension().and_then(|value| value.to_str()) == Some("audio"))
        .expect("published audio file");
    std::fs::write(&audio, &wav[..wav.len() / 2]).unwrap();
    assert!(!probe().has_published_entry());
}

#[test]
fn cache_lookup_miss_does_not_create_a_shard_directory() {
    let root = tempfile::tempdir().expect("cache root");
    let cache = RemoteAudioCache::new(
        root.path().join("audio"),
        "lookup-only",
        512 * 1024 * 1024,
        None,
        1_000,
    )
    .expect("cache");

    assert!(!cache.cache_dir.exists());
    assert!(cache.ready_path().is_none());
    assert!(!cache.cache_dir.exists());
}

#[test]
fn cache_capacity_scan_runs_only_after_a_complete_file_is_published() {
    let root = tempfile::tempdir().expect("temp cache root");
    let wav = pcm_wav(800);
    let existing = RemoteAudioCache::new(
        root.path().to_path_buf(),
        "existing-audio",
        1024 * 1024,
        Some(wav.len() as u64),
        100,
    )
    .expect("existing cache");
    let existing_path = existing
        .publish_complete_bytes(&wav)
        .expect("publish existing audio");

    let lookup = RemoteAudioCache::new(
        root.path().to_path_buf(),
        "new-audio",
        wav.len() as u64,
        Some(wav.len() as u64),
        100,
    )
    .expect("new cache lookup");
    assert!(existing_path.exists());

    lookup
        .prepare(wav.len() as u64)
        .expect("prepare cache miss");
    assert!(existing_path.exists());
    lookup.write_range(0, &wav).expect("write complete cache");
    lookup.mark_ready().expect("publish complete cache");
    assert!(!existing_path.exists());
}

#[test]
fn validated_cache_stamp_is_reused_only_for_unchanged_file_metadata() {
    let path = PathBuf::from("validated-cache-stamp-test.audio");
    let modified = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(42);
    let sha256 = "ab".repeat(32);
    let validated = ValidatedCacheFile {
        content_length: 1_024,
        sha256: sha256.clone(),
    };
    remember_validated_audio_cache(&path, Some(modified), &validated);

    assert!(reuse_validated_audio_cache(&path, 1_024, Some(modified), Some(&sha256),).is_some());
    assert!(reuse_validated_audio_cache(&path, 1_025, Some(modified), Some(&sha256),).is_none());
    assert!(reuse_validated_audio_cache(
        &path,
        1_024,
        Some(modified + Duration::from_secs(1)),
        Some(&sha256),
    )
    .is_none());
}

#[test]
fn android_alignment_published_cache_rejects_same_length_corruption_without_a_stamp() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().expect("temporary cache");
    let original = b"abcdefgh";
    let hash = hex::encode(Sha256::digest(original));
    let name = format!("cached.{hash}.audio");
    let path = root.path().join(&name);
    std::fs::write(&path, b"ABCDEFGH").expect("corrupted file");
    assert!(super::validate_published_cache_file(&path, 8, Some(8), None, &hash, &name).is_err());
}

#[test]
fn android_alignment_published_cache_accepts_a_verified_cold_file_without_reprobing_audio() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().expect("temporary cache");
    let bytes = b"abcdefgh";
    let hash = hex::encode(Sha256::digest(bytes));
    let name = format!("cached.{hash}.audio");
    let path = root.path().join(&name);
    std::fs::write(&path, bytes).expect("published file");
    assert!(super::validate_published_cache_file(&path, 8, Some(8), None, &hash, &name).is_ok());
}

#[test]
fn android_alignment_source_md5_is_checked_before_publication_and_reuse() {
    use sha2::Digest;
    let root = tempfile::tempdir().expect("temporary cache");
    let wav = pcm_wav(800);
    let checksum = hex::encode(md5::Md5::digest(&wav));
    let cache = RemoteAudioCache::new(
        root.path().to_path_buf(),
        "md5-source",
        1024 * 1024,
        Some(wav.len() as u64),
        100,
    )
    .unwrap()
    .with_expected_md5(Some(&checksum))
    .unwrap();
    cache.publish_complete_bytes(&wav).unwrap();
    assert!(cache.ready_path().is_some());
    let wrong = "00".repeat(16);
    assert!(cache
        .clone()
        .with_expected_md5(Some(&wrong))
        .unwrap()
        .ready_path()
        .is_none());
    let bad = RemoteAudioCache::new(
        root.path().to_path_buf(),
        "md5-mismatch",
        1024 * 1024,
        Some(wav.len() as u64),
        100,
    )
    .unwrap()
    .with_expected_md5(Some(&wrong))
    .unwrap();
    assert!(bad.publish_complete_bytes(&wav).is_err());
    assert!(!bad.ready_path.exists());
    assert!(cache.with_expected_md5(Some("invalid")).is_err());
}

#[test]
fn throughput_ewma_blends_successes_and_only_sinks_on_failures() {
    // 实机日志样本：512KB / 15s ≈ 34.9KB/s
    assert_eq!(throughput_sample_bps(524_288, 15_000), 34_952);
    // 首个样本直接采纳；此后按 30% 权重混入
    assert_eq!(ewma_bps_after_success(0, 100_000), 100_000);
    assert_eq!(ewma_bps_after_success(100_000, 50_000), 85_000);
    // 失败样本是吞吐上界：跑得太短不计入；高于当前估计不上调
    assert_eq!(ewma_bps_after_failure(100_000, 524_288, 1_000), 100_000);
    assert_eq!(ewma_bps_after_failure(20_000, 524_288, 15_000), 20_000);
    // 低于当前估计时按同一权重向下混合
    assert_eq!(ewma_bps_after_failure(100_000, 524_288, 15_000), 80_485);
}

#[test]
fn adaptive_block_shrinks_on_slow_and_recovers_on_fast_networks() {
    // 无样本：保持调用方默认块（冷启动行为不变）
    assert_eq!(adaptive_block_bytes(0, 256 * 1024, 256 * 1024), 256 * 1024);
    assert_eq!(adaptive_block_bytes(0, 512 * 1024, 1024 * 1024), 512 * 1024);
    // 慢网 50KB/s：8s 目标 → 400KB，向下对齐 64KB → 384KB
    assert_eq!(
        adaptive_block_bytes(51_200, 512 * 1024, 1024 * 1024),
        393_216
    );
    // 需求读上限受 mode 块钳制
    assert_eq!(
        adaptive_block_bytes(2 * 1024 * 1024, 256 * 1024, 256 * 1024),
        256 * 1024
    );
    // 快网 2MB/s：预取恢复到 1MB 上限
    assert_eq!(
        adaptive_block_bytes(2 * 1024 * 1024, 512 * 1024, 1024 * 1024),
        1024 * 1024
    );
    // 极慢网也不低于 64KB 下限
    assert_eq!(adaptive_block_bytes(1_000, 512 * 1024, 1024 * 1024), 65_536);
}

#[test]
fn request_timeout_scales_with_block_size_and_throughput() {
    // 无吞吐样本：20s 下限（原 15s 已被实机 12.4s 的成功请求贴脸打穿）
    assert_eq!(request_timeout_for(524_288, 0), Duration::from_secs(20));
    // 自适应块本身就按 8s 目标选：预算落在下限
    assert_eq!(
        request_timeout_for(262_144, 51_200),
        Duration::from_secs(20)
    );
    // 分片 8MB 大块 @ 400KB/s：预计 20.5s ×1.5 + 5s ≈ 35.7s
    assert_eq!(
        request_timeout_for(8 * 1024 * 1024, 409_600),
        Duration::from_millis(35_720)
    );
    // 上限 60s：不允许无限拉长
    assert_eq!(
        request_timeout_for(8 * 1024 * 1024, 102_400),
        Duration::from_secs(60)
    );
}

#[test]
fn prefetch_concurrency_yields_to_demand_reads_on_slow_networks() {
    // 吞吐或码率未知：维持默认并发
    assert_eq!(prefetch_lane_count(0, 45_000), REMOTE_PREFETCH_CONCURRENCY);
    assert_eq!(prefetch_lane_count(200_000, 0), REMOTE_PREFETCH_CONCURRENCY);
    // EWMA 低于码率：单 lane，带宽让给需求读
    assert_eq!(prefetch_lane_count(40_000, 45_000), 1);
    // 1-2 倍码率之间：2 lane
    assert_eq!(prefetch_lane_count(60_000, 45_000), 2);
    // 富余吞吐：恢复满并发
    assert_eq!(
        prefetch_lane_count(200_000, 45_000),
        REMOTE_PREFETCH_CONCURRENCY
    );
}

#[test]
fn halved_fetch_len_bottoms_out_at_min_block() {
    assert_eq!(halved_fetch_len(524_288), 262_144);
    assert_eq!(halved_fetch_len(131_072), 65_536);
    assert_eq!(halved_fetch_len(65_536), 65_536);
}

/// 记录到的 range 请求列表 (start, end)
type SeenRanges = Arc<Mutex<Vec<(u64, u64)>>>;

/// 本地 range 服务器：前 fail_first 个请求发送响应头后只回半个 body
/// 即断开（模拟慢网下 per-request 超时打断 body 读取的传输中断），
/// 之后的请求正常完整响应；记录每个请求的 range 供断言
fn spawn_flaky_range_server(body: Arc<Vec<u8>>, fail_first: usize) -> (String, SeenRanges) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind range server");
    let addr = listener.local_addr().expect("range server addr");
    let requests: Arc<Mutex<Vec<(u64, u64)>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    thread::spawn(move || {
        let mut served = 0usize;
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut request = Vec::new();
            let mut chunk = [0u8; 2048];
            loop {
                match std::io::Read::read(&mut stream, &mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        request.extend_from_slice(&chunk[..count]);
                        if request.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                }
            }
            let text = String::from_utf8_lossy(&request);
            let Some(spec) = text
                .lines()
                .find(|line| line.to_ascii_lowercase().starts_with("range:"))
                .and_then(|line| line.split('=').nth(1))
            else {
                continue;
            };
            let mut bounds = spec.trim().split('-');
            let start: u64 = bounds.next().unwrap_or("0").parse().unwrap_or(0);
            let end: u64 = bounds.next().unwrap_or("0").parse().unwrap_or(0);
            seen.lock().expect("requests lock").push((start, end));
            let total = body.len() as u64;
            let end = end.min(total.saturating_sub(1));
            let slice = &body[start as usize..=end as usize];
            let header = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                start,
                end,
                total,
                slice.len(),
            );
            if std::io::Write::write_all(&mut stream, header.as_bytes()).is_err() {
                continue;
            }
            served += 1;
            let payload = if served <= fail_first {
                &slice[..slice.len() / 2]
            } else {
                slice
            };
            let _ = std::io::Write::write_all(&mut stream, payload);
            let _ = std::io::Write::flush(&mut stream);
            // fail_first 内的连接在此 drop：Content-Length 未喂满即断开
        }
    });
    (format!("http://{}/audio.bin", addr), requests)
}

/// 回归：慢网超时/传输中断的需求读绝不 fatal——同一起点折半块大小重试，
/// 直到成功；解码层只看到一次变慢的 read，而不是会话死亡
#[test]
fn demand_fetch_retries_transient_failures_with_halved_blocks() {
    let total_len: u64 = 1024 * 1024;
    let body: Arc<Vec<u8>> = Arc::new((0..total_len).map(|index| (index % 251) as u8).collect());
    // 前两个请求（256KB、128KB）中途断开，第三个（64KB）放行
    let (url, requests) = spawn_flaky_range_server(Arc::clone(&body), 2);
    let inner = Arc::new(RemoteAudioInner {
        // 必须绕过系统代理：本地回环服务器走代理会拿到 502
        client: reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("no-proxy client"),
        url,
        referer: "https://example.com".into(),
        total_len,
        header_end: AtomicU64::new(0),
        prefetch_window: PrefetchWindow {
            low_water_bytes: 1,
            target_bytes: 1,
        },
        cache: Mutex::new(Vec::new()),
        disk_ranges: Mutex::new(Vec::new()),
        in_flight: Mutex::new(HashMap::new()),
        range_available: Condvar::new(),
        demand_fetches: AtomicUsize::new(0),
        last_read_pos: Mutex::new(0),
        protected_ranges: Mutex::new(Vec::new()),
        disk_cache: None,
        cache_finalize_started: AtomicBool::new(false),
        prefetch_epoch: AtomicU64::new(0),
        remote_forbidden: AtomicBool::new(false),
        demuxer_open_sequential: AtomicBool::new(false),
        virtual_body_origin: AtomicU64::new(0),
        seek_index: Mutex::new(None),
        throughput_ewma_bps: AtomicU64::new(0),
        bitrate_bps: 0,
        playback_generation: Arc::new(AtomicU64::new(1)),
        expected_generation: 1,
    });
    let mut source = RemoteAudioSource {
        inner: Arc::clone(&inner),
        pos: 0,
        access_mode: RemoteAccessMode::StandardSeekable,
        read_cancellation: None,
    };
    let mut bytes = [0u8; 4];

    source
        .read_exact(&mut bytes)
        .expect("demand read must survive transient failures");

    assert_eq!(bytes, [0, 1, 2, 3]);
    // 断言折半序列：256KB → 128KB → 64KB
    assert_eq!(
        requests.lock().expect("requests lock").clone(),
        vec![(0, 262_143), (0, 131_071), (0, 65_535)],
    );
    // 成功后在途登记必须清空（等待方不会悬挂在残留登记上）
    assert!(inner.in_flight.lock().expect("in-flight lock").is_empty());
    // 成功样本已混入吞吐估计，供后续自适应选块
    assert!(inner.throughput_bps() > 0);
}

/// 起始偏移 >= forbid_from 的 range 一律回 403（模拟 googlevideo
/// 直链过期/远端 range 风控），其余按 206 正常切片
fn spawn_forbidden_range_server(body: Arc<Vec<u8>>, forbid_from: u64) -> (String, SeenRanges) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind range server");
    let addr = listener.local_addr().expect("range server addr");
    let requests: Arc<Mutex<Vec<(u64, u64)>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut request = Vec::new();
            let mut chunk = [0u8; 2048];
            loop {
                match std::io::Read::read(&mut stream, &mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        request.extend_from_slice(&chunk[..count]);
                        if request.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                }
            }
            let text = String::from_utf8_lossy(&request);
            let Some(spec) = text
                .lines()
                .find(|line| line.to_ascii_lowercase().starts_with("range:"))
                .and_then(|line| line.split('=').nth(1))
            else {
                continue;
            };
            let mut bounds = spec.trim().split('-');
            let start: u64 = bounds.next().unwrap_or("0").parse().unwrap_or(0);
            let end: u64 = bounds.next().unwrap_or("0").parse().unwrap_or(0);
            seen.lock().expect("requests lock").push((start, end));
            if start >= forbid_from {
                let _ = std::io::Write::write_all(
                    &mut stream,
                    b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                let _ = std::io::Write::flush(&mut stream);
                continue;
            }
            let total = body.len() as u64;
            let end = end.min(total.saturating_sub(1));
            let slice = &body[start as usize..=end as usize];
            let header = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                start,
                end,
                total,
                slice.len(),
            );
            if std::io::Write::write_all(&mut stream, header.as_bytes()).is_err() {
                continue;
            }
            let _ = std::io::Write::write_all(&mut stream, slice);
            let _ = std::io::Write::flush(&mut stream);
        }
    });
    (format!("http://{}/audio.bin", addr), requests)
}

fn forbidden_test_inner(url: String, total_len: u64) -> Arc<RemoteAudioInner> {
    Arc::new(RemoteAudioInner {
        client: reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("no-proxy client"),
        url,
        referer: "https://example.com".into(),
        total_len,
        header_end: AtomicU64::new(0),
        prefetch_window: PrefetchWindow {
            low_water_bytes: 1,
            target_bytes: 1,
        },
        cache: Mutex::new(Vec::new()),
        disk_ranges: Mutex::new(Vec::new()),
        in_flight: Mutex::new(HashMap::new()),
        range_available: Condvar::new(),
        demand_fetches: AtomicUsize::new(0),
        last_read_pos: Mutex::new(0),
        protected_ranges: Mutex::new(Vec::new()),
        disk_cache: None,
        cache_finalize_started: AtomicBool::new(false),
        prefetch_epoch: AtomicU64::new(0),
        remote_forbidden: AtomicBool::new(false),
        demuxer_open_sequential: AtomicBool::new(false),
        virtual_body_origin: AtomicU64::new(0),
        seek_index: Mutex::new(None),
        throughput_ewma_bps: AtomicU64::new(0),
        bitrate_bps: 0,
        playback_generation: Arc::new(AtomicU64::new(1)),
        expected_generation: 1,
    })
}

/// 回归（YouTube seek 后 403 无限重试）：403 是永久错误，需求读必须
/// 一次失败立即上抛 PermissionDenied——绝不折半重试、绝不重复打请求，
/// 并且置位 remote_forbidden 停掉后续预取孵化
#[test]
fn range_403_fails_fast_without_retry_and_disables_prefetch() {
    let total_len: u64 = 1024 * 1024;
    let body: Arc<Vec<u8>> = Arc::new(vec![0u8; total_len as usize]);
    let (url, requests) = spawn_forbidden_range_server(Arc::clone(&body), 0);
    let inner = forbidden_test_inner(url, total_len);
    let mut source = RemoteAudioSource {
        inner: Arc::clone(&inner),
        pos: 0,
        access_mode: RemoteAccessMode::StandardSeekable,
        read_cancellation: None,
    };
    let mut bytes = [0u8; 4];

    let error = source
        .read_exact(&mut bytes)
        .expect_err("403 must fail the read");

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    // 只允许打一发请求：403 不进折半重试循环
    assert_eq!(requests.lock().expect("requests lock").len(), 1);
    assert!(inner.remote_forbidden.load(Ordering::Acquire));
    // 置位后 replenish 是空操作：不再孵化注定 403 的后台预取
    replenish_prefetch_window(&inner, 0);
    thread::sleep(Duration::from_millis(150));
    assert_eq!(requests.lock().expect("requests lock").len(), 1);
}

/// 回归（YouTube seek 卡死入口）：虚拟 body 探测遇 403 必须让整个
/// seek 立即失败（上层回滚/换直链），而不是吞掉错误继续配置、
/// 让 demuxer 打开阶段对同一 range 每 ~600ms 无限刷 403
#[test]
fn virtual_body_probe_403_aborts_seek_configuration() {
    let total_len: u64 = 1024 * 1024;
    let body: Arc<Vec<u8>> = Arc::new(vec![0u8; total_len as usize]);
    // 只有文件头可读，body 一律 403（模拟只允许顺序窗口的直链）
    let (url, requests) = spawn_forbidden_range_server(Arc::clone(&body), 1);
    let inner = forbidden_test_inner(url, total_len);
    inner.header_end.store(64, Ordering::Release);
    *inner.seek_index.lock().expect("seek index lock") = Some(RemoteSeekIndex {
        duration_ms: 200_000,
        entries: vec![
            SeekIndexEntry {
                time_ms: 0,
                byte_offset: 64,
            },
            SeekIndexEntry {
                time_ms: 100_000,
                byte_offset: 500_000,
            },
        ],
        exact_times: true,
    });
    let source = RemoteAudioSource {
        inner: Arc::clone(&inner),
        pos: 0,
        access_mode: RemoteAccessMode::StandardSeekable,
        read_cancellation: None,
    };

    let error = source
        .configure_virtual_body_for_time(120_000)
        .expect_err("probe 403 must abort the seek");

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    // 探测只打一发请求，配置未生效（不会留下半配置的虚拟 body）
    assert_eq!(requests.lock().expect("requests lock").len(), 1);
    assert_eq!(inner.virtual_body_origin.load(Ordering::Acquire), 0);
    assert!(inner.remote_forbidden.load(Ordering::Acquire));
}

/// 手工构造 version=0 的 sidx box：timescale=48000，两段各 10s / 1MB
fn sidx_atom_two_segments() -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&[0, 0, 0, 0]); // version + flags
    body.extend_from_slice(&1u32.to_be_bytes()); // reference_ID
    body.extend_from_slice(&48_000u32.to_be_bytes()); // timescale
    body.extend_from_slice(&0u32.to_be_bytes()); // earliest_pts
    body.extend_from_slice(&0u32.to_be_bytes()); // first_offset
    body.extend_from_slice(&0u16.to_be_bytes()); // reserved
    body.extend_from_slice(&2u16.to_be_bytes()); // reference_count
    for _ in 0..2 {
        body.extend_from_slice(&0x0010_0000u32.to_be_bytes()); // 1MB, media type
        body.extend_from_slice(&480_000u32.to_be_bytes()); // 10s @48k
        body.extend_from_slice(&0u32.to_be_bytes()); // SAP
    }
    let mut atom = Vec::with_capacity(8 + body.len());
    atom.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
    atom.extend_from_slice(b"sidx");
    atom.extend_from_slice(&body);
    atom
}

/// 回归（YouTube seek 慢 16s）：前端对 YouTube 常传 duration_hint=0，
/// 旧代码既要求 hint>0 才建索引、又用 .max(1) 把 time_ms 全 clamp 到 1ms，
/// 导致分片流退化到 format.seek 顺序爬完全文件。
/// sidx 自带 timescale，时长未知也必须能建出精确索引
#[test]
fn fragmented_seek_index_builds_without_duration_hint() {
    let total_len: u64 = 4 * 1024 * 1024;
    let inner = forbidden_test_inner("http://127.0.0.1:1/audio.m4a".into(), total_len);
    inner.cache.lock().expect("cache lock").push(CachedSegment {
        start: 0,
        data: sidx_atom_two_segments(),
    });

    assert!(
        super::build_seek_index_from_head(&inner, 0),
        "sidx index must build with duration_hint_ms=0",
    );

    let index = inner
        .seek_index
        .lock()
        .expect("seek index lock")
        .clone()
        .expect("index stored");
    // 三个锚点：0s / 10s / 20s，时间来自 sidx 自身 timescale，绝不能被 clamp
    assert_eq!(index.entries.len(), 3);
    assert_eq!(index.entries[1].time_ms, 10_000);
    assert_eq!(index.entries[2].time_ms, 20_000);
    assert_eq!(index.duration_ms, 20_000);
    // seek 到 15s 应落在 10s 段起点（首个 moof 后 1MB 处）
    let atom_len = sidx_atom_two_segments().len() as u64;
    assert_eq!(
        index.estimate_segment(15_000),
        (atom_len + 1024 * 1024, 10_000),
        "落点分片从 10s 开始出声，播放器要丢掉 10s 到 15s 之间的音频",
    );
    assert!(index.exact_times);
}

/// 起播首包耗时实测：NERI_PROBE_URL=<直链> cargo test -- --ignored --nocapture probe_timing
#[test]
#[ignore]
fn probe_timing_against_a_real_url() {
    let Ok(url) = std::env::var("NERI_PROBE_URL") else {
        eprintln!("NERI_PROBE_URL 未设置，跳过");
        return;
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let build = || {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(30))
            .no_proxy()
            .build()
            .unwrap()
    };
    type ClientFactory = (&'static str, fn() -> reqwest::Client);
    let configs: [ClientFactory; 3] = [
        ("default", || {
            reqwest::Client::builder().no_proxy().build().unwrap()
        }),
        ("http1", || {
            reqwest::Client::builder()
                .no_proxy()
                .http1_only()
                .build()
                .unwrap()
        }),
        ("ipv4", || {
            reqwest::Client::builder()
                .no_proxy()
                .local_address(std::net::IpAddr::from([0, 0, 0, 0]))
                .build()
                .unwrap()
        }),
    ];
    for round in 0..2 {
        for (label, make) in configs {
            let client = make();
            let started = Instant::now();
            let response = runtime.block_on(
                client
                    .get(&url)
                    .header(super::RANGE, "bytes=0-32767")
                    .header(super::USER_AGENT, super::playback_user_agent(&url))
                    .send(),
            );
            eprintln!(
                "round {round} {label}: headers status={:?} version={:?} ttfb_ms={}",
                response.as_ref().map(|r| r.status()).ok(),
                response.as_ref().map(|r| r.version()).ok(),
                started.elapsed().as_millis(),
            );
        }
    }
    let shared = build();
    for round in 0..3 {
        for (label, client) in [("fresh", build()), ("shared", shared.clone())] {
            let started = Instant::now();
            let result = runtime.block_on(super::probe_range_len(
                &client,
                &url,
                "",
                super::REMOTE_INITIAL_BLOCK_BYTES,
            ));
            eprintln!(
                "round {round} {label}: ok={} bytes={} elapsed_ms={}",
                result.is_ok(),
                result.as_ref().map_or(0, |(_, data)| data.len()),
                started.elapsed().as_millis(),
            );
        }
    }
}

/// 真实 B 站 fMP4（FLAC）实测：字节跳转后丢掉分片开头，出声的第一个样本必须就是目标位置
///
/// NERI_BILI_REPRO_FILE=<下载完的 .m4s> [NERI_BILI_REPRO_TARGET_MS=32700] cargo test -- --ignored bili_virtual_body
#[test]
#[ignore]
fn bili_virtual_body_seek_starts_exactly_at_the_target() {
    let Ok(path) = std::env::var("NERI_BILI_REPRO_FILE") else {
        eprintln!("NERI_BILI_REPRO_FILE 未设置，跳过");
        return;
    };
    let target_ms: u64 = std::env::var("NERI_BILI_REPRO_TARGET_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(32_700);
    let bytes = std::fs::read(&path).expect("repro file");
    let inner = forbidden_test_inner("http://127.0.0.1:1/bili.m4s".into(), bytes.len() as u64);
    let header_end = super::detect_mp4_header_end(&bytes).expect("mp4 header end");
    inner.header_end.store(header_end, Ordering::Release);
    inner.cache.lock().expect("cache lock").push(CachedSegment {
        start: 0,
        data: bytes,
    });
    assert!(
        super::build_seek_index_from_head(&inner, 0),
        "B 站 fMP4 首包带 sidx"
    );
    let source = RemoteAudioSource {
        inner: Arc::clone(&inner),
        pos: 0,
        access_mode: RemoteAccessMode::StandardSeekable,
        read_cancellation: None,
    };

    let segment_start_ms = source
        .configure_virtual_body_for_time(target_ms)
        .expect("configure virtual body")
        .expect("sidx 落点要给出分片起点");
    assert!(segment_start_ms <= target_ms && target_ms - segment_start_ms < 10_000);
    let mut seeked =
        SymphoniaAudioDecoder::new_remote_virtual(source, true).expect("virtual body open");
    let samples_per_ms = |ms: u64, decoder: &SymphoniaAudioDecoder| {
        (ms * u64::from(decoder.sample_rate()) / 1_000 * u64::from(decoder.channels())) as usize
    };
    let lead = samples_per_ms(target_ms - segment_start_ms, &seeked);
    assert_eq!(seeked.by_ref().take(lead).count(), lead);
    let seeked: Vec<f32> = seeked.take(4_096).collect();

    // sidx 时间按整毫秒记，允许 1ms 以内的错位
    let mut reference =
        SymphoniaAudioDecoder::new_file(std::path::Path::new(&path)).expect("reference open");
    let one_ms = samples_per_ms(1, &reference);
    let channels = usize::from(reference.channels());
    let skip = samples_per_ms(target_ms, &reference) - one_ms;
    assert_eq!(reference.by_ref().take(skip).count(), skip);
    let reference: Vec<f32> = reference.take(4_096 + 2 * one_ms).collect();

    assert_eq!(seeked.len(), 4_096);
    let shift = (0..=2 * one_ms)
        .step_by(channels)
        .find(|&shift| reference[shift..shift + seeked.len()] == seeked[..])
        .expect("跳转后的音频与从头解码到目标处不一致");
    eprintln!(
        "target {target_ms}ms segment {segment_start_ms}ms offset {} samples",
        shift as i64 - one_ms as i64
    );
}

/// 回归（Bilibili 2 小时 fMP4 seek 爬 12s）：FragmentedProgressive
/// （.m4s 直链）此前不在建索引名单里，seek 退化为顺序爬 moof。
/// 它的 sidx 同样在首包，必须与嗅探分片/LongForm 一致地建索引，
/// 且升级前（Progressive）与升级后（Seekable）都要走虚拟 body 落点
#[test]
fn fragmented_progressive_builds_index_and_prefers_virtual_body() {
    // 建索引名单矩阵：分片直链与长内容必建；短内容仅嗅探出分片才建
    use super::{needs_head_seek_index, RemoteAccessMode as Mode};
    assert!(needs_head_seek_index(Mode::FragmentedProgressive, false));
    assert!(needs_head_seek_index(Mode::LongFormProgressive, false));
    assert!(needs_head_seek_index(Mode::StandardSeekable, true));
    assert!(!needs_head_seek_index(Mode::StandardSeekable, false));

    let total_len: u64 = 4 * 1024 * 1024;
    let inner = forbidden_test_inner("http://127.0.0.1:1/audio.m4s".into(), total_len);
    inner.cache.lock().expect("cache lock").push(CachedSegment {
        start: 0,
        data: sidx_atom_two_segments(),
    });
    assert!(super::build_seek_index_from_head(&inner, 0));
    inner.header_end.store(64, Ordering::Release);

    let source = RemoteAudioSource {
        inner: Arc::clone(&inner),
        pos: 0,
        access_mode: RemoteAccessMode::FragmentedProgressive,
        read_cancellation: None,
    };
    assert!(
        source.prefers_virtual_body_seek(),
        "升级前的 .m4s 直链就必须判定为虚拟 body seek，否则误走 format.seek"
    );
    let seekable = source.seekable_clone();
    assert!(seekable.prefers_virtual_body_seek());
}

/// splice 打开期的错误分类：解码类可跳；IO 里只有瞬时类可跳，
/// 403（PermissionDenied）与主动取消必须立即上抛
#[test]
fn splice_skip_bails_on_permanent_io_errors() {
    use symphonia::core::errors::Error as SymphoniaError;

    let permanent = SymphoniaError::IoError(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "remote range request forbidden: 403 Forbidden",
    ));
    let cancelled = SymphoniaError::IoError(std::io::Error::other("remote read superseded"));
    let transient = SymphoniaError::IoError(std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        "prefetch edge",
    ));
    let decode = SymphoniaError::DecodeError("moov stco fake sample");

    assert!(!splice_skip_retryable(&permanent));
    assert!(!splice_skip_retryable(&cancelled));
    assert!(splice_skip_retryable(&transient));
    assert!(splice_skip_retryable(&decode));
}
