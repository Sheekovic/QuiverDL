use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::{Component, Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

use quiver_core::{
    BandwidthLimiter, DownloadControl, DownloadEngine, DownloadRequest, DownloadStatus,
    HostConnectionPolicy, ProgressEvent, RetryPolicy, TransferPolicy,
};
use serde::Serialize;
use tauri::{
    Manager, State, WindowEvent,
    ipc::Channel,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};
use tokio::sync::mpsc;
use url::Url;

mod browser_bridge;
mod persistence;

use browser_bridge::{acknowledge_browser_request, get_browser_bridge_info, list_browser_requests};
use persistence::{AppSettings, PersistentStore, load_app_state, save_app_state};

#[derive(Default)]
struct TransferRegistry {
    transfers: Mutex<HashMap<String, ActiveTransfer>>,
    global_limiter: Mutex<Option<BandwidthLimiter>>,
    host_policy: HostConnectionPolicy,
}

#[derive(Clone)]
struct ActiveTransfer {
    control: DownloadControl,
    reservation_keys: HashSet<String>,
}

struct PreparedDestination {
    path: PathBuf,
    reservation_keys: HashSet<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LinkInspection {
    effective_url: String,
    total_bytes: Option<String>,
    supports_ranges: bool,
    has_validator: bool,
    suggested_filename: String,
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
        suggested_filename: sanitize_filename(probe.suggested_filename.as_deref()),
    })
}

