use thiserror::Error;

#[derive(Debug, Error)]
pub enum CollectorError {
    #[error("OS API error: {0}")]
    OsApi(String),

    #[error("Permission denied accessing {resource}")]
    PermissionDenied { resource: String },

    #[error("Collection timed out after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Not supported on this platform")]
    NotSupported,

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
