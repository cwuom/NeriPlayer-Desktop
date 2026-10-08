//! 统一日志基础设施
//!
//! 前后端共用同一套日志出口：后端 `log` 宏与前端 `@tauri-apps/plugin-log`
//! 转发的记录都经由此处的 `tauri-plugin-log` 插件输出。每条日志统一带
//! `时间 [作用域] [级别] 内容` 格式；作用域取自 `log` 记录的 target
//! （模块路径或调用方显式指定的语义 target，如 `player`、`sync`）。
//!
//! 文件持久化受设置项控制：`log_to_file` 为真时追加 Folder target，
//! 落到 `<data_dir>/NeriPlayer/logs/`。受插件能力限制，文件 target 只能
//! 在启动时构建，运行时切换开关需重启生效；日志级别经 `set_runtime_level` 即时生效。

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use log::LevelFilter;
use serde::Serialize;
use tauri::Runtime;
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

/// 应用内日志查看的内存环形缓冲
///
/// 与文件日志无关、无论开关始终收集：调试页要能看到「现在正在发生什么」，
/// 而文件日志默认是关的。容量按条数封顶，单条消息截断，杜绝无界增长。
const RECENT_LOG_CAP: usize = 1_500;
const RECENT_LOG_MESSAGE_CAP: usize = 2_000;

static RECENT_LOGS: Mutex<VecDeque<RecentLogEntry>> = Mutex::new(VecDeque::new());

#[derive(Clone, Serialize)]
pub struct RecentLogEntry {
    pub timestamp_ms: i64,
    pub level: String,
    pub target: String,
    pub message: String,
}

/// 敏感信息打码：值替换为 `[REDACTED]`，键名保留
///
/// cookie_store 等第三方 crate 在 Debug 级别会把完整 cookie 值打进日志，
/// 而内存日志环会随崩溃报告与「导出诊断报告」明文带出。除 `level_for`
/// 降噪外，在入环处再做一道纵深防御——两道闸任何一道失守都不致泄漏。
fn redact_sensitive(text: &str) -> String {
    // Cookie/Authorization 整段打码：值里可能含空格（如 `Bearer xxx`）或
    // 多个分号分隔的属性，保守起见冒号/等号之后整行替换
    static FULL_LINE: OnceLock<regex::Regex> = OnceLock::new();
    // 键值打码：已知敏感键名后的单个值（分隔符前）替换
    static SENSITIVE_KV: OnceLock<regex::Regex> = OnceLock::new();
    let full_line = FULL_LINE.get_or_init(|| {
        regex::Regex::new(r"(?i)\b((?:set-)?cookie|authorization)\b(\s*[:=]\s*)[^\r\n]+")
            .expect("full-line redact regex")
    });
    let kv = SENSITIVE_KV.get_or_init(|| {
        regex::Regex::new(
            r#"(?i)\b(MUSIC_U|MUSIC_A|SESSDATA|bili_jct|SAPISID|APISID|__Secure-[\w-]+|__Host-[\w-]+|access_token|refresh_token|id_token|csrf_token|__csrf|NMTID)\b(\s*[=:]\s*)[^;,\s"'&]+"#,
        )
        .expect("sensitive kv redact regex")
    });
    let pass1 = full_line.replace_all(text, "$1$2[REDACTED]");
    let pass2 = kv.replace_all(&pass1, "$1$2[REDACTED]");
    pass2.into_owned()
}

fn push_recent(record: &log::Record, message: &std::fmt::Arguments) {
    let mut text = redact_sensitive(&message.to_string());
    if text.len() > RECENT_LOG_MESSAGE_CAP {
        let mut cut = RECENT_LOG_MESSAGE_CAP;
        // 不能在多字节字符中间截断
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push('…');
    }
    let entry = RecentLogEntry {
        timestamp_ms: chrono::Local::now().timestamp_millis(),
        level: record.level().to_string(),
        target: record.target().to_string(),
        message: text,
    };
    if let Ok(mut logs) = RECENT_LOGS.lock() {
        if logs.len() >= RECENT_LOG_CAP {
            logs.pop_front();
        }
        logs.push_back(entry);
    }
}

/// 取最近日志（新→旧），可按最低级别过滤
pub fn recent_logs(limit: usize, min_level: Option<log::Level>) -> Vec<RecentLogEntry> {
    let Ok(logs) = RECENT_LOGS.lock() else {
        return Vec::new();
    };
    logs.iter()
        .rev()
        .filter(|entry| match min_level {
            Some(min) => log::Level::from_str_loose(&entry.level)
                .map(|level| level <= min)
                .unwrap_or(true),
            None => true,
        })
        .take(limit)
        .cloned()
        .collect()
}

