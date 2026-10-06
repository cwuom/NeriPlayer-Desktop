// YouTube Music 播放客户端
// 客户端顺序、认证隔离与 challenge 处理参考 Android YouTubeMusicPlaybackRepository
// 匿名优先 VISIONOS/ANDROID_VR，登录优先 WEB_REMIX；CDN 拉流不携带登录 Cookie
// EJS 在受限 QuickJS 中处理 sig/n，WebPo 在独立匿名窗口中获取 GVS token
// 音频容器优先 AAC/mp4，保持与桌面解码器能力一致
use std::sync::Mutex;
use std::time::Duration;

use reqwest::Client;
use serde_json::{json, Value};
use tauri::Manager;

use crate::api::transport::FallbackHttp;
use crate::auth::state::YouTubeAuth;
use crate::error::{AppError, AppResult};

use super::bootstrap::PlaybackBootstrap;
use super::client::YtAudioStream;
use std::sync::LazyLock;
static AUDIO_STREAM_CACHE: LazyLock<Mutex<super::cache::AudioStreamCache>> =
    LazyLock::new(|| Mutex::new(super::cache::AudioStreamCache::default()));


const PO_TOKEN_FAST_PATH_WAIT: Duration = Duration::from_millis(150);

struct BootstrapAttempt<T> {
    value: Option<Result<T, ()>>,
}

impl<T> Default for BootstrapAttempt<T> {
    fn default() -> Self {
        Self { value: None }
    }
}

impl<T> BootstrapAttempt<T> {
    async fn get_or_fetch<F, Fut>(&mut self, fetch: F) -> Option<&T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = AppResult<T>>,
    {
        if self.value.is_none() {
            self.value = Some(fetch().await.map_err(|_| ()));
        }
        self.value.as_ref().and_then(|value| value.as_ref().ok())
    }
}

struct PoTokenAcquisition {
    handle: Option<tokio::task::JoinHandle<Option<String>>>,
    token: Option<String>,
    checked_fast_path: bool,
}

impl PoTokenAcquisition {
    fn new(future: impl std::future::Future<Output = Option<String>> + Send + 'static) -> Self {
        Self {
            handle: Some(tokio::spawn(future)),
            token: None,
            checked_fast_path: false,
        }
    }

    async fn fast_token(&mut self) -> Option<String> {
        let Some(handle) = self.handle.as_mut() else {
            return self.token.clone();
        };
        if self.checked_fast_path && !handle.is_finished() {
            return None;
        }
        self.checked_fast_path = true;
        if let Ok(result) = tokio::time::timeout(PO_TOKEN_FAST_PATH_WAIT, handle).await {
            self.token = result.ok().flatten();
            self.handle = None;
        }
        self.token.clone()
    }

    async fn finish(&mut self) -> Option<String> {
        if let Some(handle) = self.handle.as_mut() {
            self.token = match tokio::time::timeout(Duration::from_secs(65), &mut *handle).await {
                Ok(result) => result.ok().flatten(),
                Err(_) => {
                    handle.abort();
                    None
                }
            };
            self.handle = None;
        }
        self.token.clone()
    }
}

impl Drop for PoTokenAcquisition {
    fn drop(&mut self) {
        if let Some(handle) = &self.handle {
            handle.abort();
        }
    }
}

struct TokenPendingStreams {
    streams: Vec<YtAudioStream>,
    manifest: Option<String>,
    duration_ms: u64,
}

// 桌面播放端点: 非 WEB_REMIX 客户端统一走 www, 降低与 music 登录会话的关联
const PLAYER_URL_WWW: &str = "https://www.youtube.com/youtubei/v1/player";
const PLAYER_URL_MUSIC: &str = "https://music.youtube.com/youtubei/v1/player";
const ORIGIN_WWW: &str = "https://www.youtube.com";
const ORIGIN_MUSIC: &str = "https://music.youtube.com";

// googlevideo CDN 拉流 User-Agent (对齐 Android resolveYouTubeStreamUserAgent).
// CDN 会校验拉流 UA 与 stream URL 中 `c=` 客户端参数一致, 不一致直接 403.
// 因此拉流侧必须按铸造该直链的客户端选择匹配 UA, 而非固定 Chrome UA.
// 注意: 下列 UA 字符串须与 playback_client_profiles() 中对应 client_version 同步.
const STREAM_ANDROID_VR_USER_AGENT: &str =
    "com.google.android.apps.youtube.vr.oculus/1.65.10 (Linux; U; Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip";
const STREAM_IOS_USER_AGENT: &str =
    "com.google.ios.youtube/20.10.4 (iPhone16,2; U; CPU iOS 18_3_2 like Mac OS X;)";
const STREAM_ANDROID_USER_AGENT: &str =
    "com.google.android.youtube/20.10.38 (Linux; U; Android 15) gzip";
const STREAM_ANDROID_MUSIC_USER_AGENT: &str =
    "com.google.android.apps.youtube.music/8.32.52 (Linux; U; Android 15) gzip";
// WEB / TV 及未知客户端回退到桌面 Chrome UA
const STREAM_WEB_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36";
const STREAM_VISIONOS_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";

#[derive(Debug, Clone, Copy)]
enum PlayerHost {
    Www,
    Music,
}

#[derive(Debug, Clone)]
struct PlayerClientProfile {
    client_id: &'static str,
    client_name: &'static str,
    client_version: &'static str,
    user_agent: &'static str,
    platform: &'static str,
    /// 兜底 locale；实际请求以 innertube_locale() 为准，随国际化开关变化
    #[allow(dead_code)]
    hl: &'static str,
    #[allow(dead_code)]
    gl: &'static str,
    host: PlayerHost,
    android_sdk_version: Option<i32>,
    os_name: Option<&'static str>,
    os_version: Option<&'static str>,
    device_make: Option<&'static str>,
    device_model: Option<&'static str>,
    // 部分移动客户端依赖 playerParams 才吐 plain url
    player_params: Option<&'static str>,
    // ANDROID_VR 等 jsless 客户端缺 visitorData 会被 bot check 拦下
    requires_visitor_data: bool,
    supports_authenticated_context: bool,
    requires_po_token: bool,
}

// 客户端版本和自动顺序取自 Android YouTubePlayerRequestComposer
fn playback_client_profiles() -> &'static [PlayerClientProfile] {
    &[
        PlayerClientProfile {
            client_id: "101", client_name: "VISIONOS", client_version: "0.1",
            user_agent: STREAM_VISIONOS_USER_AGENT, platform: "MOBILE", hl: "en", gl: "US",
            host: PlayerHost::Www, android_sdk_version: None, os_name: Some("visionOS"),
            os_version: Some("1.3.21O771"), device_make: Some("Apple"), device_model: Some("RealityDevice14,1"),
            player_params: None, requires_visitor_data: false, supports_authenticated_context: false,
            requires_po_token: false,
        },
        PlayerClientProfile {
            client_id: "28", client_name: "ANDROID_VR", client_version: "1.65.10",
            user_agent: STREAM_ANDROID_VR_USER_AGENT, platform: "MOBILE", hl: "en", gl: "US",
            host: PlayerHost::Www, android_sdk_version: Some(32), os_name: Some("Android"),
            os_version: Some("12L"), device_make: Some("Oculus"), device_model: Some("Quest 3"),
            player_params: None, requires_visitor_data: true, supports_authenticated_context: false,
            requires_po_token: false,
        },
        PlayerClientProfile {
            client_id: "67", client_name: "WEB_REMIX", client_version: "1.20260403.09.00",
            user_agent: STREAM_WEB_USER_AGENT, platform: "DESKTOP", hl: "en", gl: "US",
            host: PlayerHost::Music, android_sdk_version: None, os_name: Some("Windows"),
            os_version: Some("10.0"), device_make: None, device_model: None,
            player_params: None, requires_visitor_data: false, supports_authenticated_context: true,
            requires_po_token: true,
        },
        PlayerClientProfile {
            client_id: "7", client_name: "TVHTML5", client_version: "7.20260114.12.00",
            user_agent: "Mozilla/5.0 (ChromiumStylePlatform) Cobalt/25.lts.30.1034943-gold (unlike Gecko), Unknown_TV_Unknown_0/Unknown (Unknown, Unknown)",
            platform: "TV", hl: "en", gl: "US", host: PlayerHost::Www, android_sdk_version: None,
            os_name: None, os_version: None, device_make: None, device_model: None,
            player_params: None, requires_visitor_data: false, supports_authenticated_context: true,
            requires_po_token: true,
        },
        PlayerClientProfile {
            client_id: "62", client_name: "WEB_CREATOR", client_version: "1.20260114.05.00",
            user_agent: STREAM_WEB_USER_AGENT, platform: "DESKTOP", hl: "en", gl: "US",
            host: PlayerHost::Www, android_sdk_version: None, os_name: Some("Windows"),
            os_version: Some("10.0"), device_make: None, device_model: None,
            player_params: None, requires_visitor_data: false, supports_authenticated_context: true,
            requires_po_token: true,
        },
        PlayerClientProfile {
            client_id: "7", client_name: "TVHTML5", client_version: "5.20260114",
            user_agent: "Mozilla/5.0 (ChromiumStylePlatform) Cobalt/Version",
            platform: "TV", hl: "en", gl: "US", host: PlayerHost::Www, android_sdk_version: None,
            os_name: None, os_version: None, device_make: None, device_model: None,
            player_params: None, requires_visitor_data: false, supports_authenticated_context: true,
            requires_po_token: true,
        },
    ]
}

