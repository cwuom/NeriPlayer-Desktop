use crate::api::bilibili::client::{
    bili_quality_key, is_lossless_stream, BiliAudioStream, BiliClient,
};
use crate::api::netease::client::NeteasePlaybackUnavailableReason;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::settings::store::{self, AppSettings, SettingsLoadResult};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

/// 平台连通性探针
///
/// 语义只测传输层：收到任何 HTTP 响应（哪怕 401/业务错误码）都算连通，
/// 只有连接建立失败才算不通。此前探针调的是「取某个具体视频的播放地址」，
/// 视频下架/缺 cid/地区限制这类业务失败全被误报成「不能连通」。
/// 走与真实请求相同的 FallbackHttp，代理兜底的可达性也如实反映。
#[tauri::command]
pub async fn probe_platform_connectivity(
    state: State<'_, AppState>,
    platform: String,
) -> AppResult<bool> {
    // 轻量端点：不依赖特定内容存在、未登录也有响应
    let url = match platform.as_str() {
        "netease" => "https://music.163.com/api/pub/search/keyword/get",
        "bilibili" => "https://api.bilibili.com/x/web-interface/nav",
        "youtube" => "https://www.youtube.com/generate_204",
        other => {
            return Err(AppError::Api(format!("unknown probe platform: {other}")));
        }
    };
    let transport = state.transport("probe");
    match transport.send(|client| client.get(url)).await {
        Ok(_response) => Ok(true),
        Err(error) => {
            log::warn!(target: "probe", "{platform} unreachable: {error}");
            Ok(false)
        }
    }
}

#[tauri::command]
pub async fn get_settings(app: AppHandle) -> AppResult<SettingsLoadResult> {
    let loaded = store::load_settings(&app)?;
    apply_runtime_settings(&loaded.settings);
    Ok(loaded)
}

#[tauri::command]
pub async fn save_settings(app: AppHandle, settings: AppSettings) -> AppResult<AppSettings> {
    let previous_cache_limit = store::load_settings(&app)
        .ok()
        .map(|loaded| loaded.settings.max_cache_size);
    let saved = store::save_settings(&app, settings)?;
    apply_runtime_settings(&saved);
    // 调小缓存上限后立刻按新上限裁剪，而不是等下一首下载完成
    if previous_cache_limit.is_some_and(|previous| saved.max_cache_size < previous) {
        let app = app.clone();
        let limit = saved.max_cache_size;
        tauri::async_runtime::spawn_blocking(move || super::player_cmd::prune_media_caches(&app, limit));
    }
    Ok(saved)
}

/// 把需要后端立即生效的设置推到各子系统
pub fn apply_runtime_settings(settings: &AppSettings) {
    crate::api::youtube::set_locale_preferences(
        settings.internationalization_enabled,
        &settings.locale,
    );
}

const BACKGROUND_DIR: &str = "background";
const MAX_BACKGROUND_BYTES: u64 = 64 * 1024 * 1024;
const BACKGROUND_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif"];

fn background_dir(app: &AppHandle) -> AppResult<std::path::PathBuf> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|error| AppError::Other(error.to_string()))?
        .join(BACKGROUND_DIR))
}

/// 自定义背景复制进应用数据目录再使用：对话框只给原文件本次运行的访问权，
/// 资源作用域外（例如其他盘符）或之后被移动的原图，重启后都会显示不出来。
/// 已在受管目录里的路径原样返回，前端可在启动时对旧设置重复调用。
#[tauri::command]
pub async fn import_background_image(app: AppHandle, source: String) -> AppResult<String> {
    let dir = background_dir(&app)?;
    let source = std::path::PathBuf::from(source.trim());
    tauri::async_runtime::spawn_blocking(move || copy_background_into(&dir, &source))
        .await
        .map_err(|error| AppError::Other(error.to_string()))?
}

