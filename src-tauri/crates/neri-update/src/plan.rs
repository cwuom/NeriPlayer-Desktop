//! 对比本地文件与新版清单，算出每个文件怎么来：不变、打补丁，还是下载完整内容

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use crate::codec::sha256_file;
use crate::layout::InstallLayout;
use crate::manifest::{FileEntry, PlatformRelease};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// 本地已是目标内容
    Unchanged,
    /// 符号链接，直接按清单重建，不需要下载
    Link,
    /// 以本地旧文件为基准打补丁；`set` 是 `PlatformRelease::patches` 的下标
    Patch {
        set: usize,
        offset: u64,
        length: u64,
    },
    /// 下载完整内容
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAction {
    pub entry: FileEntry,
    pub source: Source,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePlan {
    pub actions: Vec<FileAction>,
}

/// 给界面展示的下载量
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanSummary {
    /// 本次实际要下载的字节数
    pub download_bytes: u64,
    /// 不做任何增量时需要下载的字节数（所有文件的完整压缩内容）
    pub full_bytes: u64,
    pub changed_files: usize,
    pub total_files: usize,
}

/// 本地文件的现状，测试时可以用内存表代替文件系统
pub trait LocalState {
    /// 普通文件的 SHA-256；不存在或读不了返回 None
    fn file_hash(&self, relative: &str) -> Option<String>;
    /// 符号链接目标；不是链接返回 None
    fn link_target(&self, relative: &str) -> Option<String>;
}

pub struct FsLocalState<'a> {
    pub layout: &'a InstallLayout,
}

impl LocalState for FsLocalState<'_> {
    fn file_hash(&self, relative: &str) -> Option<String> {
        let path = self.layout.local_path(relative);
        let metadata = std::fs::symlink_metadata(&path).ok()?;
        if !metadata.is_file() {
            return None;
        }
        sha256_file(&path)
            .inspect_err(|error| log::warn!(target: "update", "hash {}: {error}", path.display()))
            .ok()
    }

    fn link_target(&self, relative: &str) -> Option<String> {
        let target = std::fs::read_link(self.layout.local_path(relative)).ok()?;
        target.to_str().map(str::to_string)
    }
}

pub fn build_plan(release: &PlatformRelease, local: &impl LocalState) -> UpdatePlan {
    // (路径, 基准哈希) → 补丁位置；同一个基准在多个补丁集里时取最小的那个
    let mut patches: HashMap<(&str, &str), (usize, u64, u64)> = HashMap::new();
    for (index, set) in release.patches.iter().enumerate() {
        for entry in &set.entries {
            let key = (entry.path.as_str(), entry.base_sha256.as_str());
            let candidate = (index, entry.offset, entry.length);
            patches
                .entry(key)
                .and_modify(|best| {
                    if candidate.2 < best.2 {
                        *best = candidate;
                    }
                })
                .or_insert(candidate);
        }
    }

    let actions = release
        .files
        .iter()
        .map(|entry| {
            let source = if let Some(target) = &entry.link {
                if local.link_target(&entry.path).as_deref() == Some(target.as_str()) {
                    Source::Unchanged
                } else {
                    Source::Link
                }
            } else {
                match local.file_hash(&entry.path) {
                    Some(hash) if hash == entry.sha256 => Source::Unchanged,
                    Some(hash) => match patches.get(&(entry.path.as_str(), hash.as_str())) {
                        Some(&(set, offset, length)) if length < entry.length => Source::Patch {
                            set,
                            offset,
                            length,
                        },
                        _ => Source::Full,
                    },
                    None => Source::Full,
                }
            };
            FileAction {
                entry: entry.clone(),
                source,
            }
        })
        .collect();
    UpdatePlan { actions }
}

impl UpdatePlan {
    pub fn summary(&self) -> PlanSummary {
        let mut summary = PlanSummary {
            download_bytes: 0,
            full_bytes: 0,
            changed_files: 0,
            total_files: self.actions.len(),
        };
        for action in &self.actions {
            if action.entry.link.is_none() {
                summary.full_bytes += action.entry.length;
            }
            match action.source {
                Source::Unchanged => {}
                Source::Link => summary.changed_files += 1,
                Source::Patch { length, .. } => {
                    summary.download_bytes += length;
                    summary.changed_files += 1;
                }
                Source::Full => {
                    summary.download_bytes += action.entry.length;
                    summary.changed_files += 1;
                }
            }
        }
        summary
    }