/// 宽松解析级别字符串（环形缓冲里存的是 Display 输出）
trait LevelFromLoose {
    fn from_str_loose(value: &str) -> Option<log::Level>;
}
impl LevelFromLoose for log::Level {
    fn from_str_loose(value: &str) -> Option<log::Level> {
        value.parse().ok()
    }
}

/// 崩溃报告目录
pub fn crash_dir() -> PathBuf {
    log_dir().join("crashes")
}

/// 安装 panic 钩子：崩溃现场落盘（对齐 Android 的崩溃收集）
///
/// 报告含版本、系统、panic 位置、回溯与最近 80 条日志尾巴——
/// 复现困难的崩溃全靠这份现场。写盘失败静默（崩溃路径不能再抛）。
pub fn install_panic_hook(app_version: &'static str) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let tail = recent_logs(80, None)
            .into_iter()
            .rev()
            .map(|entry| {
                format!(
                    "{} [{}] [{}] {}",
                    entry.timestamp_ms, entry.target, entry.level, entry.message
                )
            })
            .collect::<Vec<_>>()
            .join("
");
        let report = format!(
            "NeriPlayer Desktop crash report
             version: {}
             os: {} {}
             time: {}
             panic: {}

             backtrace:
{}

             recent logs (oldest first):
{}
",
            app_version,
            std::env::consts::OS,
            std::env::consts::ARCH,
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            info,
            backtrace,
            tail,
        );
        let dir = crash_dir();
        let _ = std::fs::create_dir_all(&dir);
        let name = format!(
            "crash-{}.txt",
            chrono::Local::now().format("%Y%m%d-%H%M%S%.3f")
        );
        let _ = std::fs::write(dir.join(name), report);
        // 写完即轮转：debug 命令或循环崩溃不能把磁盘写满
        rotate_crash_reports(&dir);
        default_hook(info);
    }));
}

/// 崩溃目录最多保留的报告份数，超出删最旧
const MAX_CRASH_REPORTS: usize = 20;

/// 崩溃报告轮转：仅保留最新 `MAX_CRASH_REPORTS` 个 `crash-*.txt`
///
/// 文件名内嵌 `%Y%m%d-%H%M%S%.3f` 时间戳，字典序即时间序，
/// 不依赖 mtime（用户拷贝/恢复文件会污染 mtime）
fn rotate_crash_reports(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            (name.starts_with("crash-") && name.ends_with(".txt"))
                .then_some((name, entry.path()))
        })
        .collect();
    if files.len() <= MAX_CRASH_REPORTS {
        return;
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let excess = files.len() - MAX_CRASH_REPORTS;
    for (_, path) in files.into_iter().take(excess) {
        let _ = std::fs::remove_file(path);
    }
}

/// 日志文件所在目录：与项目其余数据一致，统一落到 `<data_dir>/NeriPlayer/logs`
const LOG_SUBDIR: &str = "NeriPlayer";
const LOG_DIR_NAME: &str = "logs";
/// 日志文件基名（插件自动追加 `.log` 与滚动日期后缀）
const LOG_FILE_NAME: &str = "neri-player";
/// 单文件大小上限（字节），超过后按滚动策略切分
const MAX_LOG_FILE_SIZE: u128 = 8 * 1024 * 1024;
/// 保留最近的日志文件个数
const KEEP_LOG_FILES: usize = 8;

/// 解析设置中的字符串级别为 `LevelFilter`；无法识别时回退 `Info`
pub fn parse_level(level: &str) -> LevelFilter {
    match level.trim().to_ascii_lowercase().as_str() {
        "off" => LevelFilter::Off,
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        _ => LevelFilter::Info,
    }
}

/// 日志文件目录（不保证已创建，插件写入前会自行创建）
pub fn log_dir() -> PathBuf {
    let mut path = dirs_next::data_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push(LOG_SUBDIR);
    path.push(LOG_DIR_NAME);
    path
}

/// 应用标识（与 tauri.conf.json 的 identifier 保持一致）
const APP_IDENTIFIER: &str = "moe.ouom.neriplayer.desktop";
/// tauri-plugin-store 的设置文件名
const SETTINGS_STORE_FILE: &str = "settings.json";
/// settings.json 中的顶层键
const SETTINGS_STORE_KEY: &str = "appSettings";

/// 启动期日志配置：`(是否写文件, 级别)`
pub struct BootstrapConfig {
    pub log_to_file: bool,
    pub level: LevelFilter,
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        // 默认不落文件、级别 Info，与 AppSettings 默认值保持一致
        Self {
            log_to_file: false,
            level: LevelFilter::Info,
        }
    }
}

