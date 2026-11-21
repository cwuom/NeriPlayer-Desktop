//! 远程拉流需要在解码线程上同步等待异步 HTTP，并在后台并发预取。
//! 宿主应在启动时注入共享 tokio 运行时（Tauri 下为其全局运行时），保证 reqwest
//! 连接池与命令处理位于同一运行时；未注入时（单元测试）惰性创建独立的多线程运行时

use std::future::Future;
use std::sync::OnceLock;
use tokio::runtime::{Builder, Handle, Runtime};

static INSTALLED: OnceLock<Handle> = OnceLock::new();
static FALLBACK: OnceLock<Runtime> = OnceLock::new();

/// 注入宿主运行时；只有首次调用生效
pub fn install(handle: Handle) {
    let _ = INSTALLED.set(handle);
}

fn handle() -> Handle {
    if let Some(handle) = INSTALLED.get() {
        return handle.clone();
    }
    FALLBACK
        .get_or_init(|| {
            Builder::new_multi_thread()
                .enable_all()
                .thread_name("neri-playback-rt")
                .build()
                .expect("failed to build fallback playback runtime")
        })
        .handle()
        .clone()
}

/// 在非异步线程上阻塞等待；与 tokio 语义一致，禁止在异步上下文中调用
pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
    handle().block_on(future)
}

pub(crate) fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    handle().spawn(future)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_on_and_spawn_share_a_runtime() {
        let value = block_on(async { spawn(async { 21 * 2 }).await.unwrap() });
        assert_eq!(value, 42);
    }
}
