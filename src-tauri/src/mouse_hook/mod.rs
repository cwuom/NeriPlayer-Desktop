//! Windows 全局鼠标钩子：把被绑定的鼠标键事件转给前端
//!
//! 系统的全局快捷键插件（tauri-plugin-global-shortcut）的加速器语法里没有鼠标，
//! 所以全局鼠标键单独用 WH_MOUSE_LL 低级钩子实现。
//!
//! 设计要点：
//! - 低级钩子回调超时后会被系统静默摘除（LowLevelHooksTimeout），所以回调里
//!   只做「识别键位 + 投递队列」，实际派发在别的线程，避免慢操作拖垮钩子。
//! - 只上报当前被绑定的鼠标键，其它鼠标事件直接放过，不干扰用户日常操作。
//! - 钩子不吞事件（始终返回 CallNextHookEx）：绑定只是"额外触发动作"，
//!   不像键盘全局快捷键那样独占组合键。
//! - 钩子按需安装：用户第一次绑定全局鼠标键时才装，从不绑定就完全不装。
//! - 修饰键参与匹配：绑了 "Ctrl+MouseBack" 就只在按住 Ctrl 时触发，
//!   和应用内的鼠标快捷键语义一致。

use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, Emitter};

/// 鼠标键事件名；payload 是命中的组合键字符串（如 "MouseBack" / "Ctrl+MouseBack"）
pub const MOUSE_EVENT: &str = "shortcut:mouse-button";

/// 钩子能识别的鼠标键
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    /// 中键
    Middle,
    /// 后退侧键（XBUTTON1）
    Back,
    /// 前进侧键（XBUTTON2）
    Forward,
}

impl MouseButton {
    /// 与前端组合键字符串里的鼠标键名对应
    pub fn token(self) -> &'static str {
        match self {
            MouseButton::Middle => "MouseMiddle",
            MouseButton::Back => "MouseBack",
            MouseButton::Forward => "MouseForward",
        }
    }

    /// 解析前端传来的鼠标键名
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "MouseMiddle" => Some(MouseButton::Middle),
            "MouseBack" => Some(MouseButton::Back),
            "MouseForward" => Some(MouseButton::Forward),
            _ => None,
        }
    }
}

/// 修饰键掩码
mod modifiers {
    /// Ctrl（Windows 上 "Mod" 也落在 Ctrl 上）
    pub const CTRL: u8 = 1 << 0;
    /// Alt
    pub const ALT: u8 = 1 << 1;
    /// Shift
    pub const SHIFT: u8 = 1 << 2;
    /// Win 键 / Meta
    pub const META: u8 = 1 << 3;
}

/// 一条全局鼠标绑定：键位 + 要求的修饰键
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MouseBinding {
    /// 原始组合键字符串，命中时原样上报，方便前端直接比对
    combo: String,
    /// 鼠标键
    button: MouseButton,
    /// 要求的修饰键掩码
    modifiers: u8,
}

impl MouseBinding {
    /// 解析组合键字符串；不是鼠标键组合时返回 None
    pub fn parse(combo: &str) -> Option<Self> {
        let parts: Vec<&str> = combo
            .split('+')
            .map(|part| part.trim())
            .filter(|part| !part.is_empty())
            .collect();
        let (key, modifier_parts) = parts.split_last()?;
        let button = MouseButton::from_token(key)?;
        let mut mask = 0;
        for part in modifier_parts {
            mask |= match part.to_ascii_lowercase().as_str() {
                // Windows 上没有独立的 Command 键，"Mod" 就是 Ctrl
                "mod" | "ctrl" | "control" => modifiers::CTRL,
                "alt" | "option" => modifiers::ALT,
                "shift" => modifiers::SHIFT,
                "meta" | "cmd" | "command" | "super" => modifiers::META,
                _ => return None,
            };
        }
        Some(Self {
            combo: combo.to_string(),
            button,
            modifiers: mask,
        })
    }

    /// 修饰键是否完全匹配
    fn matches_modifiers(&self, held: u8) -> bool {
        self.modifiers == held
    }
}

/// 应用句柄：钩子线程用它把事件 emit 给前端
fn app_handle() -> &'static OnceLock<AppHandle> {
    static APP: OnceLock<AppHandle> = OnceLock::new();
    &APP
}

