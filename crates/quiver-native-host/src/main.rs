use std::{fs, io};

use quiver_native_host::{
    BridgeConfig, HostResponse, default_config_path, process_message, read_message, write_message,
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
    let config: BridgeConfig = serde_json::from_slice(&fs::read(config_path)?)?;
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    loop {
        let Some(message) = read_message(&mut input)? else {
            return Ok(());
        };
        let response = if message.is_empty() {
            HostResponse {
                ok: false,
                request_id: None,
                error: Some("Empty request".into()),
            }
        } else {
            process_message(&config, &message)
        };
        write_message(&mut output, &response)?;
    }
}
