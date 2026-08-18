use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

/// Stable identifier for a download across process restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DownloadId(Uuid);

impl DownloadId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for DownloadId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub id: DownloadId,
    pub url: Url,
    pub destination: PathBuf,
    pub expected_sha256: Option<[u8; 32]>,
    pub overwrite_existing: bool,
}

impl DownloadRequest {
    #[must_use]
    pub fn new(url: Url, destination: impl Into<PathBuf>) -> Self {
        Self {
            id: DownloadId::new(),
            url,
            destination: destination.into(),
            expected_sha256: None,
            overwrite_existing: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    Probing,
    Downloading,
    Paused,
    Verifying,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressEvent {
    pub id: DownloadId,
    pub status: DownloadStatus,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}
