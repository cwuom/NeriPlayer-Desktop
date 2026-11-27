//! 暂存：把需要变化的文件下载、解码、校验后写进暂存目录。全部成功之前不动安装目录，
//! 中途失败或取消时已校验的暂存文件保留，下次继续时直接复用

use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use crate::codec::{apply_patch, decompress_full, sha256_file, sha256_hex};
use crate::error::{IoContext, UpdateError, UpdateResult};
use crate::layout::InstallLayout;
use crate::manifest::{BlobRef, PlatformRelease};
use crate::plan::{staged_path, Source, UpdatePlan};

/// 从 blob 里取一段字节。`on_bytes` 每收到一块数据就回报这块的长度
pub trait RangeFetcher {
    fn fetch(
        &self,
        blob: &BlobRef,
        offset: u64,
        length: u64,
        on_bytes: &mut (dyn FnMut(u64) + Send),
    ) -> impl Future<Output = UpdateResult<Vec<u8>>> + Send;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub downloaded: u64,
    pub total: u64,
}

pub async fn stage<F: RangeFetcher + Sync>(
    layout: &InstallLayout,
    release: &PlatformRelease,
    plan: &UpdatePlan,
    staging: &Path,
    fetcher: &F,
    cancel: &AtomicBool,
    on_progress: &mut (dyn FnMut(Progress) + Send),
) -> UpdateResult<()> {
    let total = plan.summary().download_bytes;
    let mut downloaded = 0u64;
    std::fs::create_dir_all(staging).at(staging)?;

    for action in plan.changed() {
        if cancel.load(Ordering::Relaxed) {
            return Err(UpdateError::Cancelled);
        }
        let entry = &action.entry;
        let (blob, offset, length) = match action.source {
            Source::Unchanged | Source::Link => continue,
            Source::Full => (&release.blob, entry.offset, entry.length),
            Source::Patch {
                set,
                offset,
                length,
            } => (&release.patches[set].blob, offset, length),
        };
        let target = staged_path(staging, &entry.path);
        if staged_matches(&target, &entry.sha256) {
            downloaded += length;
            on_progress(Progress { downloaded, total });
            continue;
        }

        let base_downloaded = downloaded;
        let mut report = |bytes: u64| {
            downloaded = (downloaded + bytes).min(base_downloaded + length);
            on_progress(Progress { downloaded, total });
        };
        let encoded = fetcher.fetch(blob, offset, length, &mut report).await?;
        if encoded.len() as u64 != length {
            return Err(UpdateError::Corrupted {
                path: entry.path.clone(),
            });
        }
        let content = match action.source {
            Source::Patch { .. } => {
                let base_path = layout.local_path(&entry.path);
                let base = std::fs::read(&base_path).at(&base_path)?;
                apply_patch(&base, &encoded, entry.size)
            }
            _ => decompress_full(&encoded, entry.size),
        }
        .map_err(|error| {
            log::warn!(target: "update", "decode {}: {error}", entry.path);
            UpdateError::Corrupted {
                path: entry.path.clone(),
            }
        })?;
        if sha256_hex(&content) != entry.sha256 {
            return Err(UpdateError::Corrupted {
                path: entry.path.clone(),
            });
        }
        write_staged(&target, &content, entry.mode)?;
        downloaded = base_downloaded + length;
        on_progress(Progress { downloaded, total });
    }
    Ok(())
}

fn staged_matches(path: &Path, sha256: &str) -> bool {
    path.is_file() && sha256_file(path).is_ok_and(|hash| hash == sha256)
}

fn write_staged(path: &Path, content: &[u8], mode: Option<u32>) -> UpdateResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).at(parent)?;
    }
    // 先写临时名再改名，断电或崩溃不会留下一个哈希对不上却叫正式名字的文件
    let mut partial = path.as_os_str().to_owned();
    partial.push(".neri-partial");
    let partial = std::path::PathBuf::from(partial);
    std::fs::write(&partial, content).at(&partial)?;
    set_mode(&partial, mode)?;
    std::fs::rename(&partial, path).at(path)
}

