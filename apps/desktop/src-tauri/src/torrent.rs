use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
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
use librqbit_bencode::{BencodeValue, BencodeValueBorrowed, from_bytes};
use percent_encoding::{NON_ALPHANUMERIC, percent_encode};
use rand::Rng;
use serde::{Deserialize, Serialize};
use tauri::{State, ipc::Channel};
use url::Url;

use crate::persistence::AppSettings;

const PEER_BLOCKLIST: &str = r#"
unspecified-v4:0.0.0.0-0.255.255.255
private-v4-a:10.0.0.0-10.255.255.255
shared-address-space:100.64.0.0-100.127.255.255
loopback-v4:127.0.0.0-127.255.255.255
link-local-v4:169.254.0.0-169.254.255.255
private-v4-b:172.16.0.0-172.31.255.255
ietf-protocol-v4:192.0.0.0-192.0.0.255
documentation-v4-a:192.0.2.0-192.0.2.255
as112-v4:192.31.196.0-192.31.196.255
amt-v4:192.52.193.0-192.52.193.255
deprecated-relay-v4:192.88.99.0-192.88.99.255
private-v4-c:192.168.0.0-192.168.255.255
as112-direct-v4:192.175.48.0-192.175.48.255
benchmark-v4:198.18.0.0-198.19.255.255
documentation-v4-b:198.51.100.0-198.51.100.255
documentation-v4-c:203.0.113.0-203.0.113.255
multicast-v4:224.0.0.0-239.255.255.255
reserved-v4:240.0.0.0-255.255.255.255
unspecified-v6:::-::
loopback-v6:::1-::1
ipv4-mapped-v6:::ffff:0:0-::ffff:ffff:ffff
nat64-well-known-v6:64:ff9b::-64:ff9b::ffff:ffff
nat64-local-v6:64:ff9b:1::-64:ff9b:1:ffff:ffff:ffff:ffff:ffff
discard-v6:100::-100::ffff:ffff:ffff:ffff
ietf-protocol-v6:2001::-2001:1ff:ffff:ffff:ffff:ffff:ffff:ffff
documentation-v6:2001:db8::-2001:db8:ffff:ffff:ffff:ffff:ffff:ffff
six-to-four-v6:2002::-2002:ffff:ffff:ffff:ffff:ffff:ffff:ffff
documentation-v6-2:3fff::-3fff:fff:ffff:ffff:ffff:ffff:ffff:ffff
segment-routing-v6:5f00::-5f00:ffff:ffff:ffff:ffff:ffff:ffff:ffff
unique-local-v6:fc00::-fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff
link-local-v6:fe80::-febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff
multicast-v6:ff00::-ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff
"#;

const MAX_TRACKERS: usize = 32;
const MAX_TRACKER_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_INITIAL_PEERS: usize = 200;

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
    source_url: String,
    name: String,
    source_type: String,
    network_origins: Vec<String>,
    files: Vec<super::torrent_file::TorrentFileEntry>,
    total_bytes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TorrentDownloadRequest {
    task_id: String,
    source: String,
    destination_directory: String,
    settings: Option<AppSettings>,
    privacy_confirmed: bool,
    #[serde(default)]
    selected_files: Option<Vec<usize>>,
}

