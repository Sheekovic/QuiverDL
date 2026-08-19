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

#[derive(Debug, Deserialize)]
pub struct BridgeConfig {
    pub token: String,
    pub inbox_dir: PathBuf,
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
    fs::create_dir_all(inbox)?;
    let destination = inbox.join(format!("{}.json", request.id));
    let temporary = inbox.join(format!("{}.tmp", request.id));
    let bytes = serde_json::to_vec(request)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, destination)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{BridgeConfig, HostResponse, process_message, read_message, write_message};

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
}