#[cfg(unix)]
pub(crate) fn set_mode(path: &Path, mode: Option<u32>) -> UpdateResult<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(mode) = mode {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777)).at(path)?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn set_mode(_path: &Path, _mode: Option<u32>) -> UpdateResult<()> {
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::codec::{compress_full, make_patch};
    use crate::manifest::{FileEntry, PatchEntry, PatchSet};
    use crate::plan::{build_plan, FsLocalState};
    use std::collections::HashMap;

    /// 内存里的 blob 表，按 URL 取数据
    pub(crate) struct MemoryFetcher(pub HashMap<String, Vec<u8>>);

    impl RangeFetcher for MemoryFetcher {
        async fn fetch(
            &self,
            blob: &BlobRef,
            offset: u64,
            length: u64,
            on_bytes: &mut (dyn FnMut(u64) + Send),
        ) -> UpdateResult<Vec<u8>> {
            let data = &self.0[&blob.url];
            let slice = data[offset as usize..(offset + length) as usize].to_vec();
            on_bytes(length);
            Ok(slice)
        }
    }

    pub(crate) fn block_on<T>(future: impl Future<Output = T>) -> T {
        // 测试里的 future 不会真正挂起，手动轮询一次即可，省得为单测引入运行时
        let waker = std::task::Waker::noop();
        let mut context = std::task::Context::from_waker(waker);
        let mut future = std::pin::pin!(future);
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(value) => value,
            std::task::Poll::Pending => panic!("test future unexpectedly pending"),
        }
    }

    fn entry(path: &str, content: &[u8], offset: u64, length: u64) -> FileEntry {
        FileEntry {
            path: path.into(),
            size: content.len() as u64,
            sha256: sha256_hex(content),
            mode: Some(0o755),
            link: None,
            offset,
            length,
        }
    }

    #[test]
    fn stages_patched_and_full_files_and_resumes() {
        let install = tempfile::tempdir().unwrap();
        let old_exe = vec![7u8; 50_000];
        let mut new_exe = old_exe.clone();
        new_exe[100..110].copy_from_slice(b"new-binary");
        std::fs::write(install.path().join("app.exe"), &old_exe).unwrap();

        let new_dll = b"fresh dll".to_vec();
        let exe_full = compress_full(&new_exe).unwrap();
        let dll_full = compress_full(&new_dll).unwrap();
        let mut blob = exe_full.clone();
        blob.extend_from_slice(&dll_full);
        let patch = make_patch(&old_exe, &new_exe).unwrap();

        let release = PlatformRelease {
            blob: BlobRef {
                url: "full".into(),
                size: blob.len() as u64,
                sha256: sha256_hex(&blob),
            },
            files: vec![
                entry("app.exe", &new_exe, 0, exe_full.len() as u64),
                entry(
                    "ffmpeg/new.dll",
                    &new_dll,
                    exe_full.len() as u64,
                    dll_full.len() as u64,
                ),
            ],
            patches: vec![PatchSet {
                from_version: "1.0.0".into(),
                blob: BlobRef {
                    url: "patch".into(),
                    size: patch.len() as u64,
                    sha256: sha256_hex(&patch),
                },
                entries: vec![PatchEntry {
                    path: "app.exe".into(),
                    base_sha256: sha256_hex(&old_exe),
                    offset: 0,
                    length: patch.len() as u64,
                }],
            }],
        };
        let layout = InstallLayout::Directory {
            root: install.path().to_path_buf(),
        };
        let plan = build_plan(&release, &FsLocalState { layout: &layout });
        let fetcher = MemoryFetcher(HashMap::from([
            ("full".into(), blob),
            ("patch".into(), patch),
        ]));
        let staging = layout.staging_dir();
        let cancel = AtomicBool::new(false);

        let mut seen = Vec::new();
        block_on(stage(
            &layout,
            &release,
            &plan,
            &staging,
            &fetcher,
            &cancel,
            &mut |p| seen.push(p),
        ))
        .unwrap();
        assert_eq!(std::fs::read(staging.join("app.exe")).unwrap(), new_exe);
        assert_eq!(
            std::fs::read(staging.join("ffmpeg").join("new.dll")).unwrap(),
            new_dll
        );
        let total = plan.summary().download_bytes;
        assert_eq!(
            seen.last(),
            Some(&Progress {
                downloaded: total,
                total
            })
        );

        // 再跑一次：暂存文件已校验通过，不再请求网络
        let empty = MemoryFetcher(HashMap::new());
        block_on(stage(
            &layout,
            &release,
            &plan,
            &staging,
            &empty,
            &cancel,
            &mut |_| {},
        ))
        .unwrap();

        cancel.store(true, Ordering::Relaxed);
        let result = block_on(stage(
            &layout,
            &release,
            &plan,
            &staging,
            &empty,
            &cancel,
            &mut |_| {},
        ));
        assert!(matches!(result, Err(UpdateError::Cancelled)));
    }

    #[test]
    fn rejects_content_that_does_not_match_the_manifest() {
        let install = tempfile::tempdir().unwrap();
        let frame = compress_full(b"tampered").unwrap();
        let mut file = entry("app.exe", b"expected", 0, frame.len() as u64);
        file.size = 8;
        let release = PlatformRelease {
            blob: BlobRef {
                url: "full".into(),
                size: frame.len() as u64,
                sha256: sha256_hex(&frame),
            },
            files: vec![file],
            patches: vec![],
        };
        let layout = InstallLayout::Directory {
            root: install.path().to_path_buf(),
        };
        let plan = build_plan(&release, &FsLocalState { layout: &layout });
        let fetcher = MemoryFetcher(HashMap::from([("full".into(), frame)]));
        let result = block_on(stage(
            &layout,
            &release,
            &plan,
            &layout.staging_dir(),
            &fetcher,
            &AtomicBool::new(false),
            &mut |_| {},
        ));
        assert!(matches!(result, Err(UpdateError::Corrupted { .. })));
        assert!(!layout.staging_dir().join("app.exe").exists());
    }
}
