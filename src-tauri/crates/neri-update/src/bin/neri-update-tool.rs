//! 发布工具：生成签名密钥、为各平台打增量包、合并并签名清单。
//!
//! ```text
//! neri-update-tool keygen <私钥输出文件>
//! neri-update-tool pack --platform <键> --tree <目录或文件> --stem <资产前缀> --base-url <URL> --out <目录>
//!                       [--exclude <相对路径>]... [--base-manifest <旧清单> --base-blobs <旧 blob 目录>]
//! neri-update-tool finalize --version <版本> --build-epoch <秒> --release-tag <tag> --release-url <URL>
//!                           --fragments <目录> --out <目录> [--public-key <公钥>]
//!                           （私钥从环境变量 NERI_UPDATE_SIGNING_KEY 读取）
//! neri-update-tool verify --manifest <清单> --public-key <公钥>
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use neri_update::manifest::{PlatformRelease, ReleaseManifest, MANIFEST_SCHEMA};
use neri_update::pack::{pack, scan_tree, BaseRelease, PackRequest};
use neri_update::{signing, MANIFEST_ASSET, SIGNATURE_ASSET};

const SIGNING_KEY_ENV: &str = "NERI_UPDATE_SIGNING_KEY";
const FRAGMENT_SUFFIX: &str = ".fragment.json";

