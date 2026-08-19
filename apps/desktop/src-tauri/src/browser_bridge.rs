use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use url::Url;

const HOST_NAME: &str = "app.quiverdl.native";
const MAX_BRIDGE_CONFIG_BYTES: u64 = 16 * 1024;
const MAX_INBOX_ITEM_BYTES: u64 = 1024 * 1024;
const MAX_INBOX_ENTRIES_SCANNED: usize = 500;
const MAX_INBOX_RESULTS: usize = 100;

#[derive(Debug, Deserialize, Serialize)]
struct BridgeConfig {
    token: String,
    inbox_dir: PathBuf,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowserBridgeInfo {
    host_name: &'static str,
    token: String,
    config_path: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowserInboxItem {
    version: u8,
    id: String,
    url: String,
    suggested_filename: Option<String>,
}

fn bridge_directory() -> Result<PathBuf, String> {
    dirs::config_dir()
        .map(|directory| directory.join("QuiverDL"))
        .ok_or_else(|| "Could not locate the user configuration directory".into())
}

async fn ensure_config() -> Result<(PathBuf, BridgeConfig), String> {
    let directory = bridge_directory()?;
    let path = directory.join("native-bridge.json");
    if let Ok(Some(bytes)) =
        super::persistence::read_bounded_regular_file(&path, MAX_BRIDGE_CONFIG_BYTES).await
        && let Ok(config) = serde_json::from_slice::<BridgeConfig>(&bytes)
        && valid_config(&config, &directory)
    {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                .await
                .map_err(|error| format!("Could not protect the bridge directory: {error}"))?;
            tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .await
                .map_err(|error| format!("Could not protect bridge settings: {error}"))?;
        }
        return Ok((path, config));
    }

    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| format!("Could not create the bridge directory: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .await
            .map_err(|error| format!("Could not protect the bridge directory: {error}"))?;
    }
    let config = BridgeConfig {
        token: hex::encode(rand::random::<[u8; 32]>()),
        inbox_dir: directory.join("inbox"),
    };
    let bytes = serde_json::to_vec_pretty(&config)
        .map_err(|error| format!("Could not encode bridge settings: {error}"))?;
    let temporary = directory.join("native-bridge.json.tmp");
    if tokio::fs::try_exists(&temporary)
        .await
        .map_err(|error| format!("Could not inspect temporary bridge settings: {error}"))?
    {
        tokio::fs::remove_file(&temporary)
            .await
            .map_err(|error| format!("Could not replace temporary bridge settings: {error}"))?;
    }
    let mut options = tokio::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut temporary_file = options
        .open(&temporary)
        .await
        .map_err(|error| format!("Could not create protected bridge settings: {error}"))?;
    temporary_file
        .write_all(&bytes)
        .await
        .map_err(|error| format!("Could not write bridge settings: {error}"))?;
    temporary_file
        .sync_all()
        .await
        .map_err(|error| format!("Could not sync bridge settings: {error}"))?;
    drop(temporary_file);
    let commit_path = path.clone();
    tokio::task::spawn_blocking(move || {
        super::persistence::atomic_replace(&temporary, &commit_path)
    })
    .await
    .map_err(|error| format!("Bridge commit task failed: {error}"))?
    .map_err(|error| format!("Could not commit bridge settings: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .await
            .map_err(|error| format!("Could not protect bridge settings: {error}"))?;
    }
    Ok((path, config))
}

fn valid_config(config: &BridgeConfig, directory: &std::path::Path) -> bool {
    config.token.len() == 64
        && config.token.bytes().all(|byte| byte.is_ascii_hexdigit())
        && config.inbox_dir == directory.join("inbox")
}

#[tauri::command]
pub(crate) async fn get_browser_bridge_info() -> Result<BrowserBridgeInfo, String> {
    let (path, config) = ensure_config().await?;
    Ok(BrowserBridgeInfo {
        host_name: HOST_NAME,
        token: config.token,
        config_path: path.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub(crate) async fn list_browser_requests() -> Result<Vec<BrowserInboxItem>, String> {
    let (_, config) = ensure_config().await?;
    match validate_private_directory(&config.inbox_dir).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("The browser inbox is unsafe: {error}")),
    }
    let mut directory = match tokio::fs::read_dir(&config.inbox_dir).await {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Could not read the browser inbox: {error}")),
    };
    let rejected_directory = config
        .inbox_dir
        .parent()
        .expect("validated inbox path has a parent")
        .join("rejected-inbox");
    let mut requests = Vec::new();
    let mut entries_scanned = 0;
    while entries_scanned < MAX_INBOX_ENTRIES_SCANNED && requests.len() < MAX_INBOX_RESULTS {
        let Some(entry) = directory
            .next_entry()
            .await
            .map_err(|error| format!("Could not inspect the browser inbox: {error}"))?
        else {
            break;
        };
        entries_scanned += 1;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            quarantine_invalid_entry(&path, &rejected_directory)
                .await
                .map_err(|error| format!("Could not quarantine an invalid inbox entry: {error}"))?;
            continue;
        }
        let Ok(Some(bytes)) =
            super::persistence::read_bounded_regular_file(&path, MAX_INBOX_ITEM_BYTES).await
        else {
            quarantine_invalid_entry(&path, &rejected_directory)
                .await
                .map_err(|error| format!("Could not quarantine an invalid inbox entry: {error}"))?;
            continue;
        };
        let Ok(request) = parse_inbox_item(&path, &bytes) else {
            quarantine_invalid_entry(&path, &rejected_directory)
                .await
                .map_err(|error| format!("Could not quarantine an invalid inbox entry: {error}"))?;
            continue;
        };
        requests.push(request);
    }
    requests.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(requests)
}

async fn quarantine_invalid_entry(
    path: &std::path::Path,
    rejected_directory: &std::path::Path,
) -> std::io::Result<()> {
    match tokio::fs::create_dir(rejected_directory).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    validate_private_directory(rejected_directory).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(rejected_directory, std::fs::Permissions::from_mode(0o700))
            .await?;
    }
    let destination = rejected_directory.join(format!(
        "{}.rejected",
        hex::encode(rand::random::<[u8; 16]>())
    ));
    tokio::fs::rename(path, destination).await
}

async fn validate_private_directory(path: &std::path::Path) -> std::io::Result<()> {
    let metadata = tokio::fs::symlink_metadata(path).await?;
    #[cfg(windows)]
    let is_reparse_point = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    };
    #[cfg(not(windows))]
    let is_reparse_point = false;
    let invalid = !metadata.is_dir() || metadata.file_type().is_symlink() || is_reparse_point;
    if invalid {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path is not a real directory",
        ));
    }
    Ok(())
}

