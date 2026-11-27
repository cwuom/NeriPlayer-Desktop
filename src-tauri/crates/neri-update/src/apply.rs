//! 把暂存好的文件换进安装位置。任何一步失败都回滚到原样，不留下半新半旧的安装

use std::path::{Path, PathBuf};

use crate::error::{IoContext, UpdateError, UpdateResult};
use crate::layout::{join_relative, InstallLayout};
use crate::plan::{staged_path, FileAction, Source, UpdatePlan};
use crate::stage::set_mode;

const ASIDE_SUFFIX: &str = ".neri-old";
const VERSION_MARKER: &str = ".version";

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
        InstallLayout::Directory { root } => remove_aside_files(root, 0),
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

fn remove_aside_files(dir: &Path, depth: usize) {
    // 安装目录只有两三层，限制深度防止误入用户挂进来的大目录
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            remove_aside_files(&path, depth + 1);
        } else if is_aside_name(&entry.file_name().to_string_lossy()) {
            if let Err(error) = std::fs::remove_file(&path) {
                log::warn!(target: "update", "remove {}: {error}", path.display());
            }
        }
    }
}

fn is_aside_name(name: &str) -> bool {
    name.ends_with(ASIDE_SUFFIX) || name.contains(&format!("{ASIDE_SUFFIX}."))
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

/// 让位文件的名字；上次的让位文件还被占用删不掉时换个序号
fn aside_path(target: &Path) -> UpdateResult<PathBuf> {
    for index in 0..16 {
        let candidate = if index == 0 {
            with_suffix(target, ASIDE_SUFFIX)
        } else {
            with_suffix(target, &format!("{ASIDE_SUFFIX}.{index}"))
        };
        if std::fs::symlink_metadata(&candidate).is_err()
            || std::fs::remove_file(&candidate).is_ok()
        {
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

/// macOS：在 .app 旁边拼出完整的新包（未变化的文件从旧包复制），再两次改名整体换位。
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
    std::fs::rename(bundle, &old).at(bundle)?;
    if let Err(error) = std::fs::rename(&fresh, bundle) {
        let _ = std::fs::rename(&old, bundle);
        return Err(UpdateError::io(bundle, error));
    }
    Ok(())
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

    #[test]
    fn recognises_aside_names() {
        assert!(is_aside_name("app.exe.neri-old"));
        assert!(is_aside_name("app.exe.neri-old.3"));
        assert!(!is_aside_name("app.exe"));
    }
}
