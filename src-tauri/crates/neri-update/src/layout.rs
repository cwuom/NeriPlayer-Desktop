//! 识别应用是怎么装的，决定能否原地更新、文件树的根在哪里

use std::path::{Path, PathBuf};

use serde::Serialize;

/// 可原地更新的安装形态
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallLayout {
    /// Windows NSIS 安装目录：逐个文件替换，占用中的文件先改名让位
    Directory { root: PathBuf },
    /// macOS .app：在旁边拼出完整的新包再整体换名，保证签名覆盖的内容与发布包逐字节一致
    MacBundle { bundle: PathBuf },
    /// Linux AppImage：单文件整体替换
    SingleFile { path: PathBuf },
}

/// 不能原地更新的原因，前端据此引导用户手动下载
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UnsupportedReason {
    /// MSI 或其它非 NSIS 安装（目录里没有 NSIS 卸载程序）
    WindowsMsi,
    /// 直接从 DMG 里运行
    MacDiskImage,
    /// 被 Gatekeeper 随机化路径（App Translocation）运行，需要先拖进「应用程序」
    MacTranslocated,
    /// deb / rpm 等系统包，由包管理器负责
    LinuxPackage,
    Flatpak,
    /// 安装位置没有写权限
    ReadOnly,
    /// 开发构建或无法识别的位置
    Unknown,
}

/// 识别所需的外部信息，单测时可以伪造
#[derive(Debug, Clone, Default)]
pub struct Environment {
    pub appimage: Option<PathBuf>,
    pub flatpak: bool,
}

impl Environment {
    pub fn current() -> Self {
        Self {
            appimage: std::env::var_os("APPIMAGE").map(PathBuf::from),
            flatpak: std::env::var_os("FLATPAK_ID").is_some(),
        }
    }
}

/// NSIS 安装时写入的卸载程序；MSI 没有它
const NSIS_UNINSTALLER: &str = "uninstall.exe";

pub fn detect(exe: &Path, env: &Environment) -> Result<InstallLayout, UnsupportedReason> {
    let layout = if cfg!(target_os = "windows") {
        detect_windows(exe)?
    } else if cfg!(target_os = "macos") {
        detect_macos(exe)?
    } else {
        detect_linux(env)?
    };
    if !writable(layout.writable_dir()) {
        return Err(UnsupportedReason::ReadOnly);
    }
    Ok(layout)
}

fn detect_windows(exe: &Path) -> Result<InstallLayout, UnsupportedReason> {
    let root = exe.parent().ok_or(UnsupportedReason::Unknown)?;
    if root.join(NSIS_UNINSTALLER).is_file() {
        Ok(InstallLayout::Directory {
            root: root.to_path_buf(),
        })
    } else {
        Err(UnsupportedReason::WindowsMsi)
    }
}

/// `…/NeriPlayer.app/Contents/MacOS/<exe>` → `…/NeriPlayer.app`
pub fn mac_bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    is_bundle.then(|| bundle.to_path_buf())
}

fn detect_macos(exe: &Path) -> Result<InstallLayout, UnsupportedReason> {
    let bundle = mac_bundle_of(exe).ok_or(UnsupportedReason::Unknown)?;
    let text = bundle.to_string_lossy();
    if text.contains("/AppTranslocation/") {
        return Err(UnsupportedReason::MacTranslocated);
    }
    if text.starts_with("/Volumes/") && !writable(bundle.parent().unwrap_or(&bundle)) {
        return Err(UnsupportedReason::MacDiskImage);
    }
    Ok(InstallLayout::MacBundle { bundle })
}

fn detect_linux(env: &Environment) -> Result<InstallLayout, UnsupportedReason> {
    if env.flatpak {
        return Err(UnsupportedReason::Flatpak);
    }
    match &env.appimage {
        Some(path) if path.is_file() => Ok(InstallLayout::SingleFile { path: path.clone() }),
        _ => Err(UnsupportedReason::LinuxPackage),
    }
}

