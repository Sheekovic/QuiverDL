use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use futures_util::{StreamExt, future::try_join_all};
use percent_encoding::percent_decode_str;
use reqwest::{
    Client, StatusCode,
    header::{
        ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, ETAG, IF_RANGE,
        LAST_MODIFIED, RANGE,
    },
};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};
use url::Url;

use crate::{
    BandwidthLimiter, DownloadControl, DownloadRequest, DownloadStatus, Error,
    HostConnectionPolicy, ProgressEvent, Result,
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
    pub suggested_filename: Option<String>,
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
    global_limiter: Option<BandwidthLimiter>,
    host_policy: HostConnectionPolicy,
}

impl DownloadEngine {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(24 * 60 * 60))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()?;
        Ok(Self {
            client,
            global_limiter: None,
            host_policy: HostConnectionPolicy::default(),
        })
    }

    #[must_use]
    pub fn with_global_limiter(mut self, limiter: Option<BandwidthLimiter>) -> Self {
        self.global_limiter = limiter;
        self
    }

    #[must_use]
    pub fn with_host_policy(mut self, policy: HostConnectionPolicy) -> Self {
        self.host_policy = policy;
        self
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
            return Err(status_error("probe", response.status()));
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
            suggested_filename: suggested_filename(
                response.headers().get(CONTENT_DISPOSITION),
                response.url(),
            ),
        })
    }

    pub async fn download(
        &self,
        request: DownloadRequest,
        control: DownloadControl,
        progress: mpsc::Sender<ProgressEvent>,
    ) -> Result<DownloadResult> {
        let policy = request.retry_policy.normalized();
        for attempt in 1..=policy.max_attempts {
            match self
                .download_once(request.clone(), control.clone(), progress.clone())
                .await
            {
                Ok(result) => return Ok(result),
                Err(error) if error.is_retryable() && attempt < policy.max_attempts => {
                    let exponent = attempt.saturating_sub(1).min(20);
                    let delay_ms = policy
                        .initial_delay_ms
                        .saturating_mul(1_u64 << exponent)
                        .min(policy.max_delay_ms);
                    emit(&progress, &request, DownloadStatus::Retrying, 0, None);
                    tokio::select! {
                        _ = control.cancelled() => return Err(Error::Cancelled),
                        () = tokio::time::sleep(Duration::from_millis(delay_ms)) => {}
                    }
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!("a normalized retry policy always has at least one attempt")
    }

    async fn download_once(
        &self,
        request: DownloadRequest,
        control: DownloadControl,
        progress: mpsc::Sender<ProgressEvent>,
    ) -> Result<DownloadResult> {
        validate_url(&request.url)?;
        ensure_destination(&request.destination, request.overwrite_existing).await?;
        emit(&progress, &request, DownloadStatus::Probing, 0, None);

        let probe = tokio::select! {
            _ = control.cancelled() => return Err(Error::Cancelled),
            probe = self.probe(&request.url) => probe?,
        };
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
        let transfer_policy = request.transfer_policy.normalized();
        let download_limiter = transfer_policy
            .per_download_speed_limit_bps
            .and_then(BandwidthLimiter::new);

        if offset == 0
            && let Some(total) = probe.total_bytes
            && probe.supports_ranges
            && (probe.etag.is_some() || probe.last_modified.is_some())
        {
            let ranges = plan_segments(
                total,
                transfer_policy.max_segments,
                transfer_policy.min_segment_bytes,
            );
            if ranges.len() > 1 {
                let downloaded = self
                    .download_segmented(
                        &request,
                        &probe,
                        &partial_path,
                        &control,
                        &progress,
                        &ranges,
                        download_limiter.as_ref(),
                        transfer_policy.max_connections_per_host,
                    )
                    .await?;
                return finish_download(
                    &request,
                    &control,
                    &progress,
                    &partial_path,
                    &state_path,
                    downloaded,
                    probe.total_bytes,
                    false,
                )
                .await;
            }
        }

        let mut builder = self.client.get(probe.effective_url.clone());
        if offset > 0 {
            builder = builder.header(RANGE, format!("bytes={offset}-"));
            if let Some(validator) = probe.etag.as_ref().or(probe.last_modified.as_ref()) {
                builder = builder.header(IF_RANGE, validator);
            }
        }

        control.checkpoint().await?;
        let _host_permit = tokio::select! {
            _ = control.cancelled() => return Err(Error::Cancelled),
            permit = self.host_policy.acquire(
                &probe.effective_url,
                transfer_policy.max_connections_per_host,
            ) => permit,
        };
        let response = tokio::select! {
            _ = control.cancelled() => return Err(Error::Cancelled),
            response = builder.send() => response?,
        };
        if offset > 0 && response.status() != StatusCode::PARTIAL_CONTENT {
            return Err(Error::InvalidResponse(
                "server refused a validated resume request".into(),
            ));
        }
        let resume_range = if offset > 0 {
            Some(validate_resume_range(
                response.headers(),
                offset,
                probe.total_bytes,
            )?)
        } else {
            None
        };
        if offset == 0 && response.status() != StatusCode::OK {
            return Err(if response.status().is_success() {
                Error::InvalidResponse(format!(
                    "full download returned unexpected HTTP {}",
                    response.status()
                ))
            } else {
                status_error("download", response.status())
            });
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
        loop {
            control.checkpoint().await?;
            let next = tokio::select! {
                _ = control.cancelled() => return Err(Error::Cancelled),
                next = stream.next() => next,
            };
            let Some(chunk) = next else {
                break;
            };
            let chunk = chunk?;
            throttle(
                chunk.len(),
                download_limiter.as_ref(),
                self.global_limiter.as_ref(),
                &control,
            )
            .await?;
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
        control.checkpoint().await?;

        if let Some(range) = resume_range {
            let expected_end = range.end.checked_add(1).ok_or_else(|| {
                Error::InvalidResponse("resume response range overflowed u64".into())
            })?;
            if downloaded != expected_end {
                return Err(Error::InvalidResponse(format!(
                    "resume response ended at byte {} but received through byte {}",
                    range.end,
                    downloaded.saturating_sub(1)
                )));
            }
        }

        if let Some(total) = probe.total_bytes
            && downloaded != total
        {
            return Err(Error::InvalidResponse(format!(
                "expected {total} bytes but received {downloaded}"
            )));
        }

        finish_download(
            &request,
            &control,
            &progress,
            &partial_path,
            &state_path,
            downloaded,
            probe.total_bytes,
            resumed,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn download_segmented(
        &self,
        request: &DownloadRequest,
        probe: &ProbeResult,
        partial_path: &Path,
        control: &DownloadControl,
        progress: &mpsc::Sender<ProgressEvent>,
        ranges: &[ByteRange],
        download_limiter: Option<&BandwidthLimiter>,
        max_connections_per_host: u8,
    ) -> Result<u64> {
        let segment_paths: Vec<PathBuf> = (0..ranges.len())
            .map(|index| sibling_with_suffix(partial_path, &format!(".segment-{index}")))
            .collect();
        let downloaded = Arc::new(AtomicU64::new(0));
        emit(
            progress,
            request,
            DownloadStatus::Downloading,
            0,
            probe.total_bytes,
        );

        let transfers = ranges.iter().copied().enumerate().map(|(index, range)| {
            download_segment(
                self.client.clone(),
                probe.effective_url.clone(),
                probe.etag.clone().or_else(|| probe.last_modified.clone()),
                probe
                    .total_bytes
                    .expect("segmented downloads have a known size"),
                range,
                segment_paths[index].clone(),
                control.clone(),
                request.clone(),
                progress.clone(),
                Arc::clone(&downloaded),
                download_limiter.cloned(),
                self.global_limiter.clone(),
                self.host_policy.clone(),
                max_connections_per_host,
            )
        });

        let transfer_result: Result<u64> = async {
            try_join_all(transfers).await?;
            control.checkpoint().await?;
            let mut output = tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(partial_path)
                .await?;
            let mut merged = 0_u64;
            for path in &segment_paths {
                let mut segment = tokio::fs::File::open(path).await?;
                merged = merged
                    .checked_add(tokio::io::copy(&mut segment, &mut output).await?)
                    .ok_or_else(|| Error::InvalidResponse("merged size overflowed u64".into()))?;
            }
            output.flush().await?;
            output.sync_all().await?;
            if merged
                != probe
                    .total_bytes
                    .expect("segmented downloads have a known size")
            {
                return Err(Error::InvalidResponse(format!(
                    "merged download contained {merged} bytes instead of the expected {}",
                    probe.total_bytes.expect("known total")
                )));
            }
            Ok(merged)
        }
        .await;

        for path in segment_paths {
            if tokio::fs::try_exists(&path).await.unwrap_or(false) {
                let _ = tokio::fs::remove_file(path).await;
            }
        }
        transfer_result
    }
}

impl Default for DownloadEngine {
    fn default() -> Self {
        Self::new().expect("the built-in HTTP client configuration is valid")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ByteRange {
    start: u64,
    end: u64,
}

fn plan_segments(total: u64, max_segments: u8, min_segment_bytes: u64) -> Vec<ByteRange> {
    if total == 0 {
        return Vec::new();
    }
    let count = u64::from(max_segments.max(1)).min((total / min_segment_bytes.max(1)).max(1));
    let segment_size = total.div_ceil(count);
    (0..count)
        .map(|index| {
            let start = index * segment_size;
            ByteRange {
                start,
                end: (start + segment_size - 1).min(total - 1),
            }
        })
        .filter(|range| range.start <= range.end)
        .collect()
}

#[allow(clippy::too_many_arguments)]
async fn download_segment(
    client: Client,
    url: Url,
    validator: Option<String>,
    total: u64,
    range: ByteRange,
    path: PathBuf,
    control: DownloadControl,
    request: DownloadRequest,
    progress: mpsc::Sender<ProgressEvent>,
    downloaded: Arc<AtomicU64>,
    download_limiter: Option<BandwidthLimiter>,
    global_limiter: Option<BandwidthLimiter>,
    host_policy: HostConnectionPolicy,
    max_connections_per_host: u8,
) -> Result<()> {
    control.checkpoint().await?;
    let _host_permit = tokio::select! {
        _ = control.cancelled() => return Err(Error::Cancelled),
        permit = host_policy.acquire(&url, max_connections_per_host) => permit,
    };
    let mut builder = client
        .get(url)
        .header(RANGE, format!("bytes={}-{}", range.start, range.end));
    if let Some(validator) = validator {
        builder = builder.header(IF_RANGE, validator);
    }
    let response = tokio::select! {
        _ = control.cancelled() => return Err(Error::Cancelled),
        response = builder.send() => response?,
    };
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(Error::InvalidResponse(format!(
            "segment request returned HTTP {}",
            response.status()
        )));
    }
    let actual = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_content_range)
        .ok_or_else(|| {
            Error::InvalidResponse("segment response has no valid Content-Range".into())
        })?;
    if actual.start != range.start || actual.end != range.end || actual.total != Some(total) {
        return Err(Error::InvalidResponse(format!(
            "segment response range {}-{} did not match requested range {}-{}",
            actual.start, actual.end, range.start, range.end
        )));
    }

    let mut output = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .await?;
    let mut segment_bytes = 0_u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = tokio::select! {
        _ = control.cancelled() => return Err(Error::Cancelled),
        next = stream.next() => next,
    } {
        control.checkpoint().await?;
        let chunk = chunk?;
        throttle(
            chunk.len(),
            download_limiter.as_ref(),
            global_limiter.as_ref(),
            &control,
        )
        .await?;
        output.write_all(&chunk).await?;
        segment_bytes = segment_bytes
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| Error::InvalidResponse("segment size overflowed u64".into()))?;
        let aggregate =
            downloaded.fetch_add(chunk.len() as u64, Ordering::Relaxed) + chunk.len() as u64;
        emit(
            &progress,
            &request,
            DownloadStatus::Downloading,
            aggregate,
            Some(total),
        );
    }
    output.flush().await?;
    output.sync_all().await?;
    let expected = range.end - range.start + 1;
    if segment_bytes != expected {
        return Err(Error::InvalidResponse(format!(
            "segment contained {segment_bytes} bytes instead of {expected}"
        )));
    }
    Ok(())
}

async fn throttle(
    bytes: usize,
    download_limiter: Option<&BandwidthLimiter>,
    global_limiter: Option<&BandwidthLimiter>,
    control: &DownloadControl,
) -> Result<()> {
    tokio::select! {
        _ = control.cancelled() => Err(Error::Cancelled),
        () = async {
            if let Some(limiter) = download_limiter {
                limiter.wait(bytes).await;
            }
            if let Some(limiter) = global_limiter {
                limiter.wait(bytes).await;
            }
        } => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
async fn finish_download(
    request: &DownloadRequest,
    control: &DownloadControl,
    progress: &mpsc::Sender<ProgressEvent>,
    partial_path: &Path,
    state_path: &Path,
    downloaded: u64,
    total_bytes: Option<u64>,
    resumed: bool,
) -> Result<DownloadResult> {
    emit(
        progress,
        request,
        DownloadStatus::Verifying,
        downloaded,
        total_bytes,
    );
    let sha256 = hash_file(partial_path, control).await?;
    if request
        .expected_sha256
        .is_some_and(|expected| expected != sha256)
    {
        return Err(Error::ChecksumMismatch);
    }

    control.checkpoint().await?;
    promote_partial(
        partial_path,
        &request.destination,
        request.overwrite_existing,
    )
    .await?;
    if tokio::fs::try_exists(state_path).await? {
        tokio::fs::remove_file(state_path).await?;
    }
    emit(
        progress,
        request,
        DownloadStatus::Completed,
        downloaded,
        total_bytes,
    );
    Ok(DownloadResult {
        bytes_written: downloaded,
        sha256,
        resumed,
    })
}

fn validate_url(url: &Url) -> Result<()> {
    if matches!(url.scheme(), "http" | "https") {
        Ok(())
    } else {
        Err(Error::UnsupportedScheme)
    }
}

fn status_error(operation: &str, status: StatusCode) -> Error {
    let message = format!("{operation} returned HTTP {status}");
    if status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
    {
        Error::TransientResponse(message)
    } else {
        Error::InvalidResponse(message)
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
    sender: &mpsc::Sender<ProgressEvent>,
    request: &DownloadRequest,
    status: DownloadStatus,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) {
    let _ = sender.try_send(ProgressEvent {
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

fn suggested_filename(
    disposition: Option<&reqwest::header::HeaderValue>,
    effective_url: &Url,
) -> Option<String> {
    disposition
        .and_then(|value| value.to_str().ok())
        .and_then(filename_from_content_disposition)
        .or_else(|| {
            effective_url
                .path_segments()
                .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
                .and_then(|segment| percent_decode_str(segment).decode_utf8().ok())
                .map(|value| value.into_owned())
                .filter(|value| !value.trim().is_empty())
        })
}

fn filename_from_content_disposition(value: &str) -> Option<String> {
    let mut fallback = None;
    for parameter in value.split(';').skip(1) {
        let Some((name, raw_value)) = parameter.trim().split_once('=') else {
            continue;
        };
        let raw_value = raw_value.trim().trim_matches('"');
        if name.trim().eq_ignore_ascii_case("filename*") {
            let encoded = raw_value
                .split_once("''")
                .map_or(raw_value, |(_, encoded)| encoded);
            if let Ok(decoded) = percent_decode_str(encoded).decode_utf8() {
                let decoded = decoded.trim();
                if !decoded.is_empty() {
                    return Some(decoded.to_owned());
                }
            }
        } else if name.trim().eq_ignore_ascii_case("filename") && !raw_value.is_empty() {
            fallback = Some(raw_value.to_owned());
        }
    }
    fallback
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContentRange {
    start: u64,
    end: u64,
    total: Option<u64>,
}

fn parse_content_range(value: &str) -> Option<ContentRange> {
    let (unit, value) = value.trim().split_once(' ')?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }

    let (range, total) = value.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start = start.parse().ok()?;
    let end = end.parse().ok()?;
    if start > end {
        return None;
    }

    let total = match total {
        "*" => None,
        value => Some(value.parse().ok()?),
    };
    if total.is_some_and(|total| end >= total) {
        return None;
    }

    Some(ContentRange { start, end, total })
}

fn parse_content_range_total(value: &str) -> Option<u64> {
    parse_content_range(value)?.total
}

fn validate_resume_range(
    headers: &reqwest::header::HeaderMap,
    expected_start: u64,
    expected_total: Option<u64>,
) -> Result<ContentRange> {
    let value = headers
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_content_range)
        .ok_or_else(|| {
            Error::InvalidResponse("resume response has no valid Content-Range".into())
        })?;

    if value.start != expected_start {
        return Err(Error::InvalidResponse(format!(
            "resume response started at byte {} instead of {expected_start}",
            value.start
        )));
    }
    if let Some(expected_total) = expected_total
        && value.total != Some(expected_total)
    {
        return Err(Error::InvalidResponse(format!(
            "resume response total {:?} did not match expected total {expected_total}",
            value.total
        )));
    }

    Ok(value)
}

async fn file_len(path: &Path) -> Result<u64> {
    match tokio::fs::metadata(path).await {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

async fn hash_file(path: &Path, control: &DownloadControl) -> Result<[u8; 32]> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 128 * 1024];
    loop {
        control.checkpoint().await?;
        let count = tokio::select! {
            _ = control.cancelled() => return Err(Error::Cancelled),
            count = file.read(&mut buffer) => count?,
        };
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize().into())
}

async fn promote_partial(partial: &Path, destination: &Path, overwrite: bool) -> Result<()> {
    if overwrite {
        if tokio::fs::try_exists(destination).await? {
            tokio::fs::remove_file(destination).await?;
        }
        tokio::fs::rename(partial, destination).await?;
        return Ok(());
    }

    match atomic_rename_noreplace(partial, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(Error::DestinationExists(destination.to_path_buf()))
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(any(target_os = "android", target_os = "linux", target_os = "macos"))]
fn atomic_rename_noreplace(partial: &Path, destination: &Path) -> std::io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};

    renameat_with(CWD, partial, CWD, destination, RenameFlags::NOREPLACE).map_err(Into::into)
}

#[cfg(windows)]
fn atomic_rename_noreplace(partial: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;

    let partial: Vec<u16> = partial.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let result = unsafe { MoveFileExW(partial.as_ptr(), destination.as_ptr(), 0) };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(any(
    target_os = "android",
    target_os = "linux",
    target_os = "macos",
    windows
)))]
fn atomic_rename_noreplace(partial: &Path, destination: &Path) -> std::io::Result<()> {
    match std::fs::hard_link(partial, destination) {
        Ok(()) => {
            std::fs::remove_file(partial)?;
            Ok(())
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use url::Url;

    use super::{
        ByteRange, ContentRange, ProbeResult, can_resume, parse_content_range,
        parse_content_range_total, plan_segments,
    };
    use crate::{DownloadRequest, Error, state::PartialState};

    #[test]
    fn parses_content_range_total() {
        assert_eq!(parse_content_range_total("bytes 0-0/4096"), Some(4096));
        assert_eq!(parse_content_range_total("broken"), None);
        assert_eq!(parse_content_range_total("bytes 0-0/*"), None);
    }

    #[test]
    fn plans_bounded_non_overlapping_segments() {
        assert_eq!(
            plan_segments(25, 4, 8),
            vec![
                ByteRange { start: 0, end: 8 },
                ByteRange { start: 9, end: 17 },
                ByteRange { start: 18, end: 24 },
            ]
        );
        assert_eq!(
            plan_segments(7, 16, 8),
            vec![ByteRange { start: 0, end: 6 }]
        );
    }

    #[test]
    fn parses_and_rejects_invalid_content_ranges() {
        assert_eq!(
            parse_content_range("bytes 42-99/100"),
            Some(ContentRange {
                start: 42,
                end: 99,
                total: Some(100),
            })
        );
        assert_eq!(parse_content_range("items 0-1/2"), None);
        assert_eq!(parse_content_range("bytes 9-4/10"), None);
        assert_eq!(parse_content_range("bytes 0-10/10"), None);
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
            suggested_filename: Some("file".into()),
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

    #[test]
    fn discovers_filenames_from_disposition_and_url() {
        use reqwest::header::HeaderValue;

        let url = Url::parse("https://example.test/releases/archive%20one.zip")
            .expect("valid fixture URL");
        assert_eq!(
            super::suggested_filename(None, &url).as_deref(),
            Some("archive one.zip")
        );
        let disposition = HeaderValue::from_static(
            "attachment; filename=old.zip; filename*=UTF-8''Quiver%20Release.zip",
        );
        assert_eq!(
            super::suggested_filename(Some(&disposition), &url).as_deref(),
            Some("Quiver Release.zip")
        );
    }

    #[tokio::test]
    async fn no_clobber_promotion_preserves_a_racing_destination() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let partial = directory.path().join("archive.quiver-part");
        let destination = directory.path().join("archive");
        tokio::fs::write(&partial, b"new")
            .await
            .expect("partial should write");
        tokio::fs::write(&destination, b"existing")
            .await
            .expect("destination should write");

        let error = super::promote_partial(&partial, &destination, false)
            .await
            .expect_err("an existing destination must not be replaced");
        assert!(matches!(error, Error::DestinationExists(path) if path == destination));
        assert_eq!(
            tokio::fs::read(&destination)
                .await
                .expect("destination should remain readable"),
            b"existing"
        );
        assert_eq!(
            tokio::fs::read(&partial)
                .await
                .expect("partial should remain recoverable"),
            b"new"
        );
    }
}
