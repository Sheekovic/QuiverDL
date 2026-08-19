use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use quiver_core::{
    BandwidthLimiter, DownloadControl, DownloadEngine, DownloadRequest, DownloadStatus,
    HostConnectionPolicy, ProgressEvent, ProxyConfig, ProxyPolicy, RetryPolicy, TransferPolicy,
};
use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, State, WindowEvent,
    ipc::Channel,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use url::Url;

mod browser_bridge;
mod persistence;
mod proxy_credentials;

use browser_bridge::{acknowledge_browser_request, get_browser_bridge_info, list_browser_requests};
use persistence::{AppSettings, PersistentStore, load_app_state, save_app_state};
use proxy_credentials::{
    clear_proxy_credentials, has_proxy_credentials, load_proxy_password, save_proxy_credentials,
};

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

struct TransferRegistry {
    transfers: Mutex<HashMap<String, ActiveTransfer>>,
    global_limiter: BandwidthLimiter,
    host_policy: HostConnectionPolicy,
    sequential_gate: Arc<Semaphore>,
}

impl Default for TransferRegistry {
    fn default() -> Self {
        Self {
            transfers: Mutex::default(),
            global_limiter: BandwidthLimiter::unlimited(),
            host_policy: HostConnectionPolicy::default(),
            sequential_gate: Arc::new(Semaphore::new(1)),
        }
    }
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
async fn inspect_url(url: String, settings: Option<AppSettings>) -> Result<LinkInspection, String> {
    let url = Url::parse(url.trim()).map_err(|error| format!("Invalid URL: {error}"))?;
    let settings = settings.unwrap_or_default();
    settings.validate()?;
    let probe = DownloadEngine::new_with_proxy(proxy_policy(&settings).await?)
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
    scheduled_for_ms: Option<String>,
    on_event: Channel<DownloadProgress>,
) -> Result<DownloadSummary, String> {
    let task_id = validate_task_id(&task_id)?;
    let url = Url::parse(url.trim()).map_err(|error| format!("Invalid URL: {error}"))?;
    let settings = settings.unwrap_or_default();
    settings.validate()?;
    let scheduled_for_ms = scheduled_for_ms
        .as_deref()
        .map(parse_queue_timestamp)
        .transpose()?;
    let destination = prepare_destination(&destination).await?;
    let engine = DownloadEngine::new_with_proxy(proxy_policy(&settings).await?)
        .map_err(|error| error.to_string())?
        .with_global_limiter(Some(registry.global_limiter.clone()))
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

    let _queue_permit = match wait_for_queue_turn(
        &control,
        scheduled_for_ms,
        &settings.queue_mode,
        registry.sequential_gate.clone(),
    )
    .await
    {
        Ok(permit) => permit,
        Err(error) => {
            remove_transfer(&registry, &task_id);
            return Err(error);
        }
    };

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
    remove_transfer(&registry, &task_id);
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

const MAX_QUEUE_TIMESTAMP_MS: u64 = 32_503_680_000_000;
const MAX_SCHEDULE_SLEEP: Duration = Duration::from_secs(30);

pub(crate) fn parse_queue_timestamp(value: &str) -> Result<u64, String> {
    if value.is_empty() || value.len() > 14 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("A queue timestamp is invalid".into());
    }
    let timestamp = value
        .parse::<u64>()
        .map_err(|_| "A queue timestamp is invalid".to_string())?;
    if timestamp > MAX_QUEUE_TIMESTAMP_MS {
        return Err("A queue timestamp is outside the supported range".into());
    }
    Ok(timestamp)
}

fn unix_time_ms() -> Result<u64, String> {
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "The system clock is before the Unix epoch".to_string())?
        .as_millis();
    u64::try_from(milliseconds)
        .map_err(|_| "The system clock is outside the supported range".into())
}

fn schedule_sleep_duration(scheduled_for_ms: u64, now_ms: u64) -> Option<Duration> {
    scheduled_for_ms
        .checked_sub(now_ms)
        .filter(|remaining| *remaining > 0)
        .map(Duration::from_millis)
        .map(|remaining| remaining.min(MAX_SCHEDULE_SLEEP))
}

async fn wait_for_queue_turn(
    control: &DownloadControl,
    scheduled_for_ms: Option<u64>,
    queue_mode: &str,
    sequential_gate: Arc<Semaphore>,
) -> Result<Option<OwnedSemaphorePermit>, String> {
    if let Some(scheduled_for_ms) = scheduled_for_ms {
        loop {
            let now = unix_time_ms()?;
            let Some(delay) = schedule_sleep_duration(scheduled_for_ms, now) else {
                break;
            };
            tokio::select! {
                _ = control.cancelled() => return Err("download was cancelled".into()),
                () = tokio::time::sleep(delay) => {}
            }
        }
    }
    control
        .checkpoint()
        .await
        .map_err(|error| error.to_string())?;

    if queue_mode != "sequential" {
        return Ok(None);
    }
    let permit = tokio::select! {
        _ = control.cancelled() => return Err("download was cancelled".into()),
        permit = sequential_gate.acquire_owned() => {
            permit.map_err(|_| "The sequential queue is unavailable".to_string())?
        }
    };
    control
        .checkpoint()
        .await
        .map_err(|error| error.to_string())?;
    Ok(Some(permit))
}

fn remove_transfer(registry: &TransferRegistry, task_id: &str) {
    if let Ok(mut transfers) = registry.transfers.lock() {
        transfers.remove(task_id);
    }
}

