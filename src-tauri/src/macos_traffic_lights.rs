//! macOS 红绿灯位置。
//!
//! tauri.conf.json 的 `trafficLightPosition` 由 tao 只在它自己的 NSView 重绘时套用，
//! 而主窗口内容被 WKWebView 整个盖住，那次重绘几乎不会发生；AppKit 在显示、改尺寸、
//! 切换外观等时机重新布局标题栏后，按钮就回到系统默认位置（贴着窗口顶部，与自绘顶栏
//! 错位）。复位的时机不固定（系统弹窗、失焦动画结束时都可能发生），只靠事件后定时补摆会
//! 漏掉，所以同时监听按钮容器与按钮自身的 frame 变化：一被系统改动就立即摆回。

#[cfg(target_os = "macos")]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};

/// 前端顶栏的默认高度（pt）；界面缩放后由前端通过 `set_titlebar_height` 更新
pub const DEFAULT_TITLEBAR_HEIGHT: f64 = 68.0;
static TITLEBAR_HEIGHT_BITS: AtomicU64 = AtomicU64::new(DEFAULT_TITLEBAR_HEIGHT.to_bits());

/// 一次摆放的几何结果（AppKit 坐标，原点在左下角）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrafficLightLayout {
    /// 按钮容器的新高度与 y 原点：容器与自绘顶栏等高
    pub container_height: f64,
    pub container_origin_y: f64,
    /// 按钮在容器内的 y 原点：垂直居中，与顶栏品牌、播放页收起按钮同一中线
    pub button_y: f64,
    /// 三个按钮各自的 x 原点（关闭、最小化、缩放）
    pub button_x: [f64; 3],
}

/// 把按钮容器拉到顶栏高度并让按钮垂直居中；横向从配置的 x 开始，按系统原有间距排开。
/// 不沿用 tao 的「容器高度 = 按钮高度 + y」，那样按钮中线取决于系统默认偏移，和自绘顶栏对不齐
pub fn layout(
    window_height: f64,
    titlebar_height: f64,
    button_height: f64,
    button_spacing: f64,
    x: f64,
) -> TrafficLightLayout {
    let container_height = titlebar_height.max(button_height);
    TrafficLightLayout {
        container_height,
        container_origin_y: window_height - container_height,
        button_y: ((container_height - button_height) / 2.0).round(),
        button_x: [x, x + button_spacing, x + button_spacing * 2.0],
    }
}

pub fn titlebar_height() -> f64 {
    f64::from_bits(TITLEBAR_HEIGHT_BITS.load(Ordering::Relaxed))
}

/// 记录前端顶栏高度（pt），非法值回到默认
pub fn set_titlebar_height(height: f64) -> f64 {
    let height = if height.is_finite() && (24.0..=200.0).contains(&height) {
        height
    } else {
        DEFAULT_TITLEBAR_HEIGHT
    };
    TITLEBAR_HEIGHT_BITS.store(height.to_bits(), Ordering::Relaxed);
    height
}

/// 读取配置里主窗口的红绿灯位置
pub fn configured_position<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Option<(f64, f64)> {
    // y 不再使用：纵向由顶栏高度决定
    let config = app.config();
    config
        .app
        .windows
        .iter()
        .find(|window| window.label == "main")
        .and_then(|window| window.traffic_light_position.as_ref())
        .map(|position| (position.x, position.y))
}

/// AppKit 在窗口激活、改尺寸、切换外观后的下一轮布局里才会把标题栏容器复位，
/// 只在事件当下摆放会被随后的布局覆盖（按钮被顶到窗口最上沿）。
/// 所以当下摆一次，再在布局完成后补摆两次；连续事件（拖动改尺寸）只保留最后一轮补摆
#[cfg(target_os = "macos")]
const SETTLE_DELAYS_MS: [u64; 2] = [60, 260];

#[cfg(target_os = "macos")]
static APPLY_GENERATION: AtomicU64 = AtomicU64::new(0);

#[cfg(target_os = "macos")]
pub fn apply<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>, x: f64) {
    let generation = APPLY_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    apply_on_main_thread(window, x);
    let target = window.clone();
    let spawned = std::thread::Builder::new()
        .name("traffic-lights".into())
        .spawn(move || {
            let mut elapsed = 0;
            for delay in SETTLE_DELAYS_MS {
                std::thread::sleep(std::time::Duration::from_millis(delay - elapsed));
                elapsed = delay;
                if APPLY_GENERATION.load(Ordering::Relaxed) != generation {
                    return;
                }
                apply_on_main_thread(&target, x);
            }
        });
    if let Err(error) = spawned {
        log::warn!(target: "window", "could not schedule traffic light re-alignment: {error}");
    }
}

#[cfg(target_os = "macos")]
fn apply_on_main_thread<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>, x: f64) {
    let target = window.clone();
    let titlebar = titlebar_height();
    // AppKit 视图只能在主线程操作
    let _ = window.run_on_main_thread(move || {
        let Ok(pointer) = target.ns_window() else {
            return;
        };
        if pointer.is_null() {
            return;
        }
        // SAFETY: Tauri 返回的是当前存活窗口的 NSWindow 指针，且此处在主线程
        let ns_window = unsafe { &*(pointer as *const objc2_app_kit::NSWindow) };
        apply_to_ns_window(ns_window, x, titlebar);
        install_frame_observer(&target, ns_window, x);
    });
}

