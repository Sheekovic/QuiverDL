use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use url::Url;

const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(800);
const MAX_CLIPBOARD_URL_CHARS: usize = 8_192;

#[derive(Default)]
pub(crate) struct ClipboardMonitor {
    enabled: AtomicBool,
    last_text: Mutex<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClipboardCandidate {
    url: String,
    kind: &'static str,
}

#[tauri::command]
pub(crate) fn set_clipboard_monitor_enabled(
    monitor: State<'_, ClipboardMonitor>,
    enabled: bool,
) -> Result<(), String> {
    monitor.enabled.store(enabled, Ordering::Release);
    if !enabled {
        monitor
            .last_text
            .lock()
            .map_err(|_| "The clipboard monitor is unavailable".to_string())?
            .clear();
    }
    Ok(())
}

pub(crate) fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(POLL_INTERVAL).await;
            let monitor = app.state::<ClipboardMonitor>();
            if !monitor.enabled.load(Ordering::Acquire) {
                continue;
            }
            let Ok(text) = app.clipboard().read_text() else {
                continue;
            };
            let text = text.trim();
            if text.is_empty() || text.chars().count() > MAX_CLIPBOARD_URL_CHARS {
                continue;
            }
            let changed = monitor
                .last_text
                .lock()
                .map(|mut previous| {
                    if previous.as_str() == text {
                        false
                    } else {
                        *previous = text.to_owned();
                        true
                    }
                })
                .unwrap_or(false);
            if !changed {
                continue;
            }
            if let Some(candidate) = downloadable_candidate(text) {
                let _ = app.emit("clipboard-download-candidate", candidate);
            }
        }
    });
}

fn downloadable_candidate(value: &str) -> Option<ClipboardCandidate> {
    if value.bytes().any(|byte| byte.is_ascii_control()) {
        return None;
    }
    let url = Url::parse(value).ok()?;
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    if url.scheme() == "magnet" {
        return Some(ClipboardCandidate {
            url: value.to_owned(),
            kind: "torrent",
        });
    }
    if !matches!(url.scheme(), "http" | "https" | "ftp") || url.host().is_none() {
        return None;
    }

    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let path = url.path().to_ascii_lowercase();
    let media_host = [
        "youtube.com",
        "youtu.be",
        "vimeo.com",
        "tiktok.com",
        "twitch.tv",
        "soundcloud.com",
        "instagram.com",
        "facebook.com",
        "x.com",
        "twitter.com",
        "reddit.com",
    ]
    .iter()
    .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")));
    let extension_match = [
        ".zip",
        ".rar",
        ".7z",
        ".tar",
        ".gz",
        ".exe",
        ".msi",
        ".msix",
        ".iso",
        ".mp4",
        ".mkv",
        ".avi",
        ".mp3",
        ".flac",
        ".pdf",
        ".torrent",
        ".dmg",
        ".deb",
        ".rpm",
        ".appimage",
    ]
    .iter()
    .any(|extension| path.ends_with(extension));
    if !media_host && !extension_match {
        return None;
    }
    Some(ClipboardCandidate {
        url: value.to_owned(),
        kind: if path.ends_with(".torrent") {
            "torrent"
        } else if media_host {
            "media"
        } else {
            "direct"
        },
    })
}

#[cfg(test)]
mod tests {
    use super::downloadable_candidate;

    #[test]
    fn recognizes_direct_media_and_torrent_candidates_without_credentials() {
        assert_eq!(
            downloadable_candidate("https://example.test/file.zip").map(|item| item.kind),
            Some("direct")
        );
        assert_eq!(
            downloadable_candidate("https://www.youtube.com/watch?v=abc").map(|item| item.kind),
            Some("media")
        );
        assert_eq!(
            downloadable_candidate("magnet:?xt=urn:btih:0123456789abcdef").map(|item| item.kind),
            Some("torrent")
        );
        assert!(downloadable_candidate("https://user:secret@example.test/file.zip").is_none());
        assert!(downloadable_candidate("copied text").is_none());
    }
}
