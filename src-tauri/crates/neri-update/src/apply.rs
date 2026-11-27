//! 把暂存好的文件换进安装位置。任何一步失败都回滚到原样，不留下半新半旧的安装

use std::path::{Component, Path, PathBuf};

use crate::codec::sha256_file;
use crate::error::{IoContext, UpdateError, UpdateResult};
use crate::layout::{join_relative, InstallLayout};
use crate::plan::{staged_path, FileAction, Source, UpdatePlan};
use crate::stage::set_mode;

const ASIDE_SUFFIX: &str = ".neri-old";
const VERSION_MARKER: &str = ".version";
/// 记录更新器实际创建过的让位文件（相对安装根目录，一行一个）。清理只认这份记录，
/// 不按文件名猜，免得删掉用户自己恰好叫 `*.neri-old` 的文件；NSIS 卸载程序也读它
pub const ASIDE_JOURNAL: &str = ".neri-update-asides";

/// 暂存目录只服务一个目标版本；版本变了就清空，避免旧版本的残留文件越积越多
pub fn prepare_staging(staging: &Path, version: &str) -> UpdateResult<()> {
    let marker = staging.join(VERSION_MARKER);
    if std::fs::read_to_string(&marker).ok().as_deref() != Some(version) {
        if staging.exists() {
            std::fs::remove_dir_all(staging).at(staging)?;
        }
        std::fs::create_dir_all(staging).at(staging)?;
        std::fs::write(&marker, version).at(&marker)?;
    }
    Ok(())
}

pub fn apply(layout: &InstallLayout, plan: &UpdatePlan, staging: &Path) -> UpdateResult<()> {
    match layout {
        InstallLayout::Directory { root } => apply_directory(root, plan, staging)?,
        InstallLayout::MacBundle { bundle } => apply_bundle(bundle, plan, staging)?,
        InstallLayout::SingleFile { path } => apply_single(path, plan, staging)?,
    }
    // 暂存文件已经全部移走，剩下的只是标记和空目录
    let _ = std::fs::remove_dir_all(staging);
    Ok(())
}

/// 启动时清理上次更新让位出来的旧文件；正在运行的旧进程已经退出，这时才删得掉
pub fn cleanup_leftovers(layout: &InstallLayout) {
    match layout {
        InstallLayout::Directory { root } => remove_journaled_asides(root),
        InstallLayout::MacBundle { bundle } => {
            for suffix in [".neri-old", ".neri-new"] {
                let dir = sibling(bundle, suffix);
                if dir.exists() {
                    if let Err(error) = std::fs::remove_dir_all(&dir) {
                        log::warn!(target: "update", "remove {}: {error}", dir.display());
                    }
                }
            }
        }
        InstallLayout::SingleFile { .. } => {}
    }
}

/// 删除记录里的让位文件；仍被占用删不掉的留在记录里，下次启动再试
fn remove_journaled_asides(root: &Path) {
    let journal = root.join(ASIDE_JOURNAL);
    let Ok(text) = std::fs::read_to_string(&journal) else {
        return;
    };
    let mut remaining = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let relative = Path::new(line);
        // 记录被改坏时也只在安装目录内动手
        if !relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        {
            continue;
        }
        match std::fs::remove_file(root.join(relative)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                log::warn!(target: "update", "remove {line}: {error}");
                remaining.push(line);
            }
        }
    }
    let result = if remaining.is_empty() {
        std::fs::remove_file(&journal)
    } else {
        std::fs::write(&journal, remaining.join("\n") + "\n")
    };
    if let Err(error) = result {
        log::warn!(target: "update", "update {}: {error}", journal.display());
    }
}

fn record_asides(root: &Path, swaps: &[Swap]) -> UpdateResult<()> {
    use std::io::Write;
    let lines: String = swaps
        .iter()
        .filter_map(|swap| swap.aside.as_deref())
        .filter_map(|aside| aside.strip_prefix(root).ok())
        .map(|relative| format!("{}\n", relative.display()))
        .collect();
    if lines.is_empty() {
        return Ok(());
    }
    let journal = root.join(ASIDE_JOURNAL);
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&journal)
        .and_then(|mut file| file.write_all(lines.as_bytes()))
        .at(&journal)
}

