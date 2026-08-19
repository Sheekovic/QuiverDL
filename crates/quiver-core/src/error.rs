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

    #[error("the server returned a temporary response: {0}")]
    TransientResponse(String),

    #[error("invalid proxy configuration: {0}")]
    InvalidProxyConfiguration(String),

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

impl Error {
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Http(_) | Self::TransientResponse(_))
    }
}
