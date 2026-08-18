use quiver_core::DownloadEngine;
use serde::Serialize;
use url::Url;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LinkInspection {
    effective_url: String,
    total_bytes: Option<u64>,
    supports_ranges: bool,
    has_validator: bool,
}

#[tauri::command]
async fn inspect_url(url: String) -> Result<LinkInspection, String> {
    let url = Url::parse(url.trim()).map_err(|error| format!("Invalid URL: {error}"))?;
    let probe = DownloadEngine::new()
        .map_err(|error| error.to_string())?
        .probe(&url)
        .await
        .map_err(|error| error.to_string())?;

    Ok(LinkInspection {
        effective_url: probe.effective_url.to_string(),
        total_bytes: probe.total_bytes,
        supports_ranges: probe.supports_ranges,
        has_validator: probe.etag.is_some() || probe.last_modified.is_some(),
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![inspect_url])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