    pub fn is_noop(&self) -> bool {
        self.actions
            .iter()
            .all(|action| action.source == Source::Unchanged)
    }

    pub fn changed(&self) -> impl Iterator<Item = &FileAction> {
        self.actions
            .iter()
            .filter(|action| action.source != Source::Unchanged)
    }
}

/// 暂存目录里某个文件的位置
pub(crate) fn staged_path(staging: &Path, relative: &str) -> std::path::PathBuf {
    crate::layout::join_relative(staging, relative)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{BlobRef, PatchEntry, PatchSet};

    fn hash(c: char) -> String {
        c.to_string().repeat(64)
    }

    fn blob() -> BlobRef {
        BlobRef {
            url: "u".into(),
            size: 10_000,
            sha256: hash('0'),
        }
    }

    fn file(path: &str, sha: char, length: u64) -> FileEntry {
        FileEntry {
            path: path.into(),
            size: length * 2,
            sha256: hash(sha),
            mode: None,
            link: None,
            offset: 0,
            length,
        }
    }

    struct Fake(HashMap<&'static str, String>);

    impl LocalState for Fake {
        fn file_hash(&self, relative: &str) -> Option<String> {
            self.0.get(relative).cloned()
        }
        fn link_target(&self, relative: &str) -> Option<String> {
            self.0
                .get(relative)
                .and_then(|v| v.strip_prefix("->").map(str::to_string))
        }
    }

    #[test]
    fn picks_unchanged_patch_full_and_link_per_file() {
        let mut link = file("Contents/Frameworks/libx.dylib", 'f', 0);
        link.link = Some("libx.1.dylib".into());
        let release = PlatformRelease {
            blob: blob(),
            files: vec![
                file("app.exe", 'a', 1_000),
                file("ffmpeg/avcodec.dll", 'b', 3_000),
                file("new.dll", 'c', 500),
                file("stale.dll", 'd', 400),
                link,
            ],
            patches: vec![PatchSet {
                from_version: "1.0.0".into(),
                blob: blob(),
                entries: vec![
                    PatchEntry {
                        path: "app.exe".into(),
                        base_sha256: hash('1'),
                        offset: 7,
                        length: 90,
                    },
                    // 补丁比完整内容还大时不值得用
                    PatchEntry {
                        path: "stale.dll".into(),
                        base_sha256: hash('2'),
                        offset: 0,
                        length: 450,
                    },
                ],
            }],
        };
        let local = Fake(HashMap::from([
            ("app.exe", hash('1')),
            ("ffmpeg/avcodec.dll", hash('b')),
            ("stale.dll", hash('2')),
            (
                "Contents/Frameworks/libx.dylib",
                "->libx.0.dylib".to_string(),
            ),
        ]));

        let plan = build_plan(&release, &local);
        let sources: Vec<_> = plan.actions.iter().map(|a| a.source.clone()).collect();
        assert_eq!(
            sources,
            vec![
                Source::Patch {
                    set: 0,
                    offset: 7,
                    length: 90
                },
                Source::Unchanged,
                Source::Full,
                Source::Full,
                Source::Link,
            ]
        );
        assert_eq!(
            plan.summary(),
            PlanSummary {
                download_bytes: 90 + 500 + 400,
                full_bytes: 4_900,
                changed_files: 4,
                total_files: 5
            }
        );
        assert!(!plan.is_noop());
    }

    #[test]
    fn a_patch_for_another_base_falls_back_to_full() {
        let release = PlatformRelease {
            blob: blob(),
            files: vec![file("app.exe", 'a', 1_000)],
            patches: vec![PatchSet {
                from_version: "0.9.0".into(),
                blob: blob(),
                entries: vec![PatchEntry {
                    path: "app.exe".into(),
                    base_sha256: hash('9'),
                    offset: 0,
                    length: 10,
                }],
            }],
        };
        let plan = build_plan(&release, &Fake(HashMap::from([("app.exe", hash('1'))])));
        assert_eq!(plan.actions[0].source, Source::Full);

        let plan = build_plan(&release, &Fake(HashMap::from([("app.exe", hash('a'))])));
        assert!(plan.is_noop());
        assert_eq!(plan.summary().download_bytes, 0);
    }
}