type ToolResult<T> = Result<T, String>;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("keygen") => keygen(&args[1..]),
        Some("pack") => run_pack(&Options::parse(&args[1..])),
        Some("finalize") => finalize(&Options::parse(&args[1..])),
        Some("verify") => verify(&Options::parse(&args[1..])),
        _ => Err("usage: neri-update-tool <keygen|pack|finalize|verify> ...".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

/// `--key value` 形式的参数；同名参数可重复
struct Options(Vec<(String, String)>);

impl Options {
    fn parse(args: &[String]) -> Self {
        let mut pairs = Vec::new();
        let mut iter = args.iter();
        while let Some(key) = iter.next() {
            if let (Some(key), Some(value)) = (key.strip_prefix("--"), iter.next()) {
                pairs.push((key.to_string(), value.clone()));
            }
        }
        Self(pairs)
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn required(&self, key: &str) -> ToolResult<&str> {
        self.get(key).ok_or_else(|| format!("missing --{key}"))
    }

    fn all(&self, key: &str) -> Vec<String> {
        self.0
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .collect()
    }
}

fn keygen(args: &[String]) -> ToolResult<()> {
    let out = args.first().ok_or("usage: keygen <private-key-file>")?;
    let path = Path::new(out);
    if path.exists() {
        return Err(format!(
            "{out} already exists; refusing to overwrite a signing key"
        ));
    }
    let (private_key, public_key) = signing::generate_keypair().map_err(|e| e.to_string())?;
    write_private(path, &private_key)?;
    // 私钥只写文件、不打印；公钥可以公开
    println!("{public_key}");
    eprintln!("private key written to {out}; store it as the {SIGNING_KEY_ENV} secret and keep it offline");
    Ok(())
}

#[cfg(unix)]
fn write_private(path: &Path, contents: &str) -> ToolResult<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(contents.as_bytes())
        .map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn write_private(path: &Path, contents: &str) -> ToolResult<()> {
    std::fs::write(path, contents).map_err(|e| e.to_string())
}

fn run_pack(options: &Options) -> ToolResult<()> {
    let platform = options.required("platform")?;
    let tree = PathBuf::from(options.required("tree")?);
    let out = PathBuf::from(options.required("out")?);
    let request = PackRequest {
        asset_stem: options.required("stem")?,
        base_url: options.required("base-url")?.trim_end_matches('/'),
        out_dir: &out,
    };
    let entries = scan_tree(&tree, &options.all("exclude")).map_err(|e| e.to_string())?;
    let bases = load_base(options, platform)?;
    let release = pack(&request, &entries, &bases).map_err(|e| e.to_string())?;

    let patched: usize = release.patches.iter().map(|set| set.entries.len()).sum();
    eprintln!(
        "{platform}: {} files, blob {} bytes, {patched} patched file(s)",
        release.files.len(),
        release.blob.size
    );
    let fragment = out.join(format!("{platform}{FRAGMENT_SUFFIX}"));
    let json = serde_json::to_vec_pretty(&release).map_err(|e| e.to_string())?;
    std::fs::write(&fragment, json).map_err(|e| format!("{}: {e}", fragment.display()))
}

/// 上一版清单里没有这个平台（新增平台或首个带更新的版本）时不生成补丁
fn load_base(options: &Options, platform: &str) -> ToolResult<Vec<BaseRelease>> {
    let (Some(manifest), Some(blobs)) = (options.get("base-manifest"), options.get("base-blobs"))
    else {
        return Ok(Vec::new());
    };
    let bytes = std::fs::read(manifest).map_err(|e| format!("{manifest}: {e}"))?;
    let manifest = ReleaseManifest::parse(&bytes).map_err(|e| e.to_string())?;
    let Some(release) = manifest.platforms.get(platform) else {
        eprintln!(
            "{platform}: previous release {} has no package, skipping patches",
            manifest.version
        );
        return Ok(Vec::new());
    };
    let name = release.blob.url.rsplit('/').next().unwrap_or_default();
    let blob_path = Path::new(blobs).join(name);
    if !blob_path.is_file() {
        // 上一版缺了这个平台的 blob（例如上传失败）只影响补丁，客户端仍按文件比对增量下载
        eprintln!(
            "{platform}: {} not found, skipping patches",
            blob_path.display()
        );
        return Ok(Vec::new());
    }
    let blob = std::fs::read(&blob_path).map_err(|e| format!("{}: {e}", blob_path.display()))?;
    if blob.len() as u64 != release.blob.size {
        return Err(format!(
            "{} size does not match the previous manifest",
            blob_path.display()
        ));
    }
    Ok(vec![BaseRelease {
        version: manifest.version.clone(),
        release: release.clone(),
        blob,
    }])
}

fn finalize(options: &Options) -> ToolResult<()> {
    let private_key =
        std::env::var(SIGNING_KEY_ENV).map_err(|_| format!("{SIGNING_KEY_ENV} is not set"))?;
    if let Some(expected) = options
        .get("public-key")
        .filter(|key| !key.trim().is_empty())
    {
        let actual = signing::public_key_of(&private_key).map_err(|e| e.to_string())?;
        if actual != expected.trim() {
            return Err("signing key does not match the public key embedded in the app".into());
        }
    }
    let fragments = Path::new(options.required("fragments")?);
    let mut platforms = BTreeMap::new();
    for item in std::fs::read_dir(fragments).map_err(|e| e.to_string())? {
        let path = item.map_err(|e| e.to_string())?.path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let Some(platform) = name.strip_suffix(FRAGMENT_SUFFIX) else {
            continue;
        };
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let release: PlatformRelease =
            serde_json::from_slice(&bytes).map_err(|e| format!("{name}: {e}"))?;
        platforms.insert(platform.to_string(), release);
    }
    if platforms.is_empty() {
        return Err(format!("no *{FRAGMENT_SUFFIX} in {}", fragments.display()));
    }
    let manifest = ReleaseManifest {
        schema: MANIFEST_SCHEMA,
        version: options.required("version")?.to_string(),
        build_epoch: options
            .required("build-epoch")?
            .parse()
            .map_err(|_| "bad --build-epoch")?,
        release_tag: options.required("release-tag")?.to_string(),
        release_url: options.required("release-url")?.to_string(),
        platforms,
    };
    manifest.validate().map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    let signature = signing::sign(&bytes, &private_key).map_err(|e| e.to_string())?;

    let out = Path::new(options.required("out")?);
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    std::fs::write(out.join(MANIFEST_ASSET), &bytes).map_err(|e| e.to_string())?;
    std::fs::write(out.join(SIGNATURE_ASSET), signature).map_err(|e| e.to_string())?;
    eprintln!(
        "signed manifest for {} platform(s)",
        manifest.platforms.len()
    );
    Ok(())
}

fn verify(options: &Options) -> ToolResult<()> {
    let manifest = options.required("manifest")?;
    let bytes = std::fs::read(manifest).map_err(|e| format!("{manifest}: {e}"))?;
    let signature =
        std::fs::read_to_string(format!("{manifest}.sig")).map_err(|e| e.to_string())?;
    signing::verify(&bytes, &signature, options.required("public-key")?)
        .map_err(|e| e.to_string())?;
    let parsed = ReleaseManifest::parse(&bytes).map_err(|e| e.to_string())?;
    eprintln!(
        "ok: {} for {:?}",
        parsed.version,
        parsed.platforms.keys().collect::<Vec<_>>()
    );
    Ok(())
}
