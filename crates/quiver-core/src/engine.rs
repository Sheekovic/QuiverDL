use std::{path::Path, time::Duration};

use futures_util::StreamExt;
use reqwest::{
    Client, StatusCode,
    header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, ETAG, IF_RANGE, LAST_MODIFIED, RANGE},
};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};
use url::Url;

use crate::{
    DownloadControl, DownloadRequest, DownloadStatus, Error, ProgressEvent, Result,
    state::{self, PartialState, sibling_with_suffix},
};

const USER_AGENT: &str = concat!("QuiverDL/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    pub effective_url: Url,
    pub total_bytes: Option<u64>,
    pub supports_ranges: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadResult {
    pub bytes_written: u64,
    pub sha256: [u8; 32],
    pub resumed: bool,
}

#[derive(Debug, Clone)]
pub struct DownloadEngine {
    client: Client,
}

impl DownloadEngine {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(24 * 60 * 60))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()?;
        Ok(Self { client })
    }

    pub async fn probe(&self, url: &Url) -> Result<ProbeResult> {
        validate_url(url)?;
        let response = self
            .client
            .get(url.clone())
            .header(RANGE, "bytes=0-0")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(Error::InvalidResponse(format!(
                "probe returned HTTP {}",
                response.status()
            )));
        }

        let supports_ranges = response.status() == StatusCode::PARTIAL_CONTENT
            || response
                .headers()
                .get(ACCEPT_RANGES)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.eq_ignore_ascii_case("bytes"));
        let total_bytes = if response.status() == StatusCode::PARTIAL_CONTENT {
            response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_content_range_total)
        } else {
            header_u64(response.headers().get(CONTENT_LENGTH))
        };

        Ok(ProbeResult {
            effective_url: response.url().clone(),
            total_bytes,
            supports_ranges,
            etag: header_string(response.headers().get(ETAG)),
            last_modified: header_string(response.headers().get(LAST_MODIFIED)),
        })
    }

    pub async fn download(
        &self,
        request: DownloadRequest,
        control: DownloadControl,
        progress: mpsc::UnboundedSender<ProgressEvent>,
    ) -> Result<DownloadResult> {
        validate_url(&request.url)?;
        ensure_destination(&request.destination, request.overwrite_existing).await?;
        emit(&progress, &request, DownloadStatus::Probing, 0, None);

        let probe = self.probe(&request.url).await?;
        let partial_path = sibling_with_suffix(&request.destination, ".quiver-part");
        let state_path = sibling_with_suffix(&request.destination, ".quiver.json");
        let previous = state::load(&state_path).await?;
        let mut offset = file_len(&partial_path).await?;

        if offset > 0 && !can_resume(&request, &probe, previous.as_ref(), offset) {
            tokio::fs::remove_file(&partial_path).await?;
            offset = 0;
        }

        let partial_state = PartialState {
            url: request.url.to_string(),
            total_bytes: probe.total_bytes,
            etag: probe.etag.clone(),
            last_modified: probe.last_modified.clone(),
        };
        state::save(&state_path, &partial_state).await?;

        let mut builder = self.client.get(probe.effective_url.clone());
        if offset > 0 {
            builder = builder.header(RANGE, format!("bytes={offset}-"));
            if let Some(validator) = probe.etag.as_ref().or(probe.last_modified.as_ref()) {
                builder = builder.header(IF_RANGE, validator);
            }
        }

        control.checkpoint().await?;
        let response = builder.send().await?;
        if offset > 0 && response.status() != StatusCode::PARTIAL_CONTENT {
            return Err(Error::InvalidResponse(
                "server refused a validated resume request".into(),
            ));
        }
        if offset == 0 && !response.status().is_success() {
            return Err(Error::InvalidResponse(format!(
                "download returned HTTP {}",
                response.status()
            )));
        }

        let mut options = tokio::fs::OpenOptions::new();
        options.create(true).write(true);
        if offset > 0 {
            options.append(true);
        } else {
            options.truncate(true);
        }
        let mut file = options.open(&partial_path).await?;
        let mut downloaded = offset;
        let resumed = offset > 0;
        emit(
            &progress,
            &request,
            DownloadStatus::Downloading,
            downloaded,
            probe.total_bytes,
        );

        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            control.checkpoint().await?;
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            downloaded = downloaded
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| Error::InvalidResponse("download size overflowed u64".into()))?;
            emit(
                &progress,
                &request,
                DownloadStatus::Downloading,
                downloaded,
                probe.total_bytes,
            );
        }
        file.flush().await?;
        file.sync_all().await?;
        drop(file);

        if let Some(total) = probe.total_bytes
            && downloaded != total
        {
            return Err(Error::InvalidResponse(format!(
                "expected {total} bytes but received {downloaded}"
            )));
        }

        emit(
            &progress,
            &request,
            DownloadStatus::Verifying,
            downloaded,
            probe.total_bytes,
        );
        let sha256 = hash_file(&partial_path).await?;
        if request
            .expected_sha256
            .is_some_and(|expected| expected != sha256)
        {
            return Err(Error::ChecksumMismatch);
        }

        if request.overwrite_existing && tokio::fs::try_exists(&request.destination).await? {
            tokio::fs::remove_file(&request.destination).await?;
        }
        tokio::fs::rename(&partial_path, &request.destination).await?;
        if tokio::fs::try_exists(&state_path).await? {
            tokio::fs::remove_file(state_path).await?;
        }
        emit(
            &progress,
            &request,
            DownloadStatus::Completed,
            downloaded,
            probe.total_bytes,
        );

        Ok(DownloadResult {
            bytes_written: downloaded,
            sha256,
            resumed,
        })
    }
}