fn ordered_profiles(authenticated: bool) -> Vec<&'static PlayerClientProfile> {
    let profiles = playback_client_profiles();
    let indices = if authenticated {
        [2, 3, 4, 5, 0, 1]
    } else {
        [0, 1, 2, 3, 4, 5]
    };
    indices.into_iter().map(|index| &profiles[index]).collect()
}


pub fn normalize_playback_source(source: &str) -> &'static str {
    match source.trim().to_ascii_lowercase().as_str() {
        "visionos" => "visionos",
        "android_vr" => "android_vr",
        "web_remix" => "web_remix",
        "tv_html5" => "tv_html5",
        "web_creator" => "web_creator",
        _ => "automatic",
    }
}

fn ordered_profiles_with_source(
    authenticated: bool,
    source: &str,
) -> Vec<&'static PlayerClientProfile> {
    let preferred = match normalize_playback_source(source) {
        "visionos" => Some("VISIONOS"),
        "android_vr" => Some("ANDROID_VR"),
        "web_remix" => Some("WEB_REMIX"),
        "tv_html5" => Some("TVHTML5"),
        "web_creator" => Some("WEB_CREATOR"),
        _ => None,
    };
    let mut profiles = ordered_profiles(authenticated);
    if let Some(index) = preferred.and_then(|client| {
        profiles
            .iter()
            .position(|profile| profile.client_name == client)
    }) {
        let preferred = profiles.remove(index);
        profiles.insert(0, preferred);
    }
    profiles
}

fn player_endpoint(profile: &PlayerClientProfile) -> (&'static str, &'static str) {
    match profile.host {
        PlayerHost::Www => (PLAYER_URL_WWW, ORIGIN_WWW),
        PlayerHost::Music => (PLAYER_URL_MUSIC, ORIGIN_MUSIC),
    }
}

fn build_player_context(profile: &PlayerClientProfile, visitor_data: Option<&str>) -> Value {
    // 播放侧同样跟随国际化开关，否则库列表和可播曲目的区域会不一致
    let locale = super::innertube_locale();
    let mut client = json!({
        "clientName": profile.client_name,
        "clientVersion": profile.client_version,
        "hl": locale.0,
        "gl": locale.1,
        "platform": profile.platform,
        "clientScreen": "WATCH",
        "utcOffsetMinutes": chrono::Local::now().offset().local_minus_utc() / 60
    });
    if matches!(profile.client_name, "ANDROID_VR" | "TVHTML5") {
        client["userAgent"] = json!(format!("{},gzip(gfe)", profile.user_agent));
    }

    if let Some(sdk) = profile.android_sdk_version {
        client["androidSdkVersion"] = json!(sdk);
    }
    if let Some(os_name) = profile.os_name {
        client["osName"] = json!(os_name);
    }
    if let Some(os_version) = profile.os_version {
        client["osVersion"] = json!(os_version);
    }
    if let Some(device_make) = profile.device_make {
        client["deviceMake"] = json!(device_make);
    }
    if let Some(device_model) = profile.device_model {
        client["deviceModel"] = json!(device_model);
    }
    // ANDROID_VR 等 jsless 客户端: visitorData 是绕过 bot check 的关键
    if let Some(vd) = visitor_data.map(str::trim).filter(|s| !s.is_empty()) {
        client["visitorData"] = json!(vd);
    }

    json!({
        "client": client,
        "request": { "useSsl": true, "internalExperimentFlags": [], "consistencyTokenJars": [] },
        "user": { "lockedSafetyMode": false }
    })
}

fn build_player_body(
    profile: &PlayerClientProfile,
    video_id: &str,
    visitor_data: Option<&str>,
) -> Value {
    let mut body = json!({
        "context": build_player_context(profile, visitor_data),
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true,
        // 对齐 Android/桌面: 声明 HTML5 偏好, 提高 progressive 直链概率
        "playbackContext": {
            "contentPlaybackContext": {
                "html5Preference": "HTML5_PREF_WANTS",
                "lactMilliseconds": "9",
                "autonavState": "STATE_OFF",
                "autoCaptionsDefaultOn": false,
                "mdxContext": {},
                "vis": 10
            },
            "devicePlaybackCapabilities": {"supportsVp9Encoding": true, "supportXhr": true}
        }
    });

    if let Some(params) = profile.player_params {
        body["params"] = json!(params);
    }

    body
}

fn authenticated_player_body(
    profile: &PlayerClientProfile,
    video_id: &str,
    bootstrap: &PlaybackBootstrap,
) -> Value {
    let mut body = build_player_body(profile, video_id, Some(&bootstrap.visitor_data));
    if profile.supports_authenticated_context {
        if let Some(timestamp) = bootstrap.signature_timestamp {
            body["playbackContext"]["contentPlaybackContext"]["signatureTimestamp"] =
                json!(timestamp);
        }
    }
    if profile.client_name == "WEB_REMIX" {
        let client = &mut body["context"]["client"];
        client["clientVersion"] = json!(bootstrap.client_version);
        client["clientScreen"] = json!("WATCH_FULL_SCREEN");
        client["userAgent"] = json!(format!("{},gzip(gfe)", profile.user_agent));
        client["browserName"] = json!("Chrome");
        client["browserVersion"] = json!("146.0.0.0");
        client["originalUrl"] = json!(format!("{ORIGIN_MUSIC}/"));
        client["clientFormFactor"] = json!("UNKNOWN_FORM_FACTOR");
        client["playerType"] = json!("UNIPLAYER");
        client["userInterfaceTheme"] = json!("USER_INTERFACE_THEME_LIGHT");
        client["connectionType"] = json!("CONN_CELLULAR_4G");
        client["screenWidthPoints"] = json!(771);
        client["screenHeightPoints"] = json!(897);
        client["screenPixelDensity"] = json!(1);
        client["screenDensityFloat"] = json!(1.375);
        client["tvAppInfo"] = json!({"livingRoomAppMode": "LIVING_ROOM_APP_MODE_UNSPECIFIED"});
        client["deviceMake"] = json!("");
        client["deviceModel"] = json!("");
        client["acceptHeader"] = json!("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7");
        if let Ok(time_zone) = iana_time_zone::get_timezone() {
            client["timeZone"] = json!(time_zone);
        }
        client["configInfo"] = if bootstrap.config_info.is_object() {
            bootstrap.config_info.clone()
        } else {
            json!({})
        };
        for (key, value) in [
            ("rolloutToken", &bootstrap.rollout_token),
            ("deviceExperimentId", &bootstrap.device_experiment_id),
            ("remoteHost", &bootstrap.remote_host),
        ] {
            if !value.is_empty() {
                client[key] = json!(value);
            }
        }
        body["context"]["clientScreenNonce"] = json!(request_nonce());
        body["context"]["clickTracking"] = json!({"clickTrackingParams": ""});
        body["context"]["adSignalsInfo"] = web_remix_ad_signals();
        body["playbackContext"]["contentPlaybackContext"]["referer"] =
            json!(format!("{ORIGIN_MUSIC}/"));
        body["cpn"] = json!(request_nonce());
        body["captionParams"] = json!({});
        body["playlistId"] = json!(format!("RDAMVM{video_id}"));
    }
    body
}

