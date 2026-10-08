use std::collections::HashMap;
use std::path::Path;
use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex, OnceLock};
use lofty::config::ParseOptions;
use lofty::file::{AudioFile, FileType, TaggedFileExt};
use lofty::probe::Probe;
use lofty::properties::FileProperties;
use tauri::{AppHandle, Emitter, Manager};
use crate::error::{AppError, AppResult};
use crate::library::scanner;
use crate::state::TrackInfo;

static SCANS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

fn scans() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    SCANS.get_or_init(|| Mutex::new(HashMap::new()))
}

struct ScanSession(String);

impl Drop for ScanSession {
    fn drop(&mut self) {
        scans().lock().unwrap_or_else(|error| error.into_inner()).remove(&self.0);
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalScanProgress {
    session_id: String,
    visited_entries: usize,
    tracks: usize,
    skipped: usize,
    current_path: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalAudioInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    bitrate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    codec: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sample_rate_hz: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bit_depth: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel_count: Option<u8>,
}

fn local_audio_info(file_type: FileType, properties: &FileProperties) -> LocalAudioInfo {
    let (codec, format) = match file_type {
        FileType::Aac => (Some("AAC"), Some("aac")),
        FileType::Aiff => (None, Some("aiff")),
        FileType::Ape => (Some("APE"), Some("ape")),
        FileType::Flac => (Some("FLAC"), Some("flac")),
        FileType::Mpeg => (Some("MPEG"), Some("mpeg")),
        FileType::Mp4 => (None, Some("mp4")),
        FileType::Mpc => (Some("Musepack"), Some("mpc")),
        FileType::Opus => (Some("Opus"), Some("opus")),
        FileType::Vorbis => (Some("Vorbis"), Some("vorbis")),
        FileType::Speex => (Some("Speex"), Some("speex")),
        FileType::Wav => (None, Some("wav")),
        FileType::WavPack => (Some("WavPack"), Some("wavpack")),
        FileType::Custom(name) => (None, Some(name)),
        _ => (None, None),
    };
    LocalAudioInfo {
        bitrate: properties.audio_bitrate().filter(|value| *value > 0),
        codec: codec.map(str::to_string),
        format: format.map(str::to_string),
        sample_rate_hz: properties.sample_rate().filter(|value| *value > 0),
        bit_depth: properties.bit_depth().filter(|value| *value > 0),
        channel_count: properties.channels().filter(|value| *value > 0),
    }
}

fn read_local_audio_info(path: &Path) -> AppResult<LocalAudioInfo> {
    if !std::fs::metadata(path)?.is_file() {
        return Err(AppError::Metadata("音频路径不是文件".into()));
    }
    // 容器不能确定具体编码时只返回格式，避免把 MP4 中的 ALAC 当成 AAC
    let audio = Probe::open(path).map_err(|error| AppError::Metadata(error.to_string()))?
        .options(ParseOptions::new().read_tags(false).read_cover_art(false))
        .guess_file_type()?.read().map_err(|error| AppError::Metadata(error.to_string()))?;
    Ok(local_audio_info(audio.file_type(), audio.properties()))
}

#[tauri::command]
pub async fn get_local_audio_info(path: String) -> AppResult<LocalAudioInfo> {
    tokio::task::spawn_blocking(move || read_local_audio_info(Path::new(&path)))
        .await.map_err(|error| AppError::Other(error.to_string()))?
}

#[tauri::command]
pub async fn scan_local_files(
    app: AppHandle,
    session_id: String,
    dir: String,
    name_template: Option<String>,
) -> AppResult<scanner::ScanResult> {
    if session_id.is_empty() || session_id.len() > 128 {
        return Err(AppError::Other("Invalid scan session".into()));
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut active = scans().lock().unwrap_or_else(|error| error.into_inner());
        if !active.is_empty() {
            return Err(AppError::Other("A local scan is already running".into()));
        }
        active.insert(session_id.clone(), cancelled.clone());
    }
    let session = ScanSession(session_id.clone());
    tokio::task::spawn_blocking(move || {
        let _session = session;
        scanner::scan_directory_with_control(&dir, name_template.as_deref(), &cancelled, |visited_entries, tracks, skipped, path| {
            let _ = app.emit("local-scan-progress", LocalScanProgress {
                session_id: session_id.clone(), visited_entries, tracks, skipped,
                current_path: path.display().to_string(),
            });
        })
    }).await.map_err(|error| AppError::Other(error.to_string()))?
}

#[tauri::command]
pub fn cancel_local_scan(session_id: String) -> bool {
    if let Some(cancelled) = scans().lock().unwrap_or_else(|error| error.into_inner()).get(&session_id) {
        cancelled.store(true, Ordering::Release);
        return true;
    }
    false
}

#[tauri::command]
pub async fn get_local_playlist_tracks() -> AppResult<Vec<TrackInfo>> {
    tokio::task::spawn_blocking(|| crate::library::playlist::load_all_tracks(None))
        .await
        .map_err(|error| AppError::Other(error.to_string()))?
}

#[tauri::command]
pub async fn edit_local_file_tags(app: AppHandle, scan_root: String, file_path: String, title: String, artist: String, album: String) -> AppResult<()> {
    super::player_cmd::release_player_file(
        Arc::clone(&app.state::<crate::state::AppState>().player), file_path.clone(),
    ).await?;
    tokio::task::spawn_blocking(move || {
        let root = std::path::Path::new(&scan_root);
        let audio = std::path::Path::new(&file_path);
        super::download_cmd::edit_download_metadata(&app, audio, &title, &artist, &album, || {
            crate::library::local_file_tags::edit_tags(root, audio, &title, &artist, &album)
        })
    }).await.map_err(|error| AppError::Other(error.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn local_audio_info_reads_wav_properties_in_kbps_without_changing_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("downloaded-audio.mp3");
        let bytes = wav();
        std::fs::write(&path, &bytes).unwrap();
        let info = read_local_audio_info(&path).unwrap();
        assert_eq!(info.format.as_deref(), Some("wav"));
        assert_eq!(info.bitrate, Some(128));
        assert_eq!(info.sample_rate_hz, Some(8000));
        assert_eq!(info.bit_depth, Some(16));
        assert_eq!(info.channel_count, Some(1));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["sampleRateHz"], 8000);
        assert_eq!(json["bitDepth"], 16);
        assert_eq!(json["channelCount"], 1);
        assert!(!json.as_object().unwrap().contains_key("codec"));
    }

    #[test]
    fn local_audio_info_missing_file_and_directory_return_errors() {
        let directory = tempfile::tempdir().unwrap();
        assert!(matches!(read_local_audio_info(&directory.path().join("missing.wav")), Err(AppError::Io(_))));
        assert!(matches!(read_local_audio_info(directory.path()), Err(AppError::Metadata(_))));
    }

    #[test]
    fn local_audio_info_omits_missing_properties_and_does_not_guess_mp4_codec() {
        let info = local_audio_info(FileType::Mp4, &FileProperties::default());
        assert_eq!(serde_json::to_value(info).unwrap(), serde_json::json!({ "format": "mp4" }));
        let zero = FileProperties::new(std::time::Duration::ZERO, None, Some(0), Some(0), Some(0), Some(0), None);
        assert_eq!(serde_json::to_value(local_audio_info(FileType::Wav, &zero)).unwrap(), serde_json::json!({ "format": "wav" }));
    }
}
