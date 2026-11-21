//! 已下载曲目的记录结构与文件完整性校验：长度、MD5 与可解码时长都要与解析到的音源一致

use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use neri_common::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadedTrack {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub cover_url: Option<String>,
    pub source: String,
    pub file_path: String,
    pub file_size: u64,
    pub downloaded_at: u64,
}

pub fn has_download_size_mismatch(expected: u64, actual: u64) -> bool {
    expected > 0 && expected != actual
}

pub fn scan_download_duration(path: &std::path::Path, cancelled: &AtomicBool) -> AppResult<u64> {
    use symphonia::core::errors::Error;
    use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
    use symphonia::core::probe::Hint;

    let source = MediaSourceStream::new(
        Box::new(std::fs::File::open(path)?),
        MediaSourceStreamOptions::default(),
    );
    let mut format = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            source,
            &Default::default(),
            &Default::default(),
        )
        .map_err(|error| AppError::Audio(format!("Cannot inspect downloaded audio: {error}")))?
        .format;
    let track = format
        .default_track()
        .ok_or_else(|| AppError::Audio("Downloaded file contains no audio track".into()))?;
    let track_id = track.id;
    let time_base = track
        .codec_params
        .time_base
        .ok_or_else(|| AppError::Audio("Downloaded audio has no time base".into()))?;
    let mut first_timestamp = None;
    let mut end_timestamp = 0;
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err(AppError::Other("Download cancelled".into()));
        }
        match format.next_packet() {
            Ok(packet) if packet.track_id() == track_id => {
                first_timestamp =
                    Some(first_timestamp.map_or(packet.ts(), |first: u64| first.min(packet.ts())));
                end_timestamp = end_timestamp.max(packet.ts().saturating_add(packet.dur()));
            }
            Ok(_) => {}
            Err(Error::IoError(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                break
            }
            Err(error) => {
                return Err(AppError::Audio(format!(
                    "Cannot inspect downloaded audio packets: {error}"
                )));
            }
        }
    }
    let time = time_base.calc_time(end_timestamp.saturating_sub(first_timestamp.unwrap_or(0)));
    Ok(time
        .seconds
        .saturating_mul(1000)
        .saturating_add((time.frac * 1000.0).round() as u64))
}

