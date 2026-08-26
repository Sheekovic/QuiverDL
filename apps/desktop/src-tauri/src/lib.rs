use std::{
    collections::{BTreeMap, HashMap, HashSet},
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
use tokio::sync::{Notify, mpsc};
use url::Url;

mod browser_bridge;
mod clipboard_monitor;
mod media;
mod persistence;
mod proxy_credentials;
mod routing;
mod torrent;

use browser_bridge::{acknowledge_browser_request, get_browser_bridge_info, list_browser_requests};
use clipboard_monitor::{ClipboardMonitor, set_clipboard_monitor_enabled};
use media::{
    MediaRegistry, cancel_media_download, detect_media_url, inspect_media_url, start_media_download,
};
use persistence::{AppSettings, PersistentStore, load_app_state, save_app_state};
use proxy_credentials::{
    clear_proxy_credentials, has_proxy_credentials, load_proxy_password, save_proxy_credentials,
};
use routing::{resolve_category_directory, resolve_smart_destination};
use torrent::{
    TorrentRegistry, control_torrent_download, inspect_torrent_source, start_torrent_download,
};

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

struct TransferRegistry {
    transfers: Mutex<HashMap<String, ActiveTransfer>>,
    update_installing: Mutex<bool>,
    global_limiter: BandwidthLimiter,
    host_policy: HostConnectionPolicy,
    sequential_queue: Arc<SequentialQueue>,
}

impl Default for TransferRegistry {
    fn default() -> Self {
        Self {
            transfers: Mutex::default(),
            update_installing: Mutex::new(false),
            global_limiter: BandwidthLimiter::unlimited(),
            host_policy: HostConnectionPolicy::default(),
            sequential_queue: Arc::new(SequentialQueue::default()),
        }
    }
}

#[derive(Clone)]
struct ActiveTransfer {
    control: DownloadControl,
    reservation_keys: HashSet<String>,
    queue_ticket: Option<u64>,
    scheduled_for_ms: Option<u64>,
    started: bool,
}

#[derive(Default)]
struct SequentialQueue {
    state: Mutex<SequentialQueueState>,
    changed: Notify,
}

#[derive(Default)]
struct SequentialQueueState {
    active_ticket: Option<u64>,
    entries: BTreeMap<u64, Option<u64>>,
}

impl SequentialQueue {
    fn register(&self, ticket: u64, scheduled_for_ms: Option<u64>) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "The sequential queue is unavailable".to_string())?;
        match state.entries.entry(ticket) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(scheduled_for_ms);
            }
            std::collections::btree_map::Entry::Occupied(_) => {
                return Err("A download with this queue sequence is already registered".into());
            }
        }
        drop(state);
        self.changed.notify_waiters();
        Ok(())
    }

    fn remove(&self, ticket: u64) {
        if let Ok(mut state) = self.state.lock() {
            state.entries.remove(&ticket);
            if state.active_ticket == Some(ticket) {
                state.active_ticket = None;
            }
        }
        self.changed.notify_waiters();
    }

    async fn acquire(
        self: &Arc<Self>,
        ticket: u64,
        control: &DownloadControl,
    ) -> Result<SequentialPermit, String> {
        loop {
            let changed = self.changed.notified();
            {
                let now = unix_time_ms()?;
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| "The sequential queue is unavailable".to_string())?;
                let is_next_ready =
                    state
                        .entries
                        .iter()
                        .find_map(|(candidate, scheduled_for_ms)| {
                            scheduled_for_ms
                                .is_none_or(|scheduled| scheduled <= now)
                                .then_some(*candidate)
                        })
                        == Some(ticket);
                if state.active_ticket.is_none() && is_next_ready {
                    state.active_ticket = Some(ticket);
                    return Ok(SequentialPermit {
                        queue: self.clone(),
                        ticket,
                    });
                }
                if !state.entries.contains_key(&ticket) {
                    return Err("This download is no longer registered".into());
                }
            }
            tokio::select! {
                _ = control.cancelled() => {
                    self.remove(ticket);
                    return Err("download was cancelled".into());
                }
                () = changed => {}
            }
        }
    }
}

