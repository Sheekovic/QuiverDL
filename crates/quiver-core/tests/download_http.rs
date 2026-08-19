use std::sync::Arc;

use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{mpsc, oneshot},
    time::{Duration, timeout},
};
use url::Url;

use quiver_core::{
    DownloadControl, DownloadEngine, DownloadRequest, DownloadStatus, Error, ProgressEvent,
    RetryPolicy, TransferPolicy,
};

const FIXTURE: &[u8] = b"QuiverDL end-to-end transfer fixture";
const RESUME_OFFSET: usize = 9;

#[tokio::test]
async fn retries_transient_failures_with_a_visible_state() {
    let (url, server) = transient_fixture_server().await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let destination = directory.path().join("retry.bin");
    let mut request = DownloadRequest::new(url, &destination);
    request.retry_policy = RetryPolicy {
        max_attempts: 2,
        initial_delay_ms: 100,
        max_delay_ms: 100,
    };
    let (progress_tx, mut progress_rx) = mpsc::channel::<ProgressEvent>(32);

    let result = DownloadEngine::new()
        .expect("engine should initialize")
        .download(request, DownloadControl::new(), progress_tx)
        .await
        .expect("second attempt should succeed");
    assert_eq!(result.bytes_written, FIXTURE.len() as u64);
    let mut saw_retry = false;
    while let Ok(event) = progress_rx.try_recv() {
        saw_retry |= event.status == DownloadStatus::Retrying;
    }
    assert!(saw_retry);
    server
        .await
        .expect("transient fixture server should finish");
}

#[tokio::test]
async fn downloads_and_merges_validated_parallel_segments() {
    let (url, server, fixture) = segmented_fixture_server().await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let destination = directory.path().join("parallel.bin");
    let expected_hash: [u8; 32] = Sha256::digest(fixture.as_slice()).into();
    let mut request = DownloadRequest::new(url, &destination);
    request.expected_sha256 = Some(expected_hash);
    request.transfer_policy = TransferPolicy {
        max_segments: 3,
        max_connections_per_host: 3,
        min_segment_bytes: 1024 * 1024,
        per_download_speed_limit_bps: None,
    };
    let (progress_tx, progress_rx) = mpsc::channel::<ProgressEvent>(128);
    drop(progress_rx);

    let result = DownloadEngine::new()
        .expect("engine should initialize")
        .download(request, DownloadControl::new(), progress_tx)
        .await
        .expect("parallel download should succeed");

    assert_eq!(result.bytes_written, fixture.len() as u64);
    assert_eq!(result.sha256, expected_hash);
    assert_eq!(
        tokio::fs::read(destination)
            .await
            .expect("merged destination should read"),
        fixture.as_slice()
    );
    server
        .await
        .expect("segmented fixture server should finish");
}

#[tokio::test]
async fn retains_completed_segments_until_verification_succeeds() {
    let (url, server, _fixture) = segmented_fixture_server().await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let destination = directory.path().join("parallel.bin");
    let mut request = DownloadRequest::new(url, &destination);
    request.expected_sha256 = Some([0_u8; 32]);
    request.transfer_policy = TransferPolicy {
        max_segments: 3,
        max_connections_per_host: 3,
        min_segment_bytes: 1024 * 1024,
        per_download_speed_limit_bps: None,
    };
    let (progress_tx, progress_rx) = mpsc::channel::<ProgressEvent>(128);
    drop(progress_rx);

    let error = DownloadEngine::new()
        .expect("engine should initialize")
        .download(request, DownloadControl::new(), progress_tx)
        .await
        .expect_err("incorrect checksum should fail verification");
    assert!(matches!(error, Error::ChecksumMismatch));
    for index in 0..3 {
        let segment = directory
            .path()
            .join(format!("parallel.bin.quiver-part.segment-{index}"));
        assert!(
            tokio::fs::try_exists(segment)
                .await
                .expect("segment path should be inspectable")
        );
    }
    server
        .await
        .expect("segmented fixture server should finish");
}

