use std::{fmt, path::PathBuf, str::FromStr};

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

impl fmt::Display for DownloadId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for DownloadId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

impl Default for DownloadId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TransferPolicy {
    pub max_segments: u8,
    pub max_connections_per_host: u8,
    pub min_segment_bytes: u64,
    pub per_download_speed_limit_bps: Option<u64>,
}

impl Default for TransferPolicy {
    fn default() -> Self {
        Self {
            max_segments: 4,
            max_connections_per_host: 8,
            min_segment_bytes: 8 * 1024 * 1024,
            per_download_speed_limit_bps: None,
        }
    }
}

impl TransferPolicy {
    #[must_use]
    pub fn normalized(self) -> Self {
        Self {
            max_segments: self.max_segments.clamp(1, 16),
            max_connections_per_host: self.max_connections_per_host.clamp(1, 32),
            min_segment_bytes: self
                .min_segment_bytes
                .clamp(1024 * 1024, 1024 * 1024 * 1024),
            per_download_speed_limit_bps: self
                .per_download_speed_limit_bps
                .filter(|limit| *limit > 0),
        }
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay_ms: 750,
            max_delay_ms: 15_000,
        }
    }
}

impl RetryPolicy {
    #[must_use]
    pub fn normalized(self) -> Self {
        let initial_delay_ms = self.initial_delay_ms.clamp(100, 60_000);
        Self {
            max_attempts: self.max_attempts.clamp(1, 10),
            initial_delay_ms,
            max_delay_ms: self.max_delay_ms.clamp(initial_delay_ms, 300_000),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub id: DownloadId,
    pub url: Url,
    pub destination: PathBuf,
    pub expected_sha256: Option<[u8; 32]>,
    pub overwrite_existing: bool,
    pub retry_policy: RetryPolicy,
    pub transfer_policy: TransferPolicy,
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
            retry_policy: RetryPolicy::default(),
            transfer_policy: TransferPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    Probing,
    Retrying,
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
