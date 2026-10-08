// YouTube Music 后端子系统
// client/account/playlist 负责 InnerTube 音乐库、分页和认证会话
// playback/bootstrap/cache/challenge/web_po 负责 Android 对齐的客户端选择、动态配置和取流
// 远端 player.js 在不暴露主机能力的 QuickJS 中解 sig/n，token 窗口不开放应用 IPC
// CDN 请求继续按直链客户端选择 UA，不携带账号 Cookie

mod account;
mod artist;
mod bootstrap;
mod cache;
mod challenge;
pub mod client;
pub mod duration;
pub mod hls;
pub mod playback;
pub mod playlist;
pub mod refresh;
mod search;
pub mod session;
mod web_po;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::time::Duration;

/// 后台预热（首页配置刷新、player 解析、令牌铸造）推迟到起播之后再开始，
/// 慢网络下它们和首段音频抢带宽会让当前这首起播更慢
const BACKGROUND_WARMUP_DELAY: Duration = Duration::from_secs(8);

/// YouTube 国际化模式：开启后强制用海外 locale 访问，关闭则跟随应用语言
static INTERNATIONAL_MODE: AtomicBool = AtomicBool::new(false);
static APP_LOCALE: RwLock<String> = RwLock::new(String::new());

pub fn set_locale_preferences(international: bool, locale: &str) {
    INTERNATIONAL_MODE.store(international, Ordering::Release);
    if let Ok(mut slot) = APP_LOCALE.write() {
        *slot = locale.to_string();
    }
}

pub fn is_international_mode() -> bool {
    INTERNATIONAL_MODE.load(Ordering::Acquire)
}

/// YouTube Music 不提供服务的区域
///
/// 把 gl 设成这些区域会让 InnerTube 直接返回空目录（曾经因为按应用语言
/// 推导出 gl=CN 导致云端歌单整个读不到），所以只用于界面语言，不用于区域。
const UNSUPPORTED_REGIONS: &[&str] = &["CN"];

/// InnerTube 缺省区域
///
/// YouTube Music 在部分地区不可用，桌面端历来固定用 JP 兜底以保证目录可读，
/// 这里保持该行为，不要改成按语言推导。
const DEFAULT_REGION: &str = "JP";

/// 返回 InnerTube 用的 (hl, gl)
///
/// 开启国际化时统一走 en/US；关闭时界面语言跟随应用设置，但区域仍走可用区，
/// 避免把 gl 设成 YouTube Music 不服务的地区导致目录为空。
pub fn innertube_locale() -> (String, String) {
    if is_international_mode() {
        return ("en".into(), "US".into());
    }
    let locale = APP_LOCALE
        .read()
        .map(|value| value.clone())
        .unwrap_or_default();
    let (hl, gl) = match locale.as_str() {
        "zh-TW" => ("zh-TW", "TW"),
        "ja" => ("ja", "JP"),
        "en" => ("en", "US"),
        "zh-CN" => ("zh-CN", DEFAULT_REGION),
        _ => ("zh-CN", DEFAULT_REGION),
    };
    let gl = if UNSUPPORTED_REGIONS.contains(&gl) {
        DEFAULT_REGION
    } else {
        gl
    };
    (hl.into(), gl.into())
}

#[cfg(test)]
mod locale_tests {
    use super::*;

    /// locale 偏好是进程级全局状态，必须在同一个测试里顺序断言，
    /// 拆成多个测试会因并行执行互相覆盖
    #[test]
    fn locale_resolution_never_targets_an_unavailable_market() {
        for locale in ["zh-CN", "zh-TW", "ja", "en", "", "unknown"] {
            set_locale_preferences(false, locale);
            let (hl, gl) = innertube_locale();
            assert!(!hl.is_empty(), "locale {locale} resolved to an empty hl");
            assert!(
                !UNSUPPORTED_REGIONS.contains(&gl.as_str()),
                "locale {locale} resolved to unsupported region {gl}",
            );
        }

        set_locale_preferences(true, "zh-CN");
        assert_eq!(innertube_locale(), ("en".to_string(), "US".to_string()));

        set_locale_preferences(false, "zh-CN");
        assert_eq!(innertube_locale().1, DEFAULT_REGION);
    }
}