/// `.../NeriPlayer.app` + `.neri-old` → `.../.NeriPlayer.app.neri-old`
fn sibling(bundle: &Path, suffix: &str) -> PathBuf {
    let name = bundle.file_name().unwrap_or_default().to_string_lossy();
    bundle.with_file_name(format!(".{name}{suffix}"))
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// 让位文件的名字：只用空闲的名字，已存在的一律跳过、绝不删除（可能是上次仍被占用的
/// 让位文件，也可能是用户的文件）；没有空闲名字时在动任何安装文件之前报错
fn aside_path(target: &Path) -> UpdateResult<PathBuf> {
    for index in 0..16 {
        let candidate = if index == 0 {
            with_suffix(target, ASIDE_SUFFIX)
        } else {
            with_suffix(target, &format!("{ASIDE_SUFFIX}.{index}"))
        };
        if std::fs::symlink_metadata(&candidate).is_err() {
            return Ok(candidate);
        }
    }
    Err(UpdateError::Other(format!(
        "no free aside name for {}",
        target.display()
    )))
}

struct Swap {
    target: PathBuf,
    aside: Option<PathBuf>,
    placed: bool,
}

/// Windows 不能覆盖正在运行的 exe 和已加载的 dll，但允许改名：旧文件先改名让位，
/// 新文件再改名到正式位置，两步都是同盘 rename
fn apply_directory(root: &Path, plan: &UpdatePlan, staging: &Path) -> UpdateResult<()> {
    let mut swaps: Vec<Swap> = Vec::new();
    for action in plan.changed() {
        let target = join_relative(root, &action.entry.path);
        if let Err(error) = swap_in(&target, action, staging, &mut swaps) {
            rollback(&swaps);
            return Err(error);
        }
    }
    // 新文件已全部就位；记录写不进去只影响以后清理旧文件，不能为此回滚
    if let Err(error) = record_asides(root, &swaps) {
        log::warn!(target: "update", "record set-aside files: {error}");
    }
    Ok(())
}

fn swap_in(
    target: &Path,
    action: &FileAction,
    staging: &Path,
    swaps: &mut Vec<Swap>,
) -> UpdateResult<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).at(parent)?;
    }
    let aside = if std::fs::symlink_metadata(target).is_ok() {
        let aside = aside_path(target)?;
        std::fs::rename(target, &aside).at(target)?;
        Some(aside)
    } else {
        None
    };
    swaps.push(Swap {
        target: target.to_path_buf(),
        aside,
        placed: false,
    });
    place(target, action, staging)?;
    if let Some(last) = swaps.last_mut() {
        last.placed = true;
    }
    Ok(())
}

fn rollback(swaps: &[Swap]) {
    for swap in swaps.iter().rev() {
        if swap.placed {
            let _ = std::fs::remove_file(&swap.target);
        }
        if let Some(aside) = &swap.aside {
            if let Err(error) = std::fs::rename(aside, &swap.target) {
                log::error!(target: "update", "rollback {}: {error}", swap.target.display());
            }
        }
    }
}

/// 把一个变化的文件放到 `dest`：暂存文件改名过去，或按清单建符号链接
fn place(dest: &Path, action: &FileAction, staging: &Path) -> UpdateResult<()> {
    match action.source {
        Source::Link => make_link(action.entry.link.as_deref().unwrap_or_default(), dest),
        Source::Patch { .. } | Source::Full => {
            let staged = staged_path(staging, &action.entry.path);
            std::fs::rename(&staged, dest).at(&staged)?;
            // 可执行位决定 macOS/Linux 能否启动，不依赖暂存时是否设对
            set_mode(dest, action.entry.mode)
        }
        Source::Unchanged => Ok(()),
    }
}

#[cfg(unix)]
fn make_link(target: &str, dest: &Path) -> UpdateResult<()> {
    std::os::unix::fs::symlink(target, dest).at(dest)
}

#[cfg(not(unix))]
fn make_link(_target: &str, dest: &Path) -> UpdateResult<()> {
    Err(UpdateError::Other(format!(
        "symlinks are not supported here: {}",
        dest.display()
    )))
}

