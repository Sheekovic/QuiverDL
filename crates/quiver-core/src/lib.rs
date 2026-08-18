//! Native, UI-independent download engine for QuiverDL.

mod control;
mod engine;
mod error;
mod model;
mod state;

pub use control::DownloadControl;
pub use engine::{DownloadEngine, DownloadResult, ProbeResult};
pub use error::{Error, Result};
pub use model::{DownloadId, DownloadRequest, DownloadStatus, ProgressEvent};
