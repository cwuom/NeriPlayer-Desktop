//! 全局鼠标快捷键的 IPC 入口
//!
//! 键盘全局快捷键由 tauri-plugin-global-shortcut 注册，插件的加速器语法里没有鼠标，
//! 所以全局鼠标键走 Windows 原生钩子：前端把当前绑定了全局鼠标键的组合键传下来，
//! 原生侧只上报这些组合，避免把用户的每一次鼠标操作都推给前端。

use neri_common::error::AppResult;
use tauri::AppHandle;

use crate::mouse_hook;

/// 设置需要全局生效的鼠标组合键
///
/// 返回实际生效的组合键：非 Windows 平台没有全局鼠标钩子，返回空列表。
#[tauri::command]
pub fn set_global_mouse_shortcuts(app: AppHandle, buttons: Vec<String>) -> AppResult<Vec<String>> {
    let mut resolved: Vec<String> = Vec::new();
    for combo in &buttons {
        // 原生侧只认鼠标键组合，其它组合直接丢掉
        if mouse_hook::MouseBinding::parse(combo).is_none() {
            log::warn!(target: "mouse-hook", "unknown mouse combo: {combo}");
            continue;
        }
        // 同一个组合只留一次
        if !resolved.contains(combo) {
            resolved.push(combo.clone());
        }
    }
    // 先注册应用句柄再更新绑定：否则钩子线程可能早于句柄就绪，事件会被丢弃
    mouse_hook::install(app);
    mouse_hook::set_bound_bindings(&resolved);
    Ok(resolved)
}