impl InstallLayout {
    /// 原地替换需要写入的目录（新旧文件必须同盘，才能用 rename 原子换位）
    pub fn writable_dir(&self) -> &Path {
        match self {
            Self::Directory { root } => root,
            Self::MacBundle { bundle } | Self::SingleFile { path: bundle } => {
                bundle.parent().unwrap_or(bundle)
            }
        }
    }

    /// 清单里的相对路径对应的本地文件
    pub fn local_path(&self, relative: &str) -> PathBuf {
        match self {
            Self::Directory { root } => join_relative(root, relative),
            Self::MacBundle { bundle } => join_relative(bundle, relative),
            Self::SingleFile { path } => path.clone(),
        }
    }

    /// 下载与解码的暂存目录，和安装位置同盘
    pub fn staging_dir(&self) -> PathBuf {
        match self {
            Self::Directory { root } => root.join(".neri-update"),
            Self::MacBundle { bundle } | Self::SingleFile { path: bundle } => {
                let name = bundle.file_name().unwrap_or_default().to_string_lossy();
                self.writable_dir().join(format!(".{name}.neri-update"))
            }
        }
    }
}

pub(crate) fn join_relative(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

/// 实际建一个临时文件来判断：只看权限位在 Windows ACL 与 macOS 沙盒下都不可靠
fn writable(dir: &Path) -> bool {
    // 独占创建：已有同名文件时换个名字，绝不截断或删除别人的文件，只清理自己建的
    for attempt in 0..8 {
        let probe = dir.join(format!(
            ".neri-write-probe-{}-{attempt}",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
        {
            Ok(_) => {
                let _ = std::fs::remove_file(&probe);
                return true;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_bundle_root_of_a_mac_executable() {
        let exe = Path::new("/Applications/NeriPlayer.app/Contents/MacOS/neri-player-desktop");
        assert_eq!(
            mac_bundle_of(exe),
            Some(PathBuf::from("/Applications/NeriPlayer.app"))
        );
        assert_eq!(mac_bundle_of(Path::new("/usr/local/bin/neri")), None);
        assert_eq!(mac_bundle_of(Path::new("/tmp/Foo/Contents/MacOS/x")), None);
    }

    #[test]
    fn linux_layouts_follow_the_environment() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("NeriPlayer.AppImage");
        std::fs::write(&image, b"x").unwrap();

        let appimage = Environment {
            appimage: Some(image.clone()),
            flatpak: false,
        };
        assert_eq!(
            detect_linux(&appimage),
            Ok(InstallLayout::SingleFile { path: image })
        );
        let flatpak = Environment {
            appimage: None,
            flatpak: true,
        };
        assert_eq!(detect_linux(&flatpak), Err(UnsupportedReason::Flatpak));
        assert_eq!(
            detect_linux(&Environment::default()),
            Err(UnsupportedReason::LinuxPackage)
        );
    }

    #[test]
    fn windows_requires_the_nsis_uninstaller() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("neri-player-desktop.exe");
        assert_eq!(detect_windows(&exe), Err(UnsupportedReason::WindowsMsi));
        std::fs::write(dir.path().join(NSIS_UNINSTALLER), b"x").unwrap();
        assert_eq!(
            detect_windows(&exe),
            Ok(InstallLayout::Directory {
                root: dir.path().to_path_buf()
            })
        );
    }

    #[test]
    fn staging_stays_next_to_the_install() {
        let bundle = InstallLayout::MacBundle {
            bundle: PathBuf::from("/Applications/NeriPlayer.app"),
        };
        assert_eq!(
            bundle.staging_dir(),
            PathBuf::from("/Applications/.NeriPlayer.app.neri-update")
        );
        assert_eq!(
            bundle.local_path("Contents/Info.plist"),
            PathBuf::from("/Applications/NeriPlayer.app/Contents/Info.plist")
        );
        let dir = InstallLayout::Directory {
            root: PathBuf::from("/opt/neri"),
        };
        assert_eq!(
            dir.local_path("ffmpeg/a.dll"),
            PathBuf::from("/opt/neri/ffmpeg/a.dll")
        );
        assert!(writable(&std::env::temp_dir()));
    }
}
