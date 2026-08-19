use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};
use url::Url;

use quiver_core::{
    DownloadControl, DownloadEngine, DownloadRequest, DownloadStatus, ProgressEvent,
};

const FIXTURE: &[u8] = b"QuiverDL end-to-end transfer fixture";
const RESUME_OFFSET: usize = 9;

#[tokio::test]
async fn downloads_verifies_and_promotes_a_file() {
    let (url, server) = fixture_server(2).await;
    let directory = tempfile::tempdir().expect("temporary directory");
    let destination = directory.path().join("fixture.bin");
    let expected_hash: [u8; 32] = Sha256::digest(FIXTURE).into();
    let mut request = DownloadRequest::new(url, &destination);
    request.expected_sha256 = Some(expected_hash);
    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<ProgressEvent>();

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
    let (progress_tx, _progress_rx) = mpsc::unbounded_channel::<ProgressEvent>();

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
    let (progress_tx, _progress_rx) = mpsc::unbounded_channel::<ProgressEvent>();

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
