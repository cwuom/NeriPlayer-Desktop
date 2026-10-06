use crate::api::transport::FallbackHttp;
use crate::api::youtube::client::YtStreamType;
use crate::error::{AppError, AppResult};
use crate::lyrics::manager::LyricsManager;
use crate::lyrics::parser::LyricLine;
use crate::settings::store::DEFAULT_DOWNLOAD_NAME_TEMPLATE;
use crate::state::AppState;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::AsyncWriteExt;

#[path = "download_recovery.rs"]
mod recovery;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadManifestValidation {
    pub tracks: Vec<DownloadedTrack>,
    pub removed_count: usize,
    pub integrity_mismatch_count: usize,
}

/// 流式下载的空闲超时：超过该时长收不到任何数据视为连接假死。
/// 半开 TCP 下 `stream.next()` 会永久阻塞——任务卡死且因注册表判
/// "downloading" 无法重试，必须有兜底
const STREAM_STALL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Default)]
struct DownloadArtifacts {
    part: Option<PathBuf>,
    reservations: Vec<PathBuf>,
    uncommitted_audio: Option<PathBuf>,
}

impl Drop for DownloadArtifacts {
    fn drop(&mut self) {
        // 取消会直接丢弃 transfer future，清理不能依赖下一次网络回调
        if let Some(part) = &self.part {
            let _ = std::fs::remove_file(part);
        }
        for reservation in &self.reservations {
            let _ = std::fs::remove_file(reservation);
        }
        if let Some(audio) = &self.uncommitted_audio {
            remove_download_artifacts(&audio.to_string_lossy());
        }
    }
}

fn register_download_task(
    registry: &parking_lot::Mutex<
        std::collections::HashMap<String, crate::state::DownloadTaskControl>,
    >,
    track_id: String,
    launch: impl FnOnce() -> crate::state::DownloadTaskControl,
) -> AppResult<()> {
    let mut tasks = registry.lock();
    if tasks
        .get(&track_id)
        .is_some_and(|task| !task.handle.is_finished())
    {
        return Err(AppError::Other("Track is already downloading".into()));
    }
    tasks.retain(|_, control| !control.handle.is_finished());
    // 检查和启动登记共用一把锁，第二个同曲目请求不能覆盖第一个取消句柄
    tasks.insert(track_id, launch());
    Ok(())
}

fn finish_download_task(
    registry: &parking_lot::Mutex<
        std::collections::HashMap<String, crate::state::DownloadTaskControl>,
    >,
    track_id: &str,
    identity: &Arc<AtomicBool>,
) -> Option<crate::state::DownloadTaskControl> {
    let mut tasks = registry.lock();
    if tasks
        .get(track_id)
        .is_some_and(|task| Arc::ptr_eq(&task.cancel_flag, identity))
    {
        tasks.remove(track_id)
    } else {
        None
    }
}

async fn refresh_youtube_download_source(
    app: AppHandle,
    video_id: String,
    quality: String,
    avoid_direct: bool,
) -> AppResult<recovery::Source> {
    let auth = app.state::<AppState>().auth.lock().youtube.clone();
    let playback_source = crate::settings::store::load_settings(&app)?.settings.youtube_playback_source;
    let mut last_error = None;
    for (force_refresh, require_direct) in recovery::resolve_plan(true, avoid_direct) {
        let result = tokio::time::timeout(
            Duration::from_secs(18),
            crate::api::youtube::playback::resolve_audio_streams_with_source(
                &video_id,
                auth.as_ref().filter(|auth| auth.has_login()),
                Some(&app),
                force_refresh,
                avoid_direct,
                &playback_source,
            ),
        )
        .await;
        match result {
            Ok(Ok(streams)) => {
                if let Some(stream) = recovery::select_stream(streams, &quality, require_direct) {
                    return Ok(stream);
                }
            }
            Ok(Err(error)) => last_error = Some(error),
            Err(_) => {
                last_error = Some(AppError::Other(
                    "YouTube download source resolution timed out".into(),
                ))
            }
        }
    }
    Err(last_error
        .unwrap_or_else(|| AppError::NotFound("No supported YouTube download source".into())))
}

/// 文件名 stem 的最大字节数（UTF-8）：为扩展名与 " (n)" 后缀留余量，
/// 避免超出各文件系统 255 字节单文件名 / Windows MAX_PATH 限制
const MAX_FILENAME_STEM_BYTES: usize = 180;

/// Windows 设备保留名：即使带扩展名（如 CON.mp3）也会创建失败或写入设备
const WINDOWS_RESERVED_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// 按字节数截断，绝不切断多字节字符
fn truncate_at_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut cut = max_bytes;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    &s[..cut]
}

/// 清理文件名中的非法字符
///
/// 覆盖：路径分隔/Windows 非法字符、控制字符（0x00-0x1F 等）、结尾的
/// `.`/空格（Windows 非法）、Windows 设备保留名（加 `_` 前缀）、
/// 180 字节截断（扩展名由调用方追加）
fn sanitize_filename(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            _ => c,
        })
        .collect();
    let cleaned = truncate_at_char_boundary(cleaned.trim(), MAX_FILENAME_STEM_BYTES)
        .trim_end_matches(['.', ' '])
        .to_string();
    // 保留名按首个 `.` 之前的 stem 判定（CON 与 CON.tail 都保留给设备）
    let stem = cleaned.split('.').next().unwrap_or("");
    let reserved = WINDOWS_RESERVED_NAMES
        .iter()
        .any(|name| stem.eq_ignore_ascii_case(name));
    if reserved {
        format!("_{cleaned}")
    } else {
        cleaned
    }
}

/// 根据模板渲染下载文件名（不含扩展名）
/// 支持占位符：{title}, {artist}, {album}, {source}, {id}, {audioId}, {subAudioId}（%%写法同）
fn render_download_filename(
    title: &str,
    artist: &str,
    album: &str,
    source: &str,
    track_id: &str,
    template: Option<&str>,
) -> String {
    let tpl = template
        .filter(|t| !t.is_empty())
        .unwrap_or(DEFAULT_DOWNLOAD_NAME_TEMPLATE);
    // audioId = 去平台前缀的 id；subAudioId = bilibili album 中的 cid（"Bilibili|<cid>"）
    let audio_id = track_id.split_once(':').map(|(_, r)| r).unwrap_or(track_id);
    let sub_audio_id = album
        .strip_prefix("Bilibili|")
        .map(|s| s.split('|').next().unwrap_or(s))
        .unwrap_or("");
    let rendered = tpl
        .replace("{title}", title)
        .replace("{artist}", artist)
        .replace("{album}", album)
        .replace("{source}", source)
        .replace("{id}", audio_id)
        .replace("{audioId}", audio_id)
        .replace("{subAudioId}", sub_audio_id)
        .replace("%title%", title)
        .replace("%artist%", artist)
        .replace("%album%", album)
        .replace("%source%", source)
        .replace("%id%", audio_id)
        .replace("%audioId%", audio_id)
        .replace("%subAudioId%", sub_audio_id);
    // 折叠占位符替换为空后残留的多余分隔符与空括号（SC-5）
    let rendered = collapse_empty_name_separators(&rendered);
    let sanitized = sanitize_filename(&rendered);
    if sanitized.is_empty() {
        sanitize_filename(title)
    } else {
        sanitized
    }
}

