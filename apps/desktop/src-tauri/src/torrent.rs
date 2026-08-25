use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use librqbit::{
    AddTorrent, AddTorrentOptions, ConnectionOptions, ManagedTorrent, Session, SessionOptions,
    TorrentStatsState,
};
use serde::{Deserialize, Serialize};
use tauri::{State, ipc::Channel};
use url::Url;

use crate::persistence::AppSettings;

struct ActiveTorrent {
    session: Arc<Session>,
    handle: Arc<ManagedTorrent>,
    cancelled: Arc<AtomicBool>,
}

#[derive(Default)]
pub(crate) struct TorrentRegistry {
    active: Mutex<HashMap<String, ActiveTorrent>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TorrentProgress {
    status: String,
    downloaded_bytes: String,
    total_bytes: Option<String>,
    name: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TorrentSummary {
    destination: String,
    bytes_written: String,
    name: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TorrentInspection {
    name: String,
    source_type: String,
    network_origins: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TorrentDownloadRequest {
    task_id: String,
    source: String,
    destination_directory: String,
    settings: Option<AppSettings>,
    privacy_confirmed: bool,
}

#[tauri::command]
pub(crate) fn inspect_torrent_source(source: String) -> Result<TorrentInspection, String> {
    let source = validate_torrent_source(&source)?;
    let (name, source_type) = if source.to_ascii_lowercase().starts_with("magnet:") {
        let parsed = Url::parse(&source).map_err(|_| "The magnet link is invalid".to_string())?;
        let name = parsed
            .query_pairs()
            .find_map(|(key, value)| (key == "dn").then(|| value.into_owned()))
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "Magnet download".into());
        (super::sanitize_filename(Some(&name)), "magnet")
    } else {
        let parsed = Url::parse(&source).map_err(|_| "The .torrent URL is invalid".to_string())?;
        let name = parsed
            .path_segments()
            .and_then(Iterator::last)
            .filter(|name| !name.is_empty())
            .unwrap_or("Torrent download");
        (super::sanitize_filename(Some(name)), "torrentFile")
    };
    let network_origins = sanitized_network_origins(&source);
    Ok(TorrentInspection {
        name,
        source_type: source_type.into(),
        network_origins,
    })
}

#[tauri::command]
pub(crate) async fn start_torrent_download(
    registry: State<'_, TorrentRegistry>,
    transfer_registry: State<'_, super::TransferRegistry>,
    request: TorrentDownloadRequest,
    on_event: Channel<TorrentProgress>,
) -> Result<TorrentSummary, String> {
    let task_id = super::validate_task_id(&request.task_id)?;
    let settings = request.settings.unwrap_or_default();
    settings.validate()?;
    validate_network_start(&settings, request.privacy_confirmed)?;
    let (control, queue_ticket, scheduled_for_ms) = super::claim_registered_transfer(
        &transfer_registry,
        &task_id,
        None,
        settings.queue_mode == "sequential",
    )?;
    let _cleanup = super::RegisteredTransferCleanup {
        registry: &transfer_registry,
        task_id: task_id.clone(),
    };
    let _queue_permit = super::wait_for_queue_turn(
        &control,
        scheduled_for_ms,
        queue_ticket,
        transfer_registry.sequential_queue.clone(),
    )
    .await?;
    let source = validate_torrent_source(&request.source)?;
    let destination = prepare_destination_directory(&request.destination_directory).await?;
    let job_destination = destination.join(format!("QuiverDL-{task_id}"));
    tokio::fs::create_dir_all(&job_destination)
        .await
        .map_err(|error| format!("Could not create the isolated torrent folder: {error}"))?;
    let job_destination = tokio::fs::canonicalize(&job_destination)
        .await
        .map_err(|error| format!("Could not resolve the isolated torrent folder: {error}"))?;
    if !job_destination.starts_with(&destination) {
        return Err("The torrent folder escapes the selected destination".into());
    }
    let session_options = SessionOptions {
        dht: None,
        listen: None,
        connect: Some(ConnectionOptions::default()),
        concurrent_init_limit: Some(1),
        peer_limit: Some(80),
        disable_upload: true,
        disable_local_service_discovery: true,
        ..SessionOptions::default()
    };
    let session = Session::new_with_opts(job_destination, session_options)
        .await
        .map_err(|error| friendly_torrent_error("Could not initialize BitTorrent", &error))?;
    let options = AddTorrentOptions {
        // Each task owns an isolated folder, so rqbit can safely verify and resume its own files.
        overwrite: true,
        ..AddTorrentOptions::default()
    };
    let handle = session
        .add_torrent(AddTorrent::from_url(source.as_str()), Some(options))
        .await
        .map_err(|error| friendly_torrent_error("Could not add this torrent", &error))?
        .into_handle()
        .ok_or_else(|| "The torrent metadata could not be opened".to_string())?;
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut active = registry
            .active
            .lock()
            .map_err(|_| "Torrent controls are unavailable".to_string())?;
        if active.contains_key(&task_id) {
            return Err("A torrent with this identifier is already active".into());
        }
        active.insert(
            task_id.clone(),
            ActiveTorrent {
                session: session.clone(),
                handle: handle.clone(),
                cancelled: cancelled.clone(),
            },
        );
    }

    let result = loop {
        if cancelled.load(Ordering::Acquire) {
            break Err("download was cancelled".into());
        }
        let stats = handle.stats();
        let status = match stats.state {
            TorrentStatsState::Initializing { .. } => "probing",
            TorrentStatsState::Live => "downloading",
            TorrentStatsState::Paused => "paused",
            TorrentStatsState::Error => "failed",
        };
        if on_event
            .send(TorrentProgress {
                status: status.into(),
                downloaded_bytes: stats.progress_bytes.to_string(),
                total_bytes: (stats.total_bytes > 0).then(|| stats.total_bytes.to_string()),
                name: handle.name(),
            })
            .is_err()
        {
            break Err("The torrent progress listener closed".into());
        }
        if let Some(error) = stats.error {
            break Err(format!(
                "The torrent engine stopped: {}",
                bounded_message(&error)
            ));
        }
        if stats.finished {
            let name = handle.name().unwrap_or_else(|| "Torrent download".into());
            break Ok(TorrentSummary {
                destination: handle.output_folder().to_string_lossy().into_owned(),
                bytes_written: stats.progress_bytes.to_string(),
                name,
            });
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    let _ = session
        .delete(librqbit::api::TorrentIdOrHash::Id(handle.id()), false)
        .await;
    if let Ok(mut active) = registry.active.lock() {
        active.remove(&task_id);
    }
    result
}

fn sanitized_network_origins(source: &str) -> Vec<String> {
    let Ok(parsed) = Url::parse(source) else {
        return Vec::new();
    };
    let candidates = if parsed.scheme() == "magnet" {
        parsed
            .query_pairs()
            .filter_map(|(key, value)| (key == "tr").then_some(value.into_owned()))
            .filter_map(|value| Url::parse(&value).ok())
            .collect::<Vec<_>>()
    } else {
        vec![parsed]
    };
    let mut origins = candidates
        .into_iter()
        .filter_map(|tracker| {
            let host = tracker.host_str()?;
            let port = tracker
                .port()
                .map(|port| format!(":{port}"))
                .unwrap_or_default();
            Some(format!("{}://{host}{port}", tracker.scheme()))
        })
        .take(32)
        .collect::<Vec<_>>();
    origins.sort();
    origins.dedup();
    origins
}

fn validate_network_start(settings: &AppSettings, privacy_confirmed: bool) -> Result<(), String> {
    if !privacy_confirmed {
        return Err("Confirm the BitTorrent privacy disclosure before starting".into());
    }
    if settings.proxy_mode != "disabled" {
        return Err(
            "BitTorrent cannot guarantee coverage by the selected HTTP proxy. Switch to Direct connection or cancel the torrent"
                .into(),
        );
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn control_torrent_download(
    registry: State<'_, TorrentRegistry>,
    task_id: String,
    action: String,
) -> Result<(), String> {
    let task_id = super::validate_task_id(&task_id)?;
    let (session, handle, cancelled) = {
        let active = registry
            .active
            .lock()
            .map_err(|_| "Torrent controls are unavailable".to_string())?;
        let transfer = active
            .get(&task_id)
            .ok_or_else(|| "This torrent is no longer active".to_string())?;
        (
            transfer.session.clone(),
            transfer.handle.clone(),
            transfer.cancelled.clone(),
        )
    };
    match action.as_str() {
        "pause" => session
            .pause(&handle)
            .await
            .map_err(|error| friendly_torrent_error("Could not pause the torrent", &error)),
        "resume" => session
            .unpause(&handle)
            .await
            .map_err(|error| friendly_torrent_error("Could not resume the torrent", &error)),
        "cancel" => {
            cancelled.store(true, Ordering::Release);
            session
                .delete(librqbit::api::TorrentIdOrHash::Id(handle.id()), false)
                .await
                .map_err(|error| friendly_torrent_error("Could not cancel the torrent", &error))
        }
        _ => Err("Unsupported torrent control action".into()),
    }
}

fn validate_torrent_source(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 8_192 || value.chars().any(char::is_control) {
        return Err("The torrent link is invalid".into());
    }
    if value.to_ascii_lowercase().starts_with("magnet:") {
        librqbit::Magnet::parse(value).map_err(|_| "The magnet link is invalid".to_string())?;
        return Ok(value.to_owned());
    }
    let url = Url::parse(value).map_err(|_| "The .torrent URL is invalid".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(
            "Only credential-free HTTP, HTTPS, and magnet torrent links are supported".into(),
        );
    }
    Ok(value.to_owned())
}

async fn prepare_destination_directory(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if value.is_empty()
        || value.chars().count() > 4_096
        || value.chars().any(char::is_control)
        || !path.is_absolute()
    {
        return Err("The torrent destination must be an absolute local folder".into());
    }
    tokio::fs::create_dir_all(path)
        .await
        .map_err(|error| format!("Could not create the torrent destination: {error}"))?;
    tokio::fs::canonicalize(path)
        .await
        .map_err(|error| format!("Could not resolve the torrent destination: {error}"))
}

fn friendly_torrent_error(context: &str, error: &dyn std::fmt::Display) -> String {
    format!("{context}: {}", bounded_message(&error.to_string()))
}

fn bounded_message(message: &str) -> String {
    let first_line = message.lines().next().unwrap_or("unknown torrent error");
    first_line
        .split_whitespace()
        .map(|word| {
            let lower = word.to_ascii_lowercase();
            if lower.contains("magnet:") || lower.contains("http://") || lower.contains("https://")
            {
                "[torrent source]"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(500)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{sanitized_network_origins, validate_network_start, validate_torrent_source};
    use crate::persistence::AppSettings;

    #[test]
    fn accepts_magnets_and_remote_torrent_files() {
        assert!(
            validate_torrent_source("magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862")
                .is_ok()
        );
        assert!(validate_torrent_source("https://example.test/linux.torrent").is_ok());
    }

    #[test]
    fn rejects_credentials_and_local_files() {
        assert!(validate_torrent_source("https://user:secret@example.test/a.torrent").is_err());
        assert!(validate_torrent_source("file:///private/a.torrent").is_err());
    }

    #[test]
    fn requires_consent_and_a_direct_network_policy() {
        let direct = AppSettings::default();
        assert!(validate_network_start(&direct, false).is_err());
        assert!(validate_network_start(&direct, true).is_ok());
        let proxied = AppSettings {
            proxy_mode: "system".into(),
            ..AppSettings::default()
        };
        assert!(validate_network_start(&proxied, true).is_err());
    }

    #[test]
    fn tracker_previews_never_expose_paths_or_passkeys() {
        let origins = sanitized_network_origins(
            "magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862&tr=https%3A%2F%2Ftracker.example%2Fsecret%3Fpasskey%3Dabc",
        );
        assert_eq!(origins, ["https://tracker.example"]);
        assert!(!origins[0].contains("secret"));
        assert!(!origins[0].contains("abc"));
        assert_eq!(
            sanitized_network_origins("https://downloads.example/private/file.torrent?token=abc"),
            ["https://downloads.example"]
        );
    }
}
