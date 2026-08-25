use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager, State, ipc::Channel, path::BaseDirectory};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
};
use url::Url;

use crate::persistence::AppSettings;
use crate::proxy_credentials::load_proxy_password;

const MAX_BRIDGE_STDERR_BYTES: u64 = 64 * 1024;

#[derive(Default)]
pub(crate) struct MediaRegistry {
    active: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaFormat {
    format_id: String,
    label: String,
    height: Option<u32>,
    extension: String,
    audio_only: bool,
    has_audio: bool,
    approx_bytes: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaMetadata {
    title: String,
    extractor: String,
    thumbnail: Option<String>,
    duration_seconds: Option<u64>,
    formats: Vec<MediaFormat>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaProgress {
    status: String,
    downloaded_bytes: String,
    total_bytes: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaSummary {
    destination: String,
    bytes_written: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaDownloadRequest {
    task_id: String,
    url: String,
    destination_directory: String,
    quality: String,
    settings: Option<AppSettings>,
}

#[tauri::command]
pub(crate) async fn detect_media_url(
    app: AppHandle,
    url: String,
    settings: Option<AppSettings>,
) -> Result<bool, String> {
    let settings = settings.unwrap_or_default();
    settings.validate()?;
    let url = validate_media_url(&url)?;
    let events = run_bridge(
        &app,
        &settings,
        json!({ "action": "detect", "url": url.as_str() }),
        None,
        None,
    )
    .await?;
    events
        .into_iter()
        .find(|event| event.get("type").and_then(Value::as_str) == Some("detection"))
        .and_then(|event| event.get("supported").and_then(Value::as_bool))
        .ok_or_else(|| "yt-dlp did not return a URL detection result".to_string())
}

#[tauri::command]
pub(crate) async fn inspect_media_url(
    app: AppHandle,
    url: String,
    settings: Option<AppSettings>,
) -> Result<MediaMetadata, String> {
    let settings = settings.unwrap_or_default();
    settings.validate()?;
    let url = validate_media_url(&url)?;
    let request = json!({ "action": "inspect", "url": url.as_str() });
    let events = run_bridge(&app, &settings, request, None, None).await?;
    events
        .into_iter()
        .find(|event| event.get("type").and_then(Value::as_str) == Some("metadata"))
        .and_then(|event| event.get("metadata").cloned())
        .ok_or_else(|| "yt-dlp did not return media metadata".to_string())
        .and_then(|metadata| {
            serde_json::from_value(metadata)
                .map_err(|_| "yt-dlp returned malformed media metadata".to_string())
        })
}

#[tauri::command]
pub(crate) async fn start_media_download(
    app: AppHandle,
    registry: State<'_, MediaRegistry>,
    request: MediaDownloadRequest,
    on_event: Channel<MediaProgress>,
) -> Result<MediaSummary, String> {
    let task_id = super::validate_task_id(&request.task_id)?;
    let settings = request.settings.unwrap_or_default();
    settings.validate()?;
    let url = validate_media_url(&request.url)?;
    validate_quality(&request.quality)?;
    let destination = prepare_media_destination(&request.destination_directory).await?;
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut active = registry
            .active
            .lock()
            .map_err(|_| "Media download controls are unavailable".to_string())?;
        if active.insert(task_id.clone(), cancelled.clone()).is_some() {
            return Err("A media download with this identifier is already active".into());
        }
    }
    let request = json!({
        "action": "download",
        "url": url.as_str(),
        "destinationDirectory": destination,
        "quality": request.quality,
    });
    let result = run_bridge(&app, &settings, request, Some(cancelled), Some(on_event)).await;
    if let Ok(mut active) = registry.active.lock() {
        active.remove(&task_id);
    }
    let events = result?;
    events
        .into_iter()
        .find(|event| event.get("type").and_then(Value::as_str) == Some("complete"))
        .ok_or_else(|| "yt-dlp completed without a final file summary".to_string())
        .and_then(|summary| {
            serde_json::from_value(summary)
                .map_err(|_| "yt-dlp returned a malformed completion summary".to_string())
        })
}

#[tauri::command]
pub(crate) fn cancel_media_download(
    registry: State<'_, MediaRegistry>,
    task_id: String,
) -> Result<(), String> {
    let task_id = super::validate_task_id(&task_id)?;
    let active = registry
        .active
        .lock()
        .map_err(|_| "Media download controls are unavailable".to_string())?;
    let cancelled = active
        .get(&task_id)
        .ok_or_else(|| "This media download is no longer active".to_string())?;
    cancelled.store(true, Ordering::Release);
    Ok(())
}

async fn run_bridge(
    app: &AppHandle,
    settings: &AppSettings,
    mut request: Value,
    cancelled: Option<Arc<AtomicBool>>,
    on_event: Option<Channel<MediaProgress>>,
) -> Result<Vec<Value>, String> {
    apply_media_proxy(&mut request, settings).await?;
    let bridge = media_bridge_path(app)?;
    let candidates = python_candidates(&settings.media_python_path);
    let mut last_spawn_error = None;
    for (program, arguments) in candidates {
        let mut command = Command::new(&program);
        command
            .args(arguments)
            .arg(&bridge)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                last_spawn_error = Some(error);
                continue;
            }
            Err(error) => return Err(format!("Could not start the media engine: {error}")),
        };
        let request_bytes = serde_json::to_vec(&request)
            .map_err(|_| "Could not encode the media request".to_string())?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Could not open the media engine input".to_string())?;
        stdin
            .write_all(&request_bytes)
            .await
            .map_err(|error| format!("Could not send the media request: {error}"))?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|error| format!("Could not finish the media request: {error}"))?;
        drop(stdin);
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Could not read media engine output".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "Could not read media engine diagnostics".to_string())?;
        let stderr_task = tauri::async_runtime::spawn(async move {
            let mut bytes = Vec::new();
            let _ = stderr
                .take(MAX_BRIDGE_STDERR_BYTES)
                .read_to_end(&mut bytes)
                .await;
            bytes
        });
        let mut lines = BufReader::new(stdout).lines();
        let mut events = Vec::new();
        let mut reported_error = None;
        loop {
            if cancelled
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Acquire))
            {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let _ = stderr_task.await;
                return Err("download was cancelled".into());
            }
            let next =
                tokio::time::timeout(std::time::Duration::from_millis(150), lines.next_line())
                    .await;
            let line = match next {
                Err(_) => continue,
                Ok(Err(error)) => return Err(format!("Could not read media progress: {error}")),
                Ok(Ok(None)) => break,
                Ok(Ok(Some(line))) => line,
            };
            if line.len() > 1024 * 1024 {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let _ = stderr_task.await;
                return Err("The media engine returned an oversized response".into());
            }
            let event: Value = serde_json::from_str(&line)
                .map_err(|_| "The media engine returned invalid progress data".to_string())?;
            match event.get("type").and_then(Value::as_str) {
                Some("progress") => {
                    if let Some(channel) = &on_event {
                        let progress: MediaProgress = serde_json::from_value(event.clone())
                            .map_err(|_| {
                                "The media engine returned malformed progress".to_string()
                            })?;
                        channel
                            .send(progress)
                            .map_err(|_| "The media progress listener closed".to_string())?;
                    }
                }
                Some("error") => {
                    reported_error = event
                        .get("message")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
                Some("metadata" | "complete" | "detection") => events.push(event),
                _ => return Err("The media engine returned an unsupported event".into()),
            }
        }
        let status = child
            .wait()
            .await
            .map_err(|error| format!("Could not finish the media engine: {error}"))?;
        let _ = stderr_task.await;
        if !status.success() {
            return Err(reported_error.unwrap_or_else(|| {
                "yt-dlp could not process this media. It may be unavailable, private, or timed out."
                    .into()
            }));
        }
        return Ok(events);
    }
    Err(format!(
        "Python was not found for yt-dlp{}",
        last_spawn_error
            .map(|error| format!(": {error}"))
            .unwrap_or_default()
    ))
}

