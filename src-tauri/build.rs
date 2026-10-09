use std::env;
use std::fs;
use std::process::Command;
use std::time::{Duration, SystemTime};

const BUILD_EPOCH_ENV: &str = "NERI_BUILD_EPOCH";
const BUILD_GIT_SHA_ENV: &str = "NERI_BUILD_GIT_SHA";
const BUILD_UUID_ENV: &str = "NERI_BUILD_UUID";
const VERSION_TIMEZONE_OFFSET_SECONDS: u64 = 8 * 60 * 60;

fn main() {
    println!("cargo:rerun-if-env-changed={BUILD_EPOCH_ENV}");
    println!("cargo:rerun-if-env-changed={BUILD_GIT_SHA_ENV}");
    println!("cargo:rerun-if-env-changed={BUILD_UUID_ENV}");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../src");
    println!("cargo:rerun-if-changed=../package.json");
    println!("cargo:rerun-if-changed=../pnpm-lock.yaml");
    emit_git_rerun_paths();

    let build_time = resolve_build_time();
    let git_revision = resolve_git_revision();
    let build_version = format!("{}.{}", git_revision, format_version_timestamp(&build_time));
    let uuid = resolve_build_uuid();
    let timestamp = format_build_timestamp(&build_time);

    println!("cargo:rustc-env=BUILD_UUID={}", uuid);
    println!("cargo:rustc-env=BUILD_TIMESTAMP={}", timestamp);
    println!("cargo:rustc-env=BUILD_VERSION={}", build_version);
    // 方便 CI 从编译日志回收本次构建 UUID
    println!("cargo:warning=NeriPlayer build metadata: uuid={uuid} version={build_version}");

    build_ffmpeg_shim();
    tauri_build::try_build(tauri_attributes()).expect("tauri-build failed");
}

/// 托盘与菜单引入了只有 Common Controls v6 才导出的函数。tauri-build 只给应用本体嵌清单，
/// 测试程序没有清单会在加载时就失败（STATUS_ENTRYPOINT_NOT_FOUND），所以改为给所有链接目标统一嵌入
fn tauri_attributes() -> tauri_build::Attributes {
    let windows_msvc = env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows")
        && env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|target_env| target_env == "msvc");
    if !windows_msvc {
        return tauri_build::Attributes::new();
    }
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let manifest = std::path::Path::new(&manifest_dir).join("windows-app-manifest.xml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    tauri_build::Attributes::new()
        .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest())
}

/// 编译 FFmpeg 运行时加载垫片：只依赖仓库里固定版本的头文件，不需要构建机上装 FFmpeg
fn build_ffmpeg_shim() {
    println!("cargo:rerun-if-changed=native/ffmpeg/neri_ffmpeg.cpp");
    println!("cargo:rerun-if-changed=native/ffmpeg/include");
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let include = std::path::Path::new(&manifest_dir).join("native/ffmpeg/include");
    let mut build = cc::Build::new();
    build.cpp(true).file("native/ffmpeg/neri_ffmpeg.cpp");
    // cc 默认开 /W4 或 -Wall -Wextra；FFmpeg 头文件按外部头处理，告警只针对垫片自己的代码
    if build.get_compiler().is_like_msvc() {
        build
            .flag("/std:c++17")
            .flag("/utf-8")
            .flag("/external:W0")
            .flag(format!("/external:I{}", include.display()));
    } else {
        build
            .flag("-std=c++17")
            .flag("-isystem")
            .flag(include.display().to_string());
    }
    build.compile("neri_ffmpeg");
    if env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "linux") {
        println!("cargo:rustc-link-lib=dl");
    }
}

fn resolve_build_time() -> SystemTime {
    for variable in [BUILD_EPOCH_ENV, "SOURCE_DATE_EPOCH"] {
        let Ok(raw_value) = env::var(variable) else {
            continue;
        };

        if let Ok(epoch_seconds) = raw_value.trim().parse::<u64>() {
            if let Some(build_time) =
                SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(epoch_seconds))
            {
                return build_time;
            }
        }

        println!("cargo:warning=Ignoring invalid {variable} value: {raw_value}");
    }

    SystemTime::now()
}

