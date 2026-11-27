//! 应用内增量更新。
//!
//! 发布流水线为每个平台生成「安装后文件树」的清单，并把每个文件压成独立 zstd 帧拼成 blob；
//! 对上一版本还会生成以旧文件为参照的补丁。应用端校验清单签名后逐文件比对本地哈希，
//! 只下载变化文件的补丁或完整内容，校验通过后原地替换。不依赖 Tauri

pub mod apply;
#[cfg(feature = "client")]
pub mod client;
pub mod codec;
pub mod error;
pub mod layout;
pub mod manifest;
pub mod pack;
pub mod plan;
pub mod signing;
pub mod stage;

pub use error::{UpdateError, UpdateResult};

/// 清单的发布资产名；签名在同名 `.sig` 里
pub const MANIFEST_ASSET: &str = "neri-update.json";
pub const SIGNATURE_ASSET: &str = "neri-update.json.sig";

/// 当前构建对应的清单平台键，例如 `windows-x86_64`、`macos-aarch64`、`linux-x86_64`
pub fn platform_key() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// 取回并校验清单：签名不通过一律当作没有更新信息处理，不解析内容
#[cfg(feature = "client")]
pub async fn fetch_manifest(
    fetcher: &client::HttpFetcher,
    manifest_url: &str,
    public_key: &str,
) -> UpdateResult<manifest::ReleaseManifest> {
    let bytes = fetcher.fetch_small(manifest_url).await?;
    let signature = fetcher.fetch_small(&format!("{manifest_url}.sig")).await?;
    let signature = String::from_utf8(signature).map_err(|_| UpdateError::BadSignature)?;
    signing::verify(&bytes, &signature, public_key)?;
    manifest::ReleaseManifest::parse(&bytes)
}
