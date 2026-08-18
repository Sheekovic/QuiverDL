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

    if tokio::fs::try_exists(path).await? {
        tokio::fs::remove_file(path).await?;
    }
    tokio::fs::rename(temporary, path).await?;
    Ok(())
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
        };

        save(&path, &expected).await.expect("state should save");
        let actual = load(&path)
            .await
            .expect("state should load")
            .expect("state exists");
        assert_eq!(actual.url, expected.url);
        assert_eq!(actual.total_bytes, expected.total_bytes);
        assert_eq!(actual.etag, expected.etag);
    }
}
