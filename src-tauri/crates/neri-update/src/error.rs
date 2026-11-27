use std::path::PathBuf;

/// 更新流程的错误。`code()` 给前端做本地化，`Display` 只用于日志
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("network error: {0}")]
    Network(String),
    #[error("update manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("update signature does not match")]
    BadSignature,
    #[error("update signing key is invalid: {0}")]
    BadKey(String),
    #[error("{path} failed verification")]
    Corrupted { path: String },
    #[error("no update package for platform {0}")]
    NoPackageForPlatform(String),
    #[error("update was cancelled")]
    Cancelled,
    #[error("{0}")]
    Other(String),
}

impl UpdateError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    /// 稳定的错误码，前端据此选择提示文案
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io { source, .. } if source.kind() == std::io::ErrorKind::PermissionDenied => {
                "permission"
            }
            Self::Io { .. } => "io",
            Self::Network(_) => "network",
            Self::InvalidManifest(_) | Self::NoPackageForPlatform(_) => "manifest",
            Self::BadSignature | Self::BadKey(_) => "signature",
            Self::Corrupted { .. } => "corrupted",
            Self::Cancelled => "cancelled",
            Self::Other(_) => "unknown",
        }
    }
}

pub type UpdateResult<T> = Result<T, UpdateError>;

/// 给 std::io::Result 附上出错路径
pub(crate) trait IoContext<T> {
    fn at(self, path: impl Into<PathBuf>) -> UpdateResult<T>;
}

impl<T> IoContext<T> for std::io::Result<T> {
    fn at(self, path: impl Into<PathBuf>) -> UpdateResult<T> {
        self.map_err(|source| UpdateError::io(path, source))
    }
}