/// macOS：在 .app 旁边拼出完整的新包（未变化的文件从旧包复制并复验），再整体换位。
/// 新包与发布包逐字节一致，ad-hoc 签名的封印保持有效；旧包里多余的文件随旧包一起丢弃
fn apply_bundle(bundle: &Path, plan: &UpdatePlan, staging: &Path) -> UpdateResult<()> {
    let fresh = sibling(bundle, ".neri-new");
    if fresh.exists() {
        std::fs::remove_dir_all(&fresh).at(&fresh)?;
    }
    std::fs::create_dir_all(&fresh).at(&fresh)?;
    let built = build_bundle(bundle, &fresh, plan, staging);
    if let Err(error) = built {
        let _ = std::fs::remove_dir_all(&fresh);
        return Err(error);
    }

    let old = sibling(bundle, ".neri-old");
    if old.exists() {
        std::fs::remove_dir_all(&old).at(&old)?;
    }
    // 原子交换：原路径在任何时刻都是一个完整的包，进程中途退出也不会让应用消失
    match exchange_paths(&fresh, bundle) {
        Ok(()) => {
            // 交换后 fresh 指向旧包；改名失败也无妨，启动清理会删掉 .neri-new
            let _ = std::fs::rename(&fresh, &old);
            return Ok(());
        }
        Err(error) if !exchange_unsupported(&error) => {
            let _ = std::fs::remove_dir_all(&fresh);
            return Err(UpdateError::io(bundle, error));
        }
        Err(_) => {}
    }
    // 文件系统不支持交换时退回两次改名，失败则还原
    std::fs::rename(bundle, &old).at(bundle)?;
    if let Err(error) = std::fs::rename(&fresh, bundle) {
        let _ = std::fs::rename(&old, bundle);
        return Err(UpdateError::io(bundle, error));
    }
    Ok(())
}

