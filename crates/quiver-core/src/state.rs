use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PartialState {
    pub url: String,
    pub total_bytes: Option<u64>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    #[serde(default)]
    pub segment_ranges: Vec<[u64; 2]>,
}

pub(crate) fn sibling_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

pub(crate) async fn load(path: &Path) -> Result<Option<PartialState>> {
    match tokio::fs::read(path).await {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) async fn save(path: &Path, state: &PartialState) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(state)?;
    let temporary = sibling_with_suffix(path, ".tmp");
    let mut file = tokio::fs::File::create(&temporary).await?;
    file.write_all(&bytes).await?;
    file.sync_all().await?;
    drop(file);

    let destination = path.to_path_buf();
    tokio::task::spawn_blocking(move || atomic_replace(&temporary, &destination))
        .await
        .map_err(std::io::Error::other)??;
    Ok(())
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::{PartialState, load, save, sibling_with_suffix};

    #[test]
    fn suffix_is_appended_instead_of_replacing_extension() {
        let path = std::path::Path::new("archive.tar.gz");
        assert_eq!(
            sibling_with_suffix(path, ".quiver-part"),
            std::path::Path::new("archive.tar.gz.quiver-part")
        );
    }

    #[tokio::test]
    async fn state_round_trips() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("download.quiver.json");
        let expected = PartialState {
            url: "https://example.test/file.bin".into(),
            total_bytes: Some(42),
            etag: Some("fixture".into()),
            last_modified: None,
            segment_ranges: vec![[0, 20], [21, 41]],
        };

        save(&path, &expected).await.expect("state should save");
        let actual = load(&path)
            .await
            .expect("state should load")
            .expect("state exists");
        assert_eq!(actual.url, expected.url);
        assert_eq!(actual.total_bytes, expected.total_bytes);
        assert_eq!(actual.etag, expected.etag);
        assert_eq!(actual.segment_ranges, expected.segment_ranges);
    }
}
