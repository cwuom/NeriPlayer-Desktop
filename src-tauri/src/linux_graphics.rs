//! Linux WebKitGTK 图形兼容层 / Linux WebKitGTK graphics compatibility.
//!
//! 背景 / Background:
//! WebKitGTK 在 NVIDIA 私有驱动下切换页面/窗口时可能闪烁。已验证的成因有两类：
//! 一是 Wayland 的 explicit sync 与 NVIDIA 驱动配合异常；二是驱动帧队列/交换链
//! 缓冲过深，切换时把旧帧带到新界面。前者用 `__NV_DISABLE_EXPLICIT_SYNC=1` 解决，
//! 后者用 `__GL_MaxFramesAllowed=1` 限制在途帧数，两者都不关闭硬件加速。
//! WebKitGTK can flicker on NVIDIA's proprietary driver when switching pages/windows.
//! Two known causes: NVIDIA's Wayland explicit-sync path, and a deep driver frame
//! queue/swap chain that carries stale frames across a switch. The first is fixed by
//! `__NV_DISABLE_EXPLICIT_SYNC=1`, the second by `__GL_MaxFramesAllowed=1`; neither
//! disables hardware acceleration.
//!
//! 为什么不再默认关闭 DMA-BUF / Why DMA-BUF is no longer disabled by default:
//! 用户显式设置 `WEBKIT_DISABLE_DMABUF_RENDERER` 时原样尊重，但不会再叠加 SHM；
//! 两者同时设置会让 WebKit 走到没有有效 backing store 的分支。
//! An explicit `WEBKIT_DISABLE_DMABUF_RENDERER` is respected without also forcing SHM;
//! setting both can leave WebKit without a valid backing store.
//!
//! 开关 / Knobs:
//! - `NERI_DISABLE_NVIDIA_QUIRK=1`：完全跳过本兼容层 / skip this layer entirely.
//! - `NERI_NVIDIA_FRAMES_ALLOWED=N`：覆盖在途帧上限（0 = 不设置）/ override the frame cap.
//! - `NERI_NVIDIA_NO_VSYNC=1`：额外关闭垂直同步（可能撕裂，默认关闭）/
//!   additionally disable vsync (may tear; off by default).
//! - `NERI_WEBKIT_KEEP_DMABUF_ATLAS=1`：保留 DMA-BUF atlas（默认关闭）。注意这等于
//!   撤掉闪烁修复的一半：atlas 复用正是闪烁成因之一，此时若仍闪烁请配合
//!   `NERI_WEBKIT_FORCE_SHM=1` / keep the DMA-BUF atlas (disabled by default). This
//!   also removes half of the flicker fix, since atlas reuse is one of the causes; pair
//!   it with `NERI_WEBKIT_FORCE_SHM=1` if flicker returns.
//! - `NERI_WEBKIT_KEEP_NATIVE_VBLANK=1`：改用驱动原生 vblank（默认用定时 vblank）/
//!   use the driver's native vblank (timer-based vblank is the default).
//! - `NERI_WEBKIT_FORCE_SHM=1`：强制共享内存合成（默认关闭）/ force shared-memory
//!   compositing (off by default). 仍然闪烁时才打开：它会让每帧多一次 GPU→CPU 回读，
//!   混合显卡机器上还会把 Web 进程挑到独显再回读上屏 / turn this on only if flicker
//!   persists: it adds a GPU→CPU readback per frame and, on hybrid laptops, moves the
//!   Web process onto the discrete GPU before reading back for scanout. 注意它与
//!   `NERI_WEBKIT_KEEP_DMABUF=1` 冲突时后者优先 / note `NERI_WEBKIT_KEEP_DMABUF`
//!   takes precedence over it.
//! - `NERI_WEBKIT_KEEP_DMABUF=1`：保留 DMA-BUF 硬件合成（与默认一致），但它会否决
//!   `NERI_WEBKIT_FORCE_SHM=1`：两者同时设置时不会强制 SHM，可用它把旧配置里的这一项
//!   关掉 / keep DMA-BUF hardware compositing (same as the default), and it vetoes
//!   `NERI_WEBKIT_FORCE_SHM=1`: with both set, SHM is not forced.
//! - `NERI_WEBKIT_DMABUF_NO_GBM=1`：绕过 NVIDIA 的 GBM 分配路径 / avoid NVIDIA GBM allocation.
//! - `NERI_WEBKIT_FORCE_VBLANK_TIMER=1`：兼容开关，定时 vblank 现在是默认值 /
//!   compatibility switch; timer-based vblank is now the default.
//! - `NERI_WEBKIT_SOFTWARE_COMPOSITING=1`：完全关闭加速合成（最慢）/
//!   disable accelerated compositing entirely (slowest).
//!
//! 显式设置的环境变量永远优先 / Explicitly set environment variables always win.

