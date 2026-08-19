use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use url::Url;
use uuid::Uuid;

const MAX_MESSAGE_BYTES: u32 = 1024 * 1024;
const MAX_CONFIG_BYTES: u64 = 16 * 1024;

#[derive(Debug, Deserialize)]
pub struct BridgeConfig {
    pub token: String,
    pub inbox_dir: PathBuf,
}

impl BridgeConfig {
    pub fn validate(&self, config_path: &Path) -> io::Result<()> {
        let expected_inbox = config_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.join("inbox"));
        if !config_path.is_absolute()
            || self.token.len() != 64
            || !self.token.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !self.inbox_dir.is_absolute()
            || expected_inbox.as_ref() != Some(&self.inbox_dir)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "native bridge configuration failed validation",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserMessage {
    pub version: u8,
    pub action: String,
    pub token: String,
    pub url: String,
    pub suggested_filename: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostResponse {
    pub ok: bool,
    pub request_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InboxRequest {
    version: u8,
    id: String,
    url: String,
    suggested_filename: Option<String>,
}

#[must_use]
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|directory| directory.join("QuiverDL").join("native-bridge.json"))
}

pub fn load_config(path: &Path) -> io::Result<BridgeConfig> {
    let file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_CONFIG_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "native bridge configuration is not a supported regular file",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "native bridge configuration is too large",
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub fn read_message(reader: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut length = [0_u8; 4];
    match reader.read_exact(&mut length) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = u32::from_ne_bytes(length);
    if length == 0 || length > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "native message size is outside the supported range",
        ));
    }
    let mut message = vec![0_u8; length as usize];
    reader.read_exact(&mut message)?;
    Ok(Some(message))
}

pub fn write_message(writer: &mut impl Write, response: &HostResponse) -> io::Result<()> {
    let message = serde_json::to_vec(response)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let length = u32::try_from(message.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "response is too large"))?;
    writer.write_all(&length.to_ne_bytes())?;
    writer.write_all(&message)?;
    writer.flush()
}

pub fn process_message(config: &BridgeConfig, bytes: &[u8]) -> HostResponse {
    match process_message_inner(config, bytes) {
        Ok(request_id) => HostResponse {
            ok: true,
            request_id: Some(request_id),
            error: None,
        },
        Err(error) => HostResponse {
            ok: false,
            request_id: None,
            error: Some(error),
        },
    }
}

fn process_message_inner(config: &BridgeConfig, bytes: &[u8]) -> Result<String, String> {
    let message: BrowserMessage =
        serde_json::from_slice(bytes).map_err(|_| "Invalid request format".to_string())?;
    if message.version != 1 || message.action != "enqueue" {
        return Err("Unsupported bridge request".into());
    }
    if message.token.len() != config.token.len()
        || message
            .token
            .as_bytes()
            .ct_eq(config.token.as_bytes())
            .unwrap_u8()
            != 1
    {
        return Err("Authentication failed".into());
    }
    let url = Url::parse(message.url.trim()).map_err(|_| "Invalid download URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Only HTTP and HTTPS downloads are accepted".into());
    }
    let id = Uuid::new_v4().to_string();
    let request = InboxRequest {
        version: 1,
        id: id.clone(),
        url: url.to_string(),
        suggested_filename: message
            .suggested_filename
            .as_deref()
            .and_then(sanitize_filename),
    };
    write_inbox_request(&config.inbox_dir, &request).map_err(|_| "Could not queue download")?;
    Ok(id)
}

fn sanitize_filename(value: &str) -> Option<String> {
    let mut value: String = value
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
    while value.ends_with('.') || value.ends_with(' ') {
        value.pop();
    }
    (!value.is_empty() && !matches!(value.as_str(), "." | "..")).then_some(value)
}

