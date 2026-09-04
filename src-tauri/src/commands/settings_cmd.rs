use crate::api::bilibili::client::{BiliAudioStream, BiliClient};
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
    let saved = store::save_settings(&app, settings)?;
    apply_runtime_settings(&saved);
    Ok(saved)
}

/// 把需要后端立即生效的设置推到各子系统
pub fn apply_runtime_settings(settings: &AppSettings) {
    crate::api::youtube::set_locale_preferences(
        settings.internationalization_enabled,
        &settings.locale,
    );
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
    })
}

/// 获取B站音频流 URL
#[derive(Serialize)]
pub struct BiliAudioResult {
    pub url: String,
    pub bandwidth: u64,
    pub codecs: String,
    pub candidates: Vec<BiliAudioCandidate>,
}

#[derive(Serialize)]
pub struct BiliAudioCandidate {
    pub url: String,
    pub bandwidth: u64,
    pub codecs: String,
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

    // 确定 bvid 和 cid
    let (real_bvid, real_cid) = if let Some(aid) = avid {
        resolve_bili_numeric_source(&client, aid, cid).await?
    } else {
        let info = client.get_video_info(&bvid).await?;
        let c = cid.unwrap_or(info.cid);
        (bvid, c)
    };

    let streams = client.get_audio_url(&real_bvid, real_cid).await?;
    let candidates = build_bili_audio_candidates(&streams);
    let best = select_bili_audio_stream(streams, quality.as_deref())
        .ok_or_else(|| AppError::Api("No audio stream found".into()))?;
    Ok(BiliAudioResult {
        url: best.url,
        bandwidth: best.bandwidth,
        codecs: best.codecs,
        candidates,
    })
}

async fn resolve_bili_numeric_source(
    client: &BiliClient,
    aid: u64,
    cid: Option<u64>,
) -> AppResult<(String, u64)> {
    match client.get_video_info_by_avid(aid).await {
        Ok(info) => Ok((info.bvid, cid.unwrap_or(info.cid))),
        Err(direct_error) => {
            let Some(legacy) = split_legacy_bili_song_id(aid) else {
                return Err(direct_error);
            };
            let Ok(info) = client.get_video_info_by_avid(legacy.avid).await else {
                return Err(direct_error);
            };
            let resolved_cid = match cid {
                Some(value) => value,
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
    let fallback = streams.first().cloned();

    let is_dolby = |stream: &BiliAudioStream| {
        stream.quality_id == 30250 || stream.codecs.eq_ignore_ascii_case("ec-3")
    };
    let is_lossless = |stream: &BiliAudioStream| {
        stream.quality_id == 30251 || stream.codecs.eq_ignore_ascii_case("flac")
    };
    let normal_streams = || streams.iter().filter(|stream| !is_dolby(stream) && !is_lossless(stream));

    match quality.unwrap_or("high") {
        "dolby" => streams.iter().find(|stream| is_dolby(stream)).cloned(),
        "lossless" | "hires" => streams.iter().find(|stream| is_lossless(stream)).cloned(),
        "low" => normal_streams().next_back().cloned(),
        "medium" => {
            let normal: Vec<_> = normal_streams().cloned().collect();
            if normal.is_empty() {
                None
            } else {
                normal.get((normal.len() - 1) / 2).cloned()
            }
        }
        _ => normal_streams().next().cloned(),
    }
    .or(fallback)
}

fn build_bili_audio_candidates(streams: &[BiliAudioStream]) -> Vec<BiliAudioCandidate> {
    streams
        .iter()
        .map(|stream| BiliAudioCandidate {
            url: stream.url.clone(),
            bandwidth: stream.bandwidth,
            codecs: stream.codecs.clone(),
        })
        .collect()
}

/// 获取 YouTube 音频流 URL
#[derive(Serialize)]
pub struct YtAudioResult {
    pub url: String,
    pub bitrate: u64,
    pub mime_type: String,
    pub content_length: u64,
}

#[tauri::command]
pub async fn get_youtube_audio_url(
    video_id: String,
    request_generation: Option<u64>,
    state: State<'_, AppState>,
) -> AppResult<Vec<YtAudioResult>> {
    // 快速切歌时丢弃过期解析, 与网易云/QQ/B站一致
    if let Some(gen) = request_generation {
        if state.playback_generation.load(std::sync::atomic::Ordering::Acquire) > gen {
            return Err(AppError::Audio("Playback request superseded".into()));
        }
    }
    // player 请求在已登录时附带 Cookie (不附 SAPISID*HASH; mobile+hash 会 400);
    // 客户端仍用 IOS/ANDROID/ANDROID_MUSIC/TVHTML5 (非 WEB_REMIX 完整浏览器), 降低互踢
    // googlevideo CDN 拉流不附带登录 Cookie (对齐 Android stream headers)
    let yt_auth = {
        let auth = state.auth.lock();
        auth.youtube.clone()
    };
    let streams = crate::api::youtube::playback::resolve_audio_streams(
        &video_id,
        yt_auth.as_ref().filter(|a| a.has_login()),
    )
    .await?;
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
        })
        .collect())
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
    use super::{
        build_bili_audio_candidates,
        select_bili_audio_stream,
        split_legacy_bili_song_id,
        LegacyBiliSongId,
    };
    use crate::api::bilibili::client::BiliAudioStream;

    fn stream(url: &str, bandwidth: u64, codecs: &str, quality_id: u32) -> BiliAudioStream {
        BiliAudioStream {
            url: url.to_string(),
            bandwidth,
            codecs: codecs.to_string(),
            quality_id,
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
        assert_eq!(selected.map(|stream| stream.url), Some("https://audio/high".into()));
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
}
