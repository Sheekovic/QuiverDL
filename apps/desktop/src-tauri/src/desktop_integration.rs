use std::{path::Path, sync::Mutex};

#[derive(Default)]
pub(crate) struct OpenRequests(pub Mutex<Vec<String>>);

pub(crate) fn accept_arguments(state: &OpenRequests, arguments: impl Iterator<Item = String>) {
    if let Ok(mut pending) = state.0.lock() {
        for argument in arguments.take(32) {
            if pending.len() >= 32 {
                break;
            }
            if let Some(source) = external_source(&argument)
                && !pending.contains(&source)
            {
                pending.push(source);
            }
        }
    }
}

fn external_source(argument: &str) -> Option<String> {
    if argument.len() > 16_384 || argument.chars().any(char::is_control) {
        return None;
    }
    if argument.to_ascii_lowercase().starts_with("magnet:") {
        return Some(argument.into());
    }
    let path = Path::new(argument);
    if path.is_absolute()
        && path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("torrent"))
    {
        return url::Url::from_file_path(path).ok().map(Into::into);
    }
    if let Ok(url) = url::Url::parse(argument)
        && url.scheme() == "file"
        && url.path().to_ascii_lowercase().ends_with(".torrent")
    {
        return Some(url.into());
    }
    None
}

#[tauri::command]
pub(crate) fn take_open_requests(
    state: tauri::State<'_, OpenRequests>,
) -> Result<Vec<String>, String> {
    state
        .0
        .lock()
        .map(|mut pending| std::mem::take(&mut *pending))
        .map_err(|_| "Could not read opened links".into())
}

pub(crate) fn register_firefox(directory: &Path, executable: &Path) -> Result<(), String> {
    let parent = executable.parent().ok_or("Could not locate QuiverDL")?;
    let host_name = if cfg!(windows) {
        "quiver-native-host.exe"
    } else {
        "quiver-native-host"
    };
    let source = parent.join(host_name);
    if !source.is_file() {
        return Err("This QuiverDL package is missing the browser companion helper".into());
    }
    let host = directory.join(host_name);
    if source != host {
        let bytes = std::fs::read(&source).map_err(|_| "Could not read the browser helper")?;
        if std::fs::read(&host).ok().as_deref() != Some(bytes.as_slice()) {
            std::fs::copy(&source, &host).map_err(|_| "Could not install the browser helper")?;
        }
    }
    let manifest = serde_json::json!({
        "name": "app.quiverdl.native", "description": "QuiverDL browser companion",
        "path": host, "type": "stdio", "allowed_extensions": ["quiverdl@quiverdl.app"]
    });
    #[cfg(windows)]
    let manifest_path = directory.join("app.quiverdl.native.firefox.json");
    #[cfg(target_os = "linux")]
    let manifest_path = dirs::home_dir()
        .ok_or("Could not find the home folder")?
        .join(".mozilla/native-messaging-hosts/app.quiverdl.native.json");
    #[cfg(target_os = "macos")]
    let manifest_path = dirs::home_dir()
        .ok_or("Could not find the home folder")?
        .join("Library/Application Support/Mozilla/NativeMessagingHosts/app.quiverdl.native.json");
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    return Err("Browser integration is not supported on this platform".into());
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    {
        std::fs::create_dir_all(manifest_path.parent().ok_or("Invalid browser folder")?)
            .map_err(|_| "Could not create the browser folder")?;
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest)
                .map_err(|_| "Could not encode browser settings")?,
        )
        .map_err(|_| "Could not save browser settings")?;
        std::fs::write(
            directory.join("desktop-launch.json"),
            serde_json::to_vec(executable).map_err(|_| "Could not encode app location")?,
        )
        .map_err(|_| "Could not save app location")?;
        #[cfg(windows)]
        registry_value(
            "HKCU\\Software\\Mozilla\\NativeMessagingHosts\\app.quiverdl.native",
            None,
            &manifest_path.to_string_lossy(),
        )?;
        Ok(())
    }
}

#[cfg(windows)]
fn registry_value(key: &str, name: Option<&str>, value: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new("reg.exe");
    command.args(["add", key, "/f", "/t", "REG_SZ"]);
    if let Some(name) = name {
        command.args(["/v", name]);
    } else {
        command.arg("/ve");
    }
    let result = command
        .args(["/d", value])
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|_| "Could not register QuiverDL")?;
    if !result.status.success() {
        return Err("Windows could not register QuiverDL".into());
    }
    Ok(())
}

pub(crate) fn register_file_handlers(executable: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let command = format!("\"{}\" \"%1\"", executable.display());
        let classes = "HKCU\\Software\\Classes";
        registry_value(
            &format!("{classes}\\QuiverDL.Torrent"),
            None,
            "BitTorrent file",
        )?;
        registry_value(
            &format!("{classes}\\QuiverDL.Torrent\\shell\\open\\command"),
            None,
            &command,
        )?;
        registry_value(
            &format!("{classes}\\.torrent\\OpenWithProgids"),
            Some("QuiverDL.Torrent"),
            "",
        )?;
        registry_value(&format!("{classes}\\QuiverDL.Magnet"), None, "Magnet link")?;
        registry_value(
            &format!("{classes}\\QuiverDL.Magnet"),
            Some("URL Protocol"),
            "",
        )?;
        registry_value(
            &format!("{classes}\\QuiverDL.Magnet\\shell\\open\\command"),
            None,
            &command,
        )?;
        let capabilities = "HKCU\\Software\\QuiverDL\\Capabilities";
        registry_value(capabilities, Some("ApplicationName"), "QuiverDL")?;
        registry_value(
            capabilities,
            Some("ApplicationDescription"),
            "Download files, magnet links and torrents with QuiverDL",
        )?;
        registry_value(
            &format!("{capabilities}\\FileAssociations"),
            Some(".torrent"),
            "QuiverDL.Torrent",
        )?;
        registry_value(
            &format!("{capabilities}\\URLAssociations"),
            Some("magnet"),
            "QuiverDL.Magnet",
        )?;
        registry_value(
            "HKCU\\Software\\RegisteredApplications",
            Some("QuiverDL"),
            "Software\\QuiverDL\\Capabilities",
        )?;
        // Register first-install defaults only when there is no existing handler.
        // Windows retains any explicit user choice for another application.
        use std::os::windows::process::CommandExt;
        for (extension, description, protocol) in [
            (".torrent", "QuiverDL.Torrent", false),
            ("magnet", "URL:Magnet", true),
        ] {
            let existing = std::process::Command::new("reg.exe")
                .args(["query", &format!("HKCR\\{extension}"), "/ve"])
                .creation_flags(0x0800_0000)
                .output();
            if !existing.is_ok_and(|result| result.status.success()) {
                registry_value(&format!("{classes}\\{extension}"), None, description)?;
                if protocol {
                    registry_value(&format!("{classes}\\{extension}"), Some("URL Protocol"), "")?;
                    registry_value(
                        &format!("{classes}\\{extension}\\shell\\open\\command"),
                        None,
                        &command,
                    )?;
                }
            }
        }
    }
    #[cfg(not(windows))]
    let _ = executable;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn external_arguments_accept_only_magnets_and_torrent_files() {
        assert!(
            external_source("magnet:?xt=urn:btih:0123456789012345678901234567890123456789")
                .is_some()
        );
        assert!(external_source("file:///tmp/example.torrent").is_some());
        for value in [
            "--help",
            "https://example.test/file.exe",
            "file:///tmp/private.txt",
            "relative.torrent",
            "magnet:\ninvalid",
        ] {
            assert!(external_source(value).is_none());
        }
    }
}
