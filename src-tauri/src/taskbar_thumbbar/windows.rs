//! 缩略图工具栏的 Win32 实现。全部在主窗口所在的 UI 线程上执行（子类过程、setup、
//! run_on_main_thread），ITaskbarList3 不跨线程，状态放在 thread_local 里。
//!
//! 只能在 TaskbarButtonCreated 之后添加按钮；任务栏按钮每次重建（Explorer 重启、
//! 窗口从托盘恢复）都会重发这条消息，届时重新添加

use std::cell::RefCell;

use windows::core::w;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, TRUE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP, HDC,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::{
    DefSubclassProc, ITaskbarList3, RemoveWindowSubclass, SetWindowSubclass, TaskbarList,
    THBF_DISABLED, THBF_ENABLED, THBN_CLICKED, THB_FLAGS, THB_ICON, THB_TOOLTIP, THUMBBUTTON,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ChangeWindowMessageFilterEx, CreateIconIndirect, DestroyIcon, RegisterWindowMessageW, HICON,
    ICONINFO, MSGFLT_ALLOW, SM_CXSMICON, WM_COMMAND, WM_DPICHANGED, WM_NCDESTROY, WM_SETTINGCHANGE,
    WM_THEMECHANGED,
};

use super::glyph;
use super::{buttons, encode_tooltip, glyph_color, Action, ThumbbarState};

const SUBCLASS_ID: usize = 0x4e45_5249; // "NERI"
const DEFAULT_ICON_SIZE: i32 = 16;

struct Thumbbar {
    hwnd: HWND,
    created_message: u32,
    on_click: fn(Action),
    taskbar: Option<ITaskbarList3>,
    /// 当前任务栏按钮上已经加过按钮（之后只能 Update）
    added: bool,
    state: ThumbbarState,
    /// 系统还在引用的图标，换新图标成功后才销毁
    icons: Vec<HICON>,
    /// 上次提交给任务栏的内容；相同就不再调用系统接口
    sent: Option<(ThumbbarState, i32, bool)>,
}

thread_local! {
    static THUMBBAR: RefCell<Option<Thumbbar>> = const { RefCell::new(None) };
}

pub fn install(raw_hwnd: isize, on_click: fn(Action)) {
    let hwnd = HWND(raw_hwnd);
    // SAFETY: hwnd 是 Tauri 刚交出的主窗口句柄，调用发生在创建它的 UI 线程上
    unsafe {
        let created_message = RegisterWindowMessageW(w!("TaskbarButtonCreated"));
        // 以管理员身份运行时 UIPI 会拦下 Explorer 发来的这两条消息
        for message in [created_message, WM_COMMAND] {
            if let Err(error) = ChangeWindowMessageFilterEx(hwnd, message, MSGFLT_ALLOW, None) {
                log::debug!(target: "thumbbar", "message filter not relaxed for {message:#x}: {error}");
            }
        }
        THUMBBAR.with(|cell| {
            *cell.borrow_mut() = Some(Thumbbar {
                hwnd,
                created_message,
                on_click,
                taskbar: None,
                added: false,
                state: ThumbbarState::default(),
                icons: Vec::new(),
                sent: None,
            });
        });
        if !SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0).as_bool() {
            log::warn!(target: "thumbbar", "main window subclass not installed");
            THUMBBAR.with(|cell| cell.try_borrow_mut().map(|mut guard| guard.take()).ok());
            return;
        }
    }
    // 任务栏按钮可能在挂上子类之前就已建好，先试一次；失败就等 TaskbarButtonCreated
    with_thumbbar(|thumbbar| thumbbar.add_buttons());
}

pub fn apply(state: ThumbbarState) {
    with_thumbbar(|thumbbar| {
        thumbbar.state = state;
        thumbbar.sync();
    });
}

/// 系统接口可能同步地往本窗口发消息、重入子类过程；这时状态正被借用，直接跳过，
/// 不能 panic（在 extern "system" 回调里 panic 会直接终止进程）
fn with_thumbbar(action: impl FnOnce(&mut Thumbbar)) {
    THUMBBAR.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            if let Some(thumbbar) = guard.as_mut() {
                action(thumbbar);
            }
        }
    });
}

fn read_thumbbar<T>(read: impl FnOnce(&Thumbbar) -> T) -> Option<T> {
    THUMBBAR.with(|cell| cell.try_borrow().ok()?.as_ref().map(read))
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_COMMAND && (wparam.0 >> 16) as u32 & 0xffff == THBN_CLICKED {
        let clicked = Action::from_id((wparam.0 & 0xffff) as u32);
        let on_click = read_thumbbar(|thumbbar| thumbbar.on_click);
        if let (Some(action), Some(on_click)) = (clicked, on_click) {
            on_click(action);
            return LRESULT(0);
        }
    }
    let created_message = read_thumbbar(|thumbbar| thumbbar.created_message);
    if created_message == Some(message) {
        with_thumbbar(|thumbbar| {
            thumbbar.added = false;
            thumbbar.add_buttons();
        });
    } else if matches!(message, WM_SETTINGCHANGE | WM_THEMECHANGED | WM_DPICHANGED) {
        // 任务栏深浅色或 DPI 变了：sync 会发现图标尺寸/颜色不同而重画
        with_thumbbar(Thumbbar::sync);
    } else if message == WM_NCDESTROY {
        // SAFETY: 与 SetWindowSubclass 的参数一致；窗口销毁后不再收到消息
        let _ = RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID);
        THUMBBAR.with(|cell| cell.try_borrow_mut().map(|mut guard| guard.take()).ok());
    }
    DefSubclassProc(hwnd, message, wparam, lparam)
}