/// 当前生效的全局鼠标绑定；空列表表示不监听任何鼠标键
fn bound_bindings() -> &'static Mutex<Vec<MouseBinding>> {
    static BOUND: OnceLock<Mutex<Vec<MouseBinding>>> = OnceLock::new();
    BOUND.get_or_init(|| Mutex::new(Vec::new()))
}

/// 钩子回调把事件投递给派发线程
fn event_sender() -> &'static Mutex<Option<mpsc::Sender<String>>> {
    static SENDER: OnceLock<Mutex<Option<mpsc::Sender<String>>>> = OnceLock::new();
    SENDER.get_or_init(|| Mutex::new(None))
}

/// 注册应用句柄；钩子在第一次绑定全局鼠标键时才真正安装
pub fn install(app: AppHandle) {
    let _ = app_handle().set(app);
}

/// 更新需要上报的全局鼠标绑定；返回是否发生变化
///
/// 调用前必须先 `install`，否则钩子线程拿不到应用句柄，事件会被丢弃。
pub fn set_bound_bindings(combos: &[String]) -> bool {
    let mut desired: Vec<MouseBinding> = Vec::new();
    for combo in combos {
        if let Some(binding) = MouseBinding::parse(combo) {
            // 同一个组合只留一次
            if !desired.iter().any(|item| item.combo == binding.combo) {
                desired.push(binding);
            }
        } else {
            log::warn!(target: "mouse-hook", "unknown mouse combo: {combo}");
        }
    }
    let changed = {
        let mut guard = bound_bindings()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *guard == desired {
            false
        } else {
            *guard = desired.clone();
            true
        }
    };
    if changed && !desired.is_empty() {
        start_hook();
    }
    changed
}

/// 启动钩子线程与派发线程；只生效一次
fn start_hook() {
    #[cfg(windows)]
    {
        let (sender, receiver) = mpsc::channel::<String>();
        {
            let mut guard = event_sender()
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if guard.is_some() {
                // 派发线程已经在跑
                return;
            }
            *guard = Some(sender);
        }
        // 派发线程：把鼠标键事件转成前端事件
        let spawned = std::thread::Builder::new()
            .name("mouse-hook-dispatch".into())
            .spawn(move || {
                while let Ok(combo) = receiver.recv() {
                    if let Some(app) = app_handle().get() {
                        if let Err(error) = app.emit(MOUSE_EVENT, combo) {
                            log::warn!(
                                target: "mouse-hook",
                                "emitting the mouse event failed: {error}"
                            );
                        }
                    }
                }
            })
            .is_ok();
        if !spawned {
            let mut guard = event_sender()
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            *guard = None;
            return;
        }
        platform::start_hook_thread();
    }
    #[cfg(not(windows))]
    {
        log::info!(
            target: "mouse-hook",
            "global mouse shortcuts are only available on Windows"
        );
    }
}

#[cfg(windows)]
mod platform {
    use super::{bound_bindings, event_sender, modifiers, MouseButton};
    use std::sync::atomic::{AtomicBool, Ordering};