fn write_inbox_request(inbox: &Path, request: &InboxRequest) -> io::Result<()> {
    match fs::create_dir(inbox) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    validate_private_directory(inbox)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(inbox, fs::Permissions::from_mode(0o700))?;
    }
    let destination = inbox.join(format!("{}.json", request.id));
    let temporary = inbox.join(format!("{}.tmp", request.id));
    let bytes = serde_json::to_vec(request)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, destination)?;
    #[cfg(unix)]
    fs::File::open(inbox)?.sync_all()?;
    Ok(())
}

fn validate_private_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    let is_reparse_point = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    };
    #[cfg(not(windows))]
    let is_reparse_point = false;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || is_reparse_point {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "native inbox is not a real directory",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, path::Path};

    use super::{
        BridgeConfig, HostResponse, MAX_CONFIG_BYTES, load_config, process_message, read_message,
        write_message,
    };

    #[test]
    fn validates_bridge_config_against_its_location() {
        let root = if cfg!(windows) {
            Path::new("C:/Users/test/AppData/Roaming/QuiverDL")
        } else {
            Path::new("/home/test/.config/QuiverDL")
        };
        let config_path = root.join("native-bridge.json");
        let valid = BridgeConfig {
            token: "a1".repeat(32),
            inbox_dir: root.join("inbox"),
        };
        assert!(valid.validate(&config_path).is_ok());
        assert!(
            BridgeConfig {
                token: String::new(),
                inbox_dir: valid.inbox_dir.clone(),
            }
            .validate(&config_path)
            .is_err()
        );
        assert!(
            BridgeConfig {
                token: valid.token,
                inbox_dir: root.join("elsewhere"),
            }
            .validate(&config_path)
            .is_err()
        );
    }

    #[test]
    fn native_framing_round_trips() {
        let response = HostResponse {
            ok: true,
            request_id: Some("fixture".into()),
            error: None,
        };
        let mut framed = Vec::new();
        write_message(&mut framed, &response).expect("response should frame");
        let payload = read_message(&mut Cursor::new(framed))
            .expect("frame should read")
            .expect("frame exists");
        let value: serde_json::Value = serde_json::from_slice(&payload).expect("valid JSON");
        assert_eq!(value["requestId"], "fixture");
    }

    #[test]
    fn rejects_an_oversized_bridge_config_before_reading_it() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("native-bridge.json");
        let file = std::fs::File::create(&path).expect("config file");
        file.set_len(MAX_CONFIG_BYTES + 1)
            .expect("sparse config file");

        let error = load_config(&path).expect_err("oversized config must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn authenticates_and_queues_without_forwarding_secrets() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let config = BridgeConfig {
            token: "correct-token".into(),
            inbox_dir: directory.path().to_path_buf(),
        };
        let rejected = process_message(
            &config,
            br#"{"version":1,"action":"enqueue","token":"wrong-token","url":"https://example.test/file"}"#,
        );
        assert!(!rejected.ok);

        let accepted = process_message(
            &config,
            br#"{"version":1,"action":"enqueue","token":"correct-token","url":"https://example.test/file","suggestedFilename":"../safe.zip"}"#,
        );
        assert!(accepted.ok);
        let entries: Vec<_> = std::fs::read_dir(directory.path())
            .expect("inbox should read")
            .collect();
        assert_eq!(entries.len(), 1);
        let queued = std::fs::read_to_string(entries[0].as_ref().expect("entry").path())
            .expect("request should read");
        assert!(!queued.contains("correct-token"));
        assert!(queued.contains(".._safe.zip"));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_write_through_a_linked_inbox_directory() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temporary directory");
        let target = directory.path().join("target");
        let inbox = directory.path().join("inbox");
        std::fs::create_dir(&target).expect("target directory");
        symlink(&target, &inbox).expect("inbox symlink");
        let config = BridgeConfig {
            token: "correct-token".into(),
            inbox_dir: inbox,
        };

        let response = process_message(
            &config,
            br#"{"version":1,"action":"enqueue","token":"correct-token","url":"https://example.test/file"}"#,
        );
        assert!(!response.ok);
        assert_eq!(
            std::fs::read_dir(target)
                .expect("target directory should read")
                .count(),
            0
        );
    }
}