fn resolve_git_revision() -> String {
    for variable in [BUILD_GIT_SHA_ENV, "GITHUB_SHA"] {
        if let Ok(value) = env::var(variable) {
            if let Some(revision) = normalize_git_revision(&value) {
                return revision;
            }
            println!("cargo:warning=Ignoring invalid {variable} value: {value}");
        }
    }

    git_output(&["rev-parse", "--short=7", "HEAD"])
        .and_then(|value| normalize_git_revision(&value))
        .unwrap_or_else(|| "no_commit".to_string())
}

fn resolve_build_uuid() -> String {
    if let Ok(value) = env::var(BUILD_UUID_ENV) {
        if let Some(uuid) = normalize_build_uuid(&value) {
            return uuid;
        }
        println!("cargo:warning=Ignoring invalid {BUILD_UUID_ENV} value: {value}");
    }

    uuid_v4()
}

fn normalize_build_uuid(value: &str) -> Option<String> {
    let uuid = value.trim().to_ascii_lowercase();
    let mut parts = uuid.split('-');
    let expected_lengths = [8usize, 4, 4, 4, 12];
    for expected in expected_lengths {
        let part = parts.next()?;
        if part.len() != expected || !part.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
    }
    if parts.next().is_some() {
        return None;
    }
    Some(uuid)
}

fn normalize_git_revision(value: &str) -> Option<String> {
    let revision = value.trim();
    if revision.len() < 7 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }

    revision.get(..7).map(str::to_ascii_lowercase)
}

fn emit_git_rerun_paths() {
    let Some(head_path) = git_output(&["rev-parse", "--git-path", "HEAD"]) else {
        return;
    };
    println!("cargo:rerun-if-changed={head_path}");

    let Ok(head) = fs::read_to_string(&head_path) else {
        return;
    };
    let Some(reference) = head.trim().strip_prefix("ref: ") else {
        return;
    };
    if let Some(reference_path) = git_output(&["rev-parse", "--git-path", reference]) {
        println!("cargo:rerun-if-changed={reference_path}");
    }
}

fn git_output(arguments: &[&str]) -> Option<String> {
    let output = Command::new("git").args(arguments).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn format_build_timestamp(build_time: &SystemTime) -> String {
    let Some(seconds) = unix_seconds(build_time) else {
        return "unknown".to_string();
    };
    let (year, month, day, hours, minutes, seconds) = timestamp_parts(seconds);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        year, month, day, hours, minutes, seconds
    )
}

fn format_version_timestamp(build_time: &SystemTime) -> String {
    let seconds = unix_seconds(build_time)
        .unwrap_or_default()
        .saturating_add(VERSION_TIMEZONE_OFFSET_SECONDS);
    let (_, month, day, hours, minutes, _) = timestamp_parts(seconds);
    format!("{:02}{:02}{:02}{:02}", month, day, hours, minutes)
}

fn unix_seconds(build_time: &SystemTime) -> Option<u64> {
    build_time
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

fn timestamp_parts(seconds: u64) -> (u64, u64, u64, u64, u64, u64) {
    let days = seconds / 86400;
    let time_of_day = seconds % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
    let (year, month, day) = days_to_ymd(days);
    (year, month, day, hours, minutes, seconds)
}

/// 简易 UUID v4 生成（不依赖外部 crate）
fn uuid_v4() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    SystemTime::now().hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    let h1 = hasher.finish();

    let mut hasher2 = DefaultHasher::new();
    h1.hash(&mut hasher2);
    std::thread::current().id().hash(&mut hasher2);
    let h2 = hasher2.finish();

    let bytes = [
        (h1 >> 56) as u8,
        (h1 >> 48) as u8,
        (h1 >> 40) as u8,
        (h1 >> 32) as u8,
        (h1 >> 24) as u8,
        (h1 >> 16) as u8,
        ((h1 >> 8) as u8 & 0x0f) | 0x40, // version 4
        h1 as u8,
        ((h2 >> 56) as u8 & 0x3f) | 0x80, // variant 1
        (h2 >> 48) as u8,
        (h2 >> 40) as u8,
        (h2 >> 32) as u8,
        (h2 >> 24) as u8,
        (h2 >> 16) as u8,
        (h2 >> 8) as u8,
        h2 as u8,
    ];

    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11],
        bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

/// 将自 epoch 以来的天数转换为 (年, 月, 日)
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    let mut y = 1970;
    let mut remaining = days;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let months = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 0;
    for days_in_month in months {
        if remaining < days_in_month {
            break;
        }
        remaining -= days_in_month;
        m += 1;
    }
    (y, m + 1, remaining + 1)
}

fn is_leap(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}
