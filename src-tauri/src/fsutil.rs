//! 原子落盘工具：所有持久化 JSON/二进制文件必须经由此模块写入，
//! 避免崩溃/断电/并发时留下半截文件（半截 JSON 解析失败会被上层
//! 误判为空数据并二次覆盖，造成永久数据丢失）

use std::io::Write;
use std::path::{Path, PathBuf};

/// 生成同目录临时文件路径（rename 跨文件系统会失败，必须同目录）
fn temp_path_for(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("file"));
    name.push(".tmp-write");
    path.with_file_name(name)
}

fn write_temp_then_rename(path: &Path, contents: &[u8], mode: Option<u32>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let tmp = temp_path_for(path);
    let result = (|| -> std::io::Result<()> {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        if let Some(mode) = mode {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(mode);
        }
        #[cfg(not(unix))]
        let _ = mode;
        let mut file = opts.open(&tmp)?;
        file.write_all(contents)?;
        // fsync 确保 rename 之前数据已持久化，否则掉电可能出现"新文件名旧内容为空"
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// 原子写入（temp + fsync + rename）
pub fn atomic_write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    write_temp_then_rename(path.as_ref(), contents.as_ref(), None)
}

/// 原子写入且权限收敛为 0600（Unix；其余平台等同 atomic_write）
pub fn atomic_write_private(
    path: impl AsRef<Path>,
    contents: impl AsRef<[u8]>,
) -> std::io::Result<()> {
    write_temp_then_rename(path.as_ref(), contents.as_ref(), Some(0o600))
}

/// 把疑似损坏的文件移到旁边保留现场（`<name>.corrupt-<秒级时间戳>`），
/// 供用户/开发者恢复；移动失败不视为致命错误
pub fn quarantine_corrupt_file(path: impl AsRef<Path>) -> Option<PathBuf> {
    let path = path.as_ref();
    if !path.exists() {
        return None;
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("file"));
    name.push(format!(".corrupt-{ts}"));
    let dest = path.with_file_name(name);
    match std::fs::rename(path, &dest) {
        Ok(()) => Some(dest),
        Err(e) => {
            log::warn!("[fsutil] failed to quarantine corrupt file {path:?}: {e}");
            None
        }
    }
}

/// 把文件修改时间刷新为现在（缓存命中时标记最近使用）；失败不影响调用方
pub fn touch_modified(path: impl AsRef<Path>) {
    if let Ok(file) = std::fs::OpenOptions::new().write(true).open(path.as_ref()) {
        let _ = file.set_modified(std::time::SystemTime::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_and_replaces() {
        let dir = std::env::temp_dir().join(format!("neri-fsutil-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("data.json");
        atomic_write(&path, b"one").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"one");
        atomic_write(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        // 目录中不应残留临时文件
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-write"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quarantine_moves_file_aside() {
        let dir = std::env::temp_dir().join(format!("neri-fsutil-q-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("broken.json");
        std::fs::write(&path, b"garbage").unwrap();
        let moved = quarantine_corrupt_file(&path).expect("should quarantine");
        assert!(!path.exists());
        assert!(moved.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn private_write_sets_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("neri-fsutil-p-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("secret.bin");
        atomic_write_private(&path, b"s").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