use std::path::Path;
use std::sync::OnceLock;

/// 本次启动实际应用的兼容项 / What this launch actually applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppliedNvidiaQuirk {
    pub explicit_sync_disabled: bool,
    pub frames_allowed: Option<u32>,
    pub vsync_disabled: bool,
    pub dmabuf_atlas_disabled: bool,
    pub vblank_timer_forced: bool,
    pub dmabuf_shm_forced: bool,
    pub dmabuf_disabled: bool,
    pub software_compositing: bool,
}

static APPLIED: OnceLock<Option<AppliedNvidiaQuirk>> = OnceLock::new();

/// 检测 NVIDIA 私有驱动是否已加载。
/// Detect whether NVIDIA's proprietary driver is loaded.
///
/// `/proc/driver/nvidia/version` 只由私有驱动创建，nouveau 不会创建它；`/sys` 作为
/// 旧内核/受限 procfs 的兜底。仅检测到 NVIDIA 硬件不算，必须驱动真的在用。
/// `/proc/driver/nvidia/version` is created only by the proprietary driver (nouveau
/// does not create it); `/sys` is a fallback for restricted procfs. NVIDIA hardware
/// alone is not enough: the driver must actually be loaded.
pub fn nvidia_proprietary_driver_loaded() -> bool {
    Path::new("/proc/driver/nvidia/version").exists()
        || Path::new("/sys/module/nvidia/version").exists()
        || Path::new("/sys/module/nvidia").exists()
}

fn env_truthy(name: &str) -> bool {
    matches!(
        std::env::var(name).ok().as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
    )
}

fn env_u32(name: &str) -> Option<u32> {
    std::env::var(name).ok()?.trim().parse::<u32>().ok()
}