pub fn validate_download_audio(
    path: &std::path::Path,
    expected_length: Option<u64>,
    expected_md5: Option<&str>,
    expected_duration_ms: u64,
    cancelled: &AtomicBool,
) -> AppResult<u64> {
    use lofty::file::AudioFile;
    use md5::{Digest, Md5};
    use neri_playback::pcm::PcmSource;
    use std::io::Read;

    let before = std::fs::metadata(path)?.len();
    if before == 0
        || expected_length
            .filter(|length| *length > 0)
            .is_some_and(|length| length != before)
    {
        return Err(AppError::Audio(
            "Download length does not match the resolved source".into(),
        ));
    }
    if let Some(expected) = expected_md5 {
        if expected.len() != 32 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(AppError::Audio("Invalid download source MD5".into()));
        }
        let mut digest = Md5::new();
        let mut file = std::fs::File::open(path)?;
        let mut bytes = [0; 64 * 1024];
        loop {
            if cancelled.load(Ordering::Relaxed) {
                return Err(AppError::Other("Download cancelled".into()));
            }
            let read = file.read(&mut bytes)?;
            if read == 0 {
                break;
            }
            digest.update(&bytes[..read]);
        }
        if !hex::encode(digest.finalize()).eq_ignore_ascii_case(expected) {
            return Err(AppError::Audio(
                "Download MD5 does not match the resolved source".into(),
            ));
        }
    }
    // 临时文件后缀是 .part，必须根据实际内容识别容器
    let tagged = lofty::probe::Probe::open(path)
        .map_err(|error| AppError::Audio(format!("Downloaded audio is unreadable: {error}")))?
        .guess_file_type()?
        .read()
        .map_err(|error| AppError::Audio(format!("Downloaded audio is unreadable: {error}")))?;
    let mut decoder =
        neri_playback::remote::SymphoniaAudioDecoder::new_file(path).map_err(AppError::Audio)?;
    if decoder.next().is_none() {
        return Err(AppError::Audio(
            "Downloaded file contains no decodable audio".into(),
        ));
    }
    let header_duration_ms = tagged.properties().duration().as_millis() as u64;
    // 分片 MP4 的头部时长可能为零，先用播放解码器信息，再按实际音频包时间轴确认
    let actual_duration_ms = if header_duration_ms > 0 {
        header_duration_ms
    } else if let Some(duration) = decoder
        .total_duration()
        .filter(|duration| !duration.is_zero())
    {
        duration.as_millis() as u64
    } else {
        scan_download_duration(path, cancelled)?
    };
    if actual_duration_ms == 0 && (expected_duration_ms > 0 || expected_md5.is_some()) {
        return Err(AppError::Audio(
            "Cannot verify downloaded audio duration".into(),
        ));
    }
    let tolerance_ms = (expected_duration_ms / 200).clamp(1000, 2000);
    if expected_md5.is_none()
        && expected_duration_ms > 0
        && actual_duration_ms > 0
        && actual_duration_ms.abs_diff(expected_duration_ms) > tolerance_ms
    {
        return Err(AppError::Audio(
            "Downloaded audio duration does not match the resolved source".into(),
        ));
    }
    if std::fs::metadata(path)?.len() != before {
        return Err(AppError::Audio(
            "Downloaded audio changed during validation".into(),
        ));
    }
    if cancelled.load(Ordering::Relaxed) {
        return Err(AppError::Other("Download cancelled".into()));
    }
    Ok(actual_duration_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_alignment_download_checks_source_md5_and_duration() {
        use md5::{Digest, Md5};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("audio.part");
        let bytes = include_bytes!("../../neri-playback/src/fixtures/hls-silence.aac");
        std::fs::write(&path, bytes).unwrap();
        let cancelled = AtomicBool::new(false);
        let checksum = hex::encode(Md5::digest(bytes));
        assert!(
            validate_download_audio(
                &path,
                Some(bytes.len() as u64),
                Some(&checksum),
                275_000,
                &cancelled
            )
            .unwrap()
                > 0
        );
        assert!(
            validate_download_audio(&path, None, Some(&"00".repeat(16)), 0, &cancelled).is_err()
        );
        assert!(validate_download_audio(&path, None, None, 275_000, &cancelled).is_err());
        assert!(
            validate_download_audio(&path, Some(bytes.len() as u64 + 1), None, 0, &cancelled)
                .is_err()
        );
    }

    #[test]
    fn android_alignment_download_rejects_error_bodies_and_cancelled_validation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("audio.part");
        std::fs::write(&path, b"<html>Access denied</html>").unwrap();
        assert!(validate_download_audio(&path, None, None, 0, &AtomicBool::new(false)).is_err());
        let bytes = include_bytes!("../../neri-playback/src/fixtures/hls-silence.aac");
        std::fs::write(&path, bytes).unwrap();
        assert!(validate_download_audio(&path, None, None, 0, &AtomicBool::new(true)).is_err());
    }

    #[test]
    fn downloads_verify_fragmented_mp4_without_header_duration() {
        use lofty::file::AudioFile;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("audio.part");
        let mut bytes = include_bytes!("../../neri-playback/src/fixtures/hls-init.mp4").to_vec();
        bytes.extend_from_slice(include_bytes!(
            "../../neri-playback/src/fixtures/hls-segment.m4s"
        ));
        std::fs::write(&path, &bytes).unwrap();
        let tagged = lofty::probe::Probe::open(&path)
            .unwrap()
            .guess_file_type()
            .unwrap()
            .read()
            .unwrap();
        assert!(
            tagged.properties().duration().is_zero(),
            "分片 MP4 的头部没有最终时长"
        );
        let cancelled = AtomicBool::new(false);
        assert_eq!(
            validate_download_audio(&path, Some(bytes.len() as u64), None, 192, &cancelled)
                .unwrap(),
            192
        );
        assert!(
            validate_download_audio(&path, None, None, 20_000, &cancelled).is_err(),
            "可解码不代表时长匹配"
        );
        assert!(validate_download_audio(&path, None, None, 192, &AtomicBool::new(true)).is_err());
    }
}