#[tauri::command]
async fn start_download(
    registry: State<'_, TransferRegistry>,
    task_id: String,
    url: String,
    destination: String,
    settings: Option<AppSettings>,
    on_event: Channel<DownloadProgress>,
) -> Result<DownloadSummary, String> {
    let task_id = validate_task_id(&task_id)?;
    let url = Url::parse(url.trim()).map_err(|error| format!("Invalid URL: {error}"))?;
    let destination = prepare_destination(&destination).await?;
    let settings = settings.unwrap_or_default();
    let global_limiter =
        if let Some(limit) = settings.global_speed_limit_bps.filter(|limit| *limit > 0) {
            let mut limiter = registry
                .global_limiter
                .lock()
                .map_err(|_| "Global speed controls are unavailable".to_string())?;
            if let Some(existing) = limiter.as_ref() {
                existing.set_bytes_per_second(limit);
                Some(existing.clone())
            } else {
                let created = BandwidthLimiter::new(limit).expect("positive speed limit");
                *limiter = Some(created.clone());
                Some(created)
            }
        } else {
            None
        };
    let engine = DownloadEngine::new()
        .map_err(|error| error.to_string())?
        .with_global_limiter(global_limiter)
        .with_host_policy(registry.host_policy.clone());
    let mut request = DownloadRequest::new(url, &destination.path);
    request.retry_policy = RetryPolicy {
        max_attempts: settings.retry_attempts,
        initial_delay_ms: settings.retry_initial_delay_ms,
        max_delay_ms: settings.retry_max_delay_ms,
    };
    request.transfer_policy = TransferPolicy {
        max_segments: settings.max_segments,
        max_connections_per_host: settings.max_connections_per_host,
        min_segment_bytes: 8 * 1024 * 1024,
        per_download_speed_limit_bps: settings.per_download_speed_limit_bps,
    };
    let control = DownloadControl::new();

    {
        let mut transfers = registry
            .transfers
            .lock()
            .map_err(|_| "Download controls are unavailable".to_string())?;
        if transfers.contains_key(&task_id) {
            return Err("A download with this identifier is already active".into());
        }
        if transfers.values().any(|transfer| {
            !transfer
                .reservation_keys
                .is_disjoint(&destination.reservation_keys)
        }) {
            return Err(
                "Another active download is already using this destination or its recovery files"
                    .into(),
            );
        }
        transfers.insert(
            task_id.clone(),
            ActiveTransfer {
                control: control.clone(),
                reservation_keys: destination.reservation_keys,
            },
        );
    }

    let (progress_tx, mut progress_rx) = mpsc::channel::<ProgressEvent>(32);
    let forwarding_control = control.clone();
    let forwarder = tauri::async_runtime::spawn(async move {
        let mut last_download_progress = Instant::now()
            .checked_sub(Duration::from_millis(250))
            .unwrap_or_else(Instant::now);
        while let Some(event) = progress_rx.recv().await {
            if event.status == DownloadStatus::Downloading
                && last_download_progress.elapsed() < Duration::from_millis(250)
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
    let destination = normalize_path(&destination);
    if !destination.is_absolute() || destination.file_name().is_none() {
        return Err("The normalized download destination must be an absolute file path".into());
    }
    Ok(destination)
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
        reservation_keys: destination_reservation_keys(&path),
        path,
    })
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if normalized.file_name().is_some() {
                    let _ = normalized.pop();
                }
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

fn destination_reservation_keys(path: &Path) -> HashSet<String> {
    let partial = sibling_with_suffix(path, ".quiver-part");
    let state = sibling_with_suffix(path, ".quiver.json");
    let state_temporary = sibling_with_suffix(&state, ".tmp");
    let mut paths = vec![path.to_path_buf(), partial.clone(), state, state_temporary];
    paths.extend((0..16).map(|index| sibling_with_suffix(&partial, &format!(".segment-{index}"))));
    paths
        .into_iter()
        .map(|path| destination_lock_key(&path))
        .collect()
}

fn sibling_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn sanitize_filename(value: Option<&str>) -> String {
    let value = value.unwrap_or("download.bin");
    let mut sanitized: String = value
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
            {
                '_'
            } else {
                character
            }
        })
        .take(180)
        .collect();
    while sanitized.ends_with(['.', ' ']) {
        sanitized.pop();
    }
    if sanitized.is_empty() || matches!(sanitized.as_str(), "." | "..") {
        "download.bin".into()
    } else {
        sanitized
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(TransferRegistry::default())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            app.manage(PersistentStore::new(&app_data_dir));
            let show = MenuItem::with_id(app, "show", "Show QuiverDL", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray =
                TrayIconBuilder::new()
                    .menu(&menu)
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            inspect_url,
            start_download,
            control_download,
            load_app_state,
            save_app_state,
            get_browser_bridge_info,
            list_browser_requests,
            acknowledge_browser_request
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use quiver_core::DownloadStatus;

    use super::{
        DownloadProgress, prepare_destination, sanitize_filename, validate_destination,
        validate_task_id,
    };

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

        let root_escape = if cfg!(windows) {
            r"C:\..\archive.zip"
        } else {
            "/../archive.zip"
        };
        let normalized = validate_destination(root_escape).expect("root escape stays absolute");
        assert!(normalized.is_absolute());
        assert_eq!(
            normalized.file_name(),
            Some(std::ffi::OsStr::new("archive.zip"))
        );
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
        assert_eq!(direct.reservation_keys, redundant.reservation_keys);
    }

    #[tokio::test]
    async fn destination_reservations_include_recovery_sidecars() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let destination = directory.path().join("report");
        let sidecar_destination = directory.path().join("report.quiver-part");

        let destination = prepare_destination(destination.to_str().expect("UTF-8 path"))
            .await
            .expect("destination");
        let sidecar_destination =
            prepare_destination(sidecar_destination.to_str().expect("UTF-8 path"))
                .await
                .expect("sidecar destination");

        assert!(
            !destination
                .reservation_keys
                .is_disjoint(&sidecar_destination.reservation_keys)
        );
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

    #[test]
    fn sanitizes_untrusted_server_filenames() {
        assert_eq!(
            sanitize_filename(Some("../bad<name>.zip")),
            ".._bad_name_.zip"
        );
        assert_eq!(sanitize_filename(Some("...   ")), "download.bin");
        assert_eq!(sanitize_filename(None), "download.bin");
    }
}