#[tokio::test]
async fn downloads_verifies_and_promotes_a_file() {
    let (url, server) = fixture_server(2).await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let destination = directory.path().join("fixture.bin");
    let expected_hash: [u8; 32] = Sha256::digest(FIXTURE).into();
    let mut request = DownloadRequest::new(url, &destination);
    request.expected_sha256 = Some(expected_hash);
    let (progress_tx, mut progress_rx) = mpsc::channel::<ProgressEvent>(32);

    let result = DownloadEngine::new()
        .expect("engine should initialize")
        .download(request, DownloadControl::new(), progress_tx)
        .await
        .expect("download should succeed");

    assert_eq!(result.bytes_written, FIXTURE.len() as u64);
    assert_eq!(result.sha256, expected_hash);
    assert!(!result.resumed);
    assert_eq!(
        tokio::fs::read(&destination)
            .await
            .expect("completed file should exist"),
        FIXTURE
    );
    let partial = directory.path().join("fixture.bin.quiver-part");
    assert!(
        !tokio::fs::try_exists(partial)
            .await
            .expect("partial path can be checked")
    );

    let mut statuses = Vec::new();
    while let Ok(event) = progress_rx.try_recv() {
        statuses.push(event.status);
    }
    assert_eq!(statuses.first(), Some(&DownloadStatus::Probing));
    assert!(statuses.contains(&DownloadStatus::Downloading));
    assert!(statuses.contains(&DownloadStatus::Verifying));
    assert_eq!(statuses.last(), Some(&DownloadStatus::Completed));

    server.await.expect("fixture server should finish");
}

#[tokio::test]
async fn resumes_only_from_the_requested_content_range() {
    let (url, server) = resume_fixture_server(RESUME_OFFSET).await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let destination = directory.path().join("fixture.bin");
    write_resume_files(directory.path(), &url).await;
    let (progress_tx, progress_rx) = mpsc::channel::<ProgressEvent>(32);
    drop(progress_rx);

    let result = DownloadEngine::new()
        .expect("engine should initialize")
        .download(
            DownloadRequest::new(url, &destination),
            DownloadControl::new(),
            progress_tx,
        )
        .await
        .expect("validated resume should succeed");

    assert!(result.resumed);
    assert_eq!(result.bytes_written, FIXTURE.len() as u64);
    assert_eq!(
        tokio::fs::read(destination)
            .await
            .expect("completed file should exist"),
        FIXTURE
    );
    server.await.expect("fixture server should finish");
}

#[tokio::test]
async fn rejects_a_mismatched_resume_range_without_appending() {
    let (url, server) = resume_fixture_server(0).await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let destination = directory.path().join("fixture.bin");
    let partial = directory.path().join("fixture.bin.quiver-part");
    write_resume_files(directory.path(), &url).await;
    let (progress_tx, progress_rx) = mpsc::channel::<ProgressEvent>(32);
    drop(progress_rx);

    let error = DownloadEngine::new()
        .expect("engine should initialize")
        .download(
            DownloadRequest::new(url, &destination),
            DownloadControl::new(),
            progress_tx,
        )
        .await
        .expect_err("mismatched Content-Range must be rejected");

    assert!(
        error
            .to_string()
            .contains("resume response started at byte 0 instead")
    );
    assert_eq!(
        tokio::fs::read(partial)
            .await
            .expect("partial file should remain recoverable"),
        &FIXTURE[..RESUME_OFFSET]
    );
    assert!(
        !tokio::fs::try_exists(destination)
            .await
            .expect("destination can be checked")
    );
    server.await.expect("fixture server should finish");
}

#[tokio::test]
async fn rejects_a_short_unknown_length_resume_span() {
    let (url, server) = short_unknown_length_resume_server().await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let destination = directory.path().join("fixture.bin");
    let partial = directory.path().join("fixture.bin.quiver-part");
    write_unknown_length_resume_files(directory.path(), &url).await;
    let (progress_tx, progress_rx) = mpsc::channel::<ProgressEvent>(32);
    drop(progress_rx);

    let error = DownloadEngine::new()
        .expect("engine should initialize")
        .download(
            DownloadRequest::new(url, &destination),
            DownloadControl::new(),
            progress_tx,
        )
        .await
        .expect_err("a short ranged body must not be promoted");

    assert!(error.to_string().contains("resume response ended at byte"));
    assert!(
        tokio::fs::try_exists(&partial)
            .await
            .expect("partial path can be checked")
    );
    assert!(
        !tokio::fs::try_exists(destination)
            .await
            .expect("destination can be checked")
    );
    server.await.expect("fixture server should finish");
}