impl Thumbbar {
    fn taskbar(&mut self) -> Option<ITaskbarList3> {
        if self.taskbar.is_none() {
            // SAFETY: UI 线程已由 WebView2 / tao 初始化为 STA
            let created = unsafe {
                CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER)
                    .and_then(|taskbar| taskbar.HrInit().map(|_| taskbar))
            };
            match created {
                Ok(taskbar) => self.taskbar = Some(taskbar),
                Err(error) => log::warn!(target: "thumbbar", "ITaskbarList3 unavailable: {error}"),
            }
        }
        self.taskbar.clone()
    }

    fn add_buttons(&mut self) {
        let Some(taskbar) = self.taskbar() else {
            return;
        };
        let (size, light) = (icon_size(self.hwnd), light_taskbar());
        let (buttons, icons) = self.build(size, light);
        let key = (self.state.clone(), size, light);
        // SAFETY: 按钮数组与图标句柄在调用期间有效
        let result = unsafe {
            taskbar
                .ThumbBarAddButtons(self.hwnd, &buttons)
                // 子类挂上前按钮已经加过（同一个任务栏按钮只能加一次），改用 Update
                .or_else(|_| taskbar.ThumbBarUpdateButtons(self.hwnd, &buttons))
        };
        match result {
            Ok(()) => {
                self.added = true;
                self.commit(icons, key);
            }
            Err(error) => {
                log::debug!(target: "thumbbar", "thumbnail buttons not added yet: {error}");
                destroy_icons(icons);
            }
        }
    }

    fn sync(&mut self) {
        if !self.added {
            return;
        }
        let (size, light) = (icon_size(self.hwnd), light_taskbar());
        let key = (self.state.clone(), size, light);
        if self.sent.as_ref() == Some(&key) {
            return;
        }
        let Some(taskbar) = self.taskbar() else {
            return;
        };
        let (buttons, icons) = self.build(size, light);
        // SAFETY: 同上
        match unsafe { taskbar.ThumbBarUpdateButtons(self.hwnd, &buttons) } {
            Ok(()) => self.commit(icons, key),
            Err(error) => {
                log::warn!(target: "thumbbar", "thumbnail buttons not updated: {error}");
                destroy_icons(icons);
            }
        }
    }

    fn build(&self, size: i32, light: bool) -> ([THUMBBUTTON; 3], Vec<HICON>) {
        let color = glyph_color(light);
        let specs = buttons(&self.state);
        let mut icons = Vec::with_capacity(specs.len());
        let buttons = specs.map(|spec| {
            let icon = create_icon(spec.glyph, size, color).unwrap_or_default();
            icons.push(icon);
            THUMBBUTTON {
                dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
                iId: spec.action.id(),
                iBitmap: 0,
                hIcon: icon,
                szTip: encode_tooltip(&spec.tooltip),
                dwFlags: if spec.enabled {
                    THBF_ENABLED
                } else {
                    THBF_DISABLED
                },
            }
        });
        (buttons, icons)
    }

    fn commit(&mut self, icons: Vec<HICON>, key: (ThumbbarState, i32, bool)) {
        destroy_icons(std::mem::replace(&mut self.icons, icons));
        self.sent = Some(key);
    }
}

impl Drop for Thumbbar {
    fn drop(&mut self) {
        destroy_icons(std::mem::take(&mut self.icons));
    }
}

/// 系统小图标在窗口当前 DPI 下的像素边长（100% 缩放为 16）
fn icon_size(hwnd: HWND) -> i32 {
    // SAFETY: 两个调用都只读窗口与系统度量
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForWindow(hwnd)) };
    if size > 0 {
        size
    } else {
        DEFAULT_ICON_SIZE
    }
}

/// 预览浮层跟随任务栏（系统）的深浅色，而不是应用自己的主题
fn light_taskbar() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    winreg::RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|key| key.get_value::<u32, _>("SystemUsesLightTheme"))
        // 读不到时按 Windows 11 默认的深色任务栏处理
        .map(|value| value != 0)
        .unwrap_or(false)
}

fn create_icon(glyph: glyph::Glyph, size: i32, color: [u8; 3]) -> Option<HICON> {
    let side = usize::try_from(size).ok()?;
    let pixels = glyph::render(glyph, side, color);
    let header = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: size,
        // 负高度：自上而下的行序，与 render 的输出一致
        biHeight: -size,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let info = BITMAPINFO {
        bmiHeader: header,
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    // SAFETY: DIB 由系统按 header 分配 size×size×4 字节，拷贝长度与 render 输出一致；
    // 两张位图在 CreateIconIndirect 复制后即可释放
    unsafe {
        let color_bitmap = CreateDIBSection(
            HDC::default(),
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            HANDLE::default(),
            0,
        )
        .ok()?;
        if bits.is_null() {
            DeleteObject(color_bitmap);
            return None;
        }
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());
        // 32 位图标靠 alpha 通道裁剪，掩码位图只是格式要求，全 0 即可
        let mask_bitmap: HBITMAP = CreateBitmap(size, size, 1, 1, None);
        let icon_info = ICONINFO {
            fIcon: TRUE,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask_bitmap,
            hbmColor: color_bitmap,
        };
        let icon = CreateIconIndirect(&icon_info);
        DeleteObject(color_bitmap);
        DeleteObject(mask_bitmap);
        icon.ok()
    }
}

fn destroy_icons(icons: Vec<HICON>) {
    for icon in icons.into_iter().filter(|icon| !icon.is_invalid()) {
        // SAFETY: 图标都由 create_icon 创建，且系统已切换到新图标、不再引用
        let _ = unsafe { DestroyIcon(icon) };
    }
}