/// 清理占位符替换后残留的空括号、连续分隔符与首尾分隔符
fn collapse_empty_name_separators(s: &str) -> String {
    let mut out = s.to_string();
    // 移除空的 []/() 及其内部空白
    for _ in 0..3 {
        out = out.replace("[]", "").replace("()", "");
        out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    // 折叠连续 " - " 分隔符并去首尾分隔符/空白
    while out.contains("-  -") || out.contains("- -") {
        out = out.replace("-  -", "-").replace("- -", "-");
    }
    out.trim()
        .trim_matches(|c| c == '-' || c == ' ')
        .trim()
        .to_string()
}
fn ext_from_content_type(content_type: &str) -> &str {
    if content_type.contains("mp4") || content_type.contains("m4a") || content_type.contains("aac")
    {
        "m4a"
    } else if content_type.contains("ogg") || content_type.contains("opus") {
        "ogg"
    } else if content_type.contains("webm") {
        "webm"
    } else if content_type.contains("flac") {
        "flac"
    } else if content_type.contains("wav") {
        "wav"
    } else {
        "mp3"
    }
}

fn ext_from_image_content_type(content_type: &str) -> &str {
    if content_type.contains("png") {
        "png"
    } else if content_type.contains("webp") {
        "webp"
    } else if content_type.contains("gif") {
        "gif"
    } else {
        "jpg"
    }
}

fn make_lrc_timestamp(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    let centis = (ms % 1000) / 10;
    format!("{:02}:{:02}.{:02}", minutes, seconds, centis)
}

fn build_lrc_text(lines: &[LyricLine]) -> Option<String> {
    let mut out = String::new();
    for line in lines {
        let text = line.text.trim();
        if text.is_empty() {
            continue;
        }
        out.push('[');
        out.push_str(&make_lrc_timestamp(line.start_ms));
        out.push(']');
        out.push_str(text);
        out.push('\n');
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

fn build_translation_lrc_text(lines: &[LyricLine]) -> Option<String> {
    let mut out = String::new();
    for line in lines {
        let Some(translated) = line.translation.as_deref() else {
            continue;
        };
        let text = translated.trim();
        if text.is_empty() {
            continue;
        }
        out.push('[');
        out.push_str(&make_lrc_timestamp(line.start_ms));
        out.push(']');
        out.push_str(text);
        out.push('\n');
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

fn extract_netease_id(track_id: &str) -> Option<u64> {
    track_id
        .strip_prefix("netease:")
        .and_then(|id| id.parse::<u64>().ok())
}

fn extract_qq_song_mid(track_id: &str) -> Option<&str> {
    track_id.strip_prefix("qq:").filter(|id| !id.is_empty())
}

async fn write_download_sidecars(
    client: &reqwest::Client,
    file_path: &std::path::Path,
    track_id: &str,
    title: &str,
    artist: &str,
    duration_ms: u64,
    cover_url: Option<&str>,
) -> AppResult<()> {
    let mut written_files: Vec<PathBuf> = Vec::new();

    // 歌词 sidecar
    let lyrics_manager = LyricsManager::new(client);
    let youtube_video_id = track_id
        .strip_prefix("youtube:")
        .filter(|id| !id.is_empty());
    let lyrics = lyrics_manager
        .fetch_lyrics(
            title,
            artist,
            duration_ms / 1000,
            None,
            extract_netease_id(track_id),
            extract_qq_song_mid(track_id),
            youtube_video_id,
        )
        .await
        .unwrap_or_default();

    if let Some(lrc_text) = build_lrc_text(&lyrics) {
        let lrc_path = download_sidecar_path(file_path, "lrc")
            .ok_or_else(|| AppError::Other("下载文件名无效，无法写入歌词".into()))?;
        // 原子写：sidecar 是 load_local_sidecar_lyrics 的最高优先级本地源，
        // 半截写入会被读回成损坏歌词（LY-6）
        crate::fsutil::atomic_write(&lrc_path, lrc_text.as_bytes())?;
        written_files.push(lrc_path);
    }

    if let Some(tlrc_text) = build_translation_lrc_text(&lyrics) {
        let tlrc_path = download_sidecar_path(file_path, "tlrc")
            .ok_or_else(|| AppError::Other("下载文件名无效，无法写入翻译歌词".into()))?;
        crate::fsutil::atomic_write(&tlrc_path, tlrc_text.as_bytes())?;
        written_files.push(tlrc_path);
    }

    // 封面 sidecar
    if let Some(raw_cover_url) = cover_url {
        let normalized_cover_url = if raw_cover_url.starts_with("//") {
            format!("https:{}", raw_cover_url)
        } else {
            raw_cover_url.to_string()
        };
        if !normalized_cover_url.trim().is_empty() {
            if let Ok(resp) = client
                .get(&normalized_cover_url)
                .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
                .send()
                .await
            {
                if resp.status().is_success() {
                    let content_type = resp
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("image/jpeg")
                        .to_lowercase();
                    if content_type.starts_with("image/") {
                        if let Ok(bytes) = resp.bytes().await {
                            if !bytes.is_empty() {
                                let ext = ext_from_image_content_type(&content_type);
                                let cover_path = download_sidecar_path(file_path, ext)
                                    .ok_or_else(|| {
                                        AppError::Other("下载文件名无效，无法写入封面".into())
                                    })?;
                                // 原子写封面 sidecar（DL-4）
                                crate::fsutil::atomic_write(&cover_path, &bytes)?;
                                written_files.push(cover_path);
                            }
                        }
                    }
                }
            }
        }
    }

    if written_files.is_empty() {
        return Ok(());
    }
    Ok(())
}

/// 获取下载目录，不存在则创建
/// 如果提供了 custom_dir 且有效，则使用自定义目录；否则 fallback 到默认目录
fn downloads_dir(app: &AppHandle, custom_dir: Option<&str>) -> AppResult<PathBuf> {
    let dir = if let Some(cd) = custom_dir {
        if !cd.is_empty() {
            let p = PathBuf::from(cd);
            // 尝试创建目录（如果不存在）
            if !p.exists() {
                if std::fs::create_dir_all(&p).is_ok() {
                    return Ok(p);
                }
                // 创建失败(U盘拔出/权限变更): 静默回退默认目录会让用户以为文件落在
                // 所选目录、实际散落 AppData。回退前告知前端（DL-11）
                let _ = app.emit(
                    "download-dir-fallback",
                    serde_json::json!({ "requestedDir": cd }),
                );
            } else {
                return Ok(p);
            }
        }
        // 空字符串或无效路径，fallback
        default_downloads_dir(app)?
    } else {
        default_downloads_dir(app)?
    };
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(AppError::Io)?;
    }
    Ok(dir)
}

/// 默认下载目录
fn default_downloads_dir(app: &AppHandle) -> AppResult<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Other(e.to_string()))?
        .join("downloads");
    Ok(dir)
}

/// 验证并设置下载目录
#[tauri::command]
pub async fn set_download_dir(path: String) -> AppResult<String> {
    let p = PathBuf::from(&path);
    // 确保目录存在
    if !p.exists() {
        std::fs::create_dir_all(&p)
            .map_err(|e| AppError::Other(format!("Cannot create directory: {}", e)))?;
    }
    // 验证可写：创建临时文件再删除
    let test_file = p.join(".neri_write_test");
    std::fs::write(&test_file, b"test")
        .map_err(|_| AppError::Other("Directory is not writable".into()))?;
    let _ = std::fs::remove_file(&test_file);
    // 返回规范化路径
    let canonical = p.canonicalize().unwrap_or(p).to_string_lossy().to_string();
    // 移除 Windows UNC 前缀 \\?\
    let canonical = canonical
        .strip_prefix(r"\\?\")
        .unwrap_or(&canonical)
        .to_string();
    Ok(canonical)
}

/// 获取默认下载目录路径
#[tauri::command]
pub async fn get_default_download_dir(app: AppHandle) -> AppResult<String> {
    let dir = default_downloads_dir(&app)?;
    Ok(dir.to_string_lossy().to_string())
}

/// 下载并发信号量（进程级，上限 8，对齐 Android DownloadParallelism）
fn download_semaphore() -> &'static tokio::sync::Semaphore {
    static SEM: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    SEM.get_or_init(|| tokio::sync::Semaphore::new(8))
}

/// manifest.json 路径（始终存储在默认下载目录，与自定义目录无关）
fn manifest_path(app: &AppHandle) -> AppResult<PathBuf> {
    let dir = default_downloads_dir(app)?;
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(AppError::Io)?;
    }
    Ok(dir.join("manifest.json"))
}

fn download_sidecar_path(audio_path: &std::path::Path, suffix: &str) -> Option<PathBuf> {
    let file_name = audio_path.file_name()?;
    let mut sidecar_name = file_name.to_os_string();
    sidecar_name.push(format!(".{suffix}"));
    Some(audio_path.with_file_name(sidecar_name))
}

fn reserve_path_for(audio_path: &std::path::Path) -> Option<PathBuf> {
    download_sidecar_path(audio_path, "reserve")
}

fn path_exists_including_broken_symlink(path: &std::path::Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

fn has_download_size_mismatch(expected: u64, actual: u64) -> bool {
    expected > 0 && expected != actual
}

fn validate_download_audio(
    path: &std::path::Path,
    expected_length: Option<u64>,
    expected_md5: Option<&str>,
    expected_duration_ms: u64,
    cancelled: &AtomicBool,
) -> AppResult<u64> {
    use lofty::file::AudioFile;
    use md5::{Digest, Md5};
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
    let actual_duration_ms = tagged.properties().duration().as_millis() as u64;
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
    let mut decoder =
        crate::audio::remote::SymphoniaAudioDecoder::new_file(path).map_err(AppError::Audio)?;
    if decoder.next().is_none() {
        return Err(AppError::Audio(
            "Downloaded file contains no decodable audio".into(),
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

fn reserve_download_path(
    dir: &std::path::Path,
    base_name: &str,
    ext: &str,
) -> AppResult<(PathBuf, PathBuf)> {
    let mut file_path = dir.join(format!("{base_name}.{ext}"));
    let mut collision_suffix = 2_u32;
    loop {
        if path_exists_including_broken_symlink(&file_path) {
            file_path = dir.join(format!("{base_name} ({collision_suffix}).{ext}"));
            collision_suffix += 1;
            if collision_suffix > 1_000 {
                return Err(AppError::Other("Too many filename collisions".into()));
            }
            continue;
        }
        let Some(candidate_reserve) = reserve_path_for(&file_path) else {
            return Err(AppError::Other("无法为下载文件创建保留标记".into()));
        };
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate_reserve)
        {
            Ok(marker) => {
                if let Err(error) = marker.sync_all() {
                    drop(marker);
                    let _ = std::fs::remove_file(&candidate_reserve);
                    return Err(AppError::Io(error));
                }
                return Ok((file_path, candidate_reserve));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                file_path = dir.join(format!("{base_name} ({collision_suffix}).{ext}"));
                collision_suffix += 1;
                if collision_suffix > 1_000 {
                    return Err(AppError::Other("Too many filename collisions".into()));
                }
            }
            Err(e) => return Err(AppError::Io(e)),
        }
    }
}

/// 读取 manifest
fn read_manifest(app: &AppHandle) -> AppResult<Vec<DownloadedTrack>> {
    let path = manifest_path(app)?;
    if !path.exists() {
        return Ok(vec![]);
    }
    let data = std::fs::read_to_string(&path)?;
    match serde_json::from_str::<Vec<DownloadedTrack>>(&data) {
        Ok(tracks) => Ok(tracks),
        Err(e) => {
            // manifest 解析失败不能静默当空表：后续任一次 write_manifest 会用空表覆盖，
            // 全部历史下载记录永久丢失。隔离损坏文件后返回错误，保留磁盘上的原始现场（DL-6）
            log::error!(target: "download", "manifest 解析失败, 已隔离: {e}");
            let _ = crate::fsutil::quarantine_corrupt_file(&path);
            Err(AppError::Other(format!("下载清单损坏，已隔离原文件: {e}")))
        }
    }
}

/// 写入 manifest（原子写：半截 manifest 会让全部下载记录判损坏丢失）
fn write_manifest(app: &AppHandle, tracks: &[DownloadedTrack]) -> AppResult<()> {
    let path = manifest_path(app)?;
    let json = serde_json::to_string_pretty(tracks)?;
    crate::fsutil::atomic_write(&path, json)?;
    Ok(())
}

fn manifest_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    match LOCK.get_or_init(|| std::sync::Mutex::new(())).lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            log::warn!(target: "download", "manifest lock was poisoned, recovering");
            poisoned.into_inner()
        }
    }
}

const DOWNLOAD_SIDECAR_SUFFIXES: [&str; 10] = [
    "lrc",
    "tlrc",
    "txt",
    "translated.lrc",
    "translation.lrc",
    "jpg",
    "jpeg",
    "png",
    "webp",
    "gif",
];

fn has_ambiguous_legacy_sidecar(audio_path: &std::path::Path) -> bool {
    let Some(parent) = audio_path.parent() else {
        return false;
    };
    let Some(stem) = audio_path.file_stem() else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        path != audio_path
            && path.is_file()
            && path.file_stem() == Some(stem)
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    let lower = ext.to_ascii_lowercase();
                    !DOWNLOAD_SIDECAR_SUFFIXES
                        .iter()
                        .any(|suffix| *suffix == lower)
                })
    })
}