struct SequentialPermit {
    queue: Arc<SequentialQueue>,
    ticket: u64,
}

impl Drop for SequentialPermit {
    fn drop(&mut self) {
        self.queue.remove(self.ticket);
    }
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
    content_type: Option<String>,
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
        content_type: probe.content_type,
    })
}

#[tauri::command]
fn register_download(
    registry: State<'_, TransferRegistry>,
    task_id: String,
    queue_mode: String,
    queue_sequence: String,
    scheduled_for_ms: Option<String>,
) -> Result<(), String> {
    let task_id = validate_task_id(&task_id)?;
    if !matches!(queue_mode.as_str(), "parallel" | "sequential") {
        return Err("Unsupported queue mode".into());
    }
    let scheduled_for_ms = scheduled_for_ms
        .as_deref()
        .map(parse_queue_timestamp)
        .transpose()?;
    let queue_sequence = parse_queue_sequence(&queue_sequence)?;
    let update_installing = registry
        .update_installing
        .lock()
        .map_err(|_| "The update gate is unavailable".to_string())?;
    if *update_installing {
        return Err("Finish or cancel the pending app update before starting a download".into());
    }
    let mut transfers = registry
        .transfers
        .lock()
        .map_err(|_| "Download controls are unavailable".to_string())?;
    if transfers.contains_key(&task_id) {
        return Err("A download with this identifier is already registered".into());
    }
    let queue_ticket = if queue_mode == "sequential" {
        registry
            .sequential_queue
            .register(queue_sequence, scheduled_for_ms)?;
        Some(queue_sequence)
    } else {
        None
    };
    transfers.insert(
        task_id,
        ActiveTransfer {
            control: DownloadControl::new(),
            reservation_keys: HashSet::new(),
            queue_ticket,
            scheduled_for_ms,
            started: false,
        },
    );
    Ok(())
}

fn begin_update_install_guard(registry: &TransferRegistry) -> Result<(), String> {
    let mut update_installing = registry
        .update_installing
        .lock()
        .map_err(|_| "The update gate is unavailable".to_string())?;
    if *update_installing {
        return Err("An app update is already being installed".into());
    }
    if !registry
        .transfers
        .lock()
        .map_err(|_| "Download controls are unavailable".to_string())?
        .is_empty()
    {
        return Err("Finish or cancel every active and queued download before updating".into());
    }
    *update_installing = true;
    Ok(())
}

fn cancel_update_install_guard(registry: &TransferRegistry) -> Result<(), String> {
    *registry
        .update_installing
        .lock()
        .map_err(|_| "The update gate is unavailable".to_string())? = false;
    Ok(())
}

#[tauri::command]
fn begin_update_install(registry: State<'_, TransferRegistry>) -> Result<(), String> {
    begin_update_install_guard(&registry)
}

#[tauri::command]
fn cancel_update_install(registry: State<'_, TransferRegistry>) -> Result<(), String> {
    cancel_update_install_guard(&registry)
}

