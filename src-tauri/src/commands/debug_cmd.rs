// 调试台命令：应用内日志查看、诊断报告导出、崩溃报告管理
// 能力对齐 Android 调试页（NPLogger 文件日志 + 崩溃收集 + 导出）

use crate::error::{AppError, AppResult};
use crate::logging;
use serde::Serialize;

/// 拉取最近日志（新→旧）
#[tauri::command]
pub fn get_recent_logs(limit: Option<usize>, min_level: Option<String>) -> Vec<logging::RecentLogEntry> {
    let level = min_level
        .as_deref()
        .and_then(|value| value.parse::<log::Level>().ok());
    logging::recent_logs(limit.unwrap_or(300).min(1_500), level)
}

/// 导出诊断报告：构建信息 + 系统 + 全部内存日志 + 崩溃文件清单
///
/// 落到日志目录并返回路径，前端用 opener 在文件管理器里定位。
/// 不弹保存对话框：报告是给「出问题时一键收集发给开发者」用的，
/// 路径固定可预期比多一步选择更重要。
#[tauri::command]
pub fn export_debug_report(app: tauri::AppHandle) -> AppResult<String> {
    let package = app.package_info();
    let logs = logging::recent_logs(1_500, None);
    let mut body = String::with_capacity(64 * 1024);
    body.push_str(&format!(
        "NeriPlayer Desktop debug report\nversion: {}\nos: {} {}\ntime: {}\n\n",
        package.version,
        std::env::consts::OS,
        std::env::consts::ARCH,
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
    ));
    body.push_str("== crash reports ==\n");
    for report in list_crash_reports().unwrap_or_default() {
        body.push_str(&format!("{} ({} bytes)\n", report.file_name, report.size_bytes));
    }
    body.push_str("\n== recent logs (oldest first) ==\n");
    for entry in logs.into_iter().rev() {
        body.push_str(&format!(
            "{} [{}] [{}] {}\n",
            entry.timestamp_ms, entry.target, entry.level, entry.message
        ));
    }

    let dir = logging::log_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|error| AppError::Other(format!("create log dir: {error}")))?;
    let path = dir.join(format!(
        "debug-report-{}.txt",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    std::fs::write(&path, body)
        .map_err(|error| AppError::Other(format!("write report: {error}")))?;
    Ok(path.to_string_lossy().to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioEngineStats {
    pub output: crate::audio::metrics::OutputMetricsSnapshot,
    /// 最近的起播/seek 首帧耗时，新的在前
    pub recent_starts: Vec<crate::audio::metrics::StartTiming>,
}

/// 播放引擎指标：输出回调计数、欠载与最近的首帧耗时
///
/// 只读、不含敏感信息，供基线测量脚本经 CDP 拉取
#[tauri::command]
pub fn audio_engine_stats() -> AudioEngineStats {
    AudioEngineStats {
        output: crate::audio::metrics::OUTPUT_METRICS.snapshot(),
        recent_starts: crate::audio::metrics::recent_starts(),
    }
}

/// 在系统文件管理器中定位文件（macOS Finder / Windows 资源管理器 / Linux 文件管理器）
#[tauri::command]
pub fn reveal_in_file_manager(path: String) -> AppResult<()> {
    tauri_plugin_opener::reveal_item_in_dir(std::path::PathBuf::from(path))
        .map_err(|error| AppError::Other(format!("reveal failed: {error}")))
}

#[derive(Serialize)]
pub struct CrashReportInfo {
    pub file_name: String,
    pub size_bytes: u64,
    pub modified_ms: i64,
}

/// 崩溃文件名白名单校验
///
/// 名字来自前端参数，读/删前必须证明它就是本目录下的崩溃报告：
/// 固定前后缀、无路径分隔符，杜绝 ../ 穿越到日志目录之外。
fn validated_crash_path(file_name: &str) -> AppResult<std::path::PathBuf> {
    let valid = file_name.starts_with("crash-")
        && file_name.ends_with(".txt")
        && !file_name.contains(['/', '\\'])
        && !file_name.contains("..");
    if !valid {
        return Err(AppError::Other(format!("invalid crash report name: {file_name}")));
    }
    Ok(logging::crash_dir().join(file_name))
}

/// 崩溃报告清单（新→旧）
#[tauri::command]
pub fn list_crash_reports() -> AppResult<Vec<CrashReportInfo>> {
    let dir = logging::crash_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        // 目录不存在 = 没崩溃过，不是错误
        return Ok(Vec::new());
    };
    let mut reports: Vec<CrashReportInfo> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("crash-") || !name.ends_with(".txt") {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            let modified_ms = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as i64)
                .unwrap_or(0);
            Some(CrashReportInfo {
                file_name: name,
                size_bytes: metadata.len(),
                modified_ms,
            })
        })
        .collect();
    reports.sort_by_key(|report| std::cmp::Reverse(report.modified_ms));
    Ok(reports)
}