fn candidate_download_sidecars(audio_path: &std::path::Path) -> Vec<PathBuf> {
    let mut candidates = Vec::with_capacity(DOWNLOAD_SIDECAR_SUFFIXES.len() * 2);
    for suffix in DOWNLOAD_SIDECAR_SUFFIXES {
        if let Some(path) = download_sidecar_path(audio_path, suffix) {
            candidates.push(path);
        }
    }

    // 旧版本按 stem 写 sidecar。只有目录中不存在同 stem 的其它文件时才删除，
    // 否则删除一个扩展名可能误删另一个下载仍在使用的旧 sidecar
    if !has_ambiguous_legacy_sidecar(audio_path) {
        if let (Some(parent), Some(stem)) = (
            audio_path.parent(),
            audio_path.file_stem().and_then(|s| s.to_str()),
        ) {
            for suffix in DOWNLOAD_SIDECAR_SUFFIXES {
                candidates.push(parent.join(format!("{stem}.{suffix}")));
            }
        }
    }
    candidates
}

fn remove_download_artifacts(file_path: &str) {
    let audio_path = std::path::Path::new(file_path);
    // 先删同名 sidecar，再删主音频；全部忽略错误，避免手动删除/占用时影响 manifest 清理。
    for sidecar in candidate_download_sidecars(audio_path) {
        if sidecar.is_file() {
            let _ = std::fs::remove_file(sidecar);
        }
    }
    let _ = std::fs::remove_file(audio_path);
}

