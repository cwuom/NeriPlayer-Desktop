// 应用内增量更新：检查、下载暂存、安装并重启。差分与替换逻辑在 neri-update，这里只做编排
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use neri_update::client::HttpFetcher;
use neri_update::layout::{self, Environment, InstallLayout, UnsupportedReason};
use neri_update::manifest::{PlatformRelease, ReleaseManifest};
use neri_update::plan::{build_plan, FsLocalState, PlanSummary, UpdatePlan};
use neri_update::stage::{stage, Progress};
use neri_update::{apply, UpdateError};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

const MANIFEST_URL: &str =
    "https://github.com/cwuom/NeriPlayer-Desktop/releases/latest/download/neri-update.json";
const PUBLIC_KEY_FILE: &str = include_str!("../../../update-public-key.txt");
pub const PROGRESS_EVENT: &str = "update:progress";
/// 进度事件节流，避免每个网络分块都跨进程通知一次前端
const PROGRESS_INTERVAL: Duration = Duration::from_millis(120);

fn public_key() -> Option<&'static str> {
    PUBLIC_KEY_FILE
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
}

fn current_build_epoch() -> u64 {
    env!("BUILD_EPOCH").parse().unwrap_or_default()
}

struct Session {
    version: String,
    layout: InstallLayout,
    release: PlatformRelease,
    plan: UpdatePlan,
    staged: bool,
}

#[derive(Default)]
pub struct UpdaterState {
    session: tokio::sync::Mutex<Option<Session>>,
    cancel: Arc<AtomicBool>,
    busy: AtomicBool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckStatus {
    UpToDate,
    Available,
    /// 此构建没有内嵌更新公钥
    Disabled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub status: CheckStatus,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub release_url: Option<String>,
    /// 能否在应用内完成更新；否则界面引导到发布页手动下载
    pub in_app: bool,
    pub unsupported_reason: Option<UnsupportedReason>,
    pub summary: Option<PlanSummary>,
}

/// 前端按 `code` 选择本地化提示，`message` 只用于日志与问题反馈
#[derive(Debug, Serialize)]
pub struct UpdateCommandError {
    code: &'static str,
    message: String,
}

impl From<UpdateError> for UpdateCommandError {
    fn from(error: UpdateError) -> Self {
        Self {
            code: error.code(),
            message: error.to_string(),
        }
    }
}

impl UpdateCommandError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

type CommandResult<T> = Result<T, UpdateCommandError>;

fn fetcher(cancel: Arc<AtomicBool>) -> CommandResult<HttpFetcher> {
    let user_agent = format!("NeriPlayer-Desktop/{}", env!("CARGO_PKG_VERSION"));
    Ok(HttpFetcher::new(
        HttpFetcher::default_client(&user_agent)?,
        cancel,
    ))
}

fn current_layout() -> Result<InstallLayout, UnsupportedReason> {
    let exe = std::env::current_exe().map_err(|_| UnsupportedReason::Unknown)?;
    // Windows 上 current_exe 可能带 \\?\ 前缀，规范化后与安装目录一致
    let exe = exe.canonicalize().unwrap_or(exe);
    layout::detect(&exe, &Environment::current())
}

#[tauri::command]
pub async fn update_check(state: State<'_, UpdaterState>) -> CommandResult<UpdateCheck> {
    let mut result = UpdateCheck {
        status: CheckStatus::Disabled,
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        latest_version: None,
        release_url: None,
        in_app: false,
        unsupported_reason: None,
        summary: None,
    };
    let Some(public_key) = public_key() else {
        return Ok(result);
    };
    let manifest = neri_update::fetch_manifest(
        &fetcher(Arc::new(AtomicBool::new(false)))?,
        MANIFEST_URL,
        public_key,
    )
    .await?;
    if manifest.build_epoch <= current_build_epoch() {
        result.status = CheckStatus::UpToDate;
        *state.session.lock().await = None;
        return Ok(result);
    }
    result.status = CheckStatus::Available;
    result.latest_version = Some(manifest.version.clone());
    result.release_url = Some(manifest.release_url.clone());

    if state.busy.load(Ordering::Acquire) {
        // 正在下载时再次检查只刷新版本信息，不打断进行中的会话
        return Ok(result);
    }
    match prepare_session(manifest).await {
        Ok(session) => {
            result.in_app = true;
            result.summary = Some(session.plan.summary());
            *state.session.lock().await = Some(session);
        }
        Err(reason) => {
            result.unsupported_reason = reason;
            *state.session.lock().await = None;
        }
    }
    Ok(result)
}

/// 识别安装形态并比对本地文件；返回 Err(None) 表示清单里没有本平台的包
async fn prepare_session(manifest: ReleaseManifest) -> Result<Session, Option<UnsupportedReason>> {
    let layout = current_layout().map_err(Some)?;
    let release = manifest
        .platform(&neri_update::platform_key())
        .map_err(|_| None)?
        .clone();
    // 逐个文件算 SHA-256，AppImage 有上百 MB，放到阻塞线程里
    tokio::task::spawn_blocking(move || {
        let plan = build_plan(&release, &FsLocalState { layout: &layout });
        Session {
            version: manifest.version,
            layout,
            release,
            plan,
            staged: false,
        }
    })
    .await
    .map_err(|_| Some(UnsupportedReason::Unknown))
}

/// 清除 busy 标记；下载无论成功、失败还是取消都要放行下一次
struct BusyGuard<'a>(&'a AtomicBool);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[tauri::command]
pub async fn update_download(
    app: AppHandle,
    state: State<'_, UpdaterState>,
) -> CommandResult<PlanSummary> {
    if state.busy.swap(true, Ordering::AcqRel) {
        return Err(UpdateCommandError::new(
            "busy",
            "an update download is already running",
        ));
    }
    let _busy = BusyGuard(&state.busy);
    let (version, layout, release, plan) = {
        let session = state.session.lock().await;
        let session = session
            .as_ref()
            .ok_or_else(|| UpdateCommandError::new("noUpdate", "check for updates first"))?;
        (
            session.version.clone(),
            session.layout.clone(),
            session.release.clone(),
            session.plan.clone(),
        )
    };
    state.cancel.store(false, Ordering::Release);
    let staging = layout.staging_dir();
    apply::prepare_staging(&staging, &version)?;

    let fetcher = fetcher(state.cancel.clone())?;
    let mut last_emit: Option<Instant> = None;
    let mut on_progress = |progress: Progress| {
        let finished = progress.downloaded >= progress.total;
        if finished || last_emit.is_none_or(|at| at.elapsed() >= PROGRESS_INTERVAL) {
            last_emit = Some(Instant::now());
            let _ = app.emit(PROGRESS_EVENT, progress);
        }
    };
    let summary = plan.summary();
    on_progress(Progress {
        downloaded: 0,
        total: summary.download_bytes,
    });
    stage(
        &layout,
        &release,
        &plan,
        &staging,
        &fetcher,
        &state.cancel,
        &mut on_progress,
    )
    .await
    .inspect_err(|error| log::warn!(target: "update", "download failed: {error}"))?;

    if let Some(session) = state.session.lock().await.as_mut() {
        if session.version == version {
            session.staged = true;
        }
    }
    log::info!(target: "update", "update {version} staged ({} bytes)", summary.download_bytes);
    Ok(summary)
}

