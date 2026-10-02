use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path};
use url::Url;

pub(crate) const MAX_TORRENT_BYTES: usize = 8 * 1024 * 1024;

#[tauri::command]
pub(crate) fn torrent_file_url(path: String) -> Result<String, String> {
    let path = Path::new(&path);
    if !path.is_absolute()
        || !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("torrent"))
    {
        return Err("Choose a .torrent file".into());
    }
    Url::from_file_path(path)
        .map(Into::into)
        .map_err(|_| "Invalid torrent file path".into())
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct TorrentFileEntry {
    pub path: String,
    pub bytes: String,
}
pub(crate) struct Metainfo {
    pub name: String,
    pub magnet: String,
    pub files: Vec<TorrentFileEntry>,
    pub total: u64,
}

// Check canonical structure and bounded nesting before invoking the typed parser.
fn validate_bencode(bytes: &[u8]) -> Result<(), String> {
    fn string<'a>(bytes: &'a [u8], cursor: &mut usize) -> Result<&'a [u8], ()> {
        let start = *cursor;
        while bytes.get(*cursor).is_some_and(u8::is_ascii_digit) {
            *cursor += 1;
        }
        let digits = bytes.get(start..*cursor).ok_or(())?;
        if digits.is_empty()
            || (digits.len() > 1 && digits[0] == b'0')
            || bytes.get(*cursor) != Some(&b':')
        {
            return Err(());
        }
        let size: usize = std::str::from_utf8(digits)
            .map_err(|_| ())?
            .parse()
            .map_err(|_| ())?;
        *cursor += 1;
        let end = cursor.checked_add(size).ok_or(())?;
        let value = bytes.get(*cursor..end).ok_or(())?;
        *cursor = end;
        Ok(value)
    }
    fn value(bytes: &[u8], cursor: &mut usize, depth: usize, count: &mut usize) -> Result<(), ()> {
        *count += 1;
        if depth > 40 || *count > 100_000 {
            return Err(());
        }
        match bytes.get(*cursor).copied().ok_or(())? {
            b'0'..=b'9' => {
                string(bytes, cursor)?;
            }
            b'i' => {
                *cursor += 1;
                let start = *cursor;
                while bytes.get(*cursor).is_some_and(|byte| *byte != b'e') {
                    *cursor += 1;
                    if *cursor - start > 20 {
                        return Err(());
                    }
                }
                let encoded =
                    std::str::from_utf8(bytes.get(start..*cursor).ok_or(())?).map_err(|_| ())?;
                let number: i128 = encoded.parse().map_err(|_| ())?;
                if number.to_string() != encoded || bytes.get(*cursor) != Some(&b'e') {
                    return Err(());
                }
                *cursor += 1;
            }
            marker @ (b'l' | b'd') => {
                *cursor += 1;
                let mut previous: Option<&[u8]> = None;
                while bytes.get(*cursor) != Some(&b'e') {
                    if marker == b'd' {
                        let key = string(bytes, cursor)?;
                        if previous.is_some_and(|previous| previous >= key) {
                            return Err(());
                        }
                        previous = Some(key);
                    }
                    value(bytes, cursor, depth + 1, count)?;
                }
                *cursor += 1;
            }
            _ => return Err(()),
        }
        Ok(())
    }
    let mut cursor = 0;
    value(bytes, &mut cursor, 0, &mut 0)
        .map_err(|_| "The torrent file has invalid or excessive metadata")?;
    if cursor != bytes.len() {
        return Err("The torrent file has trailing data".into());
    }
    Ok(())
}

fn component(bytes: &[u8]) -> Result<String, String> {
    let value = std::str::from_utf8(bytes).map_err(|_| "Torrent filenames must use UTF-8")?;
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if value.is_empty() || value.len() > 240 || value.ends_with(['.', ' ']) || reserved.contains(&stem.as_str())
        || value.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}'))
    { return Err("The torrent contains an unsafe filename".into()); }
    Ok(value.into())
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Metainfo, String> {
    if bytes.is_empty() || bytes.len() > MAX_TORRENT_BYTES {
        return Err("Torrent files must be smaller than 8 MB".into());
    }
    validate_bencode(bytes)?;
    let meta =
        librqbit::torrent_from_bytes(bytes).map_err(|_| "Could not parse this torrent file")?;
    let name = component(
        meta.info
            .data
            .name
            .as_ref()
            .ok_or("The torrent has no name")?
            .as_ref(),
    )?;
    if meta.info.data.symlink_path.is_some() || meta.info.data.attr.is_some() {
        return Err("Special torrent files are not supported".into());
    }
    let mut files = Vec::new();
    let mut paths = BTreeSet::new();
    let mut total = 0u64;
    if let Some(entries) = &meta.info.data.files {
        if entries.is_empty() || entries.len() > 10_000 || meta.info.data.length.is_some() {
            return Err("The torrent has an invalid file list".into());
        }
        for entry in entries {
            if entry.symlink_path.is_some()
                || entry.attr.is_some()
                || entry.path.is_empty()
                || entry.path.len() > 32
            {
                return Err("The torrent contains unsupported file entries".into());
            }
            let parts = entry
                .path
                .iter()
                .map(|part| component(part.as_ref()))
                .collect::<Result<Vec<_>, _>>()?;
            let path = parts.join("/");
            if path.len() > 2048 || !paths.insert(path.to_lowercase()) {
                return Err("The torrent contains overlapping or excessive paths".into());
            }
            total = total
                .checked_add(entry.length)
                .ok_or("The torrent size is too large")?;
            files.push(TorrentFileEntry {
                path: format!("{name}/{path}"),
                bytes: entry.length.to_string(),
            });
        }
        for path in &paths {
            for (offset, _) in path.match_indices('/') {
                if paths.contains(&path[..offset]) {
                    return Err("The torrent contains overlapping paths".into());
                }
            }
        }
    } else {
        total = meta
            .info
            .data
            .length
            .ok_or("The torrent has no file length")?;
        files.push(TorrentFileEntry {
            path: name.clone(),
            bytes: total.to_string(),
        });
    }
    meta.info
        .data
        .clone()
        .validate()
        .map_err(|_| "The torrent file layout or piece hashes are invalid")?;
    let mut magnet = Url::parse(&format!(
        "magnet:?xt=urn:btih:{}",
        meta.info_hash.as_string()
    ))
    .map_err(|_| "Invalid torrent hash")?;
    magnet.query_pairs_mut().append_pair("dn", &name);
    let trackers = meta.iter_announce().collect::<Vec<_>>();
    if trackers.len() > 32 {
        return Err("The torrent contains too many trackers".into());
    }
    for tracker in trackers {
        let tracker =
            std::str::from_utf8(tracker.as_ref()).map_err(|_| "Invalid torrent tracker")?;
        magnet.query_pairs_mut().append_pair("tr", tracker);
    }
    Ok(Metainfo {
        name,
        magnet: magnet.into(),
        files,
        total,
    })
}

