use std::path::PathBuf;

/// Errors returned by the QuiverDL engine.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("only HTTP and HTTPS URLs are supported")]
    UnsupportedScheme,

    #[error("destination already exists: {0}")]
    DestinationExists(PathBuf),

    #[error("destination has no parent directory: {0}")]
    InvalidDestination(PathBuf),

    #[error("the server returned an invalid or inconsistent response: {0}")]
    InvalidResponse(String),

    #[error("download was cancelled")]
    Cancelled,

    #[error("downloaded content did not match the expected SHA-256 digest")]
    ChecksumMismatch,

    #[error(transparent)]
    Http(#[from] reqwest::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    State(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
