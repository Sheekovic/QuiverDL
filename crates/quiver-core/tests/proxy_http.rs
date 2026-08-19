use quiver_core::{
    DownloadControl, DownloadEngine, DownloadRequest, ProgressEvent, ProxyConfig, ProxyPolicy,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};
use url::Url;

const FIXTURE: &[u8] = b"downloaded through proxy";
const EXPECTED_AUTH: &str = "Proxy-Authorization: Basic cHJveHktdXNlcjpwcm94eS1zZWNyZXQ=";

#[tokio::test]
async fn downloads_through_an_authenticated_http_proxy() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("proxy fixture should bind");
    let proxy_address = listener.local_addr().expect("proxy fixture address");
    let proxy_server = tokio::spawn(async move {
        for request_number in 0..2 {
            let (mut socket, _) = listener
                .accept()
                .await
                .expect("proxy request should arrive");
            let request = read_request_headers(&mut socket).await;
            assert!(
                request.starts_with("GET http://downloads.example.invalid/fixture.bin HTTP/1.1")
            );
            assert!(
                request
                    .lines()
                    .any(|line| line.eq_ignore_ascii_case(EXPECTED_AUTH)),
                "proxy authorization header should be present"
            );

            let response = if request_number == 0 {
                let headers = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: 1\r\nContent-Range: bytes 0-0/{}\r\nAccept-Ranges: bytes\r\nETag: proxy-v1\r\nConnection: close\r\n\r\n",
                    FIXTURE.len()
                );
                [headers.as_bytes(), &FIXTURE[..1]].concat()
            } else {
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nETag: proxy-v1\r\nConnection: close\r\n\r\n",
                    FIXTURE.len()
                );
                [headers.as_bytes(), FIXTURE].concat()
            };
            socket
                .write_all(&response)
                .await
                .expect("proxy response should write");
            socket.shutdown().await.expect("proxy socket should close");
        }
    });

    let proxy = ProxyConfig::new(
        Url::parse(&format!("http://{proxy_address}")).expect("proxy URL should parse"),
    )
    .expect("proxy endpoint should be valid")
    .with_basic_auth("proxy-user", "proxy-secret")
    .expect("proxy credentials should be valid");
    let engine = DownloadEngine::new_with_proxy(ProxyPolicy::Custom(proxy))
        .expect("proxied engine should initialize");
    let directory = tempfile::tempdir().expect("temporary directory");
    let destination = directory.path().join("fixture.bin");
    let (progress_tx, progress_rx) = mpsc::channel::<ProgressEvent>(16);
    drop(progress_rx);

    let result = engine
        .download(
            DownloadRequest::new(
                Url::parse("http://downloads.example.invalid/fixture.bin")
                    .expect("download URL should parse"),
                &destination,
            ),
            DownloadControl::new(),
            progress_tx,
        )
        .await
        .expect("proxied download should succeed");

    assert_eq!(result.bytes_written, FIXTURE.len() as u64);
    assert_eq!(
        tokio::fs::read(destination)
            .await
            .expect("download should be promoted"),
        FIXTURE
    );
    proxy_server.await.expect("proxy fixture should finish");
}

async fn read_request_headers(socket: &mut tokio::net::TcpStream) -> String {
    const MAX_REQUEST_HEADERS: usize = 8 * 1024;
    let mut request = Vec::with_capacity(1024);
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        assert!(
            request.len() < MAX_REQUEST_HEADERS,
            "proxy request headers exceeded the fixture limit"
        );
        let remaining = MAX_REQUEST_HEADERS - request.len();
        let mut chunk = [0_u8; 1024];
        let chunk_limit = remaining.min(chunk.len());
        let count = socket
            .read(&mut chunk[..chunk_limit])
            .await
            .expect("proxy request should read");
        assert!(count > 0, "proxy request ended before its headers");
        request.extend_from_slice(&chunk[..count]);
    }
    String::from_utf8(request).expect("proxy request headers should be UTF-8")
}
