use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use librqbit::{AddTorrent, AddTorrentOptions, ManagedTorrent, Session, TorrentStatsState};
use serde::Serialize;
use tauri::{State, ipc::Channel};
use url::Url;

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
    Ok(TorrentInspection {
        name,
        source_type: source_type.into(),
    })
}

#[tauri::command]
pub(crate) async fn start_torrent_download(
    registry: State<'_, TorrentRegistry>,
    task_id: String,
    source: String,
    destination_directory: String,
    on_event: Channel<TorrentProgress>,
) -> Result<TorrentSummary, String> {
    let task_id = super::validate_task_id(&task_id)?;
    let source = validate_torrent_source(&source)?;
    let destination = prepare_destination_directory(&destination_directory).await?;
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
    let session = Session::new(job_destination)
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
    if let Ok(mut active) = registry.active.lock() {
        active.remove(&task_id);
    }
    result
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
    use super::validate_torrent_source;

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
}
