use std::io;

use quiver_native_host::{
    HostResponse, default_config_path, is_firefox_companion, load_config, process_firefox_message,
    process_message, read_message, write_message,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("QuiverDL native host stopped: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::var_os("QUIVERDL_NATIVE_CONFIG")
        .map(Into::into)
        .or_else(default_config_path)
        .ok_or("could not locate the user configuration directory")?;
    let config = load_config(&config_path)?;
    config.validate(&config_path)?;
    let firefox = is_firefox_companion(&std::env::args().skip(1).collect::<Vec<_>>());
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    loop {
        let Some(message) = read_message(&mut input)? else {
            return Ok(());
        };
        let mut response = if message.is_empty() {
            HostResponse {
                ok: false,
                request_id: None,
                error: Some("Empty request".into()),
            }
        } else if firefox {
            process_firefox_message(&config, &message)
        } else {
            process_message(&config, &message)
        };
        if response.ok
            && response.request_id.is_some()
            && firefox
            && launch_desktop(&config_path).is_err()
        {
            // Keep Firefox's download when the desktop cannot be started.
            if let Some(id) = response.request_id.take() {
                let _ = std::fs::remove_file(config.inbox_dir.join(format!("{id}.json")));
            }
            response.ok = false;
            response.error = Some("Open QuiverDL once to finish browser integration".into());
        }
        write_message(&mut output, &response)?;
    }
}

fn launch_desktop(config_path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let path = config_path
        .parent()
        .ok_or("invalid configuration path")?
        .join("desktop-launch.json");
    let metadata = std::fs::metadata(&path)?;
    if !metadata.is_file() || metadata.len() > 16 * 1024 {
        return Err("invalid desktop registration".into());
    }
    let executable: std::path::PathBuf = serde_json::from_slice(&std::fs::read(path)?)?;
    if !executable.is_absolute() || !executable.is_file() {
        return Err("desktop executable missing".into());
    }
    let mut command = std::process::Command::new(executable);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Escape Firefox's native-host job so closing the port cannot kill downloads.
        command.creation_flags(0x0100_0000 | 0x0000_0008);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn()?;
    Ok(())
}