#[tokio::test]
async fn cancellation_interrupts_a_stalled_response() {
    let (url, server, started, release) = stalled_fixture_server().await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let destination = directory.path().join("fixture.bin");
    let (progress_tx, progress_rx) = mpsc::channel::<ProgressEvent>(32);
    drop(progress_rx);
    let control = DownloadControl::new();
    let cancellation = control.clone();
    let transfer = tokio::spawn({
        let destination = destination.clone();
        async move {
            DownloadEngine::new()
                .expect("engine should initialize")
                .download(DownloadRequest::new(url, destination), control, progress_tx)
                .await
        }
    });

    started.await.expect("download response should start");
    cancellation.cancel();
    let error = timeout(Duration::from_secs(1), transfer)
        .await
        .expect("cancellation should wake the stalled network wait")
        .expect("transfer task should join")
        .expect_err("cancelled transfer must fail");
    assert!(matches!(error, Error::Cancelled));
    assert!(
        !tokio::fs::try_exists(destination)
            .await
            .expect("destination can be checked")
    );

    let _ = release.send(());
    server.await.expect("fixture server should finish");
}

#[cfg(unix)]
#[tokio::test]
async fn refuses_a_symlinked_partial_without_touching_its_target() {
    use std::os::unix::fs::symlink;

    let (url, server) = fixture_server(2).await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let destination = directory.path().join("fixture.bin");
    let partial = directory.path().join("fixture.bin.quiver-part");
    let target = directory.path().join("target.bin");
    tokio::fs::write(&target, b"")
        .await
        .expect("target should exist");
    symlink(&target, &partial).expect("partial symlink should be created");
    let (progress_tx, progress_rx) = mpsc::channel::<ProgressEvent>(32);
    drop(progress_rx);

    DownloadEngine::new()
        .expect("engine should initialize")
        .download(
            DownloadRequest::new(url, destination),
            DownloadControl::new(),
            progress_tx,
        )
        .await
        .expect_err("a recovery symlink must be rejected");

    assert_eq!(
        tokio::fs::read(target).await.expect("target should read"),
        b""
    );
    server.await.expect("fixture server should finish");
}

async fn fixture_server(expected_requests: usize) -> (Url, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("fixture server should bind");
    let address = listener.local_addr().expect("fixture address");
    let task = tokio::spawn(async move {
        for _ in 0..expected_requests {
            let (mut socket, _) = listener.accept().await.expect("request should arrive");
            let mut request = vec![0_u8; 4096];
            let count = socket
                .read(&mut request)
                .await
                .expect("request should read");
            let request = String::from_utf8_lossy(&request[..count]);

            let response = if request.contains("Range: bytes=0-0")
                || request.contains("range: bytes=0-0")
            {
                let headers = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: 1\r\nContent-Range: bytes 0-0/{}\r\nAccept-Ranges: bytes\r\nETag: fixture-v1\r\nConnection: close\r\n\r\n",
                    FIXTURE.len()
                );
                [headers.as_bytes(), &FIXTURE[..1]].concat()
            } else {
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nETag: fixture-v1\r\nConnection: close\r\n\r\n",
                    FIXTURE.len()
                );
                [headers.as_bytes(), FIXTURE].concat()
            };

            socket
                .write_all(&response)
                .await
                .expect("response should write");
            socket.shutdown().await.expect("socket should close");
        }
    });

    (
        Url::parse(&format!("http://{address}/fixture.bin")).expect("fixture URL"),
        task,
    )
}