fn parse_inbox_item(path: &std::path::Path, bytes: &[u8]) -> Result<BrowserInboxItem, String> {
    let request: BrowserInboxItem =
        serde_json::from_slice(bytes).map_err(|_| "Invalid browser request".to_string())?;
    let valid_url = Url::parse(&request.url)
        .ok()
        .is_some_and(|url| matches!(url.scheme(), "http" | "https"));
    if request.version != 1
        || !valid_url
        || request.id.len() > 128
        || path.file_stem().and_then(|value| value.to_str()) != Some(request.id.as_str())
        || path.extension().and_then(|value| value.to_str()) != Some("json")
        || !request
            .id
            .chars()
            .all(|character| character.is_ascii_hexdigit() || character == '-')
    {
        return Err("Invalid browser request".into());
    }
    Ok(request)
}

#[tauri::command]
pub(crate) async fn acknowledge_browser_request(id: String) -> Result<(), String> {
    let id = super::validate_task_id(&id)?;
    let (_, config) = ensure_config().await?;
    let path = config.inbox_dir.join(format!("{id}.json"));
    if tokio::fs::try_exists(&path)
        .await
        .map_err(|error| format!("Could not inspect the browser request: {error}"))?
    {
        tokio::fs::remove_file(path)
            .await
            .map_err(|error| format!("Could not acknowledge the browser request: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{BridgeConfig, parse_inbox_item, quarantine_invalid_entry, valid_config};

    #[test]
    fn accepts_the_native_host_inbox_contract() {
        let item = parse_inbox_item(
            Path::new("80cf859d-fac7-4ec2-a5e2-63a3242c9776.json"),
            br#"{"version":1,"id":"80cf859d-fac7-4ec2-a5e2-63a3242c9776","url":"https://example.test/file.zip","suggestedFilename":"file.zip"}"#,
        )
        .expect("native host request should be accepted");
        assert_eq!(item.suggested_filename.as_deref(), Some("file.zip"));
        assert!(
            parse_inbox_item(
                Path::new("escape.json"),
                br#"{"version":1,"id":"../escape","url":"file:///etc/passwd"}"#
            )
            .is_err()
        );
        assert!(
            parse_inbox_item(
                Path::new("different.json"),
                br#"{"version":1,"id":"80cf859d-fac7-4ec2-a5e2-63a3242c9776","url":"https://example.test/file.zip"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn bridge_config_requires_the_native_host_inbox_contract() {
        let directory = Path::new("C:/fixture/QuiverDL");
        let valid = BridgeConfig {
            token: "ab".repeat(32),
            inbox_dir: directory.join("inbox"),
        };
        assert!(valid_config(&valid, directory));
        assert!(!valid_config(
            &BridgeConfig {
                inbox_dir: directory.join("elsewhere"),
                ..valid
            },
            directory
        ));
    }

    #[tokio::test]
    async fn quarantining_an_invalid_entry_advances_the_live_inbox() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let inbox = directory.path().join("inbox");
        let rejected = directory.path().join("rejected-inbox");
        tokio::fs::create_dir(&inbox)
            .await
            .expect("inbox directory");
        let invalid = inbox.join("malformed.json");
        tokio::fs::write(&invalid, b"invalid")
            .await
            .expect("invalid entry");

        quarantine_invalid_entry(&invalid, &rejected)
            .await
            .expect("invalid entry should be quarantined");
        assert!(!tokio::fs::try_exists(invalid).await.expect("inspect inbox"));
        assert_eq!(
            std::fs::read_dir(rejected)
                .expect("rejected directory")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn quarantine_failure_is_reported() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let invalid = directory.path().join("malformed.json");
        let rejected = directory.path().join("rejected-inbox");
        tokio::fs::write(&invalid, b"invalid")
            .await
            .expect("invalid entry");
        tokio::fs::write(&rejected, b"not a directory")
            .await
            .expect("blocking entry");

        assert!(quarantine_invalid_entry(&invalid, &rejected).await.is_err());
        assert!(tokio::fs::try_exists(invalid).await.expect("inspect inbox"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn linked_inbox_directories_are_rejected() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temporary directory");
        let target = directory.path().join("target");
        let inbox = directory.path().join("inbox");
        tokio::fs::create_dir(&target)
            .await
            .expect("target directory");
        symlink(&target, &inbox).expect("directory symlink");

        assert!(super::validate_private_directory(&inbox).await.is_err());
    }
}