fn web_remix_ad_signals() -> Value {
    let params = [
        ("dt", now_ms().to_string()),
        ("flash", "0".into()),
        ("frm", "0".into()),
        (
            "u_tz",
            (chrono::Local::now().offset().local_minus_utc() / 60).to_string(),
        ),
        ("u_his", "5".into()),
        ("u_h", "1152".into()),
        ("u_w", "2048".into()),
        ("u_ah", "1104".into()),
        ("u_aw", "2048".into()),
        ("u_cd", "32".into()),
        ("bc", "31".into()),
        ("bih", "897".into()),
        ("biw", "757".into()),
        ("brdim", "0,0,0,0,2048,0,2048,1104,771,897".into()),
        ("vis", "1".into()),
        ("wgl", "true".into()),
        ("ca_type", "image".into()),
    ];
    json!({"params": params.into_iter().map(|(key, value)| json!({"key": key, "value": value})).collect::<Vec<_>>()})
}

fn request_nonce() -> String {
    use rand::Rng;
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut random = rand::thread_rng();
    (0..16)
        .map(|_| ALPHABET[random.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

fn parse_query_map(query: &str) -> std::collections::HashMap<String, String> {
    url::form_urlencoded::parse(query.as_bytes())
        .filter(|(key, _)| !key.is_empty())
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

/// 从 stream URL 的 `c=` 客户端参数推导拉流 User-Agent.
/// googlevideo CDN 校验拉流 UA 与铸造该直链的客户端一致, 否则 HTTP 403.
/// 对齐 Android resolveYouTubeStreamUserAgent: IOS/ANDROID/... 各用对应 app UA, 其余回退 Web.
pub fn stream_user_agent_for_url(url: &str) -> &'static str {
    let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    let client = parse_query_map(query)
        .get("c")
        .map(|s| s.trim().to_ascii_uppercase())
        .unwrap_or_default();
    match client.as_str() {
        "VISIONOS" => STREAM_VISIONOS_USER_AGENT,
        "IOS" => STREAM_IOS_USER_AGENT,
        "ANDROID" | "ANDROID_TESTSUITE" => STREAM_ANDROID_USER_AGENT,
        "ANDROID_MUSIC" => STREAM_ANDROID_MUSIC_USER_AGENT,
        "ANDROID_VR" => STREAM_ANDROID_VR_USER_AGENT,
        _ => STREAM_WEB_USER_AGENT,
    }
}

fn append_query_param(url: &str, key: &str, value: &str) -> String {
    let encoded_key = urlencoding::encode(key);
    let encoded_value = urlencoding::encode(value);
    if url.contains('?') {
        format!("{url}&{encoded_key}={encoded_value}")
    } else {
        format!("{url}?{encoded_key}={encoded_value}")
    }
}

/// 解析 format 的可播 URL
/// 支持 plain `url`, 以及已带 sig 的 signatureCipher;
/// 若仅有加密 `s` 且无解签器, 则跳过该 format (由其他客户端回退)
pub fn resolve_format_url(format: &Value) -> Option<String> {
    if let Some(url) = format.get("url").and_then(|v| v.as_str()).map(str::trim) {
        if !url.is_empty() {
            return Some(url.to_string());
        }
    }

    let cipher = format
        .get("signatureCipher")
        .and_then(|v| v.as_str())
        .or_else(|| format.get("cipher").and_then(|v| v.as_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty())?;

    let params = parse_query_map(cipher);
    let url = params.get("url").map(String::as_str).unwrap_or("").trim();
    if url.is_empty() {
        return None;
    }

    if let Some(signature) = params
        .get("sig")
        .or_else(|| params.get("signature"))
        .map(String::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let sp = params.get("sp").map(String::as_str).unwrap_or("sig").trim();
        return Some(append_query_param(url, sp, signature));
    }

    // 需要 player JS 解签的 `s=` 路径: 当前阶段跳过, 等待其它客户端给出 plain url
    if params.contains_key("s") {
        return None;
    }

    Some(url.to_string())
}

fn collect_format_arrays(resp: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(arr) = resp
        .pointer("/streamingData/adaptiveFormats")
        .and_then(|v| v.as_array())
    {
        out.extend(arr.iter().cloned());
    }
    // progressive formats 作为兜底 (部分 TV/IOS 客户端主要吐这里)
    if let Some(arr) = resp
        .pointer("/streamingData/formats")
        .and_then(|v| v.as_array())
    {
        out.extend(arr.iter().cloned());
    }
    out
}


fn response_needs_po_token(response: &Value, avoid_direct: bool) -> bool {
    let mut formats = collect_format_arrays(response);
    if formats.iter().any(is_audio_like) {
        formats.retain(is_audio_like);
    }
    let manifest_needs_token = response
        .pointer("/streamingData/hlsManifestUrl")
        .and_then(Value::as_str)
        .is_some_and(|url| {
            super::hls::is_trusted_hls_url(url) && !super::hls::has_manifest_token(url)
        });
    manifest_needs_token
        || (!avoid_direct
            && formats.iter().any(|format| {
                if let Some(url) = resolve_format_url(format) {
                    trusted_stream_url(&url) && !super::hls::has_manifest_token(&url)
                } else {
                    format
                        .get("signatureCipher")
                        .or_else(|| format.get("cipher"))
                        .and_then(Value::as_str)
                        .is_some_and(|cipher| !cipher.is_empty())
                }
            }))
}

fn is_audio_like(format: &Value) -> bool {
    let mime = format
        .get("mimeType")
        .and_then(|m| m.as_str())
        .unwrap_or("");
    if mime.starts_with("audio/") {
        return true;
    }
    // progressive 可能是 muxed, 仅在无独立 audio 时由调用方兜底
    false
}

fn format_to_stream(format: &Value) -> Option<YtAudioStream> {
    let url = resolve_format_url(format)?;
    Some(YtAudioStream {
        url,
        stream_type: super::client::YtStreamType::Direct,
        bitrate: format.get("bitrate").and_then(|v| v.as_u64()).unwrap_or(0),
        mime_type: format
            .get("mimeType")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        content_length: format
            .get("contentLength")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .or_else(|| format.get("contentLength").and_then(|v| v.as_u64()))
            .unwrap_or(0),
    })
}

fn extract_audio_streams(resp: &Value) -> Vec<YtAudioStream> {
    let formats = collect_format_arrays(resp);

    let mut streams: Vec<YtAudioStream> = formats
        .iter()
        .filter(|f| is_audio_like(f))
        .filter_map(format_to_stream)
        .collect();

    // 若完全没有 audio/* 直链, 尝试 progressive muxed 里可解析 url 的条目
    // (少数 TV 响应只有 muxed; cpal/symphonia 可解常见容器中的音轨)
    if streams.is_empty() {
        streams = formats
            .iter()
            .filter(|f| {
                f.get("mimeType")
                    .and_then(|m| m.as_str())
                    .map(|m| m.starts_with("video/") || m.starts_with("audio/"))
                    .unwrap_or(false)
            })
            .filter_map(format_to_stream)
            .collect();
    }

    streams.sort_by(|a, b| {
        // 优先 audio/mp4 (AAC, symphonia 可解) > 其它 audio/* > muxed
        // webm/opus 当前未启 symphonia opus feature, 会 unsupported codec
        let a_score = mime_playback_score(&a.mime_type);
        let b_score = mime_playback_score(&b.mime_type);
        b_score
            .cmp(&a_score)
            .then(b.bitrate.cmp(&a.bitrate))
            .then(b.content_length.cmp(&a.content_length))
    });
    streams
}

fn mime_playback_score(mime: &str) -> u8 {
    let base = mime
        .split(';')
        .next()
        .unwrap_or(mime)
        .trim()
        .to_ascii_lowercase();
    if base.starts_with("audio/mp4") || base == "audio/m4a" || base == "audio/aac" {
        3
    } else if base.starts_with("audio/") {
        2
    } else if base.starts_with("video/") {
        1
    } else {
        0
    }
}

fn playability_summary(resp: &Value) -> (String, String, String) {
    let status = resp
        .pointer("/playabilityStatus/status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let reason = resp
        .pointer("/playabilityStatus/reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let subreason = resp
        .pointer("/playabilityStatus/errorScreen/playerErrorMessageRenderer/subreason/simpleText")
        .and_then(|v| v.as_str())
        .or_else(|| {
            resp.pointer(
                "/playabilityStatus/errorScreen/playerErrorMessageRenderer/reason/simpleText",
            )
            .and_then(|v| v.as_str())
        })
        .unwrap_or("")
        .to_string();
    (status, reason, subreason)
}

fn build_playback_client(no_proxy: bool) -> AppResult<Client> {
    let mut builder = Client::builder()
        // 默认 UA 仅作兜底; 实际请求按 profile 覆盖
        .user_agent("com.google.ios.youtube/20.10.4 (iPhone16,2; U; CPU iOS 18_3_2 like Mac OS X;)")
        // 不启用 cookie_store: 登录 Cookie 仅在 player 请求上显式附带,
        // CDN 拉流路径不会被 jar 自动污染
        .cookie_store(false)
        // API 与 CDN 地址均已规范化，禁止带 token 的 Location 跨来源跟随
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20));
    if no_proxy {
        builder = builder.no_proxy();
    }
    builder
        .build()
        .map_err(|e| AppError::Other(format!("build youtube playback client: {e}")))
}

fn proxy_order(bypass_proxy: bool) -> [bool; 2] {
    [bypass_proxy, !bypass_proxy]
}

// 与主应用保持相同代理优先级，独立客户端避免自动携带账号 Cookie
fn playback_http_client(bypass_proxy: bool) -> AppResult<FallbackHttp> {
    let [primary, fallback] = proxy_order(bypass_proxy);
    Ok(FallbackHttp::with_fallback(
        &build_playback_client(primary)?,
        &build_playback_client(fallback)?,
        "youtube-playback",
    ))
}

fn build_player_request(
    client: &Client,
    profile: &PlayerClientProfile,
    video_id: &str,
    bootstrap: &PlaybackBootstrap,
) -> reqwest::RequestBuilder {
    let body = authenticated_player_body(profile, video_id, bootstrap);
    let (endpoint, origin) = player_endpoint(profile);
    let mut url = url::Url::parse(endpoint).expect("static player URL");
    url.query_pairs_mut()
        .append_pair("prettyPrint", "false")
        .append_pair("key", &bootstrap.api_key);
    if profile.client_name != "WEB_REMIX" {
        url.query_pairs_mut().append_pair("id", video_id);
    }
    let host = if matches!(profile.host, PlayerHost::Music) {
        "music.youtube.com"
    } else {
        "www.youtube.com"
    };
    let cookie_values = super::account::select_cookie_values(&bootstrap.cookies, host);
    let cookie_header = cookie_values
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("; ");
    let authorization = if profile.supports_authenticated_context && bootstrap.logged_in {
        crate::auth::youtube_hash::build_youtube_authorization(
            cookie_values.get("SAPISID").map(String::as_str),
            cookie_values.get("__Secure-1PAPISID").map(String::as_str),
            cookie_values.get("__Secure-3PAPISID").map(String::as_str),
            cookie_values.get("APISID").map(String::as_str),
            origin,
            &bootstrap.user_session_id,
        )
    } else {
        None
    };
    let version = if profile.client_name == "WEB_REMIX" {
        bootstrap.client_version.as_str()
    } else {
        profile.client_version
    };
    let referer = if profile.client_name == "WEB_REMIX" {
        format!("{origin}/watch?v={video_id}&list=RDAMVM{video_id}")
    } else {
        format!("{origin}/")
    };
    let mut request = client
        .post(url.clone())
        .header("User-Agent", profile.user_agent)
        .header("Content-Type", "application/json")
        .header(
            "Accept-Language",
            format!("{},en;q=0.8", super::innertube_locale().0),
        )
        .header("Origin", origin)
        .header("Referer", &referer)
        .header("X-YouTube-Client-Name", profile.client_id)
        .header("X-YouTube-Client-Version", version);
    if !bootstrap.visitor_data.is_empty() {
        request = request.header("X-Goog-Visitor-Id", &bootstrap.visitor_data);
    }
    if profile.client_name != "WEB_REMIX" {
        request = request.header("X-Goog-Api-Format-Version", "2");
    }
    if profile.supports_authenticated_context {
        request = request.header("X-Goog-AuthUser", &bootstrap.session_index);
        if !cookie_header.is_empty() {
            request = request.header("Cookie", &cookie_header);
        }
        if profile.client_name == "WEB_REMIX" {
            request = request.header(
                "X-YouTube-Bootstrap-Logged-In",
                bootstrap.logged_in.to_string(),
            );
        }
        if let Some(value) = &authorization {
            request = request
                .header("Authorization", value)
                .header("X-Origin", origin);
        }
        if bootstrap.logged_in {
            request = request.header("X-YouTube-Bootstrap-Logged-In", "true");
        }
        if profile.client_name == "TVHTML5" && !bootstrap.delegated_session_id.is_empty() {
            request = request.header("X-Goog-PageId", &bootstrap.delegated_session_id);
        }
    }
    request.json(&body)
}

async fn player_request(
    http: &FallbackHttp,
    profile: &PlayerClientProfile,
    video_id: &str,
    bootstrap: &PlaybackBootstrap,
) -> AppResult<Value> {
    let response = http
        .send(|client| build_player_request(client, profile, video_id, bootstrap))
        .await?;
    crate::api::transport::parse_json_response(
        response,
        &format!("youtube player {}", profile.client_name),
    )
    .await
}

fn stream_cache_key(video_id: &str, auth: Option<&YouTubeAuth>) -> String {
    let locale = super::innertube_locale();
    format!(
        "{}|{}|{}|{}",
        video_id,
        super::bootstrap::auth_fingerprint(auth),
        locale.0,
        locale.1
    )
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn trusted_stream_url(raw: &str) -> bool {
    let Ok(url) = url::Url::parse(raw) else {
        return false;
    };
    let host = url.host_str().unwrap_or_default();
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && (host == "googlevideo.com"
            || host.ends_with(".googlevideo.com")
            || host == "youtube.com"
            || host.ends_with(".youtube.com"))
}

fn replace_query(url: &str, key: &str, value: &str) -> Option<String> {
    let mut url = url::Url::parse(url).ok()?;
    let pairs = url
        .query_pairs()
        .filter(|(name, _)| name != key)
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    url.set_query(None);
    url.query_pairs_mut()
        .extend_pairs(pairs)
        .append_pair(key, value);
    Some(url.into())
}

async fn resolve_response_streams(
    http: &FallbackHttp,
    response: &Value,
    bootstrap: &PlaybackBootstrap,
) -> AppResult<Vec<YtAudioStream>> {
    let (audio_formats, mut muxed_formats): (Vec<_>, Vec<_>) = collect_format_arrays(response)
        .into_iter()
        .partition(is_audio_like);
    muxed_formats.retain(|format| {
        format
            .get("mimeType")
            .and_then(Value::as_str)
            .is_some_and(|mime| mime.starts_with("video/"))
    });
    // 成功音频不处理视频 challenge，音频不可用时保留 progressive 兜底
    match resolve_format_streams(http, audio_formats, bootstrap).await {
        Ok(streams) if !streams.is_empty() => return Ok(streams),
        Err(error) if muxed_formats.is_empty() => return Err(error),
        _ => {}
    }
    resolve_format_streams(http, muxed_formats, bootstrap).await
}

async fn resolve_format_streams(
    http: &FallbackHttp,
    formats: Vec<Value>,
    bootstrap: &PlaybackBootstrap,
) -> AppResult<Vec<YtAudioStream>> {
    let mut signatures = Vec::new();
    let mut throttling = Vec::new();
    for format in &formats {
        let cipher = format
            .get("signatureCipher")
            .or_else(|| format.get("cipher"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = parse_query_map(cipher);
        if let Some(signature) = params.get("s").filter(|value| !value.is_empty()) {
            if !signatures.contains(signature) {
                signatures.push(signature.clone());
            }
        }
        let raw = format
            .get("url")
            .and_then(Value::as_str)
            .or_else(|| params.get("url").map(String::as_str))
            .unwrap_or_default();
        if let Some(value) = url::Url::parse(raw).ok().and_then(|url| {
            url.query_pairs()
                .find(|(key, _)| key == "n")
                .map(|(_, value)| value.into_owned())
        }) {
            if !throttling.contains(&value) {
                throttling.push(value);
            }
        }
    }
    let solutions =
        super::challenge::solve(http, &bootstrap.player_js_url, signatures, throttling).await?;
    let mut resolved = Vec::new();
    for mut format in formats {
        let cipher = format
            .get("signatureCipher")
            .or_else(|| format.get("cipher"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = parse_query_map(cipher);
        let mut raw = resolve_format_url(&format);
        if raw.is_none() {
            if let (Some(url), Some(signature)) = (
                params.get("url"),
                params
                    .get("s")
                    .and_then(|value| solutions.signatures.get(value)),
            ) {
                raw = replace_query(
                    url,
                    params
                        .get("sp")
                        .filter(|value| !value.is_empty())
                        .map(String::as_str)
                        .unwrap_or("sig"),
                    signature,
                );
            }
        }
        let Some(mut raw) = raw.filter(|url| trusted_stream_url(url)) else {
            continue;
        };
        let n = url::Url::parse(&raw).ok().and_then(|url| {
            url.query_pairs()
                .find(|(key, _)| key == "n")
                .map(|(_, value)| value.into_owned())
        });
        if let Some(n) = n {
            let Some(value) = solutions.throttling.get(&n) else {
                continue;
            };
            let Some(url) = replace_query(&raw, "n", value) else {
                continue;
            };
            raw = url;
        }
        format["url"] = json!(raw);
        resolved.push(format);
    }
    Ok(extract_audio_streams(
        &json!({"streamingData": {"adaptiveFormats": resolved}}),
    ))
}

pub async fn resolve_audio_streams(
    video_id: &str,
    auth: Option<&YouTubeAuth>,
) -> AppResult<Vec<YtAudioStream>> {
    resolve_audio_streams_with_app(video_id, auth, None, false).await
}

pub async fn resolve_audio_streams_with_app(
    video_id: &str,
    auth: Option<&YouTubeAuth>,
    app: Option<&tauri::AppHandle>,
    force_refresh: bool,
) -> AppResult<Vec<YtAudioStream>> {
    resolve_audio_streams_with_strategy(video_id, auth, app, force_refresh, false).await
}


pub async fn resolve_audio_streams_with_strategy(
    video_id: &str,
    auth: Option<&YouTubeAuth>,
    app: Option<&tauri::AppHandle>,
    force_refresh: bool,
    avoid_direct: bool,
) -> AppResult<Vec<YtAudioStream>> {
    resolve_audio_streams_with_source(
        video_id,
        auth,
        app,
        force_refresh,
        avoid_direct,
        "automatic",
    )
    .await
}

pub async fn resolve_audio_streams_with_source(
    video_id: &str,
    auth: Option<&YouTubeAuth>,
    app: Option<&tauri::AppHandle>,
    force_refresh: bool,
    avoid_direct: bool,
    playback_source: &str,
) -> AppResult<Vec<YtAudioStream>> {
    let video_id = video_id.trim();
    if video_id.is_empty()
        || video_id.len() > 128
        || !video_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(AppError::Api("Invalid YouTube video id".into()));
    }
    let playback_source = normalize_playback_source(playback_source);
    let cache_key = format!(
        "{}|hls={avoid_direct}|source={playback_source}",
        stream_cache_key(video_id, auth)
    );
    if let Ok(mut cache) = AUDIO_STREAM_CACHE.lock() {
        if force_refresh {
            cache.remove(&cache_key);
        } else if let Some(streams) = cache.get(&cache_key, now_ms()) {
            return Ok(streams);
        }
    }
    let bypass_proxy = app
        .and_then(|app| app.try_state::<crate::state::AppState>())
        .map(|state| state.bypasses_system_proxy())
        .unwrap_or(true);
    let http = playback_http_client(bypass_proxy)?;
    let logged_in = auth.is_some_and(YouTubeAuth::has_login);
    let mut errors = Vec::new();
    let mut authenticated_bootstrap = BootstrapAttempt::default();
    let mut anonymous_bootstrap = BootstrapAttempt::default();
    let mut hls_fallback = Vec::new();
    let mut token_acquisition: Option<PoTokenAcquisition> = None;
    let mut token_pending_streams = Vec::new();
    for profile in ordered_profiles_with_source(logged_in, playback_source) {
        let bootstrap_slot = if profile.supports_authenticated_context {
            &mut authenticated_bootstrap
        } else {
            &mut anonymous_bootstrap
        };
        let bootstrap = bootstrap_slot
            .get_or_fetch(|| {
                super::bootstrap::fetch(
                    &http,
                    if profile.supports_authenticated_context {
                        auth
                    } else {
                        None
                    },
                    profile.supports_authenticated_context,
                    // 流级重试复用有效配置，首页过期由自身 TTL 和登录指纹判定
                    false,
                )
            })
            .await;
        let Some(mut bootstrap) = bootstrap.cloned() else {
            errors.push(format!("{}:bootstrap_failed", profile.client_name));
            continue;
        };
        if profile.supports_authenticated_context
            && bootstrap.signature_timestamp.is_none()
            && !bootstrap.player_js_url.is_empty()
        {
            if let Ok(script) =
                super::challenge::player_script(&http, &bootstrap.player_js_url).await
            {
                bootstrap.signature_timestamp = super::bootstrap::timestamp_from_script(&script);
            }
        }
        if profile.requires_visitor_data && bootstrap.visitor_data.is_empty() {
            errors.push(format!("{}:missing_visitor", profile.client_name));
            continue;
        }
        let response = match player_request(&http, profile, video_id, &bootstrap).await {
            Ok(value) => value,
            Err(_) => {
                errors.push(format!("{}:request_failed", profile.client_name));
                continue;
            }
        };
        let (status, _, _) = playability_summary(&response);
        if status != "OK" {
            errors.push(format!("{}:{}", profile.client_name, status));
            continue;
        }
        let manifest = response
            .pointer("/streamingData/hlsManifestUrl")
            .and_then(Value::as_str)
            .filter(|url| super::hls::is_trusted_hls_url(url));
        let mut requires_token =
            profile.requires_po_token && response_needs_po_token(&response, avoid_direct);
        let start_token_acquisition = || {
            let app = app?.clone();
            let session_auth = YouTubeAuth {
                cookies: bootstrap.cookies.clone(),
                nickname: None,
                avatar_url: None,
            };
            let video_id = video_id.to_owned();
            let visitor_data = bootstrap.visitor_data.clone();
            let remote_host = bootstrap.remote_host.clone();
            Some(PoTokenAcquisition::new(async move {
                super::web_po::mint(
                    &app,
                    Some(&session_auth),
                    &super::bootstrap::auth_fingerprint(Some(&session_auth)),
                    &video_id,
                    &visitor_data,
                    &remote_host,
                    force_refresh,
                )
                .await
                .ok()
            }))
        };
        if requires_token && token_acquisition.is_none() {
            token_acquisition = start_token_acquisition();
        }
        // 令牌预取与签名解析并行，短等待后让其它客户端接管
        let mut streams = if avoid_direct {
            Vec::new()
        } else {
            match resolve_response_streams(&http, &response, &bootstrap).await {
                Ok(value) => value,
                Err(_) => {
                    errors.push(format!("{}:challenge_failed", profile.client_name));
                    Vec::new()
                }
            }
        };
        // progressive 兜底可能在原始音频格式之外，需要沿用相同令牌校验
        requires_token |= profile.requires_po_token
            && streams
                .iter()
                .any(|stream| !super::hls::has_manifest_token(&stream.url));
        if requires_token && token_acquisition.is_none() {
            token_acquisition = start_token_acquisition();
        }
        let acquired_token = if requires_token {
            if let Some(acquisition) = token_acquisition.as_mut() {
                acquisition.fast_token().await
            } else {
                None
            }
        } else {
            None
        };
        let duration_ms = response
            .pointer("/videoDetails/lengthSeconds")
            .and_then(|value| {
                value
                    .as_str()
                    .and_then(|value| value.parse::<u64>().ok())
                    .or_else(|| value.as_u64())
            })
            .unwrap_or(0)
            .saturating_mul(1000);
        if requires_token && acquired_token.is_none() && token_acquisition.is_some() {
            token_pending_streams.push(TokenPendingStreams {
                streams: streams
                    .iter()
                    .filter(|stream| !super::hls::has_manifest_token(&stream.url))
                    .cloned()
                    .collect(),
                manifest: manifest
                    .filter(|url| !super::hls::has_manifest_token(url))
                    .map(str::to_owned),
                duration_ms,
            });
        }
        if profile.requires_po_token {
            if let Some(token) = &acquired_token {
                for stream in &mut streams {
                    if !super::hls::has_manifest_token(&stream.url) {
                        if let Some(url) = replace_query(&stream.url, "pot", token) {
                            stream.url = url;
                        }
                    }
                }
            }
            streams.retain(|stream| super::hls::has_manifest_token(&stream.url));
        }
        if !avoid_direct && !streams.is_empty() {
            if let Ok(mut cache) = AUDIO_STREAM_CACHE.lock() {
                cache.put(cache_key, streams.clone(), now_ms());
            }
            return Ok(streams);
        }
        if let Some(manifest) = manifest {
            let manifest = if profile.requires_po_token && !super::hls::has_manifest_token(manifest)
            {
                acquired_token
                    .as_ref()
                    .and_then(|token| super::hls::append_manifest_token(manifest, token))
            } else {
                Some(manifest.into())
            };
            if let Some(manifest) = manifest {
                if let Ok(candidates) =
                    super::hls::fetch_audio_playlists(&http, &manifest, duration_ms).await
                {
                    for candidate in candidates {
                        if !hls_fallback
                            .iter()
                            .any(|stream: &YtAudioStream| stream.url == candidate.url)
                        {
                            hls_fallback.push(candidate);
                        }
                    }
                }
            }
        }
        errors.push(format!("{}:no_playable_audio", profile.client_name));
    }
    // 所有其它来源都失败时复用预取结果，不重新请求 player 或重新铸造
    if hls_fallback.is_empty() && !token_pending_streams.is_empty() {
        if let Some(acquisition) = token_acquisition.as_mut() {
            if let Some(token) = acquisition.finish().await {
                for pending in token_pending_streams {
                    if !avoid_direct {
                        let streams = pending
                            .streams
                            .into_iter()
                            .filter_map(|mut stream| {
                                stream.url = replace_query(&stream.url, "pot", &token)?;
                                Some(stream)
                            })
                            .collect::<Vec<_>>();
                        if !streams.is_empty() {
                            if let Ok(mut cache) = AUDIO_STREAM_CACHE.lock() {
                                cache.put(cache_key, streams.clone(), now_ms());
                            }
                            return Ok(streams);
                        }
                    }
                    if let Some(manifest) = pending
                        .manifest
                        .and_then(|manifest| super::hls::append_manifest_token(&manifest, &token))
                    {
                        if let Ok(candidates) =
                            super::hls::fetch_audio_playlists(&http, &manifest, pending.duration_ms)
                                .await
                        {
                            hls_fallback.extend(candidates);
                        }
                    }
                }
            }
        }
    }
    if !hls_fallback.is_empty() {
        hls_fallback.sort_by_key(|stream| std::cmp::Reverse(stream.bitrate));
        if let Ok(mut cache) = AUDIO_STREAM_CACHE.lock() {
            cache.put(cache_key, hls_fallback.clone(), now_ms());
        }
        return Ok(hls_fallback);
    }
    Err(AppError::Api(format!(
        "YouTube playback failed: {}",
        errors.join(" | ")
    )))
}

#[cfg(test)]
mod tests {
    use super::{
        append_query_param, build_playback_client, build_player_body, build_player_request,
        collect_format_arrays, extract_audio_streams, ordered_profiles, parse_query_map,
        playback_client_profiles, replace_query, resolve_format_url, stream_user_agent_for_url,
        trusted_stream_url, STREAM_ANDROID_MUSIC_USER_AGENT, STREAM_ANDROID_USER_AGENT,
        STREAM_ANDROID_VR_USER_AGENT, STREAM_IOS_USER_AGENT, STREAM_WEB_USER_AGENT,
    };
    use serde_json::json;


    #[test]
    fn selected_source_is_first_without_losing_android_fallback_order() {
        for authenticated in [false, true] {
            for (source, client) in [
                ("visionos", "VISIONOS"),
                ("android_vr", "ANDROID_VR"),
                ("web_remix", "WEB_REMIX"),
                ("tv_html5", "TVHTML5"),
                ("web_creator", "WEB_CREATOR"),
            ] {
                let profiles = super::ordered_profiles_with_source(authenticated, source);
                assert_eq!(profiles[0].client_name, client);
                assert_eq!(profiles.len(), playback_client_profiles().len());
                let mut expected = ordered_profiles(authenticated);
                let selected = expected
                    .iter()
                    .position(|profile| profile.client_name == client)
                    .unwrap();
                let selected = expected.remove(selected);
                expected.insert(0, selected);
                assert_eq!(
                    profiles
                        .iter()
                        .map(|profile| profile.client_version)
                        .collect::<Vec<_>>(),
                    expected
                        .iter()
                        .map(|profile| profile.client_version)
                        .collect::<Vec<_>>()
                );
            }
        }
        assert_eq!(super::normalize_playback_source("unknown"), "automatic");
        assert_eq!(super::normalize_playback_source(" WEB_REMIX "), "web_remix");
    }

    #[tokio::test]
    async fn video_challenges_do_not_delay_an_already_playable_audio_format() {
        let bootstrap: super::PlaybackBootstrap = serde_json::from_value(json!({
            "api_key":"fixture", "client_version":"fixture", "visitor_data":"",
            "player_js_url":"https://untrusted.invalid/player.js", "signature_timestamp":null,
            "session_index":"0", "user_session_id":"", "delegated_session_id":"", "logged_in":false
        }))
        .unwrap();
        let client = build_playback_client(true).unwrap();
        let http = crate::api::transport::FallbackHttp::new(&client, "fixture");
        let response = json!({"streamingData":{"adaptiveFormats":[
            {"mimeType":"video/mp4", "signatureCipher":"url=https%3A%2F%2Frr.googlevideo.com%2Fvideo&s=irrelevant-video-challenge"},
            {"mimeType":"audio/mp4", "url":"https://rr.googlevideo.com/audio?c=VISIONOS", "bitrate":160000}
        ]}});
        let streams = super::resolve_response_streams(&http, &response, &bootstrap)
            .await
            .unwrap();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].bitrate, 160000);
    }

    #[tokio::test]
    async fn unusable_audio_formats_preserve_progressive_fallback() {
        let bootstrap: super::PlaybackBootstrap = serde_json::from_value(json!({
            "api_key":"fixture", "client_version":"fixture", "visitor_data":"",
            "player_js_url":"https://untrusted.invalid/player.js", "signature_timestamp":null,
            "session_index":"0", "user_session_id":"", "delegated_session_id":"", "logged_in":false
        }))
        .unwrap();
        let client = build_playback_client(true).unwrap();
        let http = crate::api::transport::FallbackHttp::new(&client, "fixture");
        for audio in [
            json!({"mimeType":"audio/mp4", "bitrate":160000}),
            json!({"mimeType":"audio/mp4", "signatureCipher":"url=https%3A%2F%2Frr.googlevideo.com%2Faudio&s=unavailable-audio-challenge"}),
        ] {
            let response = json!({"streamingData":{
                "adaptiveFormats":[audio],
                "formats":[{"mimeType":"video/mp4", "url":"https://rr.googlevideo.com/progressive?c=VISIONOS", "bitrate":128000}]
            }});
            let streams = super::resolve_response_streams(&http, &response, &bootstrap)
                .await
                .unwrap();
            assert_eq!(streams.len(), 1);
            assert_eq!(streams[0].mime_type, "video/mp4");
            assert_eq!(streams[0].bitrate, 128000);
            assert!(streams[0].url.contains("/progressive"));
        }
    }

    #[tokio::test]
    async fn failed_bootstrap_is_not_retried_for_each_client_in_the_same_resolve() {
        let mut attempt = super::BootstrapAttempt::<usize>::default();
        let mut hits = 0;
        for _ in 0..4 {
            let value = attempt
                .get_or_fetch(|| async {
                    hits += 1;
                    Err(crate::error::AppError::Api("fixture unavailable".into()))
                })
                .await;
            assert!(value.is_none());
        }
        assert_eq!(hits, 1);
    }

    #[tokio::test]
    async fn slow_po_token_yields_to_fallback_and_is_reused_without_a_second_mint() {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let mut acquisition = super::PoTokenAcquisition::new(async { receiver.await.ok() });
        let started = std::time::Instant::now();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), acquisition.fast_token())
                .await
                .unwrap()
                .is_none()
        );
        assert!(started.elapsed() >= super::PO_TOKEN_FAST_PATH_WAIT);
        assert!(acquisition.fast_token().await.is_none());
        sender.send("fixture-token".to_string()).unwrap();
        assert_eq!(acquisition.finish().await.as_deref(), Some("fixture-token"));
        assert_eq!(
            acquisition.fast_token().await.as_deref(),
            Some("fixture-token")
        );
    }

    #[tokio::test]
    async fn pending_po_token_is_cancelled_when_a_faster_client_wins() {
        struct CancelGuard(Option<tokio::sync::oneshot::Sender<()>>);
        impl Drop for CancelGuard {
            fn drop(&mut self) {
                if let Some(sender) = self.0.take() {
                    let _ = sender.send(());
                }
            }
        }
        let (started_sender, started_receiver) = tokio::sync::oneshot::channel();
        let (cancelled_sender, cancelled_receiver) = tokio::sync::oneshot::channel();
        let acquisition = super::PoTokenAcquisition::new(async move {
            let _guard = CancelGuard(Some(cancelled_sender));
            let _ = started_sender.send(());
            std::future::pending::<Option<String>>().await
        });
        started_receiver.await.unwrap();
        drop(acquisition);
        tokio::time::timeout(std::time::Duration::from_secs(1), cancelled_receiver)
            .await
            .unwrap()
            .unwrap();
    }

    #[test]
    fn http_proxy_priority_respects_the_user_preference() {
        assert_eq!(super::proxy_order(true), [true, false]);
        assert_eq!(super::proxy_order(false), [false, true]);
    }

    #[test]
    fn actual_player_requests_match_android_headers_context_and_cookie_isolation() {
        use crate::api::youtube::bootstrap::PlaybackBootstrap;
        use crate::auth::state::CookieEntry;
        let mut bootstrap: PlaybackBootstrap = serde_json::from_value(json!({
            "api_key": "fixture-key", "client_version": "dynamic-version", "visitor_data": "fixture-visitor",
            "player_js_url": "", "signature_timestamp": 12345, "session_index": "2",
            "user_session_id": "fixture-user", "delegated_session_id": "fixture-page", "logged_in": true,
            "remote_host": "fixture-remote", "config_info": {"appInstallData": "fixture-install"},
            "rollout_token": "fixture-rollout", "device_experiment_id": "fixture-device"
        })).unwrap();
        bootstrap.cookies = vec![
            CookieEntry {
                name: "SAPISID".into(),
                value: "fixture-secret".into(),
                domain: ".youtube.com".into(),
            },
            CookieEntry {
                name: "music-only".into(),
                value: "fixture".into(),
                domain: "music.youtube.com".into(),
            },
        ];
        let client = build_playback_client(true).unwrap();
        for profile in playback_client_profiles() {
            let request = build_player_request(&client, profile, "fixture-video", &bootstrap)
                .build()
                .unwrap();
            let headers = request.headers();
            let body: serde_json::Value =
                serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
            assert_eq!(headers["x-youtube-client-name"], profile.client_id);
            assert_eq!(headers["x-goog-visitor-id"], "fixture-visitor");
            assert_eq!(body["videoId"], "fixture-video");
            assert_eq!(
                request
                    .url()
                    .query_pairs()
                    .find(|(key, _)| key == "key")
                    .unwrap()
                    .1,
                "fixture-key"
            );
            if !profile.supports_authenticated_context {
                assert!(!headers.contains_key("cookie"));
                assert!(!headers.contains_key("authorization"));
                assert!(!headers.contains_key("x-goog-authuser"));
                assert!(
                    body["playbackContext"]["contentPlaybackContext"]["signatureTimestamp"]
                        .is_null()
                );
            } else {
                assert!(headers["authorization"]
                    .to_str()
                    .unwrap()
                    .contains("SAPISIDHASH"));
                assert_eq!(headers["x-goog-authuser"], "2");
                assert_eq!(
                    body["playbackContext"]["contentPlaybackContext"]["signatureTimestamp"],
                    12345
                );
            }
            if profile.client_name == "WEB_REMIX" {
                assert_eq!(request.url().host_str(), Some("music.youtube.com"));
                assert!(headers["cookie"]
                    .to_str()
                    .unwrap()
                    .contains("music-only=fixture"));
                assert_eq!(headers["x-youtube-client-version"], "dynamic-version");
                assert_eq!(
                    headers["referer"],
                    "https://music.youtube.com/watch?v=fixture-video&list=RDAMVMfixture-video"
                );
                assert_eq!(
                    body["context"]["client"]["configInfo"]["appInstallData"],
                    "fixture-install"
                );
                assert_eq!(body["context"]["client"]["remoteHost"], "fixture-remote");
                assert_eq!(body["context"]["client"]["rolloutToken"], "fixture-rollout");
                assert_eq!(body["context"]["adSignalsInfo"]["params"][0]["key"], "dt");
                assert_eq!(body["cpn"].as_str().unwrap().len(), 16);
            } else {
                assert_eq!(request.url().host_str(), Some("www.youtube.com"));
                assert!(!headers
                    .get("cookie")
                    .is_some_and(|value| value.to_str().unwrap().contains("music-only")));
                assert_eq!(headers["x-goog-api-format-version"], "2");
            }
        }
    }

    #[test]
    fn stream_ua_matches_client_param() {
        // googlevideo 直链 `c=` 客户端参数决定 CDN 允许的拉流 UA
        assert_eq!(
            stream_user_agent_for_url("https://rr1.googlevideo.com/videoplayback?c=IOS&id=1"),
            STREAM_IOS_USER_AGENT
        );
        assert_eq!(
            stream_user_agent_for_url("https://rr1.googlevideo.com/videoplayback?c=ANDROID&id=1"),
            STREAM_ANDROID_USER_AGENT
        );
        assert_eq!(
            stream_user_agent_for_url(
                "https://rr1.googlevideo.com/videoplayback?c=ANDROID_MUSIC&id=1"
            ),
            STREAM_ANDROID_MUSIC_USER_AGENT
        );
        assert_eq!(
            stream_user_agent_for_url(
                "https://rr1.googlevideo.com/videoplayback?c=ANDROID_VR&id=1"
            ),
            STREAM_ANDROID_VR_USER_AGENT
        );
        // 小写 / TVHTML5 / 缺失 c= 均回退 Web UA
        assert_eq!(
            stream_user_agent_for_url("https://rr1.googlevideo.com/videoplayback?c=ios&id=1"),
            STREAM_IOS_USER_AGENT
        );
        assert_eq!(
            stream_user_agent_for_url("https://rr1.googlevideo.com/videoplayback?c=TVHTML5&id=1"),
            STREAM_WEB_USER_AGENT
        );
        assert_eq!(
            stream_user_agent_for_url("https://rr1.googlevideo.com/videoplayback?id=1"),
            STREAM_WEB_USER_AGENT
        );
    }

    #[test]
    fn resolve_plain_url() {
        let format = json!({ "url": "https://googlevideo.com/videoplayback?id=1" });
        assert_eq!(
            resolve_format_url(&format).as_deref(),
            Some("https://googlevideo.com/videoplayback?id=1")
        );
    }

    #[test]
    fn resolve_signature_cipher_with_sig() {
        let cipher = "url=https%3A%2F%2Fgooglevideo.com%2Fvideoplayback%3Fid%3D1&sig=ABC123&sp=sig";
        let format = json!({ "signatureCipher": cipher });
        let url = resolve_format_url(&format).expect("url");
        assert!(url.starts_with("https://googlevideo.com/videoplayback?id=1"));
        assert!(url.contains("sig=ABC123"));
    }

    #[test]
    fn skip_encrypted_s_without_solver() {
        let cipher = "url=https%3A%2F%2Fgooglevideo.com%2Fvideoplayback&s=ENCRYPTED&sp=sig";
        let format = json!({ "signatureCipher": cipher });
        assert!(resolve_format_url(&format).is_none());
    }

    #[test]
    fn parse_query_and_append() {
        let map = parse_query_map("a=1&b=hello%20world");
        assert_eq!(map.get("b").map(String::as_str), Some("hello world"));
        let url = append_query_param("https://x.test/p", "sig", "a+b");
        assert_eq!(url, "https://x.test/p?sig=a%2Bb");
    }

    #[test]
    fn profiles_follow_android_authenticated_and_anonymous_order() {
        let anonymous = ordered_profiles(false);
        assert_eq!(
            anonymous
                .iter()
                .map(|profile| profile.client_name)
                .collect::<Vec<_>>(),
            vec![
                "VISIONOS",
                "ANDROID_VR",
                "WEB_REMIX",
                "TVHTML5",
                "WEB_CREATOR",
                "TVHTML5"
            ]
        );
        assert_eq!(
            ordered_profiles(true)
                .iter()
                .map(|profile| profile.client_name)
                .collect::<Vec<_>>(),
            vec![
                "WEB_REMIX",
                "TVHTML5",
                "WEB_CREATOR",
                "TVHTML5",
                "VISIONOS",
                "ANDROID_VR"
            ]
        );
        assert!(anonymous
            .iter()
            .take(2)
            .all(|profile| !profile.supports_authenticated_context));
        assert_eq!(anonymous[0].client_version, "0.1");
        assert_eq!(anonymous[2].client_version, "1.20260403.09.00");
    }

    #[test]
    fn encrypted_parameters_are_not_returned_without_solver() {
        assert!(resolve_format_url(
            &json!({"signatureCipher": "url=https%3A%2F%2Frr.googlevideo.com%2Fv&s=secret"})
        )
        .is_none());
    }

    #[test]
    fn stream_urls_and_replacements_are_restricted() {
        assert!(trusted_stream_url(
            "https://rr1.googlevideo.com/videoplayback?c=VISIONOS"
        ));
        assert!(!trusted_stream_url("https://googlevideo.com.evil.test/v"));
        assert!(!trusted_stream_url("http://rr.googlevideo.com/v"));
        assert!(!trusted_stream_url("https://user@rr.googlevideo.com/v"));
        assert!(!trusted_stream_url("https://rr.googlevideo.com:8443/v"));
        assert_eq!(
            replace_query(
                "https://rr.googlevideo.com/v?n=old&n=dup&a=1#fragment",
                "n",
                "solved"
            )
            .unwrap(),
            "https://rr.googlevideo.com/v?a=1&n=solved#fragment"
        );
        assert_eq!(parse_query_map("bad=%é").get("bad").unwrap(), "%é");
    }

    #[test]
    fn android_vr_body_includes_visitor_data() {
        let profile = playback_client_profiles()
            .iter()
            .find(|p| p.client_name == "ANDROID_VR")
            .expect("android vr profile");
        let body = build_player_body(profile, "dQw4w9WgXcQ", Some("CgtVisitorTest"));
        assert_eq!(body["context"]["client"]["visitorData"], "CgtVisitorTest");
        assert_eq!(body["context"]["client"]["clientName"], "ANDROID_VR");
    }

    #[test]
    fn extract_prefers_audio_over_muxed() {
        let resp = json!({
            "streamingData": {
                "adaptiveFormats": [
                    {
                        "mimeType": "audio/mp4",
                        "bitrate": 128000,
                        "url": "https://googlevideo.com/a",
                        "contentLength": "100"
                    }
                ],
                "formats": [
                    {
                        "mimeType": "video/mp4",
                        "bitrate": 500000,
                        "url": "https://googlevideo.com/v",
                        "contentLength": "900"
                    }
                ]
            }
        });
        let streams = extract_audio_streams(&resp);
        assert_eq!(streams.len(), 1);
        assert!(streams[0].url.ends_with("/a"));
        assert!(streams[0].mime_type.starts_with("audio/"));
    }

    #[test]
    fn extract_prefers_m4a_over_webm_opus() {
        // symphonia 未启 opus: 同码率下必须优先 AAC/mp4, 否则解码 unsupported codec
        let resp = json!({
            "streamingData": {
                "adaptiveFormats": [
                    {
                        "mimeType": "audio/webm; codecs=\"opus\"",
                        "bitrate": 160000,
                        "url": "https://googlevideo.com/opus",
                        "contentLength": "200"
                    },
                    {
                        "mimeType": "audio/mp4; codecs=\"mp4a.40.2\"",
                        "bitrate": 128000,
                        "url": "https://googlevideo.com/aac",
                        "contentLength": "180"
                    }
                ]
            }
        });
        let streams = extract_audio_streams(&resp);
        assert_eq!(streams.len(), 2);
        assert!(streams[0].url.ends_with("/aac"));
        assert!(streams[0].mime_type.contains("mp4"));
    }

    #[test]
    fn extract_falls_back_to_progressive_when_no_audio() {
        let resp = json!({
            "streamingData": {
                "formats": [
                    {
                        "mimeType": "video/mp4",
                        "bitrate": 500000,
                        "url": "https://googlevideo.com/v",
                        "contentLength": "900"
                    }
                ]
            }
        });
        let streams = extract_audio_streams(&resp);
        assert_eq!(streams.len(), 1);
        assert!(streams[0].url.ends_with("/v"));
    }

    #[test]
    fn collect_merges_adaptive_and_progressive() {
        let resp = json!({
            "streamingData": {
                "adaptiveFormats": [{ "url": "a" }],
                "formats": [{ "url": "b" }]
            }
        });
        let all = collect_format_arrays(&resp);
        assert_eq!(all.len(), 2);
    }
}