/// 清扫遗留的下载标记和半截下载文件
///
/// 只清理最后修改超过 1 小时的：活动下载的 .part 和 .reserve 由任务自身负责删除，
/// 且流有 30s 空闲超时，能活过 1 小时的标记通常是崩溃或断电遗留
fn marker_is_stale(path: &std::path::Path, stale_after: Duration) -> bool {
    std::fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.elapsed().ok())
        .is_some_and(|age| age >= stale_after)
}

fn reserve_has_fresh_part(reserve_path: &std::path::Path, stale_after: Duration) -> bool {
    let Some(file_name) = reserve_path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(audio_name) = file_name.strip_suffix(".reserve") else {
        return false;
    };
    let part_path = reserve_path.with_file_name(format!("{audio_name}.part"));
    part_path.is_file() && !marker_is_stale(&part_path, stale_after)
}

fn sweep_stale_download_markers(dirs: impl IntoIterator<Item = PathBuf>) -> usize {
    const STALE_AFTER: Duration = Duration::from_secs(3_600);
    let mut removed = 0_usize;
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for dir in dirs {
        if !seen.insert(dir.clone()) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_marker = matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("part" | "reserve")
            );
            if !is_marker || !path.is_file() {
                continue;
            }
            let stale = marker_is_stale(&path, STALE_AFTER);
            // reservation 只在任务启动时写入，长下载中自身会超过阈值。
            // 只要同名 part 仍在持续写入，就不能把 reservation 当成崩溃残留删掉
            let active_reservation = path.extension().and_then(|ext| ext.to_str())
                == Some("reserve")
                && reserve_has_fresh_part(&path, STALE_AFTER);
            if stale && !active_reservation && std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
    }
    removed
}

fn validate_manifest_files(app: &AppHandle) -> AppResult<DownloadManifestValidation> {
    let _manifest_guard = manifest_lock();
    let manifest = read_manifest(app)?;

    // 顺带清扫崩溃或断电遗留的下载标记：默认下载目录加 manifest 涉及的自定义目录
    let mut sweep_dirs: Vec<PathBuf> = manifest
        .iter()
        .filter_map(|track| {
            std::path::Path::new(&track.file_path)
                .parent()
                .map(|parent| parent.to_path_buf())
        })
        .collect();
    if let Ok(default_dir) = default_downloads_dir(app) {
        sweep_dirs.push(default_dir);
    }
    let swept = sweep_stale_download_markers(sweep_dirs);
    if swept > 0 {
        log::info!(target: "download", "swept {} stale download markers", swept);
    }

    let mut valid = Vec::with_capacity(manifest.len());
    let mut removed_count = 0_usize;
    let mut integrity_mismatch_count = 0_usize;
    let mut changed = false;

    for track in manifest {
        let path = std::path::Path::new(&track.file_path);
        if path.is_file() {
            if let Ok(meta) = std::fs::metadata(path) {
                let actual_size = meta.len();
                // 大小不一致是检测截断/外部篡改的唯一信号, 不再静默自愈改写 manifest
                // （会销毁完整性证据, DL-2）; 仅告警, 保留清单记录的期望大小
                if has_download_size_mismatch(track.file_size, actual_size) {
                    integrity_mismatch_count += 1;
                    log::warn!(
                        target: "download",
                        "下载文件大小与清单不一致(疑似截断/篡改): path={}, manifest={}, actual={}",
                        track.file_path, track.file_size, actual_size
                    );
                }
            }
            valid.push(track);
        } else {
            // 主音频已丢失时，也顺手清理同名歌词/封面 sidecar，避免残留。
            remove_download_artifacts(&track.file_path);
            removed_count += 1;
            changed = true;
        }
    }

    if changed {
        write_manifest(app, &valid)?;
    }

    Ok(DownloadManifestValidation {
        tracks: valid,
        removed_count,
        integrity_mismatch_count,
    })
}

fn emit_download_progress(
    app: &AppHandle,
    track_id: &str,
    status: &str,
    message: Option<&str>,
    file_size: Option<u64>,
    downloaded_bytes: Option<u64>,
    total_bytes: Option<u64>,
) {
    let mut payload = serde_json::json!({
        "trackId": track_id,
        "status": status,
    });
    if let Some(msg) = message {
        payload["message"] = serde_json::Value::String(msg.to_string());
    }
    if let Some(size) = file_size {
        payload["fileSize"] = serde_json::json!(size);
    }
    if let Some(downloaded) = downloaded_bytes {
        payload["downloadedBytes"] = serde_json::json!(downloaded);
    }
    if let Some(total) = total_bytes {
        payload["totalBytes"] = serde_json::json!(total);
    }
    let _ = app.emit("download-progress", payload);
}

