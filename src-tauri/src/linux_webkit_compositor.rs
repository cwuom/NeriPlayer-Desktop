//! WebKitGTK 2.54 Skia 合成器闪烁规避 / WebKitGTK 2.54 Skia compositor flicker workaround.
//!
//! 2.54 把 Web 进程的层合成器默认换成 Skia 实现后，半透明元素的 opacity / transform
//! 过渡、动态背景与页面切换会间歇性闪一两帧（WebKit bug 327524 / 326841）。上游确认
//! `WEBKIT_USE_SKIA_FOR_COMPOSITION=0` 退回 TextureMapper 后闪烁消失，且仍保留 GPU
//! 加速；修复只在 main，没有回合到 2.54.x。该变量由 Web 进程在
//! `DrawingAreaCoordinatedGraphics::updatePreferences` 里用 `getenv` 读取，且只认字面值
//! `"0"`，Flatpak 下 WebKit 也会把完整环境传给子进程，所以必须在 WebView 启动前设置。
//! 2.56 起上游移除了这个开关，旧版本没有 Skia 合成器，因此只对 2.54.x / 2.55 开发版生效。
//!
//! Since 2.54 the Web process composites layers with Skia by default, and translucent
//! opacity/transform transitions, the dynamic background and page switches flicker for a
//! frame or two (WebKit bugs 327524 / 326841). Upstream confirmed that
//! `WEBKIT_USE_SKIA_FOR_COMPOSITION=0` (back to TextureMapper) removes the flicker while
//! keeping GPU acceleration; the fix only landed on main and was not backported to
//! 2.54.x. The Web process reads the variable with `getenv` and only honours the literal
//! `"0"`, so it has to be set before the WebView starts.
//!
//! 开关 / Knobs:
//! - 显式设置的 `WEBKIT_USE_SKIA_FOR_COMPOSITION` 永远优先（`=1` 可恢复 Skia）/
//!   an explicit `WEBKIT_USE_SKIA_FOR_COMPOSITION` always wins (`=1` restores Skia).
//! - `NERI_WEBKIT_KEEP_SKIA_COMPOSITOR=1`：跳过本规避 / skip this workaround.

/// WebKit 读取的环境变量 / The variable WebKit reads.
pub const SKIA_COMPOSITION_ENV: &str = "WEBKIT_USE_SKIA_FOR_COMPOSITION";
/// 用户关闭本规避的开关 / User opt-out for this workaround.
pub const KEEP_SKIA_ENV: &str = "NERI_WEBKIT_KEEP_SKIA_COMPOSITOR";

/// 运行时 WebKitGTK 版本 / Runtime WebKitGTK version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct WebKitVersion {
    pub major: u32,
    pub minor: u32,
    pub micro: u32,
}

/// 本次启动对 Skia 合成器的处理结果 / What this launch decided for the Skia compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkiaCompositorDecision {
    /// 设置 `WEBKIT_USE_SKIA_FOR_COMPOSITION=0`，退回 TextureMapper /
    /// set the variable to `0` and fall back to TextureMapper.
    UseTextureMapper,
    /// 用户已显式设置 WebKit 变量，原样尊重 / the user already set the WebKit variable.
    UserOverride,
    /// 用户通过 `NERI_WEBKIT_KEEP_SKIA_COMPOSITOR` 关闭了规避 / opted out.
    OptedOut,
    /// 该版本没有这个问题或不再读取该变量 / version unaffected or no longer reads it.
    Unaffected,
}

/// 受影响的版本：2.54 稳定版与 2.55 开发版。
/// Affected versions: the 2.54 stable series and the 2.55 development series.
pub fn is_affected_version(version: WebKitVersion) -> bool {
    version.major == 2 && (54..=55).contains(&version.minor)
}

/// 纯决策逻辑，便于在任意平台单测。版本未知时也设置：旧版本只在值非 `"0"` 时才读取它，
/// 新版本已经不读，设置 `0` 在两种情况下都是空操作。
/// Pure decision logic so it is testable on every platform. An unknown version still gets
/// the variable: older releases only act on values other than `"0"` and newer ones no
/// longer read it, so `0` is a no-op in both cases.
pub fn decide_skia_compositor(
    version: Option<WebKitVersion>,
    webkit_var_set: bool,
    keep_skia: bool,
) -> SkiaCompositorDecision {
    if webkit_var_set {
        return SkiaCompositorDecision::UserOverride;
    }
    if keep_skia {
        return SkiaCompositorDecision::OptedOut;
    }
    match version {
        Some(version) if !is_affected_version(version) => SkiaCompositorDecision::Unaffected,
        _ => SkiaCompositorDecision::UseTextureMapper,
    }
}