#[tauri::command]
fn discard_registered_download(
    registry: State<'_, TransferRegistry>,
    task_id: String,
) -> Result<(), String> {
    let task_id = validate_task_id(&task_id)?;
    let transfer = {
        let mut transfers = registry
            .transfers
            .lock()
            .map_err(|_| "Download controls are unavailable".to_string())?;
        if transfers
            .get(&task_id)
            .is_some_and(|transfer| transfer.started)
        {
            return Err("This download has already started".into());
        }
        transfers.remove(&task_id)
    };
    if let Some(transfer) = transfer {
        transfer.control.cancel();
        if let Some(ticket) = transfer.queue_ticket {
            registry.sequential_queue.remove(ticket);
        }
    }
    Ok(())
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
    let requested_schedule = scheduled_for_ms
        .as_deref()
        .map(parse_queue_timestamp)
        .transpose()?;
    let (control, queue_ticket, scheduled_for_ms) = claim_registered_transfer(
        &registry,
        &task_id,
        requested_schedule,
        settings.queue_mode == "sequential",
    )?;
    let _cleanup = RegisteredTransferCleanup {
        registry: &registry,
        task_id: task_id.clone(),
    };
    let (destination, _queue_permit) = prepare_and_reserve_destination(
        &registry,
        &task_id,
        &destination,
        &control,
        scheduled_for_ms,
        queue_ticket,
    )
    .await?;
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

pub(crate) fn parse_queue_sequence(value: &str) -> Result<u64, String> {
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("A queue sequence is invalid".into());
    }
    value
        .parse::<u64>()
        .map_err(|_| "A queue sequence is outside the supported range".to_string())
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
    queue_ticket: Option<u64>,
    sequential_queue: Arc<SequentialQueue>,
) -> Result<Option<SequentialPermit>, String> {
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

    let Some(queue_ticket) = queue_ticket else {
        return Ok(None);
    };
    let permit = sequential_queue.acquire(queue_ticket, control).await?;
    control
        .checkpoint()
        .await
        .map_err(|error| error.to_string())?;
    Ok(Some(permit))
}

async fn prepare_and_reserve_destination(
    registry: &TransferRegistry,
    task_id: &str,
    destination: &str,
    control: &DownloadControl,
    scheduled_for_ms: Option<u64>,
    queue_ticket: Option<u64>,
) -> Result<(PreparedDestination, Option<SequentialPermit>), String> {
    // Sequential work must own its FIFO turn before it can reserve a path. Otherwise a
    // newer task whose filesystem preparation finishes first could reject the older task.
    let queue_permit = wait_for_queue_turn(
        control,
        scheduled_for_ms,
        queue_ticket,
        registry.sequential_queue.clone(),
    )
    .await?;
    let destination = prepare_destination(destination).await?;
    control
        .checkpoint()
        .await
        .map_err(|error| error.to_string())?;

    {
        let mut transfers = registry
            .transfers
            .lock()
            .map_err(|_| "Download controls are unavailable".to_string())?;
        if transfers.iter().any(|(id, transfer)| {
            id != task_id
                && !transfer
                    .reservation_keys
                    .is_disjoint(&destination.reservation_keys)
        }) {
            return Err(
                "Another active download is already using this destination or its recovery files"
                    .into(),
            );
        }
        let transfer = transfers
            .get_mut(task_id)
            .ok_or_else(|| "This download is no longer registered".to_string())?;
        transfer.reservation_keys = destination.reservation_keys.clone();
    }

    Ok((destination, queue_permit))
}

fn remove_transfer(registry: &TransferRegistry, task_id: &str) {
    let ticket = registry
        .transfers
        .lock()
        .ok()
        .and_then(|mut transfers| transfers.remove(task_id))
        .and_then(|transfer| transfer.queue_ticket);
    if let Some(ticket) = ticket {
        registry.sequential_queue.remove(ticket);
    }
}

fn claim_registered_transfer(
    registry: &TransferRegistry,
    task_id: &str,
    requested_schedule: Option<u64>,
    sequential: bool,
) -> Result<(DownloadControl, Option<u64>, Option<u64>), String> {
    let mut transfers = registry
        .transfers
        .lock()
        .map_err(|_| "Download controls are unavailable".to_string())?;
    let transfer = transfers
        .get_mut(task_id)
        .ok_or_else(|| "Register this download before starting it".to_string())?;
    if transfer.started {
        return Err("This download has already started".into());
    }
    if transfer.scheduled_for_ms != requested_schedule
        || transfer.queue_ticket.is_some() != sequential
    {
        return Err("The registered queue policy does not match this download".into());
    }
    transfer.started = true;
    Ok((
        transfer.control.clone(),
        transfer.queue_ticket,
        transfer.scheduled_for_ms,
    ))
}

struct RegisteredTransferCleanup<'a> {
    registry: &'a TransferRegistry,
    task_id: String,
}