// 播放/下载编排函数的参数都是相互独立的运行时上下文，聚成结构体只是换个地方堆字段
#[allow(clippy::too_many_arguments)]
async fn perform_download(
    app: AppHandle,
    client: reqwest::Client,
    cancel_flag: Arc<AtomicBool>,
    url: String,
    track_id: String,
    title: String,
    artist: String,
    album: String,
    duration_ms: u64,
    cover_url: Option<String>,
    source: String,
    download_dir: Option<String>,
    name_template: Option<String>,
    stream_type: YtStreamType,
    hls_http: FallbackHttp,
    expected_content_length: Option<u64>,
    expected_content_md5: Option<String>,
) -> AppResult<DownloadedTrack> {
    if cancel_flag.load(Ordering::Relaxed) {
        return Err(AppError::Other("Download cancelled".into()));
    }
    // 检查是否已下载
    let existing = {
        let _manifest_guard = manifest_lock();
        read_manifest(&app)?
    };
    if existing.iter().any(|t| t.id == track_id) {
        emit_download_progress(&app, &track_id, "already_exists", None, None, None, None);
        return Err(AppError::Other("Track already downloaded".into()));
    }

    // 根据 URL 域名动态设置 Referer（复用 player_cmd 逻辑）
    let referer = if url.contains("bilibili.com") || url.contains("bilivideo.") {
        "https://www.bilibili.com"
    } else if url.contains("youtube.com") || url.contains("googlevideo.com") {
        "https://music.youtube.com"
    } else if url.contains("qqmusic.qq.com") || url.contains("y.qq.com") {
        "https://y.qq.com"
    } else {
        "https://music.163.com"
    };

    // YouTube googlevideo CDN 校验拉流 UA 与直链 `c=` 客户端一致, 不一致 403;
    // 故 googlevideo 直链按客户端选匹配 UA, 其它平台用桌面 Chrome UA (对齐 player_cmd)
    let user_agent = if url.contains("googlevideo.com") || url.contains("youtube.com") {
        crate::api::youtube::playback::stream_user_agent_for_url(&url)
    } else {
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"
    };

    let resp = if stream_type == YtStreamType::Hls {
        None
    } else {
        Some(recovery::request(&client, &url, referer, user_agent).await?)
    };

    if cancel_flag.load(Ordering::Relaxed) {
        return Err(AppError::Other("Download cancelled".into()));
    }

    // 从 Content-Type 推断扩展名
    let content_type = resp
        .as_ref()
        .and_then(|response| response.headers().get("content-type"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("audio/mpeg")
        .to_string();
    let total_bytes = resp.as_ref().and_then(reqwest::Response::content_length);
    let ext = if stream_type == YtStreamType::Hls {
        "m4a"
    } else {
        ext_from_content_type(&content_type)
    };

    // 构造文件名：使用模板
    let base_name = render_download_filename(
        &title,
        &artist,
        &album,
        &source,
        &track_id,
        name_template.as_deref(),
    );

    let dir = downloads_dir(&app, download_dir.as_deref())?;
    // 每次下载开始顺带清扫目标目录的崩溃遗留 .part 和 .reserve: validate 的 sweep
    // 集合拿不到前端配置的自定义目录, 自定义目录首次下载即崩溃会残留标记（DL-9）
    let swept = sweep_stale_download_markers(std::iter::once(dir.clone()));
    if swept > 0 {
        log::info!(target: "download", "swept {} stale download markers in target dir", swept);
    }
    // 同名碰撞：本曲目已下载会在函数入口提前返回，走到这里时目标文件若已
    // 存在则必属于其它曲目或历史遗留。用独立 .reserve 文件做原子保留，
    // 避免两个渲染名相同的并发任务选到同一最终名后互相覆盖或争用同一 .part
    // .reserve 不会成为 rename 目标，兼容 Windows 对目标已存在的限制
    let (mut file_path, mut reserve_path) = reserve_download_path(&dir, &base_name, ext)?;
    let mut artifacts = DownloadArtifacts {
        reservations: vec![reserve_path.clone()],
        part: None,
        uncommitted_audio: None,
    };
    // 先写 `<final>.part` 临时文件，完整收尾后才 rename 为最终名：
    // 失败/取消/崩溃只会留下可识别清扫的 .part，不会产生半截"成品"文件
    let part_path = {
        let mut name = file_path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        name.push(".part");
        file_path.with_file_name(name)
    };
    let mut file = if stream_type == YtStreamType::Hls {
        None
    } else {
        Some(
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&part_path)
            {
                Ok(file) => {
                    artifacts.part = Some(part_path.clone());
                    tokio::fs::File::from_std(file)
                }
                Err(error) => {
                    let _ = tokio::fs::remove_file(&reserve_path).await;
                    return Err(AppError::Io(error));
                }
            },
        )
    };

    let mut stream = resp.map(reqwest::Response::bytes_stream);
    emit_download_progress(
        &app,
        &track_id,
        "downloading",
        None,
        None,
        Some(0),
        total_bytes,
    );
    // 集中收敛写入阶段的所有失败出口，统一在块外清理 .part
    let stream_outcome: AppResult<u64> = if stream_type == YtStreamType::Hls {
        drop(file);
        crate::audio::hls::download_hls_to_file(
            hls_http,
            url.clone(),
            part_path.clone(),
            cancel_flag.clone(),
        )
        .await
    } else {
        async {
            let mut file_size = 0_u64;
            let mut last_emit_at = Instant::now() - Duration::from_millis(500);
            let mut last_emitted_bytes = 0_u64;
            loop {
                // 空闲超时兜底：半开 TCP 下 next() 会永久挂起（见 STREAM_STALL_TIMEOUT）
                let next = match tokio::time::timeout(
                    STREAM_STALL_TIMEOUT,
                    stream.as_mut().expect("direct download stream").next(),
                )
                .await
                {
                    Ok(item) => item,
                    Err(_) => {
                        return Err(AppError::Other(
                            "Download stalled: no data received for 30s".into(),
                        ));
                    }
                };
                let Some(chunk) = next else { break };
                if cancel_flag.load(Ordering::Relaxed) {
                    return Err(AppError::Other("Download cancelled".into()));
                }
                let chunk = chunk.map_err(|error| AppError::Network(error.without_url()))?;
                if chunk.is_empty() {
                    continue;
                }
                file.as_mut()
                    .expect("direct download file")
                    .write_all(&chunk)
                    .await?;
                file_size += chunk.len() as u64;

                let should_emit = total_bytes.map(|total| file_size >= total).unwrap_or(false)
                    || file_size.saturating_sub(last_emitted_bytes) >= 256 * 1024
                    || last_emit_at.elapsed() >= Duration::from_millis(200);

                if should_emit {
                    emit_download_progress(
                        &app,
                        &track_id,
                        "downloading",
                        None,
                        None,
                        Some(file_size),
                        total_bytes,
                    );
                    last_emit_at = Instant::now();
                    last_emitted_bytes = file_size;
                }
            }
            file.as_mut().expect("direct download file").flush().await?;

            if cancel_flag.load(Ordering::Relaxed) {
                return Err(AppError::Other("Download cancelled".into()));
            }
            if file_size == 0 {
                return Err(AppError::Audio("Empty audio data received".into()));
            }
            // Content-Length 已知且实际字节数不足 => 截断流（HTTP/2 半关、代理提前 EOF、
            // CDN 改写 CL 等）。删 .part 报错, 不 rename 成品, 对齐 Android isTransferSizeComplete
            // （DL-1）。音频直链不启用压缩, 正常情况 file_size 应等于 total
            if let Some(total) = total_bytes {
                if total > 0 && file_size != total {
                    return Err(AppError::Audio(format!(
                        "下载文件不完整: 期望 {total} 字节, 实际 {file_size} 字节"
                    )));
                }
            }
            Ok(file_size)
        }
        .await
        .inspect(|_| {
            drop(file);
        })
    };
    let file_size = match stream_outcome {
        Ok(size) => {
            artifacts.part = Some(part_path.clone());
            size
        }
        Err(error) => {
            return Err(error);
        }
    };
    let validation_path = part_path.clone();
    let validation_cancel = cancel_flag.clone();
    let source_length = if stream_type == YtStreamType::Direct {
        expected_content_length
    } else {
        None
    };
    let validation = tokio::task::spawn_blocking(move || {
        validate_download_audio(
            &validation_path,
            source_length,
            expected_content_md5.as_deref(),
            duration_ms,
            &validation_cancel,
        )
    })
    .await
    .map_err(|error| AppError::Other(error.to_string()))
    .and_then(|result| result);
    let verified_duration_ms = match validation {
        Ok(duration) => duration,
        Err(error) => {
            let _ = tokio::fs::remove_file(&part_path).await;
            let _ = tokio::fs::remove_file(&reserve_path).await;
            return Err(error);
        }
    };
    let duration_ms = if verified_duration_ms > 0 {
        verified_duration_ms
    } else {
        duration_ms
    };
    if stream_type == YtStreamType::Hls {
        let final_target =
            crate::audio::hls::detect_hls_audio_extension(&part_path).and_then(|actual_ext| {
                if actual_ext == ext {
                    Ok(None)
                } else {
                    reserve_download_path(&dir, &base_name, actual_ext).map(Some)
                }
            });
        match final_target {
            Ok(Some((actual_path, actual_reserve))) => {
                artifacts.reservations.push(actual_reserve.clone());
                let _ = tokio::fs::remove_file(&reserve_path).await;
                file_path = actual_path;
                reserve_path = actual_reserve;
            }
            Ok(None) => {}
            Err(error) => {
                let _ = tokio::fs::remove_file(&part_path).await;
                let _ = tokio::fs::remove_file(&reserve_path).await;
                return Err(error);
            }
        }
    }
    if cancel_flag.load(Ordering::Relaxed) {
        let _ = tokio::fs::remove_file(&part_path).await;
        let _ = tokio::fs::remove_file(&reserve_path).await;
        return Err(AppError::Other("Download cancelled".into()));
    }
    if path_exists_including_broken_symlink(&file_path) {
        let _ = tokio::fs::remove_file(&part_path).await;
        let _ = tokio::fs::remove_file(&reserve_path).await;
        return Err(AppError::Other("下载文件名在传输期间发生冲突".into()));
    }
    // 目标文件尚未存在，rename 只提交完整的 .part，不覆盖其它曲目的成品
    if let Err(error) = std::fs::rename(&part_path, &file_path) {
        let _ = tokio::fs::remove_file(&part_path).await;
        let _ = tokio::fs::remove_file(&reserve_path).await;
        return Err(AppError::Io(error));
    }
    artifacts.part = None;
    artifacts.uncommitted_audio = Some(file_path.clone());
    let _ = tokio::fs::remove_file(&reserve_path).await;

    if cancel_flag.load(Ordering::Relaxed) {
        remove_download_artifacts(&file_path.to_string_lossy());
        return Err(AppError::Other("Download cancelled".into()));
    }

    // 下载 sidecar（歌词/翻译歌词/封面），失败不影响主音频
    if let Err(e) = write_download_sidecars(
        &client,
        &file_path,
        &track_id,
        &title,
        &artist,
        duration_ms,
        cover_url.as_deref(),
    )
    .await
    {
        log::warn!(
            target: "download",
            "write sidecars failed: track_id={}, title={}, error={}",
            track_id, title, e
        );
    }

    if cancel_flag.load(Ordering::Relaxed) {
        remove_download_artifacts(&file_path.to_string_lossy());
        return Err(AppError::Other("Download cancelled".into()));
    }

    // 构造记录
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let track = DownloadedTrack {
        id: track_id.clone(),
        title,
        artist,
        album,
        duration_ms,
        cover_url,
        source,
        file_path: file_path.to_string_lossy().to_string(),
        file_size,
        downloaded_at: now,
    };

    let _manifest_guard = manifest_lock();
    let mut manifest = read_manifest(&app)?;
    if cancel_flag.load(Ordering::Relaxed) {
        remove_download_artifacts(&file_path.to_string_lossy());
        return Err(AppError::Other("Download cancelled".into()));
    }
    manifest.push(track.clone());
    if cancel_flag.load(Ordering::Relaxed) {
        remove_download_artifacts(&file_path.to_string_lossy());
        return Err(AppError::Other("Download cancelled".into()));
    }
    write_manifest(&app, &manifest)?;
    artifacts.uncommitted_audio = None;

    emit_download_progress(
        &app,
        &track_id,
        "complete",
        None,
        Some(file_size),
        Some(file_size),
        total_bytes.or(Some(file_size)),
    );
    Ok(track)
}

/// 下载音频文件并保存到本地
// Tauri 命令签名由 IPC 契约决定：参数必须平铺，改成结构体会同时改掉前端调用点
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn download_track(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    track_id: String,
    title: String,
    artist: String,
    album: String,
    duration_ms: u64,
    cover_url: Option<String>,
    source: String,
    download_dir: Option<String>,
    name_template: Option<String>,
    stream_type: Option<YtStreamType>,
    expected_content_length: Option<u64>,
    expected_content_md5: Option<String>,
    youtube_video_id: Option<String>,
    youtube_quality: Option<String>,
) -> AppResult<()> {
    let app_handle = app.clone();
    let task_track_id = track_id.clone();
    let client = state.http();
    let hls_http = state.hls_transport();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let task_cancel_flag = cancel_flag.clone();
    register_download_task(&state.download_tasks, track_id, || {
        emit_download_progress(&app, &task_track_id, "start", None, None, None, None);
        let task_identity = task_cancel_flag.clone();
        let handle = tokio::spawn(async move {
            // 全局并发上限（对齐 Android MAX_DOWNLOAD_PARALLELISM=8）：批量下载时其余任务
            // 在此排队，避免数百并发流打崩带宽 / 触发平台风控（DL-8）
            let initial = recovery::Source {
                url,
                stream_type: stream_type.unwrap_or_default(),
                content_length: expected_content_length,
                content_md5: expected_content_md5,
            };
            let result = async {
                let _permit = recovery::cancellable(&task_cancel_flag, async {
                    download_semaphore()
                        .acquire()
                        .await
                        .map_err(|error| AppError::Other(error.to_string()))
                })
                .await?;
                let transfer = |candidate: recovery::Source| {
                    perform_download(
                        app_handle.clone(),
                        client.clone(),
                        task_cancel_flag.clone(),
                        candidate.url,
                        task_track_id.clone(),
                        title.clone(),
                        artist.clone(),
                        album.clone(),
                        duration_ms,
                        cover_url.clone(),
                        source.clone(),
                        download_dir.clone(),
                        name_template.clone(),
                        candidate.stream_type,
                        hls_http.clone(),
                        candidate.content_length,
                        candidate.content_md5,
                    )
                };
                if let Some(video_id) = youtube_video_id.filter(|_| source == "youtube") {
                    let quality = youtube_quality.unwrap_or_else(|| "high".into());
                    recovery::run(
                        initial,
                        task_cancel_flag.clone(),
                        Duration::from_secs(1),
                        |avoid_direct| {
                            refresh_youtube_download_source(
                                app_handle.clone(),
                                video_id.clone(),
                                quality.clone(),
                                avoid_direct,
                            )
                        },
                        transfer,
                    )
                    .await
                } else {
                    recovery::cancellable(&task_cancel_flag, transfer(initial)).await
                }
            }
            .await;

            if let Err(err) = result {
                let message = err.to_string();
                if message.to_lowercase().contains("cancelled")
                    || message.to_lowercase().contains("canceled")
                {
                    emit_download_progress(
                        &app_handle,
                        &task_track_id,
                        "cancelled",
                        None,
                        None,
                        None,
                        None,
                    );
                } else if !message.contains("Track already downloaded") {
                    emit_download_progress(
                        &app_handle,
                        &task_track_id,
                        "error",
                        Some(&message),
                        None,
                        None,
                        None,
                    );
                }
            }

            let _ = finish_download_task(
                &app_handle.state::<AppState>().download_tasks,
                &task_track_id,
                &task_identity,
            );
        });
        crate::state::DownloadTaskControl {
            cancel_flag,
            handle,
        }
    })
}

/// 列出所有已下载的曲目
#[tauri::command]
pub async fn list_downloads(app: AppHandle) -> AppResult<Vec<DownloadedTrack>> {
    Ok(validate_manifest_files(&app)?.tracks)
}

/// 校验下载清单，自动移除磁盘文件已不存在的记录
#[tauri::command]
pub async fn validate_downloads(app: AppHandle) -> AppResult<DownloadManifestValidation> {
    validate_manifest_files(&app)
}

/// 删除已下载的曲目（文件 + manifest 记录）
#[tauri::command]
pub async fn delete_download(app: AppHandle, track_id: String) -> AppResult<()> {
    let _manifest_guard = manifest_lock();
    let mut manifest = read_manifest(&app)?;

    // 查找并移除
    let idx = manifest.iter().position(|t| t.id == track_id);
    if let Some(i) = idx {
        let track = manifest.remove(i);
        // 删除磁盘文件 + 同名 sidecar（忽略错误，文件可能已被手动删除）
        remove_download_artifacts(&track.file_path);
        write_manifest(&app, &manifest)?;
    } else {
        return Err(AppError::NotFound("Download not found".into()));
    }

    Ok(())
}

/// 取消单个下载任务
#[tauri::command]
pub async fn cancel_download(
    app: AppHandle,
    state: State<'_, AppState>,
    track_id: String,
) -> AppResult<bool> {
    let tasks = state.download_tasks.lock();
    if let Some(control) = tasks.get(&track_id) {
        control.cancel_flag.store(true, Ordering::Relaxed);
        Ok(true)
    } else {
        let _ = app;
        Ok(false)
    }
}

/// 取消全部下载任务
#[tauri::command]
pub async fn cancel_all_downloads(app: AppHandle, state: State<'_, AppState>) -> AppResult<usize> {
    let tasks = state.download_tasks.lock();
    let cancelled = tasks.len();
    for (_track_id, control) in tasks.iter() {
        control.cancel_flag.store(true, Ordering::Relaxed);
    }
    let _ = app;
    Ok(cancelled)
}

/// 在系统文件管理器中显示文件
#[tauri::command]
pub async fn reveal_file(path: String) -> AppResult<()> {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Err(AppError::NotFound("File not found".into()));
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .args(["/select,", &path])
            .spawn()
            .map_err(|e| AppError::Other(e.to_string()))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &path])
            .spawn()
            .map_err(|e| AppError::Other(e.to_string()))?;
    }
    #[cfg(target_os = "linux")]
    {
        // xdg-open on parent directory
        if let Some(parent) = p.parent() {
            std::process::Command::new("xdg-open")
                .arg(parent)
                .spawn()
                .map_err(|e| AppError::Other(e.to_string()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn concurrent_download_registration_starts_only_one_job() {
        let registry = Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new()));
        let ready = Arc::new(std::sync::Barrier::new(2));
        let launched = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let runtime = tokio::runtime::Handle::current();
        let mut threads = Vec::new();
        for _ in 0..2 {
            let registry = registry.clone();
            let ready = ready.clone();
            let launched = launched.clone();
            let runtime = runtime.clone();
            threads.push(std::thread::spawn(move || {
                ready.wait();
                register_download_task(&registry, "youtube:same".into(), || {
                    launched.fetch_add(1, Ordering::AcqRel);
                    crate::state::DownloadTaskControl {
                        cancel_flag: Arc::new(AtomicBool::new(false)),
                        handle: runtime.spawn(std::future::pending()),
                    }
                })
            }));
        }
        let results: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(launched.load(Ordering::Acquire), 1);
        assert_eq!(registry.lock().len(), 1);
        let controls = std::mem::take(&mut *registry.lock());
        for control in controls.into_values() {
            control.handle.abort();
            let _ = control.handle.await;
        }
    }

    #[tokio::test]
    async fn stale_download_completion_cannot_remove_a_replacement_operation() {
        let registry = parking_lot::Mutex::new(std::collections::HashMap::new());
        let previous = Arc::new(AtomicBool::new(false));
        let replacement = Arc::new(AtomicBool::new(false));
        let handle = tokio::spawn(std::future::pending());
        registry.lock().insert(
            "youtube:same".into(),
            crate::state::DownloadTaskControl {
                cancel_flag: replacement.clone(),
                handle,
            },
        );
        assert!(finish_download_task(&registry, "youtube:same", &previous).is_none());
        assert!(Arc::ptr_eq(
            &registry.lock().get("youtube:same").unwrap().cancel_flag,
            &replacement
        ));
        let current = finish_download_task(&registry, "youtube:same", &replacement).unwrap();
        assert!(registry.lock().is_empty());
        current.handle.abort();
        let _ = current.handle.await;
    }

    #[tokio::test]
    async fn ranged_hls_http_errors_keep_their_download_recovery_policy() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = server.local_addr().unwrap();
        let serve = tokio::spawn(async move {
            for status in ["403 Forbidden", "410 Gone"] {
                let (mut socket, _) = server.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut chunk = [0u8; 1024];
                    let length = socket.read(&mut chunk).await.unwrap();
                    assert!(length > 0 && request.len() + length <= 8192);
                    request.extend_from_slice(&chunk[..length]);
                }
                assert!(String::from_utf8(request)
                    .unwrap()
                    .to_ascii_lowercase()
                    .contains("range: bytes=0-7"));
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for policy in [recovery::Retry::Playable, recovery::Retry::Refresh] {
            let response = client
                .get(format!("http://{address}/fragment"))
                .header(reqwest::header::RANGE, "bytes=0-7")
                .send()
                .await
                .unwrap();
            let error =
                crate::audio::hls::validate_response_status(response.status(), true).unwrap_err();
            assert_eq!(recovery::retry(&error), policy);
        }
        serve.await.unwrap();
        assert!(crate::audio::hls::validate_response_status(
            reqwest::StatusCode::PARTIAL_CONTENT,
            true
        )
        .is_ok());
        assert!(
            crate::audio::hls::validate_response_status(reqwest::StatusCode::OK, true)
                .unwrap_err()
                .to_string()
                .contains("server ignored byte range")
        );
    }

    #[tokio::test]
    async fn youtube_download_cancel_drops_inflight_transfer_and_owned_markers() {
        let root = tempfile::tempdir().unwrap();
        let part = root.path().join("cancel.aac.part");
        let reservation = root.path().join("cancel.aac.reserve");
        let other = root.path().join("unrelated.aac.part");
        std::fs::write(&other, b"keep").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let stop = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            flag.store(true, Ordering::Release);
        });
        let part_for_transfer = part.clone();
        let reserve_for_transfer = reservation.clone();
        let result: AppResult<()> = recovery::run(
            recovery::Source {
                url: "fixture:waiting".into(),
                stream_type: YtStreamType::Direct,
                content_length: None,
                content_md5: None,
            },
            cancel,
            Duration::ZERO,
            |_| async { panic!("cancelled transfer must not resolve") },
            move |_| {
                let part = part_for_transfer.clone();
                let reservation = reserve_for_transfer.clone();
                async move {
                    let file = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&part)
                        .unwrap();
                    std::fs::write(&reservation, b"").unwrap();
                    let artifacts = DownloadArtifacts {
                        part: Some(part),
                        reservations: vec![reservation],
                        uncommitted_audio: None,
                    };
                    let _: () = std::future::pending().await;
                    drop(file);
                    drop(artifacts);
                    Ok(())
                }
            },
        )
        .await;
        stop.await.unwrap();
        assert!(result.unwrap_err().to_string().contains("cancelled"));
        assert!(!part.exists());
        assert!(!reservation.exists());
        assert_eq!(std::fs::read(other).unwrap(), b"keep");
    }

    #[test]
    fn youtube_download_uncommitted_audio_is_removed_but_completed_file_survives() {
        let root = tempfile::tempdir().unwrap();
        let failed = root.path().join("failed.aac");
        let completed = root.path().join("completed.aac");
        std::fs::write(&failed, b"aac").unwrap();
        std::fs::write(&completed, b"aac").unwrap();
        {
            let _owned = DownloadArtifacts {
                uncommitted_audio: Some(failed.clone()),
                part: None,
                reservations: vec![],
            };
        }
        {
            let mut owned = DownloadArtifacts {
                uncommitted_audio: Some(completed.clone()),
                part: None,
                reservations: vec![],
            };
            owned.uncommitted_audio = None;
        }
        assert!(!failed.exists());
        assert!(completed.exists());
    }

    #[test]
    fn android_alignment_download_checks_source_md5_and_duration() {
        use md5::{Digest, Md5};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("audio.part");
        let bytes = include_bytes!("../audio/fixtures/hls-silence.aac");
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
        let bytes = include_bytes!("../audio/fixtures/hls-silence.aac");
        std::fs::write(&path, bytes).unwrap();
        assert!(validate_download_audio(&path, None, None, 0, &AtomicBool::new(true)).is_err());
    }

    /// 非法字符、控制字符替换为下划线，键盘可见字符原样保留
    #[test]
    fn sanitize_replaces_illegal_and_control_chars() {
        assert_eq!(
            sanitize_filename("a/b\\c:d*e?f\"g<h>i|j"),
            "a_b_c_d_e_f_g_h_i_j"
        );
        assert_eq!(
            sanitize_filename("bad\u{0}name\u{1f}\ttail"),
            "bad_name__tail"
        );
        assert_eq!(
            sanitize_filename("正常 - 歌名 (Live)"),
            "正常 - 歌名 (Live)"
        );
    }

    /// Windows 不允许结尾的点与空格
    #[test]
    fn sanitize_strips_trailing_dots_and_spaces() {
        assert_eq!(sanitize_filename("track name. . ."), "track name");
        assert_eq!(sanitize_filename("  spaced  "), "spaced");
    }

    /// Windows 设备保留名（含带扩展名的 stem）必须加前缀，大小写不敏感
    #[test]
    fn sanitize_prefixes_windows_reserved_names() {
        assert_eq!(sanitize_filename("CON"), "_CON");
        assert_eq!(sanitize_filename("con"), "_con");
        assert_eq!(sanitize_filename("Com1"), "_Com1");
        assert_eq!(sanitize_filename("NUL.mp3"), "_NUL.mp3");
        assert_eq!(sanitize_filename("LPT9.flac"), "_LPT9.flac");
        // 相似但不保留的名字不受影响
        assert_eq!(sanitize_filename("CONCERT"), "CONCERT");
        assert_eq!(sanitize_filename("COM10"), "COM10");
    }

    /// stem 超长时按字节截断到 180，且绝不切断多字节字符
    #[test]
    fn sanitize_truncates_stem_to_180_bytes_at_char_boundary() {
        let long_ascii = "a".repeat(400);
        assert_eq!(
            sanitize_filename(&long_ascii).len(),
            MAX_FILENAME_STEM_BYTES
        );

        // 中文 3 字节/字：180/3=60 字整除；用 61+ 字验证边界处理
        let long_cjk = "歌".repeat(100);
        let out = sanitize_filename(&long_cjk);
        assert!(out.len() <= MAX_FILENAME_STEM_BYTES);
        assert!(out.chars().all(|c| c == '歌'), "不得出现被切断的乱码字符");

        // 2 字节字符错位对齐：179 字节处落在字符中间也要安全回退
        let mixed = format!("{}é", "a".repeat(179));
        let out = sanitize_filename(&mixed);
        assert!(out.len() <= MAX_FILENAME_STEM_BYTES);
        assert!(out.is_char_boundary(out.len()));
    }

    /// 超 1 小时的下载标记会被清扫，新鲜标记与正常文件保留
    #[test]
    fn stale_download_markers_are_swept_but_fresh_ones_kept() {
        let dir = std::env::temp_dir().join(format!("neri-part-sweep-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let stale = dir.join("old.mp3.part");
        let stale_reserve = dir.join("old.flac.reserve");
        let fresh = dir.join("new.mp3.part");
        let fresh_reserve = dir.join("new.flac.reserve");
        let active_reserve = dir.join("active.m4a.reserve");
        let active_part = dir.join("active.m4a.part");
        let audio = dir.join("keep.mp3");
        std::fs::write(&stale, b"x").unwrap();
        std::fs::write(&stale_reserve, b"").unwrap();
        std::fs::write(&fresh, b"x").unwrap();
        std::fs::write(&fresh_reserve, b"").unwrap();
        std::fs::write(&active_reserve, b"").unwrap();
        std::fs::write(&active_part, b"x").unwrap();
        std::fs::write(&audio, b"x").unwrap();
        // 把 stale 的 mtime 拨回 2 小时前
        let old_time = std::time::SystemTime::now() - Duration::from_secs(7_200);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&stale)
            .unwrap();
        file.set_modified(old_time).unwrap();
        drop(file);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&active_reserve)
            .unwrap();
        file.set_modified(old_time).unwrap();
        drop(file);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&stale_reserve)
            .unwrap();
        file.set_modified(old_time).unwrap();
        drop(file);

        let removed = sweep_stale_download_markers([dir.clone()]);

        assert_eq!(removed, 2);
        assert!(!stale.exists());
        assert!(!stale_reserve.exists());
        assert!(fresh.exists());
        assert!(fresh_reserve.exists());
        assert!(active_reserve.exists());
        assert!(active_part.exists());
        assert!(audio.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sidecars_are_scoped_to_full_audio_name_when_stems_collide() {
        let dir = std::env::temp_dir().join(format!("neri-sidecar-scope-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let m4a = dir.join("Song.m4a");
        let flac = dir.join("Song.flac");
        std::fs::write(&m4a, b"audio").unwrap();
        std::fs::write(&flac, b"audio").unwrap();

        let m4a_sidecars = candidate_download_sidecars(&m4a);
        assert!(m4a_sidecars.contains(&dir.join("Song.m4a.lrc")));
        assert!(m4a_sidecars.contains(&dir.join("Song.m4a.jpg")));
        assert!(!m4a_sidecars.contains(&dir.join("Song.lrc")));
        assert!(!m4a_sidecars.contains(&dir.join("Song.flac.lrc")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reserve_download_path_is_atomic_and_collision_safe() {
        let dir = std::env::temp_dir().join(format!("neri-reserve-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let (first, first_reserve) = reserve_download_path(&dir, "Song", "m4a").unwrap();
        let (second, second_reserve) = reserve_download_path(&dir, "Song", "m4a").unwrap();
        assert_eq!(first, dir.join("Song.m4a"));
        assert_eq!(first_reserve, dir.join("Song.m4a.reserve"));
        assert_eq!(second, dir.join("Song (2).m4a"));
        assert_eq!(second_reserve, dir.join("Song (2).m4a.reserve"));
        assert!(first_reserve.is_file());
        assert!(second_reserve.is_file());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink("missing-target", dir.join("Song (3).m4a")).unwrap();
            let (third, _) = reserve_download_path(&dir, "Song", "m4a").unwrap();
            assert_eq!(third, dir.join("Song (4).m4a"));
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_size_mismatch_requires_a_known_expected_size() {
        assert!(!has_download_size_mismatch(0, 0));
        assert!(!has_download_size_mismatch(0, 12));
        assert!(!has_download_size_mismatch(12, 12));
        assert!(has_download_size_mismatch(12, 11));
        assert!(has_download_size_mismatch(12, 0));
    }
}