async fn segmented_fixture_server() -> (Url, tokio::task::JoinHandle<()>, Arc<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("fixture server should bind");
    let address = listener.local_addr().expect("fixture address");
    let fixture = Arc::new(
        (0..(3 * 1024 * 1024 + 17))
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>(),
    );
    let server_fixture = Arc::clone(&fixture);
    let task = tokio::spawn(async move {
        for _ in 0..4 {
            let (mut socket, _) = listener.accept().await.expect("request should arrive");
            let mut request = vec![0_u8; 8192];
            let count = socket
                .read(&mut request)
                .await
                .expect("request should read");
            let request = String::from_utf8_lossy(&request[..count]);
            let range = request
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("range: bytes=")
                        .map(str::to_owned)
                })
                .expect("all fixture requests include a range");
            let (start, end) = range.split_once('-').expect("valid range");
            let start: usize = start.parse().expect("valid start");
            let end: usize = if end.is_empty() {
                server_fixture.len() - 1
            } else {
                end.parse().expect("valid end")
            };
            let body = &server_fixture[start..=end];
            let headers = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{}\r\nAccept-Ranges: bytes\r\nETag: parallel-v1\r\nConnection: close\r\n\r\n",
                body.len(),
                server_fixture.len()
            );
            socket
                .write_all(&[headers.as_bytes(), body].concat())
                .await
                .expect("response should write");
            socket.shutdown().await.expect("socket should close");
        }
    });
    (
        Url::parse(&format!("http://{address}/parallel.bin")).expect("fixture URL"),
        task,
        fixture,
    )
}

async fn transient_fixture_server() -> (Url, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("fixture server should bind");
    let address = listener.local_addr().expect("fixture address");
    let task = tokio::spawn(async move {
        for index in 0..3 {
            let (mut socket, _) = listener.accept().await.expect("request should arrive");
            let mut request = vec![0_u8; 4096];
            let count = socket
                .read(&mut request)
                .await
                .expect("request should read");
            let request = String::from_utf8_lossy(&request[..count]);
            let response = if index == 0 {
                b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
            } else if request.to_ascii_lowercase().contains("range: bytes=0-0") {
                let headers = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: 1\r\nContent-Range: bytes 0-0/{}\r\nAccept-Ranges: bytes\r\nETag: retry-v1\r\nConnection: close\r\n\r\n",
                    FIXTURE.len()
                );
                [headers.as_bytes(), &FIXTURE[..1]].concat()
            } else {
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: retry-v1\r\nConnection: close\r\n\r\n",
                    FIXTURE.len()
                );
                [headers.as_bytes(), FIXTURE].concat()
            };
            socket
                .write_all(&response)
                .await
                .expect("response should write");
            socket.shutdown().await.expect("socket should close");
        }
    });
    (
        Url::parse(&format!("http://{address}/retry.bin")).expect("fixture URL"),
        task,
    )
}

async fn resume_fixture_server(response_start: usize) -> (Url, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("fixture server should bind");
    let address = listener.local_addr().expect("fixture address");
    let task = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.expect("request should arrive");
            let mut request = vec![0_u8; 4096];
            let count = socket
                .read(&mut request)
                .await
                .expect("request should read");
            let request = String::from_utf8_lossy(&request[..count]);

            let response = if request.contains("Range: bytes=0-0")
                || request.contains("range: bytes=0-0")
            {
                let headers = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: 1\r\nContent-Range: bytes 0-0/{}\r\nAccept-Ranges: bytes\r\nETag: fixture-v1\r\nConnection: close\r\n\r\n",
                    FIXTURE.len()
                );
                [headers.as_bytes(), &FIXTURE[..1]].concat()
            } else {
                assert!(
                    request.contains(&format!("Range: bytes={RESUME_OFFSET}-"))
                        || request.contains(&format!("range: bytes={RESUME_OFFSET}-"))
                );
                let body = &FIXTURE[response_start..];
                let headers = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nAccept-Ranges: bytes\r\nETag: fixture-v1\r\nConnection: close\r\n\r\n",
                    body.len(),
                    response_start,
                    FIXTURE.len() - 1,
                    FIXTURE.len()
                );
                [headers.as_bytes(), body].concat()
            };

            socket
                .write_all(&response)
                .await
                .expect("response should write");
            socket.shutdown().await.expect("socket should close");
        }
    });

    (
        Url::parse(&format!("http://{address}/fixture.bin")).expect("fixture URL"),
        task,
    )
}