async fn apply_media_proxy(request: &mut Value, settings: &AppSettings) -> Result<(), String> {
    let request = request
        .as_object_mut()
        .ok_or_else(|| "Could not prepare the media request".to_string())?;
    match settings.proxy_mode.as_str() {
        "disabled" => {
            request.insert("proxy".into(), Value::String(String::new()));
        }
        "system" => {}
        "custom" => {
            let mut endpoint = Url::parse(settings.proxy_url.trim())
                .map_err(|_| "The custom proxy URL is invalid".to_string())?;
            if !settings.proxy_username.is_empty() {
                let password =
                    load_proxy_password(endpoint.to_string(), settings.proxy_username.clone())
                        .await?
                        .ok_or_else(|| {
                            "Save proxy credentials for the configured username before connecting"
                                .to_string()
                        })?;
                endpoint
                    .set_username(&settings.proxy_username)
                    .map_err(|_| "The proxy username could not be applied".to_string())?;
                endpoint
                    .set_password(Some(&password))
                    .map_err(|_| "The proxy password could not be applied".to_string())?;
            }
            request.insert("proxy".into(), Value::String(endpoint.into()));
            if !settings.proxy_bypass.trim().is_empty() {
                request.insert(
                    "proxyBypass".into(),
                    Value::String(settings.proxy_bypass.trim().to_owned()),
                );
            }
        }
        _ => return Err("Unsupported proxy mode".into()),
    }
    Ok(())
}

