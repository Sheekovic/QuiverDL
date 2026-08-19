use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

use quiver_core::{
    DownloadControl, DownloadEngine, DownloadRequest, DownloadStatus, ProgressEvent,
};
use serde::Serialize;
use tauri::{State, ipc::Channel};
use tokio::sync::mpsc;
use url::Url;

#[derive(Default)]
struct TransferRegistry {
    transfers: Mutex<HashMap<String, ActiveTransfer>>,
}

#[derive(Clone)]
struct ActiveTransfer {
    control: DownloadControl,
    destination_key: String,
}

struct PreparedDestination {
    path: PathBuf,
    lock_key: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LinkInspection {
    effective_url: String,
    total_bytes: Option<String>,
    supports_ranges: bool,
    has_validator: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    status: DownloadStatus,
    downloaded_bytes: String,
    total_bytes: Option<String>,
}

impl From<ProgressEvent> for DownloadProgress {
    fn from(event: ProgressEvent) -> Self {
        Self {
            status: event.status,
            downloaded_bytes: event.downloaded_bytes.to_string(),
            total_bytes: event.total_bytes.map(|bytes| bytes.to_string()),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadSummary {
    bytes_written: String,
    sha256: String,
    resumed: bool,
}

#[tauri::command]
async fn inspect_url(url: String) -> Result<LinkInspection, String> {
    let url = Url::parse(url.trim()).map_err(|error| format!("Invalid URL: {error}"))?;
    let probe = DownloadEngine::new()
        .map_err(|error| error.to_string())?
        .probe(&url)
        .await
        .map_err(|error| error.to_string())?;

    Ok(LinkInspection {
        effective_url: probe.effective_url.to_string(),
        total_bytes: probe.total_bytes.map(|bytes| bytes.to_string()),
        supports_ranges: probe.supports_ranges,
        has_validator: probe.etag.is_some() || probe.last_modified.is_some(),
    })
}

#[tauri::command]
async fn start_download(
    registry: State<'_, TransferRegistry>,
    task_id: String,
    url: String,
    destination: String,
    on_event: Channel<DownloadProgress>,
) -> Result<DownloadSummary, String> {
    let task_id = validate_task_id(&task_id)?;
    let url = Url::parse(url.trim()).map_err(|error| format!("Invalid URL: {error}"))?;
    let destination = prepare_destination(&destination).await?;
    let engine = DownloadEngine::new().map_err(|error| error.to_string())?;
    let request = DownloadRequest::new(url, &destination.path);
    let control = DownloadControl::new();

    {
        let mut transfers = registry
            .transfers
            .lock()
            .map_err(|_| "Download controls are unavailable".to_string())?;
        if transfers.contains_key(&task_id) {
            return Err("A download with this identifier is already active".into());
        }
        if transfers
            .values()
            .any(|transfer| transfer.destination_key == destination.lock_key)
        {
            return Err("Another active download is already using this destination".into());
        }
        transfers.insert(
            task_id.clone(),
            ActiveTransfer {
                control: control.clone(),
                destination_key: destination.lock_key,
            },
        );
    }

    let (progress_tx, mut progress_rx) = mpsc::channel::<ProgressEvent>(32);
    let forwarding_control = control.clone();
    let forwarder = tauri::async_runtime::spawn(async move {
        let mut last_download_progress = Instant::now()
            .checked_sub(Duration::from_millis(100))
            .unwrap_or_else(Instant::now);
        while let Some(event) = progress_rx.recv().await {
            if event.status == DownloadStatus::Downloading
                && last_download_progress.elapsed() < Duration::from_millis(100)
            {
                continue;
            }
            if on_event.send(event.into()).is_err() {
                forwarding_control.cancel();
                break;
            }
            last_download_progress = Instant::now();
        }
    });

    let result = engine.download(request, control, progress_tx).await;
    if let Ok(mut transfers) = registry.transfers.lock() {
        transfers.remove(&task_id);
    }
    let _ = forwarder.await;

    let result = result.map_err(|error| error.to_string())?;
    Ok(DownloadSummary {
        bytes_written: result.bytes_written.to_string(),
        sha256: result
            .sha256
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        resumed: result.resumed,
    })
}

#[tauri::command]
fn control_download(
    registry: State<'_, TransferRegistry>,
    task_id: String,
    action: String,
) -> Result<(), String> {
    let task_id = validate_task_id(&task_id)?;
    let control = registry
        .transfers
        .lock()
        .map_err(|_| "Download controls are unavailable".to_string())?
        .get(&task_id)
        .map(|transfer| transfer.control.clone())
        .ok_or_else(|| "This download is no longer active".to_string())?;

    match action.as_str() {
        "pause" => control.pause(),
        "resume" => control.resume(),
        "cancel" => control.cancel(),
        _ => return Err("Unsupported download action".into()),
    }
    Ok(())
}

fn validate_task_id(task_id: &str) -> Result<String, String> {
    let task_id = task_id.trim();
    if task_id.is_empty()
        || task_id.len() > 128
        || !task_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("Invalid download identifier".into());
    }
    Ok(task_id.to_owned())
}

fn validate_destination(destination: &str) -> Result<PathBuf, String> {
    let destination = destination.trim();
    if destination.is_empty() {
        return Err("Choose where to save the download".into());
    }
    let destination = PathBuf::from(destination);
    if !destination.is_absolute() || destination.file_name().is_none() {
        return Err("The download destination must be an absolute file path".into());
    }
    Ok(normalize_path(&destination))
}

async fn prepare_destination(destination: &str) -> Result<PreparedDestination, String> {
    let destination = validate_destination(destination)?;
    let file_name = destination
        .file_name()
        .ok_or_else(|| "The download destination must include a file name".to_string())?
        .to_owned();
    let parent = destination
        .parent()
        .ok_or_else(|| "The download destination must include a parent directory".to_string())?;

    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| format!("Could not create the destination directory: {error}"))?;
    let canonical_parent = tokio::fs::canonicalize(parent)
        .await
        .map_err(|error| format!("Could not resolve the destination directory: {error}"))?;
    let path = canonical_parent.join(file_name);
    let path = if tokio::fs::try_exists(&path)
        .await
        .map_err(|error| format!("Could not inspect the destination: {error}"))?
    {
        tokio::fs::canonicalize(path)
            .await
            .map_err(|error| format!("Could not resolve the destination: {error}"))?
    } else {
        path
    };

    Ok(PreparedDestination {
        lock_key: destination_lock_key(&path),
        path,
    })
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn destination_lock_key(path: &Path) -> String {
    let key = path.to_string_lossy().into_owned();
    if cfg!(any(target_os = "windows", target_os = "macos")) {
        key.to_lowercase()
    } else {
        key
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(TransferRegistry::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            inspect_url,
            start_download,
            control_download
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use quiver_core::DownloadStatus;

    use super::{DownloadProgress, prepare_destination, validate_destination, validate_task_id};

    #[test]
    fn validates_download_identifiers() {
        assert_eq!(validate_task_id("download-42"), Ok("download-42".into()));
        assert!(validate_task_id("../download").is_err());
        assert!(validate_task_id("").is_err());
    }

    #[test]
    fn requires_an_absolute_destination() {
        let absolute = if cfg!(windows) {
            r"C:\Downloads\archive.zip"
        } else {
            "/tmp/archive.zip"
        };
        assert!(validate_destination(absolute).is_ok());
        assert!(validate_destination("archive.zip").is_err());
    }

    #[tokio::test]
    async fn equivalent_destinations_share_a_lock_key() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let nested = directory.path().join("nested");
        tokio::fs::create_dir_all(&nested)
            .await
            .expect("nested directory");
        let direct = directory.path().join("archive.zip");
        let redundant = nested.join("..").join("archive.zip");

        let direct = prepare_destination(direct.to_str().expect("UTF-8 path"))
            .await
            .expect("direct destination");
        let redundant = prepare_destination(redundant.to_str().expect("UTF-8 path"))
            .await
            .expect("redundant destination");
        assert_eq!(direct.lock_key, redundant.lock_key);
    }

    #[test]
    fn serializes_byte_counts_losslessly_as_strings() {
        let progress = DownloadProgress {
            status: DownloadStatus::Downloading,
            downloaded_bytes: u64::MAX.to_string(),
            total_bytes: Some(u64::MAX.to_string()),
        };
        let value = serde_json::to_value(progress).expect("progress should serialize");
        assert_eq!(
            value["downloadedBytes"],
            serde_json::Value::String(u64::MAX.to_string())
        );
        assert_eq!(
            value["totalBytes"],
            serde_json::Value::String(u64::MAX.to_string())
        );
    }
}
