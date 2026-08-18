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