async fn short_unknown_length_resume_server() -> (Url, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("fixture server should bind");
    let address = listener.local_addr().expect("fixture address");
    let task = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.expect("request should arrive");
            let mut request = vec![0_u8; 4096];
            let count = socket
                .read(&mut request)
                .await
                .expect("request should read");
            let request = String::from_utf8_lossy(&request[..count]);

            let response = if request.contains("Range: bytes=0-0")
                || request.contains("range: bytes=0-0")
            {
                let headers = "HTTP/1.1 206 Partial Content\r\nContent-Length: 1\r\nContent-Range: bytes 0-0/*\r\nAccept-Ranges: bytes\r\nETag: fixture-v1\r\nConnection: close\r\n\r\n";
                [headers.as_bytes(), &FIXTURE[..1]].concat()
            } else {
                assert!(
                    request.contains(&format!("Range: bytes={RESUME_OFFSET}-"))
                        || request.contains(&format!("range: bytes={RESUME_OFFSET}-"))
                );
                let body = &FIXTURE[RESUME_OFFSET..FIXTURE.len() - 4];
                let headers = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/*\r\nAccept-Ranges: bytes\r\nETag: fixture-v1\r\nConnection: close\r\n\r\n",
                    body.len(),
                    RESUME_OFFSET,
                    FIXTURE.len() - 1
                );
                [headers.as_bytes(), body].concat()
            };

            socket
                .write_all(&response)
                .await
                .expect("response should write");
            socket.shutdown().await.expect("socket should close");
        }
    });

    (
        Url::parse(&format!("http://{address}/fixture.bin")).expect("fixture URL"),
        task,
    )
}

async fn stalled_fixture_server() -> (
    Url,
    tokio::task::JoinHandle<()>,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("fixture server should bind");
    let address = listener.local_addr().expect("fixture address");
    let (started_tx, started_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut started_tx = Some(started_tx);
        let mut release_rx = Some(release_rx);
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.expect("request should arrive");
            let mut request = vec![0_u8; 4096];
            let count = socket
                .read(&mut request)
                .await
                .expect("request should read");
            let request = String::from_utf8_lossy(&request[..count]);

            if request.contains("Range: bytes=0-0") || request.contains("range: bytes=0-0") {
                let headers = "HTTP/1.1 206 Partial Content\r\nContent-Length: 1\r\nContent-Range: bytes 0-0/100\r\nAccept-Ranges: bytes\r\nETag: fixture-v1\r\nConnection: close\r\n\r\n";
                socket
                    .write_all(&[headers.as_bytes(), b"x"].concat())
                    .await
                    .expect("probe response should write");
                socket.shutdown().await.expect("probe socket should close");
            } else {
                let headers = "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nETag: fixture-v1\r\nConnection: close\r\n\r\n";
                socket
                    .write_all(&[headers.as_bytes(), b"abc"].concat())
                    .await
                    .expect("partial response should write");
                socket.flush().await.expect("partial response should flush");
                let _ = started_tx.take().expect("start signal").send(());
                let _ = release_rx.take().expect("release signal").await;
                let _ = socket.shutdown().await;
            }
        }
    });

    (
        Url::parse(&format!("http://{address}/fixture.bin")).expect("fixture URL"),
        task,
        started_rx,
        release_tx,
    )
}

async fn write_resume_files(directory: &std::path::Path, url: &Url) {
    tokio::fs::write(
        directory.join("fixture.bin.quiver-part"),
        &FIXTURE[..RESUME_OFFSET],
    )
    .await
    .expect("partial file should write");
    let state = serde_json::json!({
        "url": url.as_str(),
        "total_bytes": FIXTURE.len(),
        "etag": "fixture-v1",
        "last_modified": null
    });
    tokio::fs::write(
        directory.join("fixture.bin.quiver.json"),
        serde_json::to_vec_pretty(&state).expect("state should serialize"),
    )
    .await
    .expect("state file should write");
}

async fn write_unknown_length_resume_files(directory: &std::path::Path, url: &Url) {
    tokio::fs::write(
        directory.join("fixture.bin.quiver-part"),
        &FIXTURE[..RESUME_OFFSET],
    )
    .await
    .expect("partial file should write");
    let state = serde_json::json!({
        "url": url.as_str(),
        "total_bytes": null,
        "etag": "fixture-v1",
        "last_modified": null
    });
    tokio::fs::write(
        directory.join("fixture.bin.quiver.json"),
        serde_json::to_vec_pretty(&state).expect("state should serialize"),
    )
    .await
    .expect("state file should write");
}
