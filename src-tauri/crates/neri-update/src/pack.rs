//! 发布侧：扫描安装后的文件树，生成完整内容 blob、相对上一版的补丁 blob 和平台清单

use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::codec::{compress_full, decompress_full, make_patch, sha256_hex};
use crate::error::{IoContext, UpdateError, UpdateResult};
use crate::manifest::{
    validate_relative_path, BlobRef, FileEntry, PatchEntry, PatchSet, PlatformRelease,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeContent {
    File(PathBuf),
    Link(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    pub path: String,
    pub content: TreeContent,
    pub mode: Option<u32>,
}

/// 扫描文件树；`root` 是单个文件时（AppImage）清单里只有这一个文件，路径为文件名。
/// `exclude` 是要排除的相对路径，例如 NSIS 安装时现生成的 uninstall.exe
pub fn scan_tree(root: &Path, exclude: &[String]) -> UpdateResult<Vec<TreeEntry>> {
    let metadata = std::fs::symlink_metadata(root).at(root)?;
    let mut entries = Vec::new();
    if metadata.is_file() {
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| UpdateError::Other("tree has no file name".into()))?;
        entries.push(TreeEntry {
            path: name,
            content: TreeContent::File(root.into()),
            mode: unix_mode(&metadata),
        });
    } else {
        walk(root, "", &mut entries)?;
    }
    entries.retain(|entry| !exclude.iter().any(|skip| skip == &entry.path));
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    for entry in &entries {
        validate_relative_path(&entry.path).map_err(UpdateError::Other)?;
    }
    Ok(entries)
}

fn walk(dir: &Path, prefix: &str, entries: &mut Vec<TreeEntry>) -> UpdateResult<()> {
    for item in std::fs::read_dir(dir).at(dir)? {
        let item = item.at(dir)?;
        let path = item.path();
        let name = item.file_name().to_string_lossy().into_owned();
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let metadata = std::fs::symlink_metadata(&path).at(&path)?;
        if metadata.file_type().is_symlink() {
            let target = std::fs::read_link(&path).at(&path)?;
            entries.push(TreeEntry {
                path: relative,
                content: TreeContent::Link(target.to_string_lossy().into_owned()),
                mode: None,
            });
        } else if metadata.is_dir() {
            walk(&path, &relative, entries)?;
        } else {
            entries.push(TreeEntry {
                path: relative,
                content: TreeContent::File(path),
                mode: unix_mode(&metadata),
            });
        }
    }
    Ok(())
}

#[cfg(unix)]
fn unix_mode(metadata: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn unix_mode(_metadata: &std::fs::Metadata) -> Option<u32> {
    None
}

/// 上一个版本：它的平台清单与完整内容 blob，用来生成补丁
pub struct BaseRelease {
    pub version: String,
    pub release: PlatformRelease,
    pub blob: Vec<u8>,
}

impl BaseRelease {
    fn content_of(&self, path: &str) -> UpdateResult<Option<(String, Vec<u8>)>> {
        let Some(file) = self
            .release
            .files
            .iter()
            .find(|f| f.path == path && f.link.is_none())
        else {
            return Ok(None);
        };
        let start =
            usize::try_from(file.offset).map_err(|_| UpdateError::Other("offset".into()))?;
        let end = start
            + usize::try_from(file.length).map_err(|_| UpdateError::Other("length".into()))?;
        let frame = self.blob.get(start..end).ok_or_else(|| {
            UpdateError::Other(format!("base blob of {} is truncated", self.version))
        })?;
        let content = decompress_full(frame, file.size)?;
        if sha256_hex(&content) != file.sha256 {
            return Err(UpdateError::Corrupted {
                path: format!("{}@{}", path, self.version),
            });
        }
        Ok(Some((file.sha256.clone(), content)))
    }
}

pub struct PackRequest<'a> {
    /// 资产文件名前缀，例如 `NeriPlayer-1.1.0-windows-x86_64`
    pub asset_stem: &'a str,
    /// Release 附件下载地址前缀，例如 `https://github.com/o/r/releases/download/v1.1.0`
    pub base_url: &'a str,
    pub out_dir: &'a Path,
}

/// 补丁不比完整内容小一成以上就不值得多存一份
fn worth_patching(patch_len: usize, full_len: u64) -> bool {
    (patch_len as u64) * 10 < full_len * 9
}

pub fn pack(
    request: &PackRequest,
    entries: &[TreeEntry],
    bases: &[BaseRelease],
) -> UpdateResult<PlatformRelease> {
    std::fs::create_dir_all(request.out_dir).at(request.out_dir)?;
    let blob_name = format!("{}.update", request.asset_stem);
    let blob_path = request.out_dir.join(&blob_name);
    let mut blob = BlobWriter::create(&blob_path)?;
    let mut files = Vec::with_capacity(entries.len());
    let mut contents: HashMap<&str, Vec<u8>> = HashMap::new();

    for entry in entries {
        match &entry.content {
            TreeContent::Link(target) => files.push(FileEntry {
                path: entry.path.clone(),
                size: 0,
                sha256: sha256_hex(target.as_bytes()),
                mode: None,
                link: Some(target.clone()),
                offset: 0,
                length: 0,
            }),
            TreeContent::File(source) => {
                let data = std::fs::read(source).at(source)?;
                let (offset, length) = blob.append(&compress_full(&data)?)?;
                files.push(FileEntry {
                    path: entry.path.clone(),
                    size: data.len() as u64,
                    sha256: sha256_hex(&data),
                    mode: entry.mode,
                    link: None,
                    offset,
                    length,
                });
                contents.insert(&entry.path, data);
            }
        }
    }
    let blob = blob.finish(format!("{}/{blob_name}", request.base_url))?;

    let mut patches = Vec::new();
    for base in bases {
        if let Some(set) = pack_patches(request, base, &files, &contents)? {
            patches.push(set);
        }
    }
    Ok(PlatformRelease {
        blob,
        files,
        patches,
    })
}

fn pack_patches(
    request: &PackRequest,
    base: &BaseRelease,
    files: &[FileEntry],
    contents: &HashMap<&str, Vec<u8>>,
) -> UpdateResult<Option<PatchSet>> {
    let name = format!("{}.from-{}.patch", request.asset_stem, base.version);
    let path = request.out_dir.join(&name);
    let mut writer = BlobWriter::create(&path)?;
    let mut entries = Vec::new();
    for file in files.iter().filter(|f| f.link.is_none()) {
        let Some((base_sha256, old)) = base.content_of(&file.path)? else {
            continue;
        };
        if base_sha256 == file.sha256 {
            continue;
        }
        let patch = make_patch(&old, &contents[file.path.as_str()])?;
        if !worth_patching(patch.len(), file.length) {
            continue;
        }
        let (offset, length) = writer.append(&patch)?;
        entries.push(PatchEntry {
            path: file.path.clone(),
            base_sha256,
            offset,
            length,
        });
    }
    if entries.is_empty() {
        drop(writer);
        std::fs::remove_file(&path).at(&path)?;
        return Ok(None);
    }
    let blob = writer.finish(format!("{}/{name}", request.base_url))?;
    Ok(Some(PatchSet {
        from_version: base.version.clone(),
        blob,
        entries,
    }))
}

struct BlobWriter {
    path: PathBuf,
    file: File,
    hasher: sha2::Sha256,
    size: u64,
}

impl BlobWriter {
    fn create(path: &Path) -> UpdateResult<Self> {
        use sha2::Digest;
        Ok(Self {
            path: path.into(),
            file: File::create(path).at(path)?,
            hasher: sha2::Sha256::new(),
            size: 0,
        })
    }

    fn append(&mut self, frame: &[u8]) -> UpdateResult<(u64, u64)> {
        use sha2::Digest;
        self.file.write_all(frame).at(&self.path)?;
        self.hasher.update(frame);
        let offset = self.size;
        self.size += frame.len() as u64;
        Ok((offset, frame.len() as u64))
    }

    fn finish(mut self, url: String) -> UpdateResult<BlobRef> {
        use sha2::Digest;
        self.file.flush().at(&self.path)?;
        Ok(BlobRef {
            url,
            size: self.size,
            sha256: hex::encode(self.hasher.finalize()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::InstallLayout;
    use crate::plan::{build_plan, FsLocalState, Source};
    use crate::stage::tests::{block_on, MemoryFetcher};
    use std::sync::atomic::AtomicBool;

    fn request<'a>(stem: &'a str, out: &'a Path) -> PackRequest<'a> {
        PackRequest {
            asset_stem: stem,
            base_url: "https://dl.invalid",
            out_dir: out,
        }
    }

    fn exe(version: u8) -> Vec<u8> {
        let mut data: Vec<u8> = (0..300_000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        data[1_000] = version;
        data
    }

    /// 端到端：打两个版本的包，旧安装只下载补丁就能变成新版本
    #[test]
    fn old_install_reaches_new_tree_through_patches() {
        let work = tempfile::tempdir().unwrap();
        let tree_v1 = work.path().join("v1");
        let tree_v2 = work.path().join("v2");
        for (tree, version) in [(&tree_v1, 1u8), (&tree_v2, 2u8)] {
            std::fs::create_dir_all(tree.join("ffmpeg")).unwrap();
            std::fs::write(tree.join("app.exe"), exe(version)).unwrap();
            std::fs::write(tree.join("ffmpeg/avcodec.dll"), vec![5u8; 40_000]).unwrap();
            std::fs::write(tree.join("uninstall.exe"), b"generated at install time").unwrap();
        }
        let exclude = vec!["uninstall.exe".to_string()];
        let out = work.path().join("out");

        let v1 = pack(
            &request("N-1.0.0-w", &out),
            &scan_tree(&tree_v1, &exclude).unwrap(),
            &[],
        )
        .unwrap();
        assert!(v1.patches.is_empty());
        assert!(v1.files.iter().all(|f| f.path != "uninstall.exe"));
        let base = BaseRelease {
            version: "1.0.0".into(),
            release: v1,
            blob: std::fs::read(out.join("N-1.0.0-w.update")).unwrap(),
        };
        let v2 = pack(
            &request("N-1.1.0-w", &out),
            &scan_tree(&tree_v2, &exclude).unwrap(),
            &[base],
        )
        .unwrap();
        assert_eq!(v2.patches.len(), 1);
        assert_eq!(
            v2.patches[0].entries.len(),
            1,
            "only the changed exe gets a patch"
        );

        // 旧安装 = v1 文件树；按计划下载补丁并应用
        let layout = InstallLayout::Directory {
            root: tree_v1.clone(),
        };
        let plan = build_plan(&v2, &FsLocalState { layout: &layout });
        assert!(matches!(plan.actions[0].source, Source::Patch { .. }));
        assert_eq!(plan.actions[1].source, Source::Unchanged);
        let summary = plan.summary();
        assert!(
            summary.download_bytes * 10 < summary.full_bytes,
            "{summary:?}"
        );

        let fetcher = MemoryFetcher(HashMap::from([
            (
                v2.blob.url.clone(),
                std::fs::read(out.join("N-1.1.0-w.update")).unwrap(),
            ),
            (
                v2.patches[0].blob.url.clone(),
                std::fs::read(out.join("N-1.1.0-w.from-1.0.0.patch")).unwrap(),
            ),
        ]));
        let staging = layout.staging_dir();
        crate::apply::prepare_staging(&staging, "1.1.0").unwrap();
        block_on(crate::stage::stage(
            &layout,
            &v2,
            &plan,
            &staging,
            &fetcher,
            &AtomicBool::new(false),
            &mut |_| {},
        ))
        .unwrap();
        crate::apply::apply(&layout, &plan, &staging).unwrap();
        assert_eq!(std::fs::read(tree_v1.join("app.exe")).unwrap(), exe(2));
    }

    #[test]
    fn single_file_tree_uses_the_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("NeriPlayer_1.0.0_amd64.AppImage");
        std::fs::write(&image, b"elf").unwrap();
        let entries = scan_tree(&image, &[]).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "NeriPlayer_1.0.0_amd64.AppImage");
    }

    #[test]
    fn patches_are_only_kept_when_clearly_smaller() {
        assert!(worth_patching(10, 100));
        assert!(!worth_patching(90, 100));
    }
}