    use windows_sys::Win32::Foundation::{HINSTANCE, HMODULE};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION, MSG,
        MSLLHOOKSTRUCT, WH_MOUSE_LL, WM_MBUTTONDOWN, WM_XBUTTONDOWN,
    };

    /// 钩子线程是否已经启动
    static RUNNING: AtomicBool = AtomicBool::new(false);

    /// 启动钩子线程：低级钩子必须装在有消息泵的线程上
    pub fn start_hook_thread() {
        if RUNNING.swap(true, Ordering::AcqRel) {
            return;
        }
        let spawned = std::thread::Builder::new()
            .name("mouse-hook".into())
            .spawn(|| unsafe { hook_thread() })
            .is_ok();
        if !spawned {
            RUNNING.store(false, Ordering::Release);
        }
    }

    /// 钩子线程：装钩子后进入消息循环
    unsafe fn hook_thread() {
        let module: HMODULE = GetModuleHandleW(std::ptr::null());
        if module.is_null() {
            log::warn!(target: "mouse-hook", "the current module handle is unavailable");
            RUNNING.store(false, Ordering::Release);
            return;
        }
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module as HINSTANCE, 0);
        if hook.is_null() {
            log::warn!(target: "mouse-hook", "SetWindowsHookExW returned null");
            RUNNING.store(false, Ordering::Release);
            return;
        }
        log::info!(target: "mouse-hook", "the global mouse hook is installed");
        let mut message: MSG = std::mem::zeroed();
        // 消息循环：GetMessageW 返回 0（收到 WM_QUIT）后退出
        loop {
            if GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) <= 0 {
                break;
            }
        }
        UnhookWindowsHookEx(hook);
        log::info!(target: "mouse-hook", "the global mouse hook was removed");
        RUNNING.store(false, Ordering::Release);
    }

    /// 当前按住的修饰键掩码
    fn held_modifiers() -> u8 {
        let mut mask = 0;
        if key_down(VK_CONTROL) {
            mask |= modifiers::CTRL;
        }
        if key_down(VK_MENU) {
            mask |= modifiers::ALT;
        }
        if key_down(VK_SHIFT) {
            mask |= modifiers::SHIFT;
        }
        if key_down(VK_LWIN) || key_down(VK_RWIN) {
            mask |= modifiers::META;
        }
        mask
    }

    /// 某个虚拟键当前是否按下
    fn key_down(virtual_key: u16) -> bool {
        // 高位为 1 表示当前按下（GetKeyState 返回 i16，先转成无符号再取高位）
        unsafe { (GetKeyState(virtual_key as i32) as u16 & 0x8000) != 0 }
    }

    /// 低级鼠标钩子回调：识别键位后入队，其余事件一律放过
    unsafe extern "system" fn mouse_proc(code: i32, wparam: usize, lparam: isize) -> isize {
        if code == HC_ACTION as i32 {
            let message = wparam as u32;
            let button = match message {
                WM_MBUTTONDOWN => Some(MouseButton::Middle),
                WM_XBUTTONDOWN => {
                    // XBUTTON 编号在 mouseData 的高位字里
                    let data = *(lparam as *const MSLLHOOKSTRUCT);
                    match (data.mouseData >> 16) & 0xFFFF {
                        1 => Some(MouseButton::Back),
                        2 => Some(MouseButton::Forward),
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some(button) = button {
                let held = held_modifiers();
                let combo = bound_bindings()
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .iter()
                    .find(|binding| binding.button == button && binding.matches_modifiers(held))
                    .map(|binding| binding.combo.clone());
                if let Some(combo) = combo {
                    let sender = event_sender()
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .clone();
                    if let Some(sender) = sender {
                        let _ = sender.send(combo);
                    }
                }
            }
        }
        // 不吞事件：绑定只是额外触发动作
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_button_tokens_round_trip() {
        for button in [MouseButton::Middle, MouseButton::Back, MouseButton::Forward] {
            assert_eq!(MouseButton::from_token(button.token()), Some(button));
        }
        assert_eq!(MouseButton::from_token("MouseLeft"), None);
        assert_eq!(MouseButton::from_token("Space"), None);
    }

    #[test]
    fn bindings_parse_mouse_combos_with_modifiers() {
        let plain = MouseBinding::parse("MouseBack").expect("plain combo");
        assert_eq!(plain.button, MouseButton::Back);
        assert_eq!(plain.modifiers, 0);

        let with_ctrl = MouseBinding::parse("Ctrl+MouseBack").expect("ctrl combo");
        assert_eq!(with_ctrl.button, MouseButton::Back);
        assert_eq!(with_ctrl.modifiers, modifiers::CTRL);

        // Mod 在 Windows 上就是 Ctrl
        let with_mod = MouseBinding::parse("Mod+MouseBack").expect("mod combo");
        assert_eq!(with_mod.modifiers, modifiers::CTRL);

        assert!(MouseBinding::parse("Space").is_none());
        assert!(MouseBinding::parse("Ctrl+Space").is_none());
    }

    #[test]
    fn bound_bindings_update_is_reported_once_per_change() {
        let combos = vec!["MouseBack".to_string()];
        assert!(set_bound_bindings(&combos));
        assert!(!set_bound_bindings(&combos));
        assert!(set_bound_bindings(&[]));
    }
}
