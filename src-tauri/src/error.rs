use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Audio error: {0}")]
    Audio(String),

    #[error("Metadata error: {0}")]
    Metadata(String),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Crypto error: {0}")]
    Crypto(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("API error: {0}")]
    Api(String),

    #[error("{0}")]
    Sync(#[from] crate::sync::failure::SyncFailure),

    #[error("{0}")]
    Other(String),
}

// Tauri command 要求错误类型实现 Serialize
impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