#[cfg(target_os = "macos")]
static FRAME_OBSERVER_INSTALLED: AtomicBool = AtomicBool::new(false);

/// 系统改动按钮或其容器的 frame 后立刻摆回。只装一次，观察者随应用存活；
/// 回调里异步排队摆放，摆放发现位置已正确就不再写 frame，不会自激循环
#[cfg(target_os = "macos")]
fn install_frame_observer<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    ns_window: &objc2_app_kit::NSWindow,
    x: f64,
) {
    use objc2_app_kit::{NSView, NSViewFrameDidChangeNotification, NSWindowButton};
    use objc2_foundation::NSNotificationCenter;

    let Some(close) = ns_window.standardWindowButton(NSWindowButton::CloseButton) else {
        return;
    };
    // SAFETY: 只读取视图层级
    let Some(container) = (unsafe { close.superview().and_then(|view| view.superview()) }) else {
        return;
    };
    if FRAME_OBSERVER_INSTALLED.swap(true, Ordering::AcqRel) {
        return;
    }

    let target = window.clone();
    let block = block2::RcBlock::new(move |_notification| {
        apply_on_main_thread(&target, x);
    });
    let center = NSNotificationCenter::defaultCenter();
    let views: [&NSView; 2] = [&container, &close];
    for view in views {
        view.setPostsFrameChangedNotifications(true);
        // SAFETY: object 传入被观察的视图；queue 为 None 时回调在发通知的主线程同步执行，
        // 回调只往主线程队列投递一次摆放，不跨线程使用 AppKit 对象
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSViewFrameDidChangeNotification),
                Some(view),
                None,
                &block,
            )
        };
        // 主窗口与应用同生命周期，观察者无需注销
        std::mem::forget(token);
    }
}

#[cfg(target_os = "macos")]
fn apply_to_ns_window(window: &objc2_app_kit::NSWindow, x: f64, titlebar: f64) {
    use objc2_app_kit::NSWindowButton;

    let (Some(close), Some(minimize), Some(zoom)) = (
        window.standardWindowButton(NSWindowButton::CloseButton),
        window.standardWindowButton(NSWindowButton::MiniaturizeButton),
        window.standardWindowButton(NSWindowButton::ZoomButton),
    ) else {
        return;
    };
    // SAFETY: superview 只读取视图层级，按钮存活期间其父视图同样存活
    let Some(container) = (unsafe { close.superview().and_then(|view| view.superview()) }) else {
        return;
    };
    // 全屏时系统接管标题栏，不去改动
    if window
        .styleMask()
        .contains(objc2_app_kit::NSWindowStyleMask::FullScreen)
    {
        return;
    }

    let close_frame = close.frame();
    let geometry = layout(
        window.frame().size.height,
        titlebar,
        close_frame.size.height,
        minimize.frame().origin.x - close_frame.origin.x,
        x,
    );
    // 位置已经正确时不写 frame：写入会再发 frame 变化通知
    let mut container_frame = container.frame();
    if container_frame.size.height != geometry.container_height
        || container_frame.origin.y != geometry.container_origin_y
    {
        container_frame.size.height = geometry.container_height;
        container_frame.origin.y = geometry.container_origin_y;
        container.setFrame(container_frame);
    }

    for (button, button_x) in [close, minimize, zoom].iter().zip(geometry.button_x) {
        let mut origin = button.frame().origin;
        if origin.x == button_x && origin.y == geometry.button_y {
            continue;
        }
        origin.x = button_x;
        origin.y = geometry.button_y;
        button.setFrameOrigin(origin);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn apply<R: tauri::Runtime>(_window: &tauri::WebviewWindow<R>, _x: f64) {}

#[cfg(test)]
mod tests {
    use super::{layout, set_titlebar_height, DEFAULT_TITLEBAR_HEIGHT};

    #[test]
    fn buttons_are_centred_in_the_custom_titlebar() {
        let geometry = layout(800.0, 52.0, 14.0, 20.0, 20.0);
        assert_eq!(geometry.container_height, 52.0);
        assert_eq!(geometry.container_origin_y, 748.0);
        assert_eq!(geometry.button_y, 19.0);
        // 按钮中线 = 顶栏中线
        assert_eq!(geometry.button_y + 14.0 / 2.0, 52.0 / 2.0);
        assert_eq!(geometry.button_x, [20.0, 40.0, 60.0]);
    }

    #[test]
    fn layout_follows_a_scaled_titlebar_and_keeps_system_spacing() {
        let geometry = layout(600.0, 65.0, 16.0, 23.0, 20.0);
        assert_eq!(
            geometry.container_origin_y + geometry.container_height,
            600.0
        );
        assert_eq!(geometry.button_x[2] - geometry.button_x[1], 23.0);
        assert!((geometry.button_y + 8.0 - 32.5).abs() <= 0.5);
    }

    #[test]
    fn invalid_titlebar_heights_fall_back_to_the_default() {
        assert_eq!(set_titlebar_height(f64::NAN), DEFAULT_TITLEBAR_HEIGHT);
        assert_eq!(set_titlebar_height(5.0), DEFAULT_TITLEBAR_HEIGHT);
        assert_eq!(set_titlebar_height(65.0), 65.0);
        set_titlebar_height(DEFAULT_TITLEBAR_HEIGHT);
    }
}