#[tauri::command]
pub(crate) async fn inspect_torrent_source(
    source: String,
    settings: Option<AppSettings>,
) -> Result<TorrentInspection, String> {
    if !source.to_ascii_lowercase().starts_with("magnet:") {
        let parsed =
            Url::parse(&source).map_err(|_| "Choose a torrent file or paste a magnet link")?;
        let bytes = if parsed.scheme() == "file" {
            super::torrent_file::read_local(&source).await?
        } else if matches!(parsed.scheme(), "http" | "https") {
            let settings = settings.unwrap_or_default();
            settings.validate()?;
            if settings.proxy_mode != "disabled" {
                return Err("Torrent metadata downloads require Direct connection mode".into());
            }
            fetch_torrent_file(parsed).await?
        } else {
            return Err("Unsupported torrent source".into());
        };
        let meta = super::torrent_file::parse(&bytes)?;
        validate_magnet_trackers(&meta.magnet)?;
        let source_url = super::torrent_file::cache(&bytes).await?;
        return Ok(TorrentInspection {
            source_url,
            name: meta.name,
            source_type: "torrentFile".into(),
            network_origins: sanitized_network_origins(&meta.magnet),
            files: meta.files,
            total_bytes: Some(meta.total.to_string()),
        });
    }
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
        source_url: source,
        name,
        source_type: source_type.into(),
        network_origins,
        files: Vec::new(),
        total_bytes: None,
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
    let (source, torrent_bytes) = if request.source.starts_with("file:") {
        let bytes = super::torrent_file::read_local(&request.source).await?;
        let meta = super::torrent_file::parse(&bytes)?;
        validate_file_selection(request.selected_files.as_deref(), meta.files.len())?;
        (meta.magnet, Some(bytes))
    } else {
        if request.selected_files.is_some() {
            return Err("Load the magnet file list before selecting files".into());
        }
        (validate_torrent_source(&request.source)?, None)
    };
    let trackers = validate_magnet_trackers(&source)?;
    let approved_trackers = resolve_tracker_addresses(&trackers, &control).await?;
    let initial_peers = fetch_tracker_peers(&source, &approved_trackers, &control).await?;
    control
        .checkpoint()
        .await
        .map_err(|error| error.to_string())?;
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
    let blocklist_path = job_destination.join(".quiverdl-peer-blocklist");
    tokio::fs::write(&blocklist_path, PEER_BLOCKLIST)
        .await
        .map_err(|error| format!("Could not prepare the torrent network policy: {error}"))?;
    let blocklist_url = Url::from_file_path(&blocklist_path)
        .map_err(|_| "Could not prepare the torrent network policy".to_string())?
        .into();
    let session_options = SessionOptions {
        dht: None,
        listen: None,
        connect: Some(ConnectionOptions::default()),
        disable_trackers: true,
        concurrent_init_limit: Some(1),
        peer_limit: Some(80),
        blocklist_url: Some(blocklist_url),
        disable_upload: true,
        disable_local_service_discovery: true,
        ..SessionOptions::default()
    };
    let session_result = tokio::select! {
        result = Session::new_with_opts(job_destination, session_options) => result,
        _ = control.cancelled() => {
            let _ = tokio::fs::remove_file(&blocklist_path).await;
            return Err("download was cancelled".into());
        }
    };
    let _ = tokio::fs::remove_file(&blocklist_path).await;
    let session = session_result
        .map_err(|error| friendly_torrent_error("Could not initialize BitTorrent", &error))?;
    let options = AddTorrentOptions {
        // Each task owns an isolated folder, so rqbit can safely verify and resume its own files.
        overwrite: true,
        initial_peers: Some(initial_peers),
        only_files: request.selected_files,
        ..AddTorrentOptions::default()
    };
    let input = match torrent_bytes {
        Some(bytes) => AddTorrent::from_bytes(bytes),
        None => AddTorrent::from_url(source.as_str()),
    };
    let added = tokio::select! {
        added = session.add_torrent(input, Some(options)) => added,
        _ = control.cancelled() => {
            session.cancellation_token().cancel();
            return Err("download was cancelled".into());
        }
    };
    let handle = added
        .map_err(|error| friendly_torrent_error("Could not add this torrent", &error))?
        .into_handle()
        .ok_or_else(|| "The torrent metadata could not be opened".to_string())?;
    if let Err(error) = control.checkpoint().await {
        let _ = session
            .delete(librqbit::api::TorrentIdOrHash::Id(handle.id()), false)
            .await;
        session.cancellation_token().cancel();
        return Err(error.to_string());
    }
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
        tokio::select! {
            _ = control.cancelled() => break Err("download was cancelled".into()),
            () = tokio::time::sleep(Duration::from_millis(500)) => {}
        }
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

fn validate_file_selection(selection: Option<&[usize]>, count: usize) -> Result<(), String> {
    if let Some(selection) = selection {
        let unique = selection.iter().copied().collect::<HashSet<_>>();
        if selection.is_empty()
            || selection.len() > count
            || unique.len() != selection.len()
            || selection.iter().any(|index| *index >= count)
        {
            return Err("Select at least one valid torrent file".into());
        }
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn preview_magnet_files(
    source: String,
    settings: Option<AppSettings>,
    privacy_confirmed: bool,
) -> Result<TorrentInspection, String> {
    let settings = settings.unwrap_or_default();
    settings.validate()?;
    validate_network_start(&settings, privacy_confirmed)?;
    let source = validate_torrent_source(&source)?;
    let control = quiver_core::DownloadControl::default();
    let trackers = validate_magnet_trackers(&source)?;
    let approved = resolve_tracker_addresses(&trackers, &control).await?;
    let peers = fetch_tracker_peers(&source, &approved, &control).await?;
    let directory = dirs::config_dir()
        .ok_or("Could not locate the app folder")?
        .join("QuiverDL")
        .join(format!(
            "preview-{}",
            hex::encode(rand::random::<[u8; 16]>())
        ));
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|_| "Could not prepare metadata preview")?;
    let blocklist = directory.join("peer-blocklist");
    tokio::fs::write(&blocklist, PEER_BLOCKLIST)
        .await
        .map_err(|_| "Could not prepare metadata preview")?;
    let session = Session::new_with_opts(
        directory.clone(),
        SessionOptions {
            dht: None,
            listen: None,
            connect: Some(ConnectionOptions::default()),
            disable_trackers: true,
            disable_upload: true,
            disable_local_service_discovery: true,
            peer_limit: Some(40),
            blocklist_url: Some(
                Url::from_file_path(&blocklist)
                    .map_err(|_| "Invalid preview folder")?
                    .into(),
            ),
            ..SessionOptions::default()
        },
    )
    .await
    .map_err(|_| "Could not start magnet metadata preview")?;
    let result = tokio::time::timeout(
        Duration::from_secs(60),
        session.add_torrent(
            AddTorrent::from_url(source.as_str()),
            Some(AddTorrentOptions {
                list_only: true,
                initial_peers: Some(peers),
                ..AddTorrentOptions::default()
            }),
        ),
    )
    .await;
    session.cancellation_token().cancel();
    let _ = tokio::fs::remove_file(&blocklist).await;
    let _ = tokio::fs::remove_dir(&directory).await;
    let result = result
        .map_err(|_| "No peers returned the file list within one minute; try again")?
        .map_err(|_| "Could not retrieve the magnet file list")?;
    let librqbit::AddTorrentResponse::ListOnly(preview) = result else {
        return Err("Could not preview magnet metadata".into());
    };
    let metadata = super::torrent_file::parse(&preview.torrent_bytes)?;
    let source_url = super::torrent_file::cache(&preview.torrent_bytes).await?;
    Ok(TorrentInspection {
        source_url,
        name: metadata.name,
        source_type: "torrentFile".into(),
        files: metadata.files,
        total_bytes: Some(metadata.total.to_string()),
        network_origins: sanitized_network_origins(&source),
    })
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
        validate_magnet_trackers(value)?;
        return Ok(value.to_owned());
    }
    Err("Remote .torrent URLs are not enabled until embedded trackers can be validated before network contact; use a magnet with trackers".into())
}

async fn fetch_torrent_file(mut url: Url) -> Result<Vec<u8>, String> {
    for _ in 0..6 {
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("Unsupported torrent URL".into());
        }
        let host = url.host_str().ok_or("Torrent URL has no host")?;
        let port = url
            .port_or_known_default()
            .ok_or("Torrent URL has no port")?;
        let addresses = tokio::time::timeout(
            Duration::from_secs(10),
            tokio::net::lookup_host((host, port)),
        )
        .await
        .map_err(|_| "Torrent host lookup timed out")?
        .map_err(|_| "Could not resolve torrent host")?
        .collect::<Vec<_>>();
        if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(address.ip())) {
            return Err(
                "Local torrent URLs are blocked; open a downloaded torrent file instead".into(),
            );
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .resolve_to_addrs(host, &addresses)
            .build()
            .map_err(|_| "Could not prepare torrent download")?;
        let mut response = client
            .get(url.clone())
            .send()
            .await
            .map_err(|_| "Could not fetch torrent metadata")?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or("Invalid torrent redirect")?;
            url = url.join(location).map_err(|_| "Invalid torrent redirect")?;
            continue;
        }
        if !response.status().is_success() {
            return Err("The server refused the torrent download".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "Could not read torrent metadata")?
        {
            if bytes.len().saturating_add(chunk.len()) > super::torrent_file::MAX_TORRENT_BYTES {
                return Err("Torrent files must be smaller than 8 MB".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(bytes);
    }
    Err("The torrent download redirected too many times".into())
}

fn validate_magnet_trackers(value: &str) -> Result<Vec<Url>, String> {
    let magnet = Url::parse(value).map_err(|_| "The magnet link is invalid".to_string())?;
    if magnet.scheme() != "magnet" {
        return Err("The magnet link is invalid".into());
    }
    let mut trackers = Vec::new();
    for (count, (_, value)) in magnet
        .query_pairs()
        .filter(|(key, _)| key == "tr")
        .enumerate()
    {
        if count >= MAX_TRACKERS {
            return Err("The magnet link contains too many trackers".into());
        }
        let Ok(tracker) = Url::parse(&value) else {
            continue;
        };
        if !matches!(tracker.scheme(), "http" | "https" | "udp")
            || tracker.host().is_none()
            || tracker.port_or_known_default().is_none_or(|port| port == 0)
            || (tracker.scheme() == "udp"
                && (!tracker.username().is_empty() || tracker.password().is_some()))
            || tracker.host_str().is_some_and(|host| {
                host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost")
            })
        {
            continue;
        }
        trackers.push(tracker);
    }
    if trackers.is_empty() {
        return Err("No supported tracker: use HTTP, HTTPS, or UDP (with a port). DHT is currently disabled".into());
    }
    Ok(trackers)
}

async fn resolve_tracker_addresses(
    trackers: &[Url],
    control: &quiver_core::DownloadControl,
) -> Result<Vec<(Url, Vec<SocketAddr>)>, String> {
    let mut approved = Vec::with_capacity(trackers.len());
    for tracker in trackers {
        let host = tracker
            .host_str()
            .ok_or_else(|| "A magnet tracker URL has no host".to_string())?;
        let port = tracker
            .port_or_known_default()
            .ok_or("A tracker port is required")?;
        let addresses = tokio::select! {
            result = tokio::time::timeout(
                Duration::from_secs(10),
                tokio::net::lookup_host((host, port)),
            ) => match result {
                Ok(Ok(addresses)) => addresses.collect::<Vec<_>>(),
                _ => continue,
            },
            _ = control.cancelled() => return Err("download was cancelled".into()),
        };
        if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(address.ip())) {
            continue;
        }
        approved.push((tracker.clone(), addresses));
    }
    Ok(approved)
}

async fn fetch_tracker_peers(
    source: &str,
    trackers: &[(Url, Vec<SocketAddr>)],
    control: &quiver_core::DownloadControl,
) -> Result<Vec<SocketAddr>, String> {
    let magnet =
        librqbit::Magnet::parse(source).map_err(|_| "The magnet link is invalid".to_string())?;
    let info_hash = magnet.as_id20().ok_or_else(|| {
        "Only BitTorrent v1 or hybrid magnet links are currently supported".to_string()
    })?;
    let mut peer_id = [0_u8; 20];
    peer_id[..8].copy_from_slice(b"-QD0200-");
    rand::rng().fill_bytes(&mut peer_id[8..]);

    let mut peers = HashSet::new();
    for (tracker, addresses) in trackers {
        let result = if tracker.scheme() == "udp" {
            udp::announce(tracker, addresses, &info_hash.0, &peer_id, control).await
        } else {
            fetch_http_tracker(tracker, addresses, &info_hash.0, &peer_id, control).await
        };
        if let Ok(found) = result {
            peers.extend(found.into_iter().take(MAX_INITIAL_PEERS - peers.len()));
        }
        if !peers.is_empty() {
            break;
        }
    }
    if peers.is_empty() {
        return Err("No tracker returned usable peers; trackers may be unavailable or this torrent has no peers".into());
    }
    Ok(peers.into_iter().collect())
}

async fn fetch_http_tracker(
    tracker: &Url,
    addresses: &[SocketAddr],
    info_hash: &[u8; 20],
    peer_id: &[u8; 20],
    control: &quiver_core::DownloadControl,
) -> Result<Vec<SocketAddr>, String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .resolve_to_addrs(
            tracker.host_str().ok_or("Tracker host is missing")?,
            addresses,
        )
        .build()
        .map_err(|_| "Could not create the tracker client")?;
    let announce = tracker_announce_url(tracker, info_hash, peer_id)?;
    let response = tokio::select! {
        result = client.get(announce).send() => result.map_err(|_| "A tracker request failed")?,
        _ = control.cancelled() => return Err("download was cancelled".into()),
    };
    if !response.status().is_success() {
        return Err("A tracker refused the announce".into());
    }
    parse_tracker_peers(&read_bounded_tracker_response(response, control).await?)
}

#[path = "torrent_tracker_udp.rs"]
mod udp;

fn tracker_announce_url(
    tracker: &Url,
    info_hash: &[u8; 20],
    peer_id: &[u8; 20],
) -> Result<Url, String> {
    let mut announce = tracker.clone();
    let existing = announce.query().unwrap_or_default();
    let request = format!(
        "info_hash={}&peer_id={}&port=0&uploaded=0&downloaded=0&left=1&compact=1&no_peer_id=1&numwant={MAX_INITIAL_PEERS}",
        percent_encode(info_hash, NON_ALPHANUMERIC),
        percent_encode(peer_id, NON_ALPHANUMERIC),
    );
    let query = if existing.is_empty() {
        request
    } else {
        format!("{existing}&{request}")
    };
    announce.set_query(Some(&query));
    announce.set_fragment(None);
    Ok(announce)
}

async fn read_bounded_tracker_response(
    mut response: reqwest::Response,
    control: &quiver_core::DownloadControl,
) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_TRACKER_RESPONSE_BYTES as u64)
    {
        return Err("A tracker response exceeded the safety limit".into());
    }
    let mut body = Vec::new();
    loop {
        let chunk = tokio::select! {
            result = response.chunk() => result
                .map_err(|_| "Could not read the tracker response".to_string())?,
            _ = control.cancelled() => return Err("download was cancelled".into()),
        };
        let Some(chunk) = chunk else {
            break;
        };
        if body.len().saturating_add(chunk.len()) > MAX_TRACKER_RESPONSE_BYTES {
            return Err("A tracker response exceeded the safety limit".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn parse_tracker_peers(body: &[u8]) -> Result<Vec<SocketAddr>, String> {
    let value: BencodeValueBorrowed<'_> =
        from_bytes(body).map_err(|_| "The tracker response is invalid".to_string())?;
    let BencodeValue::Dict(fields) = value else {
        return Err("The tracker response is invalid".into());
    };
    if fields.keys().any(|key| key.as_ref() == b"failure reason") {
        return Err("The tracker rejected the announce".into());
    }
    let mut peers = Vec::new();
    for (key, value) in fields {
        let bytes = match (key.as_ref(), value) {
            (b"peers", BencodeValue::Bytes(bytes)) => (bytes.as_ref().to_vec(), 6_usize),
            (b"peers6", BencodeValue::Bytes(bytes)) => (bytes.as_ref().to_vec(), 18_usize),
            (b"peers" | b"peers6", _) => {
                return Err("Only compact tracker peer responses are supported".into());
            }
            _ => continue,
        };
        if bytes.0.len() % bytes.1 != 0 {
            return Err("The tracker returned malformed peer addresses".into());
        }
        for entry in bytes.0.chunks_exact(bytes.1) {
            let address = if bytes.1 == 6 {
                SocketAddr::new(
                    IpAddr::V4(Ipv4Addr::new(entry[0], entry[1], entry[2], entry[3])),
                    u16::from_be_bytes([entry[4], entry[5]]),
                )
            } else {
                let mut octets = [0_u8; 16];
                octets.copy_from_slice(&entry[..16]);
                SocketAddr::new(
                    IpAddr::V6(Ipv6Addr::from(octets)),
                    u16::from_be_bytes([entry[16], entry[17]]),
                )
            };
            if address.port() == 0 || !is_public_ip(address.ip()) {
                return Err("A tracker returned a blocked peer address".into());
            }
            peers.push(address);
            if peers.len() > MAX_INITIAL_PEERS {
                return Err("A tracker returned too many peers".into());
            }
        }
    }
    Ok(peers)
}

fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_ipv4(address),
        IpAddr::V6(address) => is_public_ipv6(address),
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let [first, second, third, _] = address.octets();
    !(address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_multicast()
        || address.is_broadcast()
        || address.is_unspecified()
        || first == 0
        || first >= 240
        || (first == 100 && (64..=127).contains(&second))
        || (first == 192 && second == 0 && third == 0)
        || (first == 192 && second == 0 && third == 2)
        || (first == 192 && second == 31 && third == 196)
        || (first == 192 && second == 52 && third == 193)
        || (first == 192 && second == 175 && third == 48)
        || (first == 198 && (second == 18 || second == 19))
        || (first == 198 && second == 51 && third == 100)
        || (first == 203 && second == 0 && third == 113))
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_public_ipv4(mapped);
    }
    let segments = address.segments();
    !(address.is_loopback()
        || address.is_unspecified()
        || address.is_multicast()
        || (segments[0] == 0x0064 && segments[1] == 0xff9b && segments[2] <= 1)
        || (segments[0] == 0x0100 && segments[1] == 0)
        || (segments[0] == 0x2001 && (segments[1] & 0xfe00) == 0)
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || segments[0] == 0x2002
        || (segments[0] & 0xfff0) == 0x3ff0
        || segments[0] == 0x5f00)
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
            if lower.contains("magnet:")
                || lower.contains("http://")
                || lower.contains("https://")
                || lower.contains("udp://")
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
    #[test]
    fn torrent_file_selection_is_bounded_and_nonempty() {
        assert!(super::validate_file_selection(Some(&[0, 2]), 3).is_ok());
        assert!(super::validate_file_selection(Some(&[]), 3).is_err());
        assert!(super::validate_file_selection(Some(&[1, 1]), 3).is_err());
        assert!(super::validate_file_selection(Some(&[3]), 3).is_err());
    }
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::{
        is_public_ip, parse_tracker_peers, sanitized_network_origins, tracker_announce_url,
        validate_network_start, validate_torrent_source,
    };
    use crate::persistence::AppSettings;

    #[test]
    fn accepts_https_tracker_magnets_and_defers_remote_torrent_files() {
        assert!(
            validate_torrent_source("magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862&tr=https%3A%2F%2Ftracker.example%2Fannounce")
                .is_ok()
        );
        assert!(validate_torrent_source("https://example.test/linux.torrent").is_err());
        assert!(
            validate_torrent_source("magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862&tr=udp%3A%2F%2Ftracker.example%3A80")
                .is_ok()
        );
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

    #[test]
    fn blocks_non_public_tracker_addresses() {
        assert!(!is_public_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(!is_public_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(!is_public_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(!is_public_ip(IpAddr::V6(
            "::ffff:127.0.0.1".parse().unwrap()
        )));
        assert!(!is_public_ip(IpAddr::V6(
            "::ffff:10.0.0.1".parse().unwrap()
        )));
        assert!(is_public_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
        assert!(is_public_ip(IpAddr::V6(
            "2606:4700:4700::1111".parse().unwrap()
        )));
    }

    #[test]
    fn parses_only_compact_public_tracker_peers() {
        let response = b"d5:peers6:\x01\x01\x01\x01\x1a\xe1e";
        assert_eq!(
            parse_tracker_peers(response).unwrap(),
            ["1.1.1.1:6881".parse().unwrap()]
        );

        let mut mapped_private = b"d6:peers618:".to_vec();
        mapped_private.extend_from_slice(&"::ffff:127.0.0.1".parse::<Ipv6Addr>().unwrap().octets());
        mapped_private.extend_from_slice(&6881_u16.to_be_bytes());
        mapped_private.push(b'e');
        assert!(parse_tracker_peers(&mapped_private).is_err());
    }

    #[test]
    fn tracker_announce_keeps_the_approved_origin_and_drops_fragments() {
        let tracker =
            url::Url::parse("https://tracker.example/announce?passkey=private#ignored").unwrap();
        let announce = tracker_announce_url(&tracker, &[1; 20], &[2; 20]).unwrap();
        assert_eq!(announce.origin(), tracker.origin());
        assert!(announce.fragment().is_none());
        assert!(announce.query().unwrap().starts_with("passkey=private&"));
        assert!(announce.query().unwrap().contains("info_hash="));
    }

    #[test]
    fn mixed_tracker_lists_keep_supported_trackers_and_authentication() {
        let mut magnet =
            url::Url::parse("magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862")
                .unwrap();
        for tracker in [
            "wss://unsupported.example/",
            "http://user:synthetic@tracker.example/announce?passkey=synthetic",
            "udp://tracker.example:80/announce",
            "https://localhost/announce",
        ] {
            magnet.query_pairs_mut().append_pair("tr", tracker);
        }
        let trackers = super::validate_magnet_trackers(magnet.as_str()).unwrap();
        assert_eq!(trackers.len(), 2);
        let request = reqwest::Client::new()
            .get(super::tracker_announce_url(&trackers[0], &[1; 20], &[2; 20]).unwrap())
            .build()
            .unwrap();
        assert!(request.headers().contains_key("authorization"));
        assert_eq!(request.url().username(), "");
        assert!(
            request
                .url()
                .query()
                .unwrap()
                .starts_with("passkey=synthetic&")
        );
        assert_eq!(
            sanitized_network_origins(magnet.as_str())[0],
            "http://tracker.example"
        );
    }

    #[tokio::test]
    async fn unavailable_http_tracker_falls_back_without_public_network() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = server.local_addr().unwrap();
        let task = tokio::spawn(async move {
            for status in ["503 Unavailable", "200 OK"] {
                let (mut stream, _) = server.accept().await.unwrap();
                let mut buf = [0; 4096];
                let size = stream.read(&mut buf).await.unwrap();
                assert!(
                    String::from_utf8_lossy(&buf[..size]).contains("passkey=synthetic&info_hash=")
                );
                let body = b"d5:peers6:\x01\x01\x01\x01\x1a\xe1e";
                let headers = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(headers.as_bytes()).await.unwrap();
                stream.write_all(body).await.unwrap();
            }
        });
        let tracker = url::Url::parse(&format!(
            "http://tracker.invalid:{}/announce?passkey=synthetic",
            address.port()
        ))
        .unwrap();
        // Supply a loopback fixture directly after the production DNS policy boundary.
        let trackers = vec![(tracker.clone(), vec![address]), (tracker, vec![address])];
        let peers = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            super::fetch_tracker_peers(
                "magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862",
                &trackers,
                &quiver_core::DownloadControl::new(),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(peers, ["1.1.1.1:6881".parse().unwrap()]);
        task.await.unwrap();
    }
}