/// 在插件初始化前读取持久化设置，决定是否写文件及日志级别。
///
/// 此时还没有 Tauri app handle，因此直接按 plugin-store 的落盘路径
/// （`config_dir/{identifier}/settings.json`）解析。任何缺失或解析失败
/// 都回退到默认配置，绝不 panic。
pub fn load_bootstrap_config() -> BootstrapConfig {
    let mut path = match dirs_next::config_dir() {
        Some(p) => p,
        None => return BootstrapConfig::default(),
    };
    path.push(APP_IDENTIFIER);
    path.push(SETTINGS_STORE_FILE);

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return BootstrapConfig::default(),
    };
    let root: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return BootstrapConfig::default(),
    };
    let settings = &root[SETTINGS_STORE_KEY];

    let defaults = BootstrapConfig::default();
    let log_to_file = settings["logToFile"]
        .as_bool()
        .unwrap_or(defaults.log_to_file);
    let level = settings["logLevel"]
        .as_str()
        .map(parse_level)
        .unwrap_or(defaults.level);

    BootstrapConfig {
        log_to_file,
        level,
    }
}

/// 统一日志格式：`2026-07-19 12:00:00.123 [scope] [LEVEL] message`
fn format_record(
    out: tauri_plugin_log::fern::FormatCallback,
    message: &std::fmt::Arguments,
    record: &log::Record,
) {
    push_recent(record, message);
    // 使用本地时区的墙钟时间，毫秒精度，便于与用户操作时间对齐
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    // 写入 sink（文件/stdout）前同样打码：push_recent 的打码只作用于内存环副本，
    // 这里若直写原始 message，logToFile 开启时 Warn+ 级别带 cookie/Authorization 会明文落盘（AU-04）
    let redacted = redact_sensitive(&message.to_string());
    out.finish(format_args!(
        "{} [{}] [{}] {}",
        ts,
        record.target(),
        record.level(),
        redacted
    ));
}

/// 级别对应的 ANSI 前景色（仅用于 stdout 彩色输出）
fn level_ansi(level: log::Level) -> &'static str {
    match level {
        log::Level::Error => "\x1b[31m", // 红
        log::Level::Warn => "\x1b[33m",  // 黄
        log::Level::Info => "\x1b[32m",  // 绿
        log::Level::Debug => "\x1b[36m", // 青
        log::Level::Trace => "\x1b[90m", // 亮黑（灰）
    }
}

/// 彩色日志格式：时间戳灰、作用域青、级别按级配色。
/// 仅用于 stdout；写文件时改用 `format_record` 以免 ANSI 转义污染日志文件。
fn format_record_colored(
    out: tauri_plugin_log::fern::FormatCallback,
    message: &std::fmt::Arguments,
    record: &log::Record,
) {
    push_recent(record, message);
    const RESET: &str = "\x1b[0m";
    const DIM: &str = "\x1b[90m";
    const SCOPE: &str = "\x1b[36m";
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    // 与 format_record 一致：sink 前打码，防明文凭据落 stdout（AU-04）
    let redacted = redact_sensitive(&message.to_string());
    out.finish(format_args!(
        "{dim}{ts}{reset} {scope}[{target}]{reset} {lc}[{level}]{reset} {msg}",
        dim = DIM,
        reset = RESET,
        scope = SCOPE,
        ts = ts,
        target = record.target(),
        lc = level_ansi(record.level()),
        level = record.level(),
        msg = redacted,
    ));
}

/// 运行时日志级别（`LevelFilter as usize`）。
///
/// 插件的 dispatch 级别在构建时就固定了，而前端日志经 `log::logger().log()`
/// 直接进入 dispatch、不看 `log::max_level`，所以只调 `set_max_level`
/// 对前端日志无效。dispatch 按 Trace 构建，再用这个原子值逐条过滤。
static RUNTIME_LEVEL: AtomicUsize = AtomicUsize::new(LevelFilter::Info as usize);

fn level_from_usize(value: usize) -> LevelFilter {
    match value {
        0 => LevelFilter::Off,
        1 => LevelFilter::Error,
        2 => LevelFilter::Warn,
        3 => LevelFilter::Info,
        4 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    }
}

pub fn runtime_level() -> LevelFilter {
    level_from_usize(RUNTIME_LEVEL.load(Ordering::Relaxed))
}

/// 立即切换日志级别：前后端日志都按新级别过滤
pub fn set_runtime_level(level: LevelFilter) {
    RUNTIME_LEVEL.store(level as usize, Ordering::Relaxed);
    log::set_max_level(level);
}