/// 读取单份崩溃报告正文
#[tauri::command]
pub fn read_crash_report(file_name: String) -> AppResult<String> {
    let path = validated_crash_path(&file_name)?;
    std::fs::read_to_string(&path)
        .map_err(|error| AppError::Other(format!("read crash report: {error}")))
}

/// 清空全部崩溃报告
#[tauri::command]
pub fn clear_crash_reports() -> AppResult<usize> {
    let mut removed = 0usize;
    for report in list_crash_reports()? {
        let path = validated_crash_path(&report.file_name)?;
        if std::fs::remove_file(path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// 后端侧开发者模式校验
///
/// debug 命令对任意 webview JS 都是可 invoke 的，只靠前端隐藏入口不构成
/// 鉴权；崩溃触发类命令必须在后端确认 `devModeEnabled` 已开启。
fn ensure_dev_mode(app: &tauri::AppHandle) -> AppResult<()> {
    let enabled = crate::settings::store::load_settings(app)
        .map(|result| result.settings.dev_mode_enabled)
        .unwrap_or(false);
    if enabled {
        Ok(())
    } else {
        Err(AppError::Other(
            "developer mode is disabled; enable it in settings first".into(),
        ))
    }
}

/// 测试异常触发（对齐 Android 调试页的「测试异常」）
///
/// 用来端到端验证崩溃收集：panic 落盘的报告应出现在崩溃列表里。
/// 后端校验开发者模式后才执行；`panic_command` 的 panic 发生在
/// `spawn_blocking` 的 tokio 阻塞线程上——panic 钩子照常落盘，JoinError
/// 被本命令捕获转成 IPC 错误返回，进程存活。
#[tauri::command]
pub async fn debug_trigger_crash(app: tauri::AppHandle, kind: String) -> AppResult<String> {
    ensure_dev_mode(&app)?;
    match kind.as_str() {
        "handled" => {
            log::error!(
                target: "debug-test",
                "handled test error: 这是调试页触发的受控错误，用于验证日志链路",
            );
            Ok("handled error logged".into())
        }
        "panic_command" => {
            // 不能直接在命令上下文 panic：tauri 命令路径没有 catch_unwind，
            // macOS 上 panic 穿过 objc FFI 边界可能直接 abort 整个进程。
            // 放进阻塞线程后 panic 只终结该线程，tokio 把它包成 JoinError
            let joined = tauri::async_runtime::spawn_blocking(|| {
                panic!("debug test panic (command thread)");
            })
            .await;
            match joined {
                // panic 必然让 join 失败；走到 Ok 说明 panic 未发生
                Ok(()) => Err(AppError::Other(
                    "debug test panic did not fire (unexpected)".into(),
                )),
                Err(_) => Err(AppError::Other(
                    "debug test panic (command thread): captured, process alive".into(),
                )),
            }
        }
        "panic_thread" => {
            std::thread::Builder::new()
                .name("debug-test-panic".into())
                .spawn(|| panic!("debug test panic (worker thread)"))
                .map_err(|error| AppError::Other(format!("spawn failed: {error}")))?;
            Ok("panic thread spawned".into())
        }
        other => Err(AppError::Other(format!("unknown crash kind: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 崩溃文件名来自前端，必须挡住路径穿越与任意文件读取
    #[test]
    fn crash_names_outside_the_whitelist_are_rejected() {
        for bad in [
            "../settings.json",
            "crash-../../etc/passwd.txt",
            "crash-a/b.txt",
            "crash-a\\b.txt",
            "notes.txt",
            "crash-1.log",
        ] {
            assert!(validated_crash_path(bad).is_err(), "should reject {bad}");
        }
        assert!(validated_crash_path("crash-20260726-120000.txt").is_ok());
    }
}