#[cfg(target_os = "linux")]
mod runtime {
    use super::*;
    use std::ffi::c_uint;
    use std::sync::OnceLock;

    // libwebkit2gtk-4.1 已经由 wry 链接；这三个函数只返回库里编译进去的常量，
    // 不依赖 GTK 初始化，读到的是实际加载的库版本而不是编译时头文件版本。
    // libwebkit2gtk-4.1 is already linked through wry; these return constants baked into
    // the loaded library, need no GTK init, and report the runtime (not header) version.
    extern "C" {
        fn webkit_get_major_version() -> c_uint;
        fn webkit_get_minor_version() -> c_uint;
        fn webkit_get_micro_version() -> c_uint;
    }

    static APPLIED: OnceLock<(Option<WebKitVersion>, SkiaCompositorDecision)> = OnceLock::new();

    fn runtime_version() -> Option<WebKitVersion> {
        // SAFETY: 无参数、无副作用的 C 函数，只读取库内常量。
        let version = unsafe {
            WebKitVersion {
                major: webkit_get_major_version(),
                minor: webkit_get_minor_version(),
                micro: webkit_get_micro_version(),
            }
        };
        (version.major > 0).then_some(version)
    }

    fn env_truthy(name: &str) -> bool {
        matches!(
            std::env::var(name).ok().as_deref(),
            Some("1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
        )
    }

    /// 在 WebView 创建前调用；重复调用返回首次结果。
    /// Call before any WebView is created; repeated calls return the first result.
    pub fn apply_skia_compositor_workaround() -> (Option<WebKitVersion>, SkiaCompositorDecision) {
        *APPLIED.get_or_init(|| {
            let version = runtime_version();
            let decision = decide_skia_compositor(
                version,
                std::env::var_os(SKIA_COMPOSITION_ENV).is_some(),
                env_truthy(KEEP_SKIA_ENV),
            );
            if decision == SkiaCompositorDecision::UseTextureMapper {
                std::env::set_var(SKIA_COMPOSITION_ENV, "0");
            }
            eprintln!("neri: WebKitGTK {version:?}; skia compositor decision: {decision:?}");
            (version, decision)
        })
    }
}

#[cfg(target_os = "linux")]
pub use runtime::apply_skia_compositor_workaround;

#[cfg(test)]
mod tests {
    use super::*;

    const fn v(major: u32, minor: u32, micro: u32) -> Option<WebKitVersion> {
        Some(WebKitVersion {
            major,
            minor,
            micro,
        })
    }

    #[test]
    fn affected_series_falls_back_to_texture_mapper() {
        for version in [v(2, 54, 0), v(2, 54, 2), v(2, 54, 3), v(2, 55, 1)] {
            assert_eq!(
                decide_skia_compositor(version, false, false),
                SkiaCompositorDecision::UseTextureMapper,
                "{version:?}"
            );
        }
    }

    #[test]
    fn versions_without_the_skia_switch_are_left_alone() {
        for version in [
            v(2, 44, 0),
            v(2, 52, 6),
            v(2, 53, 92),
            v(2, 56, 0),
            v(3, 0, 0),
        ] {
            assert_eq!(
                decide_skia_compositor(version, false, false),
                SkiaCompositorDecision::Unaffected,
                "{version:?}"
            );
        }
    }

    #[test]
    fn unknown_version_still_applies_the_harmless_default() {
        assert_eq!(
            decide_skia_compositor(None, false, false),
            SkiaCompositorDecision::UseTextureMapper
        );
    }

    #[test]
    fn explicit_webkit_variable_wins_over_everything() {
        assert_eq!(
            decide_skia_compositor(v(2, 54, 2), true, false),
            SkiaCompositorDecision::UserOverride
        );
        assert_eq!(
            decide_skia_compositor(v(2, 54, 2), true, true),
            SkiaCompositorDecision::UserOverride
        );
    }

    #[test]
    fn opt_out_keeps_skia_on_affected_versions() {
        assert_eq!(
            decide_skia_compositor(v(2, 54, 2), false, true),
            SkiaCompositorDecision::OptedOut
        );
    }
}
