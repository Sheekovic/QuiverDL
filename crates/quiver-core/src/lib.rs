//! Native, UI-independent download engine for QuiverDL.

mod bandwidth;
mod control;
mod engine;
mod error;
mod host_policy;
mod model;
mod state;

pub use bandwidth::BandwidthLimiter;
pub use control::DownloadControl;
pub use engine::{DownloadEngine, DownloadResult, ProbeResult};
pub use error::{Error, Result};
pub use host_policy::HostConnectionPolicy;
pub use model::{
    DownloadId, DownloadRequest, DownloadStatus, ProgressEvent, RetryPolicy, TransferPolicy,
};