pub(crate) async fn read_local(source: &str) -> Result<Vec<u8>, String> {
    let url = Url::parse(source).map_err(|_| "Invalid torrent file location")?;
    if url.scheme() != "file" || url.host_str().is_some_and(|host| host != "localhost") {
        return Err("Choose a local torrent file".into());
    }
    let path = url
        .to_file_path()
        .map_err(|_| "Invalid torrent file location")?;
    let bytes = super::persistence::read_bounded_regular_file(&path, MAX_TORRENT_BYTES as u64)
        .await
        .map_err(|_| "Could not read the torrent file")?
        .ok_or("The torrent file is missing")?;
    let cache = dirs::config_dir().map(|base| base.join("QuiverDL").join("torrent-imports"));
    if cache.as_deref() == path.parent()
        && path.file_stem().and_then(|name| name.to_str())
            != Some(hex::encode(Sha256::digest(&bytes)).as_str())
    {
        return Err("The previewed torrent metadata changed; open the file again".into());
    }
    Ok(bytes)
}

pub(crate) async fn cache(bytes: &[u8]) -> Result<String, String> {
    use tokio::io::AsyncWriteExt;
    let directory = dirs::config_dir()
        .ok_or("Could not locate the app folder")?
        .join("QuiverDL")
        .join("torrent-imports");
    let parent = directory.parent().ok_or("Invalid torrent imports folder")?;
    super::browser_bridge::ensure_private_directory(parent)
        .await
        .map_err(|_| "Could not prepare app configuration")?;
    super::browser_bridge::ensure_private_directory(&directory)
        .await
        .map_err(|_| "Could not create the torrent imports folder")?;
    let path = directory.join(format!("{}.torrent", hex::encode(Sha256::digest(bytes))));
    let mut options = tokio::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    match options.open(&path).await {
        Ok(mut file) => {
            file.write_all(bytes)
                .await
                .map_err(|_| "Could not save the torrent metadata")?;
            file.sync_all()
                .await
                .map_err(|_| "Could not save the torrent metadata")?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing =
                super::persistence::read_bounded_regular_file(&path, MAX_TORRENT_BYTES as u64)
                    .await
                    .map_err(|_| "Invalid cached torrent")?;
            if existing.as_deref() != Some(bytes) {
                return Err("The cached torrent metadata changed".into());
            }
        }
        Err(_) => return Err("Could not save the torrent metadata".into()),
    }
    Url::from_file_path(Path::new(&path))
        .map(Into::into)
        .map_err(|_| "Invalid torrent cache location".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_malformed_and_unsafe_metadata() {
        for bytes in [
            b"d1:ai1e1:ai2ee".as_slice(),
            b"i01e",
            b"i-0e",
            b"999999999999999999999999:x",
            b"lee",
        ] {
            assert!(validate_bencode(bytes).is_err());
        }
        assert!(validate_bencode(&[vec![b'l'; 50], vec![b'e'; 50]].concat()).is_err());
        for path in ["..", "CON.txt", "a/b", "a\\b", "a:", "a."] {
            assert!(component(path.as_bytes()).is_err());
        }
    }
    #[test]
    fn parses_a_local_single_file_torrent_without_network() {
        let bytes = b"d8:announce32:https://tracker.example/announce4:infod6:lengthi1e4:name8:test.bin12:piece lengthi16384e6:pieces20:01234567890123456789ee";
        let parsed = parse(bytes).expect("valid fixture");
        assert_eq!(parsed.total, 1);
        assert_eq!(parsed.files[0].path, "test.bin");
        assert!(parsed.magnet.starts_with("magnet:?xt=urn:btih:"));
    }

    #[test]
    fn multifile_preview_preserves_engine_indices_and_rejects_case_collisions() {
        let bytes = b"d8:announce32:https://tracker.example/announce4:infod5:filesld6:lengthi1e4:pathl5:a.txteed6:lengthi2e4:pathl5:b.txteee4:name4:demo12:piece lengthi16384e6:pieces20:01234567890123456789ee";
        let parsed = parse(bytes).expect("valid multifile fixture");
        assert_eq!(parsed.files[0].path, "demo/a.txt");
        assert_eq!(parsed.files[1].path, "demo/b.txt");
        assert_eq!(parsed.total, 3);
        let duplicate = String::from_utf8(bytes.to_vec())
            .unwrap()
            .replace("b.txt", "A.txt");
        assert!(parse(duplicate.as_bytes()).is_err());
    }
}