fn copy_background_into(dir: &std::path::Path, source: &std::path::Path) -> AppResult<String> {
    if source.parent() == Some(dir) && source.is_file() {
        return Ok(source.to_string_lossy().into_owned());
    }
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|value| BACKGROUND_EXTENSIONS.contains(&value.as_str()))
        .ok_or_else(|| AppError::Other("unsupported background image type".into()))?;
    let metadata = std::fs::metadata(source)?;
    if !metadata.is_file() || metadata.len() > MAX_BACKGROUND_BYTES {
        return Err(AppError::Other("background image is missing or too large".into()));
    }
    std::fs::create_dir_all(dir)?;
    let destination = dir.join(format!(
        "background-{}.{extension}",
        chrono::Utc::now().timestamp_millis()
    ));
    std::fs::copy(source, &destination)?;
    remove_backgrounds_except(dir, Some(&destination));
    Ok(destination.to_string_lossy().into_owned())
}

fn remove_backgrounds_except(dir: &std::path::Path, keep: Option<&std::path::Path>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if Some(path.as_path()) == keep || !path.is_file() {
            continue;
        }
        if let Err(error) = std::fs::remove_file(&path) {
            log::warn!(target: "settings", "删除旧背景图失败 {}: {error}", path.display());
        }
    }
}

/// 清除自定义背景时一并删除受管目录里的副本
#[tauri::command]
pub async fn clear_background_images(app: AppHandle) -> AppResult<()> {
    let dir = background_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || remove_backgrounds_except(&dir, None))
        .await
        .map_err(|error| AppError::Other(error.to_string()))
}

