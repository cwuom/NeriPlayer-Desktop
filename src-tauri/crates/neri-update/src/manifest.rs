//! 发布清单：每个平台一份「安装后文件树」的描述，外加从旧版本到新版本的补丁索引。
//!
//! 文件内容不单独上传成附件，而是把每个文件压成独立的 zstd 帧顺序拼进一个 blob，
//! 客户端按清单里的 offset/length 用 HTTP Range 只取需要的那几段

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::error::{UpdateError, UpdateResult};

pub const MANIFEST_SCHEMA: u32 = 1;
/// 清单本身很小，超过这个大小说明拿到的不是清单
pub const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseManifest {
    pub schema: u32,
    pub version: String,
    /// 构建时间（Unix 秒）。版本号有语义化与时间戳两种格式，新旧只按它比较
    pub build_epoch: u64,
    pub release_tag: String,
    pub release_url: String,
    pub platforms: BTreeMap<String, PlatformRelease>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformRelease {
    pub blob: BlobRef,
    pub files: Vec<FileEntry>,
    #[serde(default)]
    pub patches: Vec<PatchSet>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobRef {
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    /// 相对安装根目录、以 `/` 分隔的路径
    pub path: String,
    pub size: u64,
    /// 解压后内容的 SHA-256；符号链接是链接目标字符串的 SHA-256
    pub sha256: String,
    /// Unix 权限位；Windows 包不写
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    /// macOS 包里的符号链接，值为链接目标
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// 完整内容的 zstd 帧在 blob 里的位置；符号链接为 0
    pub offset: u64,
    pub length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchSet {
    pub from_version: String,
    pub blob: BlobRef,
    pub entries: Vec<PatchEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchEntry {
    pub path: String,
    /// 补丁以这个内容的旧文件为参照；本地文件哈希不同就不能用
    pub base_sha256: String,
    pub offset: u64,
    pub length: u64,
}

impl ReleaseManifest {
    /// 解析并做结构校验。签名校验在 [`crate::signing`]，必须先于本函数调用
    pub fn parse(bytes: &[u8]) -> UpdateResult<Self> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(UpdateError::InvalidManifest("manifest too large".into()));
        }
        let manifest: Self = serde_json::from_slice(bytes)
            .map_err(|error| UpdateError::InvalidManifest(error.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> UpdateResult<()> {
        if self.schema != MANIFEST_SCHEMA {
            return Err(UpdateError::InvalidManifest(format!(
                "unsupported schema {}",
                self.schema
            )));
        }
        if self.version.trim().is_empty() {
            return Err(UpdateError::InvalidManifest("empty version".into()));
        }
        for (platform, release) in &self.platforms {
            release
                .validate()
                .map_err(|reason| UpdateError::InvalidManifest(format!("{platform}: {reason}")))?;
        }
        Ok(())
    }

    pub fn platform(&self, key: &str) -> UpdateResult<&PlatformRelease> {
        self.platforms
            .get(key)
            .ok_or_else(|| UpdateError::NoPackageForPlatform(key.to_string()))
    }
}

impl PlatformRelease {
    fn validate(&self) -> Result<(), String> {
        if self.files.is_empty() {
            return Err("no files".into());
        }
        let mut seen = HashSet::new();
        for file in &self.files {
            validate_relative_path(&file.path)?;
            validate_sha256(&file.sha256)?;
            if !seen.insert(file.path.to_ascii_lowercase()) {
                // Windows 与 macOS 默认大小写不敏感，大小写不同的同名文件会互相覆盖
                return Err(format!("duplicate path {}", file.path));
            }
            if file.link.is_none() {
                check_range(&self.blob, file.offset, file.length, &file.path)?;
            }
        }
        for set in &self.patches {
            for entry in &set.entries {
                validate_relative_path(&entry.path)?;
                validate_sha256(&entry.base_sha256)?;
                if !seen.contains(&entry.path.to_ascii_lowercase()) {
                    return Err(format!("patch for unknown file {}", entry.path));
                }
                check_range(&set.blob, entry.offset, entry.length, &entry.path)?;
            }
        }
        Ok(())
    }
}

fn check_range(blob: &BlobRef, offset: u64, length: u64, path: &str) -> Result<(), String> {
    let end = offset.checked_add(length).ok_or("range overflow")?;
    if length == 0 || end > blob.size {
        return Err(format!("{path} range {offset}+{length} outside blob"));
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(format!("bad sha256 {value}"))
    }
}

/// 清单来自网络，即使签名通过也只接受安装根目录内的普通相对路径，杜绝路径穿越
pub fn validate_relative_path(path: &str) -> Result<(), String> {
    let bad = path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.contains('\0')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..");
    if bad {
        Err(format!("unsafe path {path:?}"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    fn sample() -> ReleaseManifest {
        let blob = BlobRef {
            url: "https://example.invalid/a.update".into(),
            size: 100,
            sha256: HASH.into(),
        };
        ReleaseManifest {
            schema: MANIFEST_SCHEMA,
            version: "1.1.0".into(),
            build_epoch: 1,
            release_tag: "v1.1.0".into(),
            release_url: "https://example.invalid".into(),
            platforms: BTreeMap::from([(
                "windows-x86_64".into(),
                PlatformRelease {
                    blob: blob.clone(),
                    files: vec![FileEntry {
                        path: "ffmpeg/avcodec.dll".into(),
                        size: 10,
                        sha256: HASH.into(),
                        mode: None,
                        link: None,
                        offset: 0,
                        length: 100,
                    }],
                    patches: vec![PatchSet {
                        from_version: "1.0.0".into(),
                        blob,
                        entries: vec![PatchEntry {
                            path: "ffmpeg/avcodec.dll".into(),
                            base_sha256: HASH.into(),
                            offset: 10,
                            length: 90,
                        }],
                    }],
                },
            )]),
        }
    }

    #[test]
    fn round_trips_through_json() {
        let manifest = sample();
        let bytes = serde_json::to_vec(&manifest).unwrap();
        assert_eq!(ReleaseManifest::parse(&bytes).unwrap(), manifest);
    }

    #[test]
    fn rejects_paths_that_escape_the_install_root() {
        for path in [
            "../evil.exe",
            "/etc/passwd",
            "a/../../b",
            "C:/x",
            "a\\b",
            "a//b",
            "./a",
            "",
        ] {
            assert!(
                validate_relative_path(path).is_err(),
                "{path:?} should be rejected"
            );
        }
        assert!(validate_relative_path("Contents/MacOS/NeriPlayer").is_ok());
    }

    #[test]
    fn rejects_ranges_outside_the_blob() {
        let mut manifest = sample();
        manifest.platforms.get_mut("windows-x86_64").unwrap().files[0].length = 101;
        assert!(matches!(
            manifest.validate(),
            Err(UpdateError::InvalidManifest(_))
        ));
    }

    #[test]
    fn rejects_case_insensitive_duplicates_and_orphan_patches() {
        let mut manifest = sample();
        let release = manifest.platforms.get_mut("windows-x86_64").unwrap();
        let mut twin = release.files[0].clone();
        twin.path = "FFmpeg/avcodec.dll".into();
        release.files.push(twin);
        assert!(manifest.validate().is_err());

        let mut manifest = sample();
        manifest
            .platforms
            .get_mut("windows-x86_64")
            .unwrap()
            .patches[0]
            .entries[0]
            .path = "other.dll".into();
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn rejects_unknown_schema_and_uppercase_hashes() {
        let mut manifest = sample();
        manifest.schema = 2;
        assert!(manifest.validate().is_err());

        let mut manifest = sample();
        manifest.platforms.get_mut("windows-x86_64").unwrap().files[0].sha256 =
            HASH.replace('0', "A");
        assert!(manifest.validate().is_err());
    }
}