#[tauri::command]
pub fn update_cancel(state: State<'_, UpdaterState>) {
    state.cancel.store(true, Ordering::Release);
}

#[tauri::command]
pub async fn update_install(app: AppHandle, state: State<'_, UpdaterState>) -> CommandResult<()> {
    let session = state
        .session
        .lock()
        .await
        .take()
        .ok_or_else(|| UpdateCommandError::new("noUpdate", "no staged update"))?;
    if !session.staged {
        let message = "download the update before installing";
        *state.session.lock().await = Some(session);
        return Err(UpdateCommandError::new("noUpdate", message));
    }
    let version = session.version.clone();
    tokio::task::spawn_blocking(move || {
        apply::apply(
            &session.layout,
            &session.plan,
            &session.layout.staging_dir(),
        )
    })
    .await
    .map_err(|error| UpdateCommandError::new("unknown", error.to_string()))?
    .inspect_err(|error| log::error!(target: "update", "install failed: {error}"))?;

    record_installed_version(&version);
    log::info!(target: "update", "update {version} installed, restarting");
    app.request_restart();
    Ok(())
}

/// 启动时清理上次更新让位出来的旧文件
pub fn cleanup_after_update() {
    if let Ok(layout) = current_layout() {
        apply::cleanup_leftovers(&layout);
    }
}

/// 「应用和功能」里显示的版本号由 NSIS 写在注册表里，原地更新后同步过去
#[cfg(target_os = "windows")]
fn record_installed_version(version: &str) {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    use winreg::RegKey;

    const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\NeriPlayer";
    let result = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(UNINSTALL_KEY, KEY_SET_VALUE)
        .and_then(|key| key.set_value("DisplayVersion", &version));
    if let Err(error) = result {
        log::warn!(target: "update", "update DisplayVersion: {error}");
    }
}

#[cfg(not(target_os = "windows"))]
fn record_installed_version(_version: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_key_file_ignores_comments() {
        // 仓库里的公钥文件只允许注释或一行 Base64 公钥
        if let Some(key) = public_key() {
            assert_eq!(
                key.len(),
                44,
                "Ed25519 public key in Base64 is 44 characters"
            );
        }
        assert!(current_build_epoch() > 0);
    }
}