impl Default for DownloadEngine {
    fn default() -> Self {
        Self::new().expect("the built-in HTTP client configuration is valid")
    }
}

fn validate_url(url: &Url) -> Result<()> {
    if matches!(url.scheme(), "http" | "https") {
        Ok(())
    } else {
        Err(Error::UnsupportedScheme)
    }
}

async fn ensure_destination(path: &Path, overwrite: bool) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| Error::InvalidDestination(path.to_path_buf()))?;
    tokio::fs::create_dir_all(parent).await?;
    if !overwrite && tokio::fs::try_exists(path).await? {
        return Err(Error::DestinationExists(path.to_path_buf()));
    }
    Ok(())
}

fn can_resume(
    request: &DownloadRequest,
    probe: &ProbeResult,
    state: Option<&PartialState>,
    offset: u64,
) -> bool {
    let Some(state) = state else { return false };
    if state.url != request.url.as_str() || !probe.supports_ranges {
        return false;
    }
    if probe.total_bytes.is_some_and(|total| offset >= total)
        || state.total_bytes != probe.total_bytes
    {
        return false;
    }

    match (&state.etag, &probe.etag) {
        (Some(old), Some(current)) => old == current,
        _ => matches!(
            (&state.last_modified, &probe.last_modified),
            (Some(old), Some(current)) if old == current
        ),
    }
}

fn emit(
    sender: &mpsc::UnboundedSender<ProgressEvent>,
    request: &DownloadRequest,
    status: DownloadStatus,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) {
    let _ = sender.send(ProgressEvent {
        id: request.id,
        status,
        downloaded_bytes,
        total_bytes,
    });
}

fn header_u64(value: Option<&reqwest::header::HeaderValue>) -> Option<u64> {
    value?.to_str().ok()?.parse().ok()
}

fn header_string(value: Option<&reqwest::header::HeaderValue>) -> Option<String> {
    value?.to_str().ok().map(ToOwned::to_owned)
}

fn parse_content_range_total(value: &str) -> Option<u64> {
    value.rsplit_once('/')?.1.parse().ok()
}

async fn file_len(path: &Path) -> Result<u64> {
    match tokio::fs::metadata(path).await {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

async fn hash_file(path: &Path) -> Result<[u8; 32]> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 128 * 1024];
    loop {
        let count = file.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize().into())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use url::Url;

    use super::{ProbeResult, can_resume, parse_content_range_total};
    use crate::{DownloadRequest, state::PartialState};

    #[test]
    fn parses_content_range_total() {
        assert_eq!(parse_content_range_total("bytes 0-0/4096"), Some(4096));
        assert_eq!(parse_content_range_total("broken"), None);
        assert_eq!(parse_content_range_total("bytes 0-0/*"), None);
    }

    #[test]
    fn resumes_only_with_a_matching_validator() {
        let request = DownloadRequest::new(
            Url::parse("https://example.test/file").expect("valid fixture URL"),
            Path::new("C:/Downloads/file"),
        );
        let probe = ProbeResult {
            effective_url: request.url.clone(),
            total_bytes: Some(100),
            supports_ranges: true,
            etag: Some("v1".into()),
            last_modified: None,
        };
        let state = PartialState {
            url: request.url.to_string(),
            total_bytes: Some(100),
            etag: Some("v1".into()),
            last_modified: None,
        };

        assert!(can_resume(&request, &probe, Some(&state), 50));
        assert!(!can_resume(&request, &probe, Some(&state), 100));

        let changed = ProbeResult {
            etag: Some("v2".into()),
            ..probe
        };
        assert!(!can_resume(&request, &changed, Some(&state), 50));
    }
}