#[tauri::command]
pub async fn get_app_data_dir(app: tauri::AppHandle) -> AppResult<String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Other(e.to_string()))?;
    Ok(dir.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn get_log_dir() -> AppResult<String> {
    Ok(crate::logging::log_dir().to_string_lossy().to_string())
}

/// 获取网易云歌曲播放 URL
#[derive(Serialize)]
pub struct SongUrlResult {
    pub url: Option<String>,
    pub bitrate: u64,
    pub format: String,
    pub expected_content_length: Option<u64>,
    pub is_preview: bool,
    pub unavailable_reason: Option<NeteasePlaybackUnavailableReason>,
    pub level: Option<String>,
    pub expected_content_md5: Option<String>,
    pub duration_ms: Option<u64>,
    pub song_id: Option<u64>,
}

#[tauri::command]
pub async fn get_netease_song_url(
    song_id: u64,
    quality: String,
    request_generation: Option<u64>,
    state: State<'_, AppState>,
) -> AppResult<SongUrlResult> {
    // 快速切歌时，过期请求直接中止，释放连接池
    if let Some(gen) = request_generation {
        if state.playback_generation.load(std::sync::atomic::Ordering::Acquire) > gen {
            return Err(AppError::Audio("Playback request superseded".into()));
        }
    }
    let client = state.netease();
    let result = client.get_song_url(song_id, &quality).await?;
    Ok(SongUrlResult {
        url: result.url,
        bitrate: result.br,
        format: result.r#type,
        expected_content_length: (result.size > 0).then_some(result.size),
        is_preview: result.is_preview,
        unavailable_reason: result.unavailable_reason,
        level: result.level,
        expected_content_md5: result.content_md5,
        duration_ms: result.duration_ms,
        song_id: result.song_id,
    })
}

#[tauri::command]
pub async fn get_qq_song_url(
    song_mid: String,
    quality: String,
    request_generation: Option<u64>,
    state: State<'_, AppState>,
) -> AppResult<SongUrlResult> {
    if let Some(gen) = request_generation {
        if state.playback_generation.load(std::sync::atomic::Ordering::Acquire) > gen {
            return Err(AppError::Audio("Playback request superseded".into()));
        }
    }
    let client = state.qq();
    let result = client.get_song_url(&song_mid, &quality).await?;
    Ok(SongUrlResult {
        url: result.url,
        bitrate: result.bitrate,
        format: result.format,
        expected_content_length: None,
        is_preview: false,
        unavailable_reason: None,
        level: None,
        expected_content_md5: None,
        duration_ms: None,
        song_id: None,
    })
}

/// 获取B站音频流 URL
#[derive(Serialize)]
pub struct BiliAudioResult {
    pub url: String,
    pub bandwidth: u64,
    pub codecs: String,
    pub candidates: Vec<BiliAudioCandidate>,
    pub quality_key: String,
    pub mime_type: String,
}

#[derive(Serialize)]
pub struct BiliAudioCandidate {
    pub url: String,
    pub bandwidth: u64,
    pub codecs: String,
    pub quality_key: String,
    pub mime_type: String,
}

const LEGACY_BILI_ID_MULTIPLIER: u64 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LegacyBiliSongId {
    avid: u64,
    page: u64,
}

#[tauri::command]
pub async fn get_bili_audio_url(
    bvid: String,
    avid: Option<u64>,
    cid: Option<u64>,
    quality: Option<String>,
    request_generation: Option<u64>,
    state: State<'_, AppState>,
) -> AppResult<BiliAudioResult> {
    if let Some(gen) = request_generation {
        if state.playback_generation.load(std::sync::atomic::Ordering::Acquire) > gen {
            return Err(AppError::Audio("Playback request superseded".into()));
        }
    }
    let client = state.bilibili();

    // 视频信息取不到时带固定前缀，前端据此提示「暂时无法获取视频音频信息」（对齐 Android）
    let (real_bvid, real_cid) = resolve_bili_playback_target(&client, bvid, avid, cid)
        .await
        .map_err(|error| AppError::Api(format!("{BILI_VIDEO_INFO_UNAVAILABLE}: {error}")))?;

    let streams = client.get_audio_url(&real_bvid, real_cid).await?;
    let best = select_bili_audio_stream(streams.clone(), quality.as_deref())
        .ok_or_else(|| AppError::Api("No audio stream found".into()))?;
    let mut fallback_streams = vec![best.clone()];
    fallback_streams.extend(streams.into_iter().filter(|stream| stream.url != best.url));
    let candidates = build_bili_audio_candidates(&fallback_streams);
    let quality_key = bili_quality_key(&best).to_string();
    Ok(BiliAudioResult {
        url: best.url,
        bandwidth: best.bandwidth,
        codecs: best.codecs,
        candidates,
        quality_key,
        mime_type: best.mime_type,
    })
}

const BILI_VIDEO_INFO_UNAVAILABLE: &str = "Bilibili video info unavailable";

async fn resolve_bili_playback_target(
    client: &BiliClient,
    bvid: String,
    avid: Option<u64>,
    cid: Option<u64>,
) -> AppResult<(String, u64)> {
    if let Some(aid) = avid {
        return resolve_bili_numeric_source(client, aid, cid).await;
    }
    let info = client.get_video_info(&bvid).await?;
    if let Some(cid) = cid {
        if !info.pages.iter().any(|page| page.cid == cid) {
            return Err(AppError::Api(
                "Requested Bilibili part does not belong to this video".into(),
            ));
        }
    }
    Ok((bvid, cid.unwrap_or(info.cid)))
}

async fn resolve_bili_numeric_source(
    client: &BiliClient,
    aid: u64,
    cid: Option<u64>,
) -> AppResult<(String, u64)> {
    match client.get_video_info_by_avid(aid).await {
        Ok(info) if cid.is_none_or(|cid| info.pages.iter().any(|page| page.cid == cid)) => {
            Ok((info.bvid, cid.unwrap_or(info.cid)))
        }
        Ok(_) => {
            let legacy = split_legacy_bili_song_id(aid).ok_or_else(|| {
                AppError::Api("Requested Bilibili part does not belong to this video".into())
            })?;
            let info = client.get_video_info_by_avid(legacy.avid).await?;
            let preferred_cid = cid.expect("explicit cid was checked");
            if !info.pages.iter().any(|page| page.cid == preferred_cid) {
                return Err(AppError::Api(
                    "Requested Bilibili part does not belong to this video".into(),
                ));
            }
            Ok((info.bvid, preferred_cid))
        }
        Err(direct_error) => {
            let Some(legacy) = split_legacy_bili_song_id(aid) else {
                return Err(direct_error);
            };
            let Ok(info) = client.get_video_info_by_avid(legacy.avid).await else {
                return Err(direct_error);
            };
            let resolved_cid = match cid {
                Some(value) if info.pages.iter().any(|page| page.cid == value) => value,
                Some(_) => return Err(direct_error),
                None => match client.get_video_page_cid(&info.bvid, legacy.page).await {
                    Ok(Some(value)) => value,
                    _ => return Err(direct_error),
                },
            };
            Ok((info.bvid, resolved_cid))
        }
    }
}

fn split_legacy_bili_song_id(id: u64) -> Option<LegacyBiliSongId> {
    let avid = id / LEGACY_BILI_ID_MULTIPLIER;
    let page = id % LEGACY_BILI_ID_MULTIPLIER;
    (avid > 0 && page > 0).then_some(LegacyBiliSongId { avid, page })
}

fn select_bili_audio_stream(
    mut streams: Vec<BiliAudioStream>,
    quality: Option<&str>,
) -> Option<BiliAudioStream> {
    streams.sort_by_key(|stream| std::cmp::Reverse(stream.bandwidth));
    let regular: Vec<_> = streams
        .iter()
        .filter(|stream| stream.quality_tag.is_none())
        .collect();
    let ordered: Vec<_> = regular
        .iter()
        .copied()
        .chain(streams.iter().filter(|stream| stream.quality_tag.is_some()))
        .collect();
    let qualities = ["dolby", "hires", "lossless", "high", "medium", "low"];
    let preferred = quality.unwrap_or("high").trim().to_ascii_lowercase();
    let start = qualities
        .iter()
        .position(|key| *key == preferred)
        .unwrap_or(3);
    for key in &qualities[start..] {
        let selected = match *key {
            "dolby" => ordered
                .iter()
                .copied()
                .find(|stream| stream.quality_tag.as_deref() == Some("dolby")),
            "hires" => ordered
                .iter()
                .copied()
                .find(|stream| stream.quality_tag.as_deref() == Some("hires")),
            "lossless" => ordered
                .iter()
                .copied()
                .find(|stream| is_lossless_stream(stream))
                .or_else(|| {
                    regular
                        .iter()
                        .copied()
                        .find(|stream| (500_000..1_000_000).contains(&stream.bandwidth))
                }),
            "high" => regular
                .iter()
                .copied()
                .find(|stream| (180_000..500_000).contains(&stream.bandwidth)),
            "medium" => regular
                .iter()
                .copied()
                .find(|stream| (120_000..180_000).contains(&stream.bandwidth)),
            _ => regular
                .iter()
                .copied()
                .find(|stream| (60_000..120_000).contains(&stream.bandwidth)),
        };
        if let Some(stream) = selected {
            return Some(stream.clone());
        }
    }
    ordered.first().map(|stream| (**stream).clone())
}

fn build_bili_audio_candidates(streams: &[BiliAudioStream]) -> Vec<BiliAudioCandidate> {
    let mut candidates = Vec::new();
    for stream in streams {
        for url in std::iter::once(&stream.url).chain(stream.candidate_urls.iter()) {
            if !candidates
                .iter()
                .any(|candidate: &BiliAudioCandidate| candidate.url == *url)
            {
                candidates.push(BiliAudioCandidate {
                    url: url.clone(),
                    bandwidth: stream.bandwidth,
                    codecs: stream.codecs.clone(),
                    quality_key: bili_quality_key(stream).into(),
                    mime_type: stream.mime_type.clone(),
                });
            }
        }
    }
    candidates
}

/// 获取 YouTube 音频流 URL
#[derive(Serialize)]
pub struct YtAudioResult {
    pub url: String,
    pub bitrate: u64,
    pub mime_type: String,
    pub content_length: u64,
    pub stream_type: crate::api::youtube::client::YtStreamType,
}

#[tauri::command]
pub async fn get_youtube_audio_url(
    video_id: String,
    playback_source: Option<String>,
    force_refresh: Option<bool>,
    avoid_direct: Option<bool>,
    request_generation: Option<u64>,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Vec<YtAudioResult>> {
    // 快速切歌时丢弃过期解析, 与网易云/QQ/B站一致
    if let Some(gen) = request_generation {
        if state.playback_generation.load(std::sync::atomic::Ordering::Acquire) > gen {
            return Err(AppError::Audio("Playback request superseded".into()));
        }
    }
    // 平台取流层按客户端隔离匿名与登录请求，CDN 拉流不附带登录 Cookie
    let yt_auth = {
        let auth = state.auth.lock();
        auth.youtube.clone()
    };
    let playback_source = match playback_source {
        Some(source) => source,
        None => store::load_settings(&app)?.settings.youtube_playback_source,
    };
    let resolution = crate::api::youtube::playback::resolve_audio_streams_with_source(
        &video_id,
        yt_auth.as_ref().filter(|a| a.has_login()),
        Some(&app),
        force_refresh.unwrap_or(false),
        avoid_direct.unwrap_or(false),
        &playback_source,
    );
    let streams = await_current_playback_resolution(resolution, request_generation, &state.playback_generation).await?;
    if let Some(gen) = request_generation {
        if state.playback_generation.load(std::sync::atomic::Ordering::Acquire) > gen {
            return Err(AppError::Audio("Playback request superseded".into()));
        }
    }
    Ok(streams
        .into_iter()
        .map(|s| YtAudioResult {
            url: s.url,
            bitrate: s.bitrate,
            mime_type: s.mime_type,
            content_length: s.content_length,
            stream_type: s.stream_type,
        })
        .collect())
}

async fn await_current_playback_resolution<T>(
    resolution: impl std::future::Future<Output = AppResult<T>>,
    request_generation: Option<u64>,
    active_generation: &std::sync::atomic::AtomicU64,
) -> AppResult<T> {
    tokio::select! {
        result = resolution => result,
        _ = async {
            let Some(generation) = request_generation else {
                return std::future::pending::<()>().await;
            };
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                if active_generation.load(std::sync::atomic::Ordering::Acquire) > generation {
                    return;
                }
            }
        } => Err(AppError::Audio("Playback request superseded".into())),
    }
}

/// 将字节数据保存到本地文件（供前端封面保存等场景使用）
#[tauri::command]
pub async fn save_file_bytes(path: String, data: Vec<u8>) -> AppResult<()> {
    std::fs::write(&path, &data).map_err(|e| AppError::Other(e.to_string()))
}

/// 设置绕过代理（前端保存设置后通知后端重建 HTTP Client）
#[tauri::command]
pub async fn set_bypass_proxy(bypass: bool, state: State<'_, AppState>) -> AppResult<()> {
    state.rebuild_http(bypass);
    Ok(())
}

/// 获取构建信息
#[derive(Serialize)]
pub struct BuildInfo {
    /// package.json / Cargo.toml 语义版本，如 1.0.0
    pub app_version: String,
    /// 构建指纹 UUID，可用于对齐 CI 产物
    pub build_uuid: String,
    pub build_timestamp: String,
    /// 构建版本：短 git sha + 时区时间戳，如 361c08f.07140636
    pub version: String,
}

#[tauri::command]
pub async fn get_build_info() -> AppResult<BuildInfo> {
    Ok(BuildInfo {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        build_uuid: env!("BUILD_UUID").to_string(),
        build_timestamp: env!("BUILD_TIMESTAMP").to_string(),
        version: env!("BUILD_VERSION").to_string(),
    })
}

/// 读取系统强调色，供「自动跟随系统取色」使用，返回 "rgb(r, g, b)"。
/// Windows 从注册表 Explorer\Accent 读取 SystemAccentColor（ABGR，Win10 1903+），
/// 旧系统回退 AccentColorMenu；其余平台返回 None（由前端 CSS accent 关键字兜底）。
#[tauri::command]
pub fn get_system_accent_color() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let accent = hkcu
            .open_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Accent")
            .ok()?;
        let raw: u32 = accent
            .get_value("SystemAccentColor")
            .or_else(|_| accent.get_value("AccentColorMenu"))
            .ok()?;
        let r = raw & 0xFF;
        let g = (raw >> 8) & 0xFF;
        let b = (raw >> 16) & 0xFF;
        return Some(format!("rgb({r}, {g}, {b})"));
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::await_current_playback_resolution;
    use crate::error::AppResult;

    #[test]
    fn background_import_copies_into_managed_dir_and_keeps_one_copy() {
        let root = tempfile::tempdir().unwrap();
        let managed = root.path().join("background");
        let original = root.path().join("Wallpaper.JPG");
        std::fs::write(&original, b"jpeg").unwrap();

        let first = super::copy_background_into(&managed, &original).unwrap();
        let first = std::path::PathBuf::from(first);
        assert_eq!(first.parent(), Some(managed.as_path()));
        assert_eq!(first.extension().and_then(|value| value.to_str()), Some("jpg"));
        assert_eq!(std::fs::read(&first).unwrap(), b"jpeg");

        // 已受管的路径原样返回，不再复制
        let again = super::copy_background_into(&managed, &first).unwrap();
        assert_eq!(std::path::PathBuf::from(again), first);

        std::thread::sleep(std::time::Duration::from_millis(2));
        let other = root.path().join("next.png");
        std::fs::write(&other, b"png").unwrap();
        let second = std::path::PathBuf::from(super::copy_background_into(&managed, &other).unwrap());
        assert!(!first.exists(), "the previous copy is removed");
        assert_eq!(std::fs::read_dir(&managed).unwrap().count(), 1);
        assert!(second.exists());

        let text = root.path().join("notes.txt");
        std::fs::write(&text, b"x").unwrap();
        assert!(super::copy_background_into(&managed, &text).is_err());
    }

    #[tokio::test]
    async fn superseded_youtube_resolution_drops_its_pending_resources() {
        use std::sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}};
        struct DropFlag(Arc<AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) { self.0.store(true, Ordering::Release); }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let flag = DropFlag(dropped.clone());
        let active = AtomicU64::new(1);
        let resolution = async move {
            let _flag = flag;
            std::future::pending::<AppResult<()>>().await
        };
        let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            tokio::join!(await_current_playback_resolution(resolution, Some(1), &active), async {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                active.store(2, Ordering::Release);
            })
        }).await.expect("an old stream must stop without waiting for its network timeout");
        assert!(result.unwrap_err().to_string().contains("superseded"));
        assert!(dropped.load(Ordering::Acquire));
    }

    use super::{
        build_bili_audio_candidates, select_bili_audio_stream, split_legacy_bili_song_id,
        LegacyBiliSongId,
    };
    use crate::api::bilibili::client::BiliAudioStream;

    fn stream(url: &str, bandwidth: u64, codecs: &str, quality_id: u32) -> BiliAudioStream {
        BiliAudioStream {
            url: url.to_string(),
            bandwidth,
            codecs: codecs.to_string(),
            quality_id,
            mime_type: if codecs == "flac" {
                "audio/flac"
            } else {
                "audio/mp4"
            }
            .into(),
            quality_tag: match quality_id {
                30251 => Some("hires".into()),
                30250 => Some("dolby".into()),
                _ => None,
            },
            candidate_urls: vec![url.to_string()],
        }
    }

    #[test]
    fn bili_candidates_keep_all_fallback_streams() {
        let streams = vec![
            stream("https://audio/high", 320_000, "flac", 30251),
            stream("https://audio/medium", 192_000, "mp4a.40.2", 30280),
        ];

        let candidates = build_bili_audio_candidates(&streams);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].url, "https://audio/high");
        assert_eq!(candidates[1].bandwidth, 192_000);
    }

    #[test]
    fn bili_quality_falls_back_to_available_stream() {
        let streams = vec![stream("https://audio/high", 320_000, "flac", 30251)];

        let selected = select_bili_audio_stream(streams, Some("dolby"));
        assert_eq!(
            selected.map(|stream| stream.url),
            Some("https://audio/high".into())
        );
    }

    #[test]
    fn splits_android_legacy_bili_song_id() {
        assert_eq!(
            split_legacy_bili_song_id(12_345_678_900_007),
            Some(LegacyBiliSongId {
                avid: 1_234_567_890,
                page: 7,
            })
        );
        assert_eq!(split_legacy_bili_song_id(123_450_000), None);
    }

    #[test]
    fn android_alignment_bili_medium_uses_bitrate_band_instead_of_list_midpoint() {
        let streams = vec![
            stream("https://audio/high", 192_000, "mp4a.40.2", 30280),
            stream("https://audio/medium", 132_000, "mp4a.40.2", 30232),
            stream("https://audio/medium-low", 125_000, "mp4a.40.2", 30232),
            stream("https://audio/low", 64_000, "mp4a.40.2", 30216),
            stream("https://audio/lowest", 32_000, "mp4a.40.2", 30216),
        ];
        assert_eq!(
            select_bili_audio_stream(streams, Some("medium"))
                .unwrap()
                .url,
            "https://audio/medium"
        );
    }

    #[test]
    fn android_alignment_bili_dolby_missing_degrades_to_hires_before_regular_highest_bitrate() {
        let streams = vec![
            stream("https://audio/regular", 1_500_000, "mp4a.40.2", 30280),
            stream("https://audio/hires", 1_000_000, "flac", 30251),
        ];
        assert_eq!(
            select_bili_audio_stream(streams, Some("dolby"))
                .unwrap()
                .url,
            "https://audio/hires"
        );
    }

    #[test]
    fn android_alignment_bili_realistic_bitrates_follow_android_low_band() {
        let streams = vec![
            stream("https://audio/lowest", 48_000, "mp4a.40.2", 30216),
            stream("https://audio/medium", 92_000, "mp4a.40.2", 30232),
            stream("https://audio/high", 200_000, "mp4a.40.2", 30280),
        ];
        assert_eq!(
            select_bili_audio_stream(streams.clone(), Some("high"))
                .unwrap()
                .url,
            "https://audio/high"
        );
        assert_eq!(
            select_bili_audio_stream(streams.clone(), Some("medium"))
                .unwrap()
                .url,
            "https://audio/medium"
        );
        assert_eq!(
            select_bili_audio_stream(streams, Some("low")).unwrap().url,
            "https://audio/medium"
        );
    }

    #[test]
    fn android_alignment_bili_candidates_preserve_selected_cdn_metadata() {
        let mut selected = stream("https://audio/medium", 132_000, "mp4a.40.2", 30232);
        selected.candidate_urls.push("https://backup/medium".into());
        let lower = stream("https://audio/low", 64_000, "mp4a.40.2", 30216);
        let candidates = build_bili_audio_candidates(&[selected, lower]);
        assert_eq!(candidates.len(), 3);
        assert_eq!(candidates[1].url, "https://backup/medium");
        assert_eq!(candidates[1].quality_key, "medium");
        assert_eq!(candidates[1].bandwidth, 132_000);
        assert_eq!(candidates[2].quality_key, "low");
        assert_eq!(candidates[2].mime_type, "audio/mp4");
    }
}