async fn proxy_policy(settings: &AppSettings) -> Result<ProxyPolicy, String> {
    match settings.proxy_mode.as_str() {
        "disabled" => Ok(ProxyPolicy::Disabled),
        "system" => Ok(ProxyPolicy::System),
        "custom" => {
            let endpoint = Url::parse(settings.proxy_url.trim())
                .map_err(|_| "The custom proxy URL is invalid".to_string())?;
            let credential_endpoint = endpoint.to_string();
            let mut config = ProxyConfig::new(endpoint).map_err(|error| error.to_string())?;
            if !settings.proxy_bypass.trim().is_empty() {
                config = config
                    .with_bypass_list(settings.proxy_bypass.clone())
                    .map_err(|error| error.to_string())?;
            }
            if !settings.proxy_username.is_empty() {
                let password =
                    load_proxy_password(credential_endpoint, settings.proxy_username.clone())
                        .await?
                        .ok_or_else(|| {
                            "Save proxy credentials for the configured username before connecting"
                                .to_string()
                        })?;
                config = config
                    .with_basic_auth(settings.proxy_username.clone(), password)
                    .map_err(|error| error.to_string())?;
            }
            Ok(ProxyPolicy::Custom(config))
        }
        _ => Err("Unsupported proxy mode".into()),
    }
}

#[tauri::command]
fn validate_proxy_configuration(settings: AppSettings) -> Result<(), String> {
    settings.validate()
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

#[tauri::command]
fn set_global_speed_limit(
    registry: State<'_, TransferRegistry>,
    bytes_per_second: Option<u64>,
) -> Result<(), String> {
    if bytes_per_second.is_some_and(|limit| !(1024..=1024_u64.pow(4)).contains(&limit)) {
        return Err("Global speed limit must be between 1 KiB/s and 1 TiB/s".into());
    }
    registry
        .global_limiter
        .set_bytes_per_second(bytes_per_second.unwrap_or(0));
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
        .plugin(tauri_plugin_single_instance::init(
            |app, _arguments, _working_directory| {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            },
        ))
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
                        "quit" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.emit("quit-requested", ());
                            }
                        }
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
            set_global_speed_limit,
            load_app_state,
            save_app_state,
            get_browser_bridge_info,
            list_browser_requests,
            acknowledge_browser_request,
            save_proxy_credentials,
            clear_proxy_credentials,
            has_proxy_credentials,
            validate_proxy_configuration,
            quit_app
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use quiver_core::DownloadStatus;

    use super::{
        AppSettings, DownloadProgress, TransferRegistry, parse_queue_timestamp,
        prepare_destination, proxy_policy, sanitize_filename, schedule_sleep_duration,
        validate_destination, validate_task_id, wait_for_queue_turn,
    };

    #[test]
    fn validates_download_identifiers() {
        assert_eq!(validate_task_id("download-42"), Ok("download-42".into()));
        assert!(validate_task_id("../download").is_err());
        assert!(validate_task_id("").is_err());
    }

    #[test]
    fn validates_queue_timestamps_without_lossy_number_conversion() {
        assert_eq!(
            parse_queue_timestamp("1770000000000"),
            Ok(1_770_000_000_000)
        );
        assert!(parse_queue_timestamp("").is_err());
        assert!(parse_queue_timestamp("1770.5").is_err());
        assert!(parse_queue_timestamp("32503680000001").is_err());
    }

    #[test]
    fn schedule_waits_recheck_long_wall_clock_deadlines() {
        assert_eq!(
            schedule_sleep_duration(1_000_000, 1),
            Some(std::time::Duration::from_secs(30))
        );
        assert_eq!(
            schedule_sleep_duration(1_001, 1_000),
            Some(std::time::Duration::from_millis(1))
        );
        assert_eq!(schedule_sleep_duration(1_000, 1_000), None);
        assert_eq!(schedule_sleep_duration(999, 1_000), None);
    }

    #[tokio::test]
    async fn sequential_queue_waits_for_the_previous_permit() {
        let registry = TransferRegistry::default();
        let first = registry
            .sequential_gate
            .clone()
            .acquire_owned()
            .await
            .expect("first queue permit");
        let waiter = tokio::spawn({
            let gate = registry.sequential_gate.clone();
            async move {
                wait_for_queue_turn(
                    &quiver_core::DownloadControl::new(),
                    None,
                    "sequential",
                    gate,
                )
                .await
            }
        });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        drop(first);
        assert!(waiter.await.expect("queue waiter should join").is_ok());
    }

    #[tokio::test]
    async fn scheduled_wait_can_be_cancelled() {
        let control = quiver_core::DownloadControl::new();
        let waiter = tokio::spawn({
            let control = control.clone();
            async move {
                wait_for_queue_turn(
                    &control,
                    Some(32_503_680_000_000),
                    "parallel",
                    std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
                )
                .await
            }
        });
        control.cancel();
        assert!(waiter.await.expect("scheduled waiter should join").is_err());
    }

    #[tokio::test]
    async fn rejects_credentials_embedded_in_custom_proxy_urls() {
        let settings = AppSettings {
            proxy_mode: "custom".into(),
            proxy_url: "http://user:secret@proxy.example:8080".into(),
            ..AppSettings::default()
        };
        let error = proxy_policy(&settings)
            .await
            .expect_err("embedded credentials must be rejected");
        assert!(error.contains("must not be embedded"));
        assert!(!error.contains("secret"));
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
