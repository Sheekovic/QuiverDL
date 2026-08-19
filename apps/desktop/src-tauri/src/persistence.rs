use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};
use url::Url;

const SCHEMA_VERSION: u32 = 1;
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
            || self.name.len() > 255
            || self.error.as_ref().is_some_and(|value| value.len() > 4_096)
            || !matches!(
                self.status.as_str(),
                "starting"
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
        self.schema_version = SCHEMA_VERSION;
        self.settings.validate()?;
        if self.downloads.len() > MAX_DOWNLOADS {
            return Err("The saved queue is too large".into());
        }
        for download in &self.downloads {
            download.validate()?;
        }
        Ok(())
    }
}

#[tauri::command]
pub(crate) async fn load_app_state(
    store: State<'_, PersistentStore>,
) -> Result<AppSnapshot, String> {
    let _guard = store.gate.lock().await;
    let bytes = match read_bounded_regular_file(&store.path).await {
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
    let mut options = tokio::fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    #[cfg(windows)]
    options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    let file = options.open(path).await?;
    if !file.metadata().await?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "queue path is not a regular file",
        ));
    }
    Ok(file)
}

async fn read_bounded_regular_file(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
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
    let metadata = file.metadata().await?;
    if !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "queue path is not a regular file",
        ));
    }
    if metadata.len() > MAX_STATE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "saved queue exceeds the supported size",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_STATE_BYTES + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "saved queue exceeds the supported size",
        ));
    }
    Ok(Some(bytes))
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
    use super::{AppSettings, AppSnapshot, MAX_STATE_BYTES};

    #[test]
    fn defaults_are_valid_and_private() {
        let mut snapshot = AppSnapshot::default();
        snapshot.validate().expect("default snapshot is valid");
        assert!(snapshot.settings.notifications);
        assert_eq!(snapshot.settings.theme, "system");
        assert_eq!(snapshot.downloads.len(), 0);
    }

    #[test]
    fn rejects_unbounded_transfer_settings() {
        let settings = AppSettings {
            max_segments: 255,
            ..AppSettings::default()
        };
        assert!(settings.validate().is_err());
    }

    #[tokio::test]
    async fn rejects_an_oversized_queue_before_reading_it() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("state.json");
        let file = std::fs::File::create(&path).expect("state file");
        file.set_len(MAX_STATE_BYTES + 1)
            .expect("sparse state file");

        let error = super::read_bounded_regular_file(&path)
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

        assert!(super::open_private_regular_file(&temporary).await.is_err());
        assert_eq!(
            tokio::fs::read(target).await.expect("target should read"),
            b"preserve me"
        );
    }
}
