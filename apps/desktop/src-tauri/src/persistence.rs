use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};
use url::Url;

const SCHEMA_VERSION: u32 = 4;
const MAX_DOWNLOADS: usize = 10_000;
const MAX_STATE_BYTES: u64 = 16 * 1024 * 1024;

pub(crate) struct PersistentStore {
    path: PathBuf,
    gate: Mutex<()>,
}

impl PersistentStore {
    pub(crate) fn new(app_data_dir: &Path) -> Self {
        Self {
            path: app_data_dir.join("state.json"),
            gate: Mutex::new(()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct AppSettings {
    pub theme: String,
    pub language: String,
    pub notifications: bool,
    pub retry_attempts: u32,
    pub retry_initial_delay_ms: u64,
    pub retry_max_delay_ms: u64,
    pub max_segments: u8,
    pub max_connections_per_host: u8,
    pub per_download_speed_limit_bps: Option<u64>,
    pub global_speed_limit_bps: Option<u64>,
    pub queue_mode: String,
    pub history_retention_days: Option<u32>,
    pub proxy_mode: String,
    pub proxy_url: String,
    pub proxy_username: String,
    pub proxy_bypass: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            language: "en".into(),
            notifications: true,
            retry_attempts: 3,
            retry_initial_delay_ms: 750,
            retry_max_delay_ms: 15_000,
            max_segments: 4,
            max_connections_per_host: 8,
            per_download_speed_limit_bps: None,
            global_speed_limit_bps: None,
            queue_mode: "parallel".into(),
            history_retention_days: None,
            proxy_mode: "disabled".into(),
            proxy_url: String::new(),
            proxy_username: String::new(),
            proxy_bypass: String::new(),
        }
    }
}

impl AppSettings {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !matches!(self.theme.as_str(), "system" | "light" | "dark") {
            return Err("Unsupported theme setting".into());
        }
        if !matches!(self.language.as_str(), "en" | "ar") {
            return Err("Unsupported language setting".into());
        }
        if !(1..=10).contains(&self.retry_attempts) {
            return Err("Retry attempts must be between 1 and 10".into());
        }
        if !(100..=60_000).contains(&self.retry_initial_delay_ms)
            || !(self.retry_initial_delay_ms..=300_000).contains(&self.retry_max_delay_ms)
        {
            return Err("Retry delays are outside the supported range".into());
        }
        if !(1..=16).contains(&self.max_segments) {
            return Err("Segment count must be between 1 and 16".into());
        }
        if !(1..=32).contains(&self.max_connections_per_host) {
            return Err("Per-server connection count must be between 1 and 32".into());
        }
        if !matches!(self.queue_mode.as_str(), "parallel" | "sequential") {
            return Err("Unsupported queue mode".into());
        }
        if self
            .history_retention_days
            .is_some_and(|days| !matches!(days, 7 | 30 | 90))
        {
            return Err("History retention must be forever, 7, 30, or 90 days".into());
        }
        if !matches!(self.proxy_mode.as_str(), "disabled" | "system" | "custom") {
            return Err("Unsupported proxy mode".into());
        }
        if self.proxy_url.chars().count() > 2_048
            || self.proxy_url.chars().any(char::is_control)
            || self.proxy_bypass.chars().count() > 8 * 1024
            || self.proxy_bypass.chars().any(char::is_control)
            || self.proxy_username.chars().count() > 512
            || self.proxy_username.contains(':')
            || self.proxy_username.chars().any(char::is_control)
        {
            return Err("The proxy settings are too long or contain unsupported characters".into());
        }
        if self.proxy_mode == "custom" && self.proxy_url.trim().is_empty() {
            return Err("The custom proxy URL is required".into());
        }
        if !self.proxy_url.trim().is_empty() {
            let endpoint =
                Url::parse(self.proxy_url.trim()).map_err(|_| "The custom proxy URL is invalid")?;
            let mut config =
                quiver_core::ProxyConfig::new(endpoint).map_err(|error| error.to_string())?;
            if !self.proxy_bypass.trim().is_empty() {
                config = config
                    .with_bypass_list(self.proxy_bypass.clone())
                    .map_err(|error| error.to_string())?;
            }
            drop(config);
        } else if !self.proxy_bypass.trim().is_empty() {
            return Err("A proxy bypass list requires a custom proxy URL".into());
        }
        for limit in [
            self.per_download_speed_limit_bps,
            self.global_speed_limit_bps,
        ]
        .into_iter()
        .flatten()
        {
            if !(1024..=1024_u64.pow(4)).contains(&limit) {
                return Err("Speed limits must be between 1 KiB/s and 1 TiB/s".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredDownload {
    pub id: String,
    pub name: String,
    pub url: String,
    pub destination: String,
    pub status: String,
    pub downloaded_bytes: String,
    pub total_bytes: Option<String>,
    pub sha256: Option<String>,
    pub resumed: Option<bool>,
    pub error: Option<String>,
    #[serde(default)]
    pub queued_at_ms: Option<String>,
    #[serde(default)]
    pub scheduled_for_ms: Option<String>,
    #[serde(default)]
    pub queue_sequence: Option<String>,
    #[serde(default)]
    pub completed_at_ms: Option<String>,
}

impl StoredDownload {
    fn validate(&self) -> Result<(), String> {
        super::validate_task_id(&self.id)?;
        let url = Url::parse(&self.url).map_err(|_| "A saved download has an invalid URL")?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("A saved download uses an unsupported URL scheme".into());
        }
        super::validate_destination(&self.destination)?;
        if self.name.is_empty()
            || self.name.chars().count() > 255
            || self
                .error
                .as_ref()
                .is_some_and(|value| value.chars().count() > 4_096)
            || !matches!(
                self.status.as_str(),
                "starting"
                    | "queued"
                    | "scheduled"
                    | "probing"
                    | "retrying"
                    | "downloading"
                    | "paused"
                    | "verifying"
                    | "cancelling"
                    | "completed"
                    | "cancelled"
                    | "failed"
            )
        {
            return Err("A saved download contains an invalid field".into());
        }
        let queued_at = self
            .queued_at_ms
            .as_deref()
            .map(super::parse_queue_timestamp)
            .transpose()?;
        let scheduled_for = self
            .scheduled_for_ms
            .as_deref()
            .map(super::parse_queue_timestamp)
            .transpose()?;
        let queue_sequence = self
            .queue_sequence
            .as_deref()
            .map(super::parse_queue_sequence)
            .transpose()?;
        let completed_at = self
            .completed_at_ms
            .as_deref()
            .map(super::parse_queue_timestamp)
            .transpose()?;
        if matches!(self.status.as_str(), "queued" | "scheduled") && queued_at.is_none() {
            return Err("A queued download is missing its enqueue time".into());
        }
        if self.status == "scheduled" && scheduled_for.is_none() {
            return Err("A scheduled download is missing its start time".into());
        }
        if matches!(self.status.as_str(), "queued" | "scheduled") && queue_sequence.is_none() {
            return Err("A queued download is missing its FIFO sequence".into());
        }
        if self.status != "completed" && completed_at.is_some() {
            return Err("Only a completed download can have a completion time".into());
        }
        let downloaded = self
            .downloaded_bytes
            .parse::<u64>()
            .map_err(|_| "A saved byte count is invalid")?;
        if let Some(total) = &self.total_bytes {
            let total = total
                .parse::<u64>()
                .map_err(|_| "A saved total byte count is invalid")?;
            if downloaded > total {
                return Err("A saved download exceeds its total byte count".into());
            }
        }
        if self.sha256.as_ref().is_some_and(|digest| {
            digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err("A saved checksum is invalid".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct AppSnapshot {
    pub schema_version: u32,
    pub settings: AppSettings,
    pub downloads: Vec<StoredDownload>,
}

impl Default for AppSnapshot {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            settings: AppSettings::default(),
            downloads: Vec::new(),
        }
    }
}

impl AppSnapshot {
    fn validate(&mut self) -> Result<(), String> {
        if self.schema_version > SCHEMA_VERSION {
            return Err("This queue was created by a newer QuiverDL version".into());
        }
        if self.schema_version < 3 {
            let mut next_sequence = 0_u64;
            for download in self.downloads.iter_mut().rev() {
                if matches!(download.status.as_str(), "queued" | "scheduled")
                    && download.queue_sequence.is_none()
                {
                    download.queue_sequence = Some(next_sequence.to_string());
                    next_sequence = next_sequence.checked_add(1).ok_or_else(|| {
                        "The saved queue has exhausted its FIFO sequence range".to_string()
                    })?;
                }
            }
        }
        self.schema_version = SCHEMA_VERSION;
        self.settings.validate()?;
        if self.downloads.len() > MAX_DOWNLOADS {
            return Err("The saved queue is too large".into());
        }
        let mut sequences = HashSet::new();
        for download in &self.downloads {
            download.validate()?;
            if let Some(sequence) = download.queue_sequence.as_deref() {
                let sequence = super::parse_queue_sequence(sequence)?;
                if !sequences.insert(sequence) {
                    return Err("The saved queue contains duplicate FIFO sequences".into());
                }
            }
        }
        Ok(())
    }
}

#[tauri::command]
pub(crate) async fn load_app_state(
    store: State<'_, PersistentStore>,
) -> Result<AppSnapshot, String> {
    let _guard = store.gate.lock().await;
    let bytes = match read_bounded_regular_file(&store.path, MAX_STATE_BYTES).await {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(AppSnapshot::default()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AppSnapshot::default());
        }
        Err(error) => return Err(format!("Could not read the saved queue: {error}")),
    };
    let mut snapshot: AppSnapshot = serde_json::from_slice(&bytes)
        .map_err(|error| format!("The saved queue is damaged: {error}"))?;
    snapshot.validate()?;
    for download in &mut snapshot.downloads {
        if matches!(
            download.status.as_str(),
            "starting" | "probing" | "retrying" | "downloading" | "verifying" | "cancelling"
        ) {
            download.status = "paused".into();
            download.error =
                Some("Interrupted by the previous app shutdown; ready to resume".into());
        }
    }
    Ok(snapshot)
}

#[tauri::command]
pub(crate) async fn save_app_state(
    store: State<'_, PersistentStore>,
    mut snapshot: AppSnapshot,
) -> Result<(), String> {
    snapshot.validate()?;
    let bytes = serde_json::to_vec_pretty(&snapshot)
        .map_err(|error| format!("Could not encode the queue: {error}"))?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return Err("The saved queue exceeds the supported size".into());
    }
    let _guard = store.gate.lock().await;
    let parent = store
        .path
        .parent()
        .ok_or_else(|| "The application data path is invalid".to_string())?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| format!("Could not create the application data directory: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .await
            .map_err(|error| {
                format!("Could not protect the application data directory: {error}")
            })?;
    }
    let temporary = store.path.with_extension("json.tmp");
    let mut file = open_private_regular_file(&temporary)
        .await
        .map_err(|error| format!("Could not create the queue file: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .await
            .map_err(|error| format!("Could not protect the queue file: {error}"))?;
    }
    file.write_all(&bytes)
        .await
        .map_err(|error| format!("Could not write the queue: {error}"))?;
    file.sync_all()
        .await
        .map_err(|error| format!("Could not flush the queue: {error}"))?;
    drop(file);
    let destination = store.path.clone();
    tokio::task::spawn_blocking(move || atomic_replace(&temporary, &destination))
        .await
        .map_err(|error| format!("Queue commit task failed: {error}"))?
        .map_err(|error| format!("Could not commit the queue file: {error}"))?;
    Ok(())
}

async fn open_private_regular_file(path: &Path) -> std::io::Result<tokio::fs::File> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut options = tokio::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    #[cfg(windows)]
    options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    let file = options.open(path).await?;
    validate_private_handle(&file).await?;
    Ok(file)
}

pub(crate) async fn read_bounded_regular_file(
    path: &Path,
    maximum_bytes: u64,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut options = tokio::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    #[cfg(windows)]
    options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    let file = match options.open(path).await {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let metadata = validate_private_handle(&file).await?;
    if metadata.len() > maximum_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file exceeds the supported size",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(maximum_bytes + 1).read_to_end(&mut bytes).await?;
    if bytes.len() as u64 > maximum_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file exceeds the supported size",
        ));
    }
    Ok(Some(bytes))
}

async fn validate_private_handle(file: &tokio::fs::File) -> std::io::Result<std::fs::Metadata> {
    let metadata = file.metadata().await?;
    if !metadata.is_file() || !has_single_link(file, &metadata)? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "private state path is not an exclusively linked regular file",
        ));
    }
    Ok(metadata)
}

#[cfg(unix)]
fn has_single_link(_file: &tokio::fs::File, metadata: &std::fs::Metadata) -> std::io::Result<bool> {
    use std::os::unix::fs::MetadataExt;

    Ok(metadata.nlink() == 1)
}

#[cfg(windows)]
fn has_single_link(file: &tokio::fs::File, _metadata: &std::fs::Metadata) -> std::io::Result<bool> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    let result = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) };
    if result == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(information.nNumberOfLinks == 1)
}

