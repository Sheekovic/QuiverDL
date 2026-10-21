use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FileAction {
    Open,
    Reveal,
}

#[tauri::command]
pub(crate) async fn open_download_destination(
    destination: String,
    action: FileAction,
) -> Result<(), String> {
    // OS handlers may block; keep them off the UI and async runtime threads.
    tauri::async_runtime::spawn_blocking(move || {
        let path = checked_destination(&destination)?;
        perform_action(&path, action)
    })
    .await
    .map_err(|_| "The file action could not be completed".to_string())?
}

fn checked_destination(destination: &str) -> Result<PathBuf, String> {
    let path = Path::new(destination);
    if destination.chars().any(char::is_control) || !path.is_absolute() {
        return Err("The download location must be an absolute file or folder path".into());
    }
    let metadata = std::fs::metadata(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => {
            "This download has been moved or deleted from its saved location".to_string()
        }
        _ => "The download location is not accessible".to_string(),
    })?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err("The download location is not a regular file or folder".into());
    }
    Ok(path.to_owned())
}

#[cfg(windows)]
fn perform_action(path: &Path, action: FileAction) -> Result<(), String> {
    use std::{os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::{
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
        UI::{
            Shell::{SHOpenFolderAndSelectItems, SHParseDisplayName, ShellExecuteW},
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    };

    struct ComGuard(bool);
    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: balances this thread's successful CoInitializeEx call.
                unsafe { CoUninitialize() };
            }
        }
    }

    // SAFETY: null reserved argument; COM initialization is local to this worker thread.
    let initialized = unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    // RPC_E_CHANGED_MODE means this worker already has a usable COM apartment.
    if initialized < 0 && initialized != 0x80010106_u32 as i32 {
        return Err("Windows could not initialize the file action".into());
    }
    let _com = ComGuard(initialized >= 0);
    let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    if matches!(action, FileAction::Reveal) && path.is_file() {
        let mut item = ptr::null_mut();
        // SAFETY: the path is NUL-terminated and item is a valid output pointer.
        let parsed = unsafe {
            SHParseDisplayName(
                wide_path.as_ptr(),
                ptr::null_mut(),
                &mut item,
                0,
                ptr::null_mut(),
            )
        };
        if parsed < 0 {
            return Err("Windows could not locate the downloaded file".into());
        }
        // SAFETY: item is the absolute PIDL returned by SHParseDisplayName. A zero
        // item count selects that file in its parent; the allocation is then freed.
        let result = unsafe {
            let result = SHOpenFolderAndSelectItems(item, 0, ptr::null(), 0);
            CoTaskMemFree(item.cast());
            result
        };
        if result < 0 {
            return Err("Windows could not show the downloaded file in its folder".into());
        }
    } else {
        let verb: Vec<u16> = "open\0".encode_utf16().collect();
        // SAFETY: strings are NUL-terminated. The validated absolute path is passed
        // directly to the OS association handler, never through a command shell.
        let result = unsafe {
            ShellExecuteW(
                ptr::null_mut(),
                verb.as_ptr(),
                wide_path.as_ptr(),
                ptr::null(),
                ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize <= 32 {
            return Err("Windows could not open this file or folder. Check its default app".into());
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn perform_action(path: &Path, action: FileAction) -> Result<(), String> {
    let mut command = std::process::Command::new("/usr/bin/open");
    if matches!(action, FileAction::Reveal) && path.is_file() {
        command.arg("-R");
    }
    run_opener(command.arg(path))
}

#[cfg(target_os = "linux")]
fn perform_action(path: &Path, action: FileAction) -> Result<(), String> {
    // Opening the parent works across desktop file managers without requiring a
    // particular D-Bus service. Torrent destinations are already folders.
    let target = if matches!(action, FileAction::Reveal) && path.is_file() {
        path.parent()
            .ok_or("The download has no containing folder")?
    } else {
        path
    };
    run_opener(std::process::Command::new("xdg-open").arg(target))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn run_opener(command: &mut std::process::Command) -> Result<(), String> {
    let result = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|_| "Could not start the system file opener")?;
    if result.success() {
        Ok(())
    } else {
        Err("The system could not open this file or folder. Check its default app".into())
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn perform_action(_path: &Path, _action: FileAction) -> Result<(), String> {
    Err("Opening downloaded files is not supported on this platform".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_existing_files_and_torrent_folders_without_interpreting_names() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory
            .path()
            .join("report & notes, 100% '\u{0639}\u{0631}\u{0628}\u{064a}'.txt");
        std::fs::write(&file, b"test").unwrap();
        assert_eq!(checked_destination(file.to_str().unwrap()).unwrap(), file);
        assert_eq!(
            checked_destination(directory.path().to_str().unwrap()).unwrap(),
            directory.path()
        );
    }

    #[test]
    fn missing_downloads_return_an_actionable_error_without_disclosing_paths() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("private-name.txt");
        let error = checked_destination(file.to_str().unwrap()).unwrap_err();
        assert!(error.contains("moved or deleted"));
        assert!(!error.contains("private-name"));
    }

    #[test]
    fn rejects_relative_paths_urls_and_control_characters() {
        for input in [
            "",
            "report.txt",
            "../report.txt",
            "https://example.test/file",
            "file:///tmp/file",
            "\0",
        ] {
            assert!(checked_destination(input).is_err());
        }
        let directory = tempfile::tempdir().unwrap();
        let input = format!("{}\n", directory.path().display());
        assert!(checked_destination(&input).is_err());
    }
}