/// 用 renamex_np(RENAME_SWAP) 原子交换两个目录项
#[cfg(target_os = "macos")]
fn exchange_paths(a: &Path, b: &Path) -> std::io::Result<()> {
    use std::ffi::{c_char, c_int, c_uint, CString};
    use std::os::unix::ffi::OsStrExt;

    const RENAME_SWAP: c_uint = 0x0000_0002;
    extern "C" {
        fn renamex_np(from: *const c_char, to: *const c_char, flags: c_uint) -> c_int;
    }
    let a = CString::new(a.as_os_str().as_bytes())?;
    let b = CString::new(b.as_os_str().as_bytes())?;
    // SAFETY: 两个参数都是有效的、以 NUL 结尾的路径字符串，调用期间一直存活
    let result = unsafe { renamex_np(a.as_ptr(), b.as_ptr(), RENAME_SWAP) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn exchange_paths(_a: &Path, _b: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// 不支持交换（老文件系统、网络卷）时返回 ENOTSUP 或 EINVAL，此时可以退回两次改名
fn exchange_unsupported(error: &std::io::Error) -> bool {
    const EINVAL: i32 = 22;
    error.kind() == std::io::ErrorKind::Unsupported || error.raw_os_error() == Some(EINVAL)
}

fn build_bundle(
    bundle: &Path,
    fresh: &Path,
    plan: &UpdatePlan,
    staging: &Path,
) -> UpdateResult<()> {
    for action in &plan.actions {
        let dest = join_relative(fresh, &action.entry.path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).at(parent)?;
        }
        match (&action.source, &action.entry.link) {
            (Source::Unchanged, Some(target)) => make_link(target, &dest)?,
            (Source::Unchanged, None) => {
                let source = join_relative(bundle, &action.entry.path);
                std::fs::copy(&source, &dest).at(&source)?;
                // 规划之后旧包里的文件可能又被改过；复制品必须仍与清单一致，否则新包签名会失效
                if sha256_file(&dest)? != action.entry.sha256 {
                    return Err(UpdateError::Corrupted {
                        path: action.entry.path.clone(),
                    });
                }
                set_mode(&dest, action.entry.mode)?;
            }
            _ => place(&dest, action, staging)?,
        }
    }
    Ok(())
}

fn apply_single(path: &Path, plan: &UpdatePlan, staging: &Path) -> UpdateResult<()> {
    let [action] = plan.actions.as_slice() else {
        return Err(UpdateError::InvalidManifest(
            "single-file package must contain one file".into(),
        ));
    };
    if action.source == Source::Unchanged {
        return Ok(());
    }
    let staged = staged_path(staging, &action.entry.path);
    set_mode(&staged, Some(action.entry.mode.unwrap_or(0o755)))?;
    // Linux 上 rename 覆盖正在运行的 AppImage 是安全的：旧进程持有的是旧 inode
    std::fs::rename(&staged, path).at(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::sha256_hex;
    use crate::manifest::FileEntry;

    fn action(path: &str, content: &[u8], source: Source) -> FileAction {
        FileAction {
            entry: FileEntry {
                path: path.into(),
                size: content.len() as u64,
                sha256: sha256_hex(content),
                mode: Some(0o644),
                link: None,
                offset: 0,
                length: 1,
            },
            source,
        }
    }

    fn stage_file(staging: &Path, path: &str, content: &[u8]) {
        let dest = staged_path(staging, path);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(dest, content).unwrap();
    }

    #[test]
    fn directory_update_sets_old_files_aside_and_cleans_them_later() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("app.exe"), b"old").unwrap();
        std::fs::write(root.path().join("keep.dll"), b"same").unwrap();
        let layout = InstallLayout::Directory {
            root: root.path().to_path_buf(),
        };
        let staging = layout.staging_dir();
        prepare_staging(&staging, "1.1.0").unwrap();
        stage_file(&staging, "app.exe", b"new");
        stage_file(&staging, "ffmpeg/new.dll", b"dll");

        let plan = UpdatePlan {
            actions: vec![
                action("app.exe", b"new", Source::Full),
                action("keep.dll", b"same", Source::Unchanged),
                action("ffmpeg/new.dll", b"dll", Source::Full),
            ],
        };
        apply(&layout, &plan, &staging).unwrap();
        assert_eq!(std::fs::read(root.path().join("app.exe")).unwrap(), b"new");
        assert_eq!(
            std::fs::read(root.path().join("app.exe.neri-old")).unwrap(),
            b"old"
        );
        assert_eq!(
            std::fs::read(root.path().join("ffmpeg/new.dll")).unwrap(),
            b"dll"
        );
        assert!(!staging.exists());

        cleanup_leftovers(&layout);
        assert!(!root.path().join("app.exe.neri-old").exists());
        assert!(root.path().join("keep.dll").exists());
        assert!(!root.path().join(ASIDE_JOURNAL).exists());
    }

    #[test]
    fn cleanup_only_touches_files_the_updater_set_aside() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("app.exe"), b"old").unwrap();
        // 用户自己的文件恰好占用了让位名字，或者名字里带 .neri-old
        std::fs::write(root.path().join("app.exe.neri-old"), b"user").unwrap();
        std::fs::write(root.path().join("notes.neri-old.txt"), b"user").unwrap();
        let layout = InstallLayout::Directory {
            root: root.path().to_path_buf(),
        };
        let staging = layout.staging_dir();
        prepare_staging(&staging, "1.1.0").unwrap();
        stage_file(&staging, "app.exe", b"new");
        let plan = UpdatePlan {
            actions: vec![action("app.exe", b"new", Source::Full)],
        };
        apply(&layout, &plan, &staging).unwrap();
        assert_eq!(
            std::fs::read(root.path().join("app.exe.neri-old.1")).unwrap(),
            b"old",
            "an occupied aside name is skipped, never deleted"
        );

        // 记录被改坏指向安装目录外时不跟随
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("victim"), b"keep").unwrap();
        let journal = root.path().join(ASIDE_JOURNAL);
        let mut text = std::fs::read_to_string(&journal).unwrap();
        text.push_str(&format!("{}\n", outside.path().join("victim").display()));
        text.push_str("../victim\n");
        std::fs::write(&journal, text).unwrap();

        cleanup_leftovers(&layout);
        assert!(!root.path().join("app.exe.neri-old.1").exists());
        assert_eq!(
            std::fs::read(root.path().join("app.exe.neri-old")).unwrap(),
            b"user"
        );
        assert_eq!(
            std::fs::read(root.path().join("notes.neri-old.txt")).unwrap(),
            b"user"
        );
        assert!(outside.path().join("victim").exists());
    }

    #[test]
    fn directory_update_rolls_back_when_a_staged_file_is_missing() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.exe"), b"old-a").unwrap();
        std::fs::write(root.path().join("b.dll"), b"old-b").unwrap();
        let layout = InstallLayout::Directory {
            root: root.path().to_path_buf(),
        };
        let staging = layout.staging_dir();
        prepare_staging(&staging, "1.1.0").unwrap();
        stage_file(&staging, "a.exe", b"new-a");

        let plan = UpdatePlan {
            actions: vec![
                action("a.exe", b"new-a", Source::Full),
                action("b.dll", b"new-b", Source::Full),
            ],
        };
        assert!(apply(&layout, &plan, &staging).is_err());
        assert_eq!(std::fs::read(root.path().join("a.exe")).unwrap(), b"old-a");
        assert_eq!(std::fs::read(root.path().join("b.dll")).unwrap(), b"old-b");
        assert!(!root.path().join("a.exe.neri-old").exists());
    }

    #[test]
    fn staging_is_wiped_when_the_target_version_changes() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join(".neri-update");
        prepare_staging(&staging, "1.1.0").unwrap();
        std::fs::write(staging.join("leftover"), b"x").unwrap();
        prepare_staging(&staging, "1.1.0").unwrap();
        assert!(staging.join("leftover").exists());
        prepare_staging(&staging, "1.2.0").unwrap();
        assert!(!staging.join("leftover").exists());
    }

    #[cfg(unix)]
    #[test]
    fn bundle_update_swaps_in_an_exact_new_tree() {
        let parent = tempfile::tempdir().unwrap();
        let bundle = parent.path().join("NeriPlayer.app");
        std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        std::fs::write(bundle.join("Contents/MacOS/neri"), b"old-exe").unwrap();
        std::fs::write(bundle.join("Contents/Info.plist"), b"plist").unwrap();
        std::fs::write(bundle.join("Contents/obsolete"), b"gone").unwrap();
        let layout = InstallLayout::MacBundle {
            bundle: bundle.clone(),
        };
        let staging = layout.staging_dir();
        prepare_staging(&staging, "1.1.0").unwrap();
        stage_file(&staging, "Contents/MacOS/neri", b"new-exe");

        let mut link = action("Contents/Frameworks/libx.dylib", b"", Source::Link);
        link.entry.link = Some("libx.1.dylib".into());
        let mut exe = action(
            "Contents/MacOS/neri",
            b"new-exe",
            Source::Patch {
                set: 0,
                offset: 0,
                length: 1,
            },
        );
        exe.entry.mode = Some(0o755);
        let plan = UpdatePlan {
            actions: vec![
                exe,
                action("Contents/Info.plist", b"plist", Source::Unchanged),
                link,
            ],
        };
        apply(&layout, &plan, &staging).unwrap();

        assert_eq!(
            std::fs::read(bundle.join("Contents/MacOS/neri")).unwrap(),
            b"new-exe"
        );
        assert_eq!(
            std::fs::read(bundle.join("Contents/Info.plist")).unwrap(),
            b"plist"
        );
        assert!(!bundle.join("Contents/obsolete").exists());
        assert_eq!(
            std::fs::read_link(bundle.join("Contents/Frameworks/libx.dylib")).unwrap(),
            PathBuf::from("libx.1.dylib")
        );
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(bundle.join("Contents/MacOS/neri"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);

        assert!(sibling(&bundle, ".neri-old").exists());
        cleanup_leftovers(&layout);
        assert!(!sibling(&bundle, ".neri-old").exists());
    }

    #[cfg(unix)]
    #[test]
    fn single_file_update_replaces_the_appimage() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("NeriPlayer.AppImage");
        std::fs::write(&image, b"old").unwrap();
        let layout = InstallLayout::SingleFile {
            path: image.clone(),
        };
        let staging = layout.staging_dir();
        prepare_staging(&staging, "1.1.0").unwrap();
        stage_file(&staging, "NeriPlayer.AppImage", b"new");
        let mut file = action("NeriPlayer.AppImage", b"new", Source::Full);
        file.entry.mode = None;
        apply(
            &layout,
            &UpdatePlan {
                actions: vec![file],
            },
            &staging,
        )
        .unwrap();
        assert_eq!(std::fs::read(&image).unwrap(), b"new");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&image).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[cfg(unix)]
    #[test]
    fn bundle_update_stops_when_an_unchanged_file_was_modified_since_planning() {
        let parent = tempfile::tempdir().unwrap();
        let bundle = parent.path().join("NeriPlayer.app");
        std::fs::create_dir_all(bundle.join("Contents")).unwrap();
        std::fs::write(bundle.join("Contents/Info.plist"), b"edited after planning").unwrap();
        let layout = InstallLayout::MacBundle {
            bundle: bundle.clone(),
        };
        let staging = layout.staging_dir();
        prepare_staging(&staging, "1.1.0").unwrap();
        let plan = UpdatePlan {
            actions: vec![action("Contents/Info.plist", b"plist", Source::Unchanged)],
        };
        assert!(matches!(
            apply(&layout, &plan, &staging),
            Err(UpdateError::Corrupted { .. })
        ));
        assert_eq!(
            std::fs::read(bundle.join("Contents/Info.plist")).unwrap(),
            b"edited after planning"
        );
        assert!(!sibling(&bundle, ".neri-new").exists());
        assert!(!sibling(&bundle, ".neri-old").exists());
    }
}