/// 构建日志插件。
///
/// - `log_to_file`：为真时追加文件 target，否则仅输出到 stdout
/// - `level`：启动时的日志级别，之后可由 `set_runtime_level` 调整；
///   插件初始化会把 `log::max_level` 设为 Trace，宿主须在 setup 里再调用一次 `set_runtime_level`
pub fn build_plugin<R: Runtime>(
    log_to_file: bool,
    level: LevelFilter,
) -> tauri::plugin::TauriPlugin<R> {
    RUNTIME_LEVEL.store(level as usize, Ordering::Relaxed);
    let mut targets = vec![Target::new(TargetKind::Stdout)];
    if log_to_file {
        targets.push(Target::new(TargetKind::Folder {
            path: log_dir(),
            file_name: Some(LOG_FILE_NAME.to_string()),
        }));
    }

    let builder = tauri_plugin_log::Builder::new()
        .level(LevelFilter::Trace)
        .filter(|metadata| metadata.level() <= runtime_level())
        // 降噪：第三方库的 target 前缀过滤到 Warn 以上，避免刷屏
        .level_for("hyper", LevelFilter::Warn)
        .level_for("reqwest", LevelFilter::Warn)
        .level_for("rustls", LevelFilter::Warn)
        .level_for("tao", LevelFilter::Warn)
        .level_for("wry", LevelFilter::Warn)
        // cookie_store 在 Debug 级别会打印非 Secure cookie 的完整值（P0 隐私），
        // 必须压到 Warn；打码兜底见 redact_sensitive
        .level_for("cookie_store", LevelFilter::Warn)
        .level_for("hyper_util", LevelFilter::Info)
        .level_for("tungstenite", LevelFilter::Info)
        .level_for("tokio_tungstenite", LevelFilter::Info)
        .timezone_strategy(TimezoneStrategy::UseLocal)
        .max_file_size(MAX_LOG_FILE_SIZE)
        .rotation_strategy(RotationStrategy::KeepSome(KEEP_LOG_FILES))
        .clear_format();

    // 写文件时用纯文本（避免 ANSI 转义落盘），仅 stdout 时用彩色输出
    let builder = if log_to_file {
        builder.format(format_record)
    } else {
        builder.format(format_record_colored)
    };

    builder.targets(targets).build()
}

#[cfg(test)]
mod tests {
    use super::redact_sensitive;

    #[test]
    fn runtime_level_round_trips_every_filter() {
        use log::LevelFilter;
        for level in [
            LevelFilter::Off,
            LevelFilter::Error,
            LevelFilter::Warn,
            LevelFilter::Info,
            LevelFilter::Debug,
            LevelFilter::Trace,
        ] {
            assert_eq!(super::level_from_usize(level as usize), level);
        }
        assert!(log::Level::Warn <= LevelFilter::Info);
        assert!(log::Level::Error > LevelFilter::Off, "Off drops every record");
    }

    /// Set-Cookie 行必须整段打码，键名保留（cookie_store P0 泄漏路径）
    #[test]
    fn set_cookie_lines_are_redacted() {
        let input = "cookie_store Set-Cookie: __Secure-YNID=abc123; Path=/; HttpOnly";
        let output = redact_sensitive(input);
        assert!(output.contains("Set-Cookie: [REDACTED]"), "got: {output}");
        assert!(!output.contains("abc123"));

        let input = "request header cookie=MUSIC_U=deadbeef; NMTID=xyz";
        let output = redact_sensitive(input);
        assert!(!output.contains("deadbeef"), "got: {output}");
        assert!(!output.contains("xyz"));
    }

    /// Authorization 值可能含空格（Bearer xxx），必须整行打码
    #[test]
    fn authorization_header_is_fully_redacted() {
        let output = redact_sensitive("Authorization: Bearer eyJhbGciOi.secret.sig");
        assert!(output.contains("Authorization: [REDACTED]"), "got: {output}");
        assert!(!output.contains("eyJhbGciOi"));
    }

    /// 已知敏感键的键值对：值打码、键名保留
    #[test]
    fn sensitive_key_values_are_redacted_keeping_key_names() {
        for (input, key) in [
            ("got MUSIC_U=0011223344556677", "MUSIC_U"),
            ("SESSDATA: 8f%2Cxyz refreshed", "SESSDATA"),
            ("query SAPISID=abcd/efgh done", "SAPISID"),
            ("token access_token=ya29.a0Af persists", "access_token"),
            ("jar has __Secure-3PSID=g.a000secret", "__Secure-3PSID"),
        ] {
            let output = redact_sensitive(input);
            assert!(output.contains(key), "key name lost in: {output}");
            assert!(output.contains("[REDACTED]"), "value kept in: {output}");
        }
        assert!(!redact_sensitive("got MUSIC_U=0011223344556677").contains("0011"));
    }

    /// 普通日志不受影响
    #[test]
    fn normal_lines_pass_through_unchanged() {
        for input in [
            "player started track id=42 duration=200000",
            "sync finished: 3 playlists updated",
            "cookie jar loaded from disk",
        ] {
            assert_eq!(redact_sensitive(input), input);
        }
    }
}