fn media_bridge_path(app: &AppHandle) -> Result<PathBuf, String> {
    let development = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("python/quiver_media.py");
    if development.is_file() {
        return Ok(development);
    }
    app.path()
        .resolve("python/quiver_media.py", BaseDirectory::Resource)
        .map_err(|error| format!("Could not locate the bundled media bridge: {error}"))
}

fn python_candidates(configured: &str) -> Vec<(String, Vec<String>)> {
    if !configured.trim().is_empty() {
        return vec![(configured.trim().to_owned(), Vec::new())];
    }
    if cfg!(windows) {
        vec![
            ("py".into(), vec!["-3".into()]),
            ("python".into(), Vec::new()),
            ("python3".into(), Vec::new()),
        ]
    } else {
        vec![
            ("python3".into(), Vec::new()),
            ("python".into(), Vec::new()),
        ]
    }
}

fn validate_media_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value.trim()).map_err(|_| "The media URL is invalid".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || value.chars().count() > 8_192
    {
        return Err("Only credential-free HTTP and HTTPS media URLs are supported".into());
    }
    Ok(url)
}

fn validate_quality(value: &str) -> Result<(), String> {
    if matches!(
        value,
        "best" | "2160" | "1440" | "1080" | "720" | "480" | "360" | "audio-mp3" | "audio-m4a"
    ) || value.strip_prefix("format:").is_some_and(|id| {
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
    }) {
        Ok(())
    } else {
        Err("The selected media quality is unsupported".into())
    }
}

async fn prepare_media_destination(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if value.is_empty()
        || value.chars().count() > 4_096
        || value.chars().any(char::is_control)
        || !path.is_absolute()
    {
        return Err("The media destination must be an absolute local folder".into());
    }
    tokio::fs::create_dir_all(path)
        .await
        .map_err(|error| format!("Could not create the media destination: {error}"))?;
    tokio::fs::canonicalize(path)
        .await
        .map_err(|error| format!("Could not resolve the media destination: {error}"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{apply_media_proxy, validate_media_url, validate_quality};
    use crate::persistence::AppSettings;

    #[test]
    fn validates_media_urls_without_leaking_credentials() {
        assert!(validate_media_url("https://example.test/watch?v=1").is_ok());
        assert!(validate_media_url("https://user:secret@example.test/watch").is_err());
        assert!(validate_media_url("file:///private/video.mp4").is_err());
    }

    #[test]
    fn quality_selection_is_bounded() {
        assert!(validate_quality("best").is_ok());
        assert!(validate_quality("1080").is_ok());
        assert!(validate_quality("audio-mp3").is_ok());
        assert!(validate_quality("format:137").is_ok());
        assert!(validate_quality("format:../../secret").is_err());
    }

    #[tokio::test]
    async fn media_requests_follow_the_active_proxy_mode() {
        let mut direct_request = json!({ "action": "inspect" });
        apply_media_proxy(&mut direct_request, &AppSettings::default())
            .await
            .unwrap();
        assert_eq!(direct_request["proxy"], "");

        let system_settings = AppSettings {
            proxy_mode: "system".into(),
            ..AppSettings::default()
        };
        let mut system_request = json!({ "action": "inspect" });
        apply_media_proxy(&mut system_request, &system_settings)
            .await
            .unwrap();
        assert!(system_request.get("proxy").is_none());

        let custom_settings = AppSettings {
            proxy_mode: "custom".into(),
            proxy_url: "http://proxy.example:8080".into(),
            proxy_bypass: "localhost,.internal.example".into(),
            ..AppSettings::default()
        };
        let mut custom_request = json!({ "action": "download" });
        apply_media_proxy(&mut custom_request, &custom_settings)
            .await
            .unwrap();
        assert_eq!(custom_request["proxy"], "http://proxy.example:8080/");
        assert_eq!(custom_request["proxyBypass"], "localhost,.internal.example");
    }
}