#[cfg(not(any(unix, windows)))]
fn has_single_link(
    _file: &tokio::fs::File,
    _metadata: &std::fs::Metadata,
) -> std::io::Result<bool> {
    Ok(true)
}

#[cfg(windows)]
pub(crate) fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
pub(crate) fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)?;
    let parent = destination.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "queue destination has no parent directory",
        )
    })?;
    std::fs::File::open(parent)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::{AppSettings, AppSnapshot, MAX_STATE_BYTES, StoredDownload};

    #[test]
    fn defaults_are_valid_and_private() {
        let mut snapshot = AppSnapshot::default();
        snapshot.validate().expect("default snapshot is valid");
        assert!(snapshot.settings.notifications);
        assert_eq!(snapshot.settings.theme, "system");
        assert_eq!(snapshot.settings.proxy_mode, "disabled");
        assert_eq!(snapshot.settings.queue_mode, "parallel");
        assert_eq!(snapshot.settings.history_retention_days, None);
        assert_eq!(snapshot.downloads.len(), 0);
    }

    #[test]
    fn older_snapshots_default_to_direct_proxy_routing() {
        let mut snapshot: AppSnapshot =
            serde_json::from_str(r#"{"schemaVersion":1,"settings":{},"downloads":[]}"#)
                .expect("legacy snapshot should deserialize");
        snapshot
            .validate()
            .expect("legacy snapshot should remain valid");
        assert_eq!(snapshot.settings.proxy_mode, "disabled");
        let saved = serde_json::to_string(&snapshot).expect("snapshot should serialize");
        assert!(!saved.to_ascii_lowercase().contains("password"));
    }

    #[test]
    fn rejects_unbounded_transfer_settings() {
        let settings = AppSettings {
            max_segments: 255,
            ..AppSettings::default()
        };
        assert!(settings.validate().is_err());
    }

    #[test]
    fn rejects_proxy_credentials_before_settings_can_be_saved() {
        let settings = AppSettings {
            proxy_url: "http://user:password@proxy.example:8080".into(),
            ..AppSettings::default()
        };
        let error = settings
            .validate()
            .expect_err("embedded credentials must never reach state.json");
        assert!(error.contains("must not be embedded"));
        assert!(!error.contains("password"));
    }

    #[test]
    fn accepts_multibyte_names_within_the_character_limit() {
        let destination = if cfg!(windows) {
            "C:/Downloads/file.bin"
        } else {
            "/tmp/file.bin"
        };
        let download = StoredDownload {
            id: "80cf859d-fac7-4ec2-a5e2-63a3242c9776".into(),
            name: "測".repeat(100),
            url: "https://example.test/file.bin".into(),
            destination: destination.into(),
            status: "paused".into(),
            downloaded_bytes: "0".into(),
            total_bytes: None,
            sha256: None,
            resumed: None,
            error: None,
            queued_at_ms: None,
            scheduled_for_ms: None,
            queue_sequence: None,
            completed_at_ms: None,
        };

        download.validate().expect("multibyte name should be valid");
    }

    #[test]
    fn version_two_pending_items_receive_stable_fifo_sequences() {
        let destination = if cfg!(windows) {
            "C:/Downloads/file.bin"
        } else {
            "/tmp/file.bin"
        };
        let mut snapshot = AppSnapshot {
            schema_version: 2,
            settings: AppSettings::default(),
            downloads: vec![StoredDownload {
                id: "legacy-queued-download".into(),
                name: "file.bin".into(),
                url: "https://example.test/file.bin".into(),
                destination: destination.into(),
                status: "queued".into(),
                downloaded_bytes: "0".into(),
                total_bytes: None,
                sha256: None,
                resumed: None,
                error: None,
                queued_at_ms: Some("1770000000000".into()),
                scheduled_for_ms: None,
                queue_sequence: None,
                completed_at_ms: None,
            }],
        };

        snapshot
            .validate()
            .expect("version two queue should migrate");
        assert_eq!(snapshot.schema_version, 4);
        assert_eq!(snapshot.downloads[0].queue_sequence.as_deref(), Some("0"));
    }

    #[test]
    fn version_three_completed_items_migrate_without_inventing_a_completion_time() {
        let destination = if cfg!(windows) {
            "C:/Downloads/file.bin"
        } else {
            "/tmp/file.bin"
        };
        let mut snapshot: AppSnapshot = serde_json::from_value(serde_json::json!({
            "schemaVersion": 3,
            "settings": {},
            "downloads": [{
                "id": "80cf859d-fac7-4ec2-a5e2-63a3242c9776",
                "name": "file.bin",
                "url": "https://example.test/file.bin",
                "destination": destination,
                "status": "completed",
                "downloadedBytes": "1024",
                "totalBytes": "1024",
                "sha256": "a".repeat(64),
                "queuedAtMs": "1770000000000",
                "scheduledForMs": null,
                "queueSequence": "0"
            }]
        }))
        .expect("version three history should deserialize");

        snapshot
            .validate()
            .expect("version three history should migrate");
        assert_eq!(snapshot.schema_version, 4);
        assert_eq!(snapshot.downloads[0].completed_at_ms, None);
    }

    #[test]
    fn validates_history_retention_and_completion_times() {
        for days in [None, Some(7), Some(30), Some(90)] {
            let settings = AppSettings {
                history_retention_days: days,
                ..AppSettings::default()
            };
            settings.validate().expect("supported retention is valid");
        }
        let invalid_settings = AppSettings {
            history_retention_days: Some(1),
            ..AppSettings::default()
        };
        assert!(invalid_settings.validate().is_err());

        let destination = if cfg!(windows) {
            "C:/Downloads/file.bin"
        } else {
            "/tmp/file.bin"
        };
        let completed = StoredDownload {
            id: "80cf859d-fac7-4ec2-a5e2-63a3242c9776".into(),
            name: "file.bin".into(),
            url: "https://example.test/file.bin".into(),
            destination: destination.into(),
            status: "completed".into(),
            downloaded_bytes: "1024".into(),
            total_bytes: Some("1024".into()),
            sha256: Some("a".repeat(64)),
            resumed: Some(false),
            error: None,
            queued_at_ms: Some("1770000000000".into()),
            scheduled_for_ms: None,
            queue_sequence: Some("0".into()),
            completed_at_ms: Some("1770003600000".into()),
        };
        completed
            .validate()
            .expect("completion time should be valid");

        let mut invalid_time = completed.clone();
        invalid_time.completed_at_ms = Some("not-a-time".into());
        assert!(invalid_time.validate().is_err());

        let mut unfinished = completed;
        unfinished.status = "failed".into();
        assert!(unfinished.validate().is_err());
    }

    #[test]
    fn validates_durable_queue_metadata() {
        let destination = if cfg!(windows) {
            "C:/Downloads/file.bin"
        } else {
            "/tmp/file.bin"
        };
        let scheduled = StoredDownload {
            id: "scheduled-download".into(),
            name: "file.bin".into(),
            url: "https://example.test/file.bin".into(),
            destination: destination.into(),
            status: "scheduled".into(),
            downloaded_bytes: "0".into(),
            total_bytes: None,
            sha256: None,
            resumed: None,
            error: None,
            queued_at_ms: Some("1770000000000".into()),
            scheduled_for_ms: Some("1770003600000".into()),
            queue_sequence: Some("42".into()),
            completed_at_ms: None,
        };

        scheduled
            .validate()
            .expect("complete schedule metadata should be valid");
        let mut missing_time = scheduled.clone();
        missing_time.scheduled_for_ms = None;
        assert!(missing_time.validate().is_err());
        let mut missing_sequence = scheduled.clone();
        missing_sequence.queue_sequence = None;
        assert!(missing_sequence.validate().is_err());
        let mut duplicate = scheduled.clone();
        duplicate.id = "duplicate-sequence".into();
        let mut snapshot = AppSnapshot {
            schema_version: 4,
            settings: AppSettings::default(),
            downloads: vec![scheduled.clone(), duplicate],
        };
        assert!(snapshot.validate().is_err());
        let mut invalid_time = scheduled;
        invalid_time.queued_at_ms = Some("not-a-time".into());
        assert!(invalid_time.validate().is_err());
    }

    #[tokio::test]
    async fn rejects_an_oversized_queue_before_reading_it() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("state.json");
        let file = std::fs::File::create(&path).expect("state file");
        file.set_len(MAX_STATE_BYTES + 1)
            .expect("sparse state file");

        let error = super::read_bounded_regular_file(&path, MAX_STATE_BYTES)
            .await
            .expect_err("oversized queues must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn queue_temp_write_does_not_follow_a_symlink() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temporary directory");
        let target = directory.path().join("unrelated");
        let temporary = directory.path().join("state.json.tmp");
        tokio::fs::write(&target, b"preserve me")
            .await
            .expect("target should write");
        symlink(&target, &temporary).expect("symlink should be created");

        super::open_private_regular_file(&temporary)
            .await
            .expect("symlink entry should be replaced safely");
        assert_eq!(
            tokio::fs::read(target).await.expect("target should read"),
            b"preserve me"
        );
    }

    #[tokio::test]
    async fn queue_temp_write_does_not_truncate_a_hard_link_target() {
        use tokio::io::AsyncWriteExt;

        let directory = tempfile::tempdir().expect("temporary directory");
        let target = directory.path().join("unrelated");
        let temporary = directory.path().join("state.json.tmp");
        tokio::fs::write(&target, b"preserve me")
            .await
            .expect("target should write");
        std::fs::hard_link(&target, &temporary).expect("hard link should be created");

        let mut file = super::open_private_regular_file(&temporary)
            .await
            .expect("hard-link entry should be replaced safely");
        file.write_all(b"new queue")
            .await
            .expect("temporary queue should write");
        file.sync_all().await.expect("temporary queue should sync");
        drop(file);

        assert_eq!(
            tokio::fs::read(target).await.expect("target should read"),
            b"preserve me"
        );
        assert_eq!(
            tokio::fs::read(temporary)
                .await
                .expect("temporary queue should read"),
            b"new queue"
        );
    }
}
