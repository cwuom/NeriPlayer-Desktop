//! 所有窗口共用一个 WebView2 环境（同一个用户数据目录），附加浏览器参数必须与主窗口完全一致，
//! 否则之后创建的窗口会因环境参数不同而创建失败。参数只在 tauri.conf.json 的主窗口里写一份
use tauri::{Manager, Runtime, WebviewWindowBuilder};

pub trait MainBrowserArgs<R: Runtime>: Sized {
    /// 套用主窗口配置的附加浏览器参数
    fn main_browser_args(self, manager: &impl Manager<R>) -> Self;
}

impl<'a, R: Runtime, M: Manager<R>> MainBrowserArgs<R> for WebviewWindowBuilder<'a, R, M> {
    fn main_browser_args(self, manager: &impl Manager<R>) -> Self {
        let config = manager.config();
        let args = config
            .app
            .windows
            .iter()
            .find(|window| window.label == "main")
            .or_else(|| config.app.windows.first())
            .and_then(|window| window.additional_browser_args.as_deref());
        match args {
            Some(args) => self.additional_browser_args(args),
            None => self,
        }
    }
}
