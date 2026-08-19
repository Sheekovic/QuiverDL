use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use url::Url;

const HOST_NAME: &str = "app.quiverdl.native";

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
    if let Ok(bytes) = tokio::fs::read(&path).await
        && let Ok(config) = serde_json::from_slice::<BridgeConfig>(&bytes)
        && config.token.len() == 64
        && config.token.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .await
                .map_err(|error| format!("Could not protect bridge settings: {error}"))?;
        }
        return Ok((path, config));
    }

    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| format!("Could not create the bridge directory: {error}"))?;
    let config = BridgeConfig {
        token: hex::encode(rand::random::<[u8; 32]>()),
        inbox_dir: directory.join("inbox"),
    };
    let bytes = serde_json::to_vec_pretty(&config)
        .map_err(|error| format!("Could not encode bridge settings: {error}"))?;
    let temporary = directory.join("native-bridge.json.tmp");
    tokio::fs::write(&temporary, bytes)
        .await
        .map_err(|error| format!("Could not write bridge settings: {error}"))?;
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
    let mut directory = match tokio::fs::read_dir(&config.inbox_dir).await {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Could not read the browser inbox: {error}")),
    };
    let mut requests = Vec::new();
    while let Some(entry) = directory
        .next_entry()
        .await
        .map_err(|error| format!("Could not inspect the browser inbox: {error}"))?
    {
        if requests.len() >= 100
            || entry.path().extension().and_then(|value| value.to_str()) != Some("json")
        {
            continue;
        }
        let Ok(bytes) = tokio::fs::read(entry.path()).await else {
            continue;
        };
        let Ok(request) = parse_inbox_item(&bytes) else {
            continue;
        };
        requests.push(request);
    }
    requests.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(requests)
}

fn parse_inbox_item(bytes: &[u8]) -> Result<BrowserInboxItem, String> {
    let request: BrowserInboxItem =
        serde_json::from_slice(bytes).map_err(|_| "Invalid browser request".to_string())?;
    let valid_url = Url::parse(&request.url)
        .ok()
        .is_some_and(|url| matches!(url.scheme(), "http" | "https"));
    if request.version != 1
        || !valid_url
        || request.id.len() > 128
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
    use super::parse_inbox_item;

    #[test]
    fn accepts_the_native_host_inbox_contract() {
        let item = parse_inbox_item(
            br#"{"version":1,"id":"80cf859d-fac7-4ec2-a5e2-63a3242c9776","url":"https://example.test/file.zip","suggestedFilename":"file.zip"}"#,
        )
        .expect("native host request should be accepted");
        assert_eq!(item.suggested_filename.as_deref(), Some("file.zip"));
        assert!(
            parse_inbox_item(br#"{"version":1,"id":"../escape","url":"file:///etc/passwd"}"#)
                .is_err()
        );
    }
}