/// 在 GTK/WebKit 初始化前应用 NVIDIA 兼容设置。
/// Apply NVIDIA compatibility settings before GTK/WebKit initializes.
///
/// 返回 `None` 表示不是 NVIDIA、被用户关闭，或已经应用过。
/// Returns `None` when not on NVIDIA, disabled by the user, or already applied.
pub fn apply_nvidia_webkit_workarounds() -> Option<AppliedNvidiaQuirk> {
    *APPLIED.get_or_init(|| {
        if !nvidia_proprietary_driver_loaded() {
            return None;
        }
        // 用户明确要求跳过 / Explicit opt-out.
        if env_truthy("NERI_DISABLE_NVIDIA_QUIRK") {
            eprintln!(
                "neri: NVIDIA WebKitGTK quirk disabled by NERI_DISABLE_NVIDIA_QUIRK; \
                 flicker may return"
            );
            return None;
        }

        // Wayland explicit sync：不改变合成路径，代价最低，优先应用。
        // Wayland explicit sync: cheapest fix, does not change the compositing path.
        let explicit_sync_disabled = if std::env::var_os("__NV_DISABLE_EXPLICIT_SYNC").is_some() {
            false
        } else {
            std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
            true
        };

        // 在途帧上限：切换页面时不再把旧帧排到新界面之后。默认 1，0 表示不设置。
        // Frame cap: stop old frames from being presented after a page switch. Default 1;
        // 0 means leave it unset.
        let requested_frames = env_u32("NERI_NVIDIA_FRAMES_ALLOWED").unwrap_or(1);
        let frames_allowed =
            if requested_frames > 0 && std::env::var_os("__GL_MaxFramesAllowed").is_none() {
                std::env::set_var("__GL_MaxFramesAllowed", requested_frames.to_string());
                Some(requested_frames)
            } else {
                None
            };

        // 垂直同步：默认保持开启；显式要求时才关闭（可能撕裂）。
        // Vsync: on by default; disabled only on explicit request (may tear).
        let vsync_disabled = if env_truthy("NERI_NVIDIA_NO_VSYNC")
            && std::env::var_os("__GL_SYNC_TO_VBLANK").is_none()
        {
            std::env::set_var("__GL_SYNC_TO_VBLANK", "0");
            true
        } else {
            false
        };

        // DMA-BUF atlas：WebKit 会复用/缓存 GPU 缓冲区；NVIDIA 下复用中的缓冲区
        // 可能被再次呈现，正好对应“切换界面时只闪、不卡”。关掉 atlas 仍走 DMA-BUF
        // 硬件合成，代价只是每层各占一份 GPU 缓冲，远小于强制 SHM（后者会让每帧多一次
        // GPU→CPU 回读，在混合显卡机器上还会把 Web 进程推去独显）。
        // DMA-BUF atlas: WebKit caches/reuses GPU buffers. On NVIDIA a buffer still
        // being presented can be reused, which matches "flicker without slowness".
        // Disabling it keeps the DMA-BUF hardware path; the only cost is one GPU buffer
        // per layer, far below forced SHM (a GPU→CPU readback per frame that also moves
        // the Web process onto the discrete GPU on hybrid machines).
        let dmabuf_atlas_disabled = if std::env::var_os("WEBKIT_DISABLE_DMABUF_ATLAS").is_some()
            || env_truthy("NERI_WEBKIT_KEEP_DMABUF_ATLAS")
        {
            false
        } else {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_ATLAS", "1");
            true
        };

        // 定时 vblank：实测“关 atlas + 定时 vblank”才是 NVIDIA 下 WebGL 背景与转场闪烁的
        // 真正解，而且仍然走 DMA-BUF，不需要强制 SHM。默认开启；想用驱动原生 vblank 可设
        // `NERI_WEBKIT_KEEP_NATIVE_VBLANK=1`（`NERI_WEBKIT_FORCE_VBLANK_TIMER` 保留为
        // 兼容开关，默认已开启）。
        // Timer-based vblank: measured to be the other half of the actual fix for WebGL
        // background and transition flicker on NVIDIA, while still using DMA-BUF, so no
        // forced SHM is needed. On by default; `NERI_WEBKIT_KEEP_NATIVE_VBLANK=1` opts
        // out (`NERI_WEBKIT_FORCE_VBLANK_TIMER` is kept as a compatibility switch, now a
        // no-op because the timer is already the default).
        let vblank_timer_forced = if std::env::var_os("WEBKIT_FORCE_VBLANK_TIMER").is_some()
            || env_truthy("NERI_WEBKIT_KEEP_NATIVE_VBLANK")
        {
            false
        } else {
            std::env::set_var("WEBKIT_FORCE_VBLANK_TIMER", "1");
            true
        };

        // 备选路径：NVIDIA 的 GBM 分配也可能导致闪烁，默认不动，
        // 仅在用户显式要求时切换，便于逐项定位。
        // Optional path: NVIDIA's GBM allocation can also flicker; left alone by default
        // so users can bisect one knob at a time.
        if env_truthy("NERI_WEBKIT_DMABUF_NO_GBM")
            && std::env::var_os("WEBKIT_DMABUF_RENDERER_DISABLE_GBM").is_none()
        {
            std::env::set_var("WEBKIT_DMABUF_RENDERER_DISABLE_GBM", "1");
        }
        // 用户显式关闭 DMA-BUF renderer 时，不要再叠加 FORCE_SHM：WebKit 会先走
        // disable 分支，两者同时设置会落到没有有效 backing store 的崩溃路径。
        // When the user explicitly disables the DMA-BUF renderer, do not also force
        // SHM: WebKit takes the disable branch first, and setting both can crash with
        // no valid backing store.
        let dmabuf_disabled = std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_some();

        // DMA-BUF 共享内存合成：WebKitGTK 在 NVIDIA 下用 GPU 缓冲区直接合成时，
        // 切换页面/播放页仍会闪；强制 SHM 后实测稳定，代价是每帧一次 GPU→CPU 回读。
        // 代价不止合成开销：实测只要强制 SHM，Web 进程就会在混合显卡机器上被挑到
        // 独显渲染，再回读到核显上屏（跨 GPU 三跳），CPU、显存与功耗一起上去，
        // 所以默认关闭，只在用户显式要求或逐项定位闪烁时打开。
        // 想保留硬件合成可用 NERI_WEBKIT_KEEP_DMABUF=1，显式设置的环境变量优先。
        // Shared-memory DMA-BUF compositing: direct GPU-buffer compositing still
        // flickers on NVIDIA when switching pages/now-playing; SHM was stable in
        // testing, at the cost of one GPU→CPU readback per frame. That cost is not only
        // compositing: measured on a hybrid laptop, forcing SHM also moves the Web
        // process onto the discrete GPU and reads back to the integrated one for
        // scanout, so CPU, VRAM and power all rise. Off by default; enable only when
        // explicitly requested while bisecting flicker.
        let dmabuf_shm_forced = if dmabuf_disabled
            || std::env::var_os("WEBKIT_DMABUF_RENDERER_FORCE_SHM").is_some()
            || std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_some()
            || !env_truthy("NERI_WEBKIT_FORCE_SHM")
            || env_truthy("NERI_WEBKIT_KEEP_DMABUF")
            || env_truthy("NERI_WEBKIT_SOFTWARE_COMPOSITING")
        {
            false
        } else {
            std::env::set_var("WEBKIT_DMABUF_RENDERER_FORCE_SHM", "1");
            true
        };

        // 兜底：仍闪烁时用户可显式要求软件合成（完全关闭加速合成，代价最大）。
        // Last resort: explicit software compositing for stubborn flicker (fully
        // disables accelerated compositing; the highest cost).
        let software_compositing = if env_truthy("NERI_WEBKIT_SOFTWARE_COMPOSITING")
            && std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none()
        {
            std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
            true
        } else {
            false
        };

        eprintln!(
            "neri: NVIDIA proprietary driver detected; explicit_sync_disabled={explicit_sync_disabled} \
             frames_allowed={frames_allowed:?} vsync_disabled={vsync_disabled} \
             dmabuf_atlas_disabled={dmabuf_atlas_disabled} vblank_timer_forced={vblank_timer_forced} \
             dmabuf_shm_forced={dmabuf_shm_forced} \
             dmabuf_disabled={dmabuf_disabled} software_compositing={software_compositing}"
        );
        Some(AppliedNvidiaQuirk {
            explicit_sync_disabled,
            frames_allowed,
            vsync_disabled,
            dmabuf_atlas_disabled,
            vblank_timer_forced,
            dmabuf_shm_forced,
            dmabuf_disabled,
            software_compositing,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truthy_values_are_recognized() {
        // 环境变量是进程级的，测试只覆盖纯解析辅助函数的分支，避免互相污染。
        // Env vars are process-global; only exercise the pure parsing branches here.
        for value in ["1", "true", "TRUE", "yes", "YES", "on", "ON"] {
            std::env::set_var("NERI_QUIRK_TEST_TRUTHY", value);
            assert!(env_truthy("NERI_QUIRK_TEST_TRUTHY"), "{value}");
        }
        for value in ["0", "false", "off", ""] {
            std::env::set_var("NERI_QUIRK_TEST_TRUTHY", value);
            assert!(!env_truthy("NERI_QUIRK_TEST_TRUTHY"), "{value}");
        }
        std::env::remove_var("NERI_QUIRK_TEST_TRUTHY");
    }

    #[test]
    fn frame_limit_parses_and_zero_disables_it() {
        std::env::set_var("NERI_QUIRK_TEST_FRAMES", "2");
        assert_eq!(env_u32("NERI_QUIRK_TEST_FRAMES"), Some(2));
        std::env::set_var("NERI_QUIRK_TEST_FRAMES", "0");
        assert_eq!(env_u32("NERI_QUIRK_TEST_FRAMES"), Some(0));
        std::env::set_var("NERI_QUIRK_TEST_FRAMES", "not-a-number");
        assert_eq!(env_u32("NERI_QUIRK_TEST_FRAMES"), None);
        std::env::remove_var("NERI_QUIRK_TEST_FRAMES");
    }

    #[test]
    fn applied_result_is_stable_across_calls() {
        // 第二次调用必须返回同一结果，不能重复改写环境变量。
        // Repeated calls must return the same result without rewriting env vars.
        let first = apply_nvidia_webkit_workarounds();
        let second = apply_nvidia_webkit_workarounds();
        assert_eq!(first, second);
    }
}