impl Drop for RegisteredTransferCleanup<'_> {
    fn drop(&mut self) {
        remove_transfer(self.registry, &self.task_id);
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
    let context = tauri::generate_context!();
    let updater_configured = context
        .config()
        .plugins
        .0
        .get("updater")
        .is_some_and(|config| !config.is_null());
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(
            |app, _arguments, _working_directory| {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            },
        ))
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_clipboard_manager::init());
    let builder = if updater_configured {
        builder.plugin(tauri_plugin_updater::Builder::new().build())
    } else {
        builder
    };
    builder
        .manage(TransferRegistry::default())
        .manage(ClipboardMonitor::default())
        .manage(MediaRegistry::default())
        .manage(TorrentRegistry::default())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            app.manage(PersistentStore::new(&app_data_dir));
            clipboard_monitor::start(app.handle().clone());
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
            inspect_media_url,
            detect_media_url,
            resolve_category_directory,
            resolve_smart_destination,
            set_clipboard_monitor_enabled,
            register_download,
            discard_registered_download,
            start_download,
            start_media_download,
            cancel_media_download,
            start_torrent_download,
            control_torrent_download,
            inspect_torrent_source,
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
            begin_update_install,
            cancel_update_install,
            quit_app
        ])
        .run(context)
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use quiver_core::DownloadStatus;

    use super::{
        ActiveTransfer, AppSettings, DownloadProgress, SequentialQueue, TransferRegistry,
        begin_update_install_guard, cancel_update_install_guard, claim_registered_transfer,
        parse_queue_sequence, parse_queue_timestamp, prepare_and_reserve_destination,
        prepare_destination, proxy_policy, remove_transfer, sanitize_filename,
        schedule_sleep_duration, validate_destination, validate_task_id, wait_for_queue_turn,
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
    fn validates_lossless_queue_sequences() {
        assert_eq!(parse_queue_sequence("18446744073709551615"), Ok(u64::MAX));
        assert!(parse_queue_sequence("18446744073709551616").is_err());
        assert!(parse_queue_sequence("1.5").is_err());
    }

    #[test]
    fn duplicate_queue_sequence_does_not_replace_the_original_deadline() {
        let queue = SequentialQueue::default();
        queue.register(7, None).expect("original ticket");
        assert!(queue.register(7, Some(32_503_680_000_000)).is_err());
        let state = queue.state.lock().expect("queue state");
        assert_eq!(state.entries.get(&7), Some(&None));
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

    #[test]
    fn duplicate_start_cannot_claim_or_remove_the_active_registration() {
        let registry = TransferRegistry::default();
        registry
            .sequential_queue
            .register(0, None)
            .expect("queue ticket");
        let ticket = 0;
        registry
            .transfers
            .lock()
            .expect("transfer registry")
            .insert(
                "duplicate-start".into(),
                ActiveTransfer {
                    control: quiver_core::DownloadControl::new(),
                    reservation_keys: std::collections::HashSet::new(),
                    queue_ticket: Some(ticket),
                    scheduled_for_ms: None,
                    started: false,
                },
            );

        claim_registered_transfer(&registry, "duplicate-start", None, true)
            .expect("first start owns cleanup");
        assert!(claim_registered_transfer(&registry, "duplicate-start", None, true).is_err());
        let transfers = registry.transfers.lock().expect("transfer registry");
        assert!(
            transfers
                .get("duplicate-start")
                .is_some_and(|item| item.started)
        );
    }

    #[test]
    fn update_install_guard_requires_an_idle_registry_and_can_be_released() {
        let registry = TransferRegistry::default();
        begin_update_install_guard(&registry).expect("idle registry can enter update mode");
        assert!(begin_update_install_guard(&registry).is_err());
        cancel_update_install_guard(&registry).expect("update mode can be cancelled");

        registry
            .transfers
            .lock()
            .expect("transfer registry")
            .insert(
                "active-download".into(),
                ActiveTransfer {
                    control: quiver_core::DownloadControl::new(),
                    reservation_keys: std::collections::HashSet::new(),
                    queue_ticket: None,
                    scheduled_for_ms: None,
                    started: true,
                },
            );
        assert!(begin_update_install_guard(&registry).is_err());
    }

    #[tokio::test]
    async fn sequential_queue_uses_registered_ticket_order() {
        let queue = std::sync::Arc::new(SequentialQueue::default());
        queue.register(0, None).expect("first ticket");
        queue.register(1, None).expect("second ticket");
        let first_ticket = 0;
        let second_ticket = 1;
        let second = tokio::spawn({
            let queue = queue.clone();
            async move {
                queue
                    .acquire(second_ticket, &quiver_core::DownloadControl::new())
                    .await
            }
        });
        tokio::task::yield_now().await;
        assert!(!second.is_finished());

        let first = queue
            .acquire(first_ticket, &quiver_core::DownloadControl::new())
            .await
            .expect("first permit");
        assert!(!second.is_finished());

        drop(first);
        assert!(second.await.expect("queue waiter should join").is_ok());
    }

    #[tokio::test]
    async fn older_sequential_ticket_reserves_a_shared_destination_first() {
        let registry = std::sync::Arc::new(TransferRegistry::default());
        registry
            .sequential_queue
            .register(0, None)
            .expect("older ticket");
        registry
            .sequential_queue
            .register(1, None)
            .expect("newer ticket");
        let older_control = quiver_core::DownloadControl::new();
        let newer_control = quiver_core::DownloadControl::new();
        {
            let mut transfers = registry.transfers.lock().expect("transfer registry");
            for (task_id, ticket, control) in [
                ("older", 0, older_control.clone()),
                ("newer", 1, newer_control.clone()),
            ] {
                transfers.insert(
                    task_id.into(),
                    ActiveTransfer {
                        control,
                        reservation_keys: std::collections::HashSet::new(),
                        queue_ticket: Some(ticket),
                        scheduled_for_ms: None,
                        started: true,
                    },
                );
            }
        }
        let directory = tempfile::tempdir().expect("temporary directory");
        let destination = directory.path().join("shared.bin");
        let destination = destination.to_string_lossy().into_owned();

        let newer = tokio::spawn({
            let registry = registry.clone();
            let destination = destination.clone();
            async move {
                prepare_and_reserve_destination(
                    &registry,
                    "newer",
                    &destination,
                    &newer_control,
                    None,
                    Some(1),
                )
                .await
            }
        });
        tokio::task::yield_now().await;
        assert!(!newer.is_finished());

        let (_destination, older_permit) = prepare_and_reserve_destination(
            &registry,
            "older",
            &destination,
            &older_control,
            None,
            Some(0),
        )
        .await
        .expect("older task should reserve first");
        assert!(!newer.is_finished());

        remove_transfer(&registry, "older");
        drop(older_permit);
        assert!(
            newer
                .await
                .expect("newer task should join")
                .expect("newer task should reserve after older cleanup")
                .1
                .is_some()
        );
    }

    #[tokio::test]
    async fn a_future_ticket_does_not_block_ready_work() {
        let queue = std::sync::Arc::new(SequentialQueue::default());
        queue
            .register(0, Some(32_503_680_000_000))
            .expect("future ticket");
        queue.register(1, None).expect("ready ticket");
        let ready = 1;
        queue
            .acquire(ready, &quiver_core::DownloadControl::new())
            .await
            .expect("ready work should acquire");
    }

    #[tokio::test]
    async fn a_due_scheduled_ticket_blocks_newer_ready_work_before_preparation_finishes() {
        let queue = std::sync::Arc::new(SequentialQueue::default());
        queue.register(0, Some(0)).expect("older due ticket");
        queue.register(1, None).expect("newer ready ticket");
        let newer = 1;
        let waiter = tokio::spawn({
            let queue = queue.clone();
            async move {
                queue
                    .acquire(newer, &quiver_core::DownloadControl::new())
                    .await
            }
        });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        waiter.abort();
        let _ = waiter.await;
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
                    None,
                    std::sync::Arc::new(SequentialQueue::default()),
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
