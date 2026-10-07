//! Automatic browser transport. All queue writes stay inside the desktop process,
//! so MSIX needs neither a native-host registration nor shared AppData files.
use std::{path::PathBuf, sync::Arc, time::Duration};

use axum::{
    Router,
    body::to_bytes,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use subtle::ConstantTimeEq;
use tokio::{
    net::TcpListener,
    sync::{Mutex, Semaphore},
};

use crate::{HostResponse, load_config, process_firefox_message};

pub const PORT: u16 = 47831;
const BODY_LIMIT: usize = 64 * 1024;
const QUEUE_LIMIT: usize = 1000;

struct Server {
    config_path: PathBuf,
    authority: String,
    // Serializes queue-limit checks and bounds active body readers/writers.
    request: Arc<Semaphore>,
    session: Mutex<Session>,
    rate: Mutex<(std::time::Instant, u32)>,
}

struct Session {
    token: String,
    created: std::time::Instant,
}

impl Session {
    fn new() -> Self {
        Self {
            token: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
            created: std::time::Instant::now(),
        }
    }
    fn expired(&self) -> bool {
        self.created.elapsed() >= Duration::from_secs(3600)
    }
}

pub fn router(config_path: PathBuf, port: u16) -> Router {
    Router::new()
        .route("/v1/session", post(session))
        .route("/v1/message", post(message))
        .with_state(Arc::new(Server {
            config_path,
            authority: format!("127.0.0.1:{port}"),
            request: Arc::new(Semaphore::new(1)),
            session: Mutex::new(Session::new()),
            rate: Mutex::new((std::time::Instant::now(), 0)),
        }))
}

pub async fn bind() -> std::io::Result<TcpListener> {
    TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, PORT)).await
}

pub async fn serve(listener: TcpListener, config_path: PathBuf) -> std::io::Result<()> {
    let port = listener.local_addr()?.port();
    axum::serve(listener, router(config_path, port)).await
}

fn allowed_headers(headers: &HeaderMap, authority: &str) -> bool {
    if headers.get("host").and_then(|v| v.to_str().ok()) != Some(authority)
        || headers
            .get("x-quiverdl-connector")
            .and_then(|v| v.to_str().ok())
            != Some("1")
        || headers.get("content-type").and_then(|v| v.to_str().ok()) != Some("application/json")
    {
        return false;
    }
    // Host permissions let the extension fetch without CORS. Never grant CORS
    // to websites, and reject their origins even if a token has been supplied.
    headers.get("origin").is_none_or(|origin| {
        origin.to_str().ok().is_some_and(|origin| {
            url::Url::parse(origin).ok().is_some_and(|url| {
                url.scheme() == "moz-extension"
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.port().is_none()
                    && matches!(url.path(), "" | "/")
                    && url.query().is_none()
                    && url.fragment().is_none()
            })
        })
    })
}

fn failure(status: StatusCode) -> Response {
    (status, [("Cache-Control", "no-store")], "Request rejected").into_response()
}

async fn session(State(server): State<Arc<Server>>, request: Request) -> Response {
    if !allowed_headers(request.headers(), &server.authority) {
        return failure(StatusCode::FORBIDDEN);
    }
    let Ok(_permit) = server.request.try_acquire() else {
        return failure(StatusCode::SERVICE_UNAVAILABLE);
    };
    let Ok(Ok(bytes)) =
        tokio::time::timeout(Duration::from_secs(3), to_bytes(request.into_body(), 64)).await
    else {
        return failure(StatusCode::PAYLOAD_TOO_LARGE);
    };
    if bytes.as_ref() != b"{}" {
        return failure(StatusCode::BAD_REQUEST);
    }
    // Browser request guards protect against websites, not other extensions or
    // local programs. This credential manages a session; it is not addon identity.
    let mut session = server.session.lock().await;
    if session.expired() {
        *session = Session::new();
    }
    (
        [
            ("Content-Type", "application/json"),
            ("Cache-Control", "no-store"),
        ],
        serde_json::json!({"protocol": "quiverdl", "version": 1, "token": session.token})
            .to_string(),
    )
        .into_response()
}

async fn message(State(server): State<Arc<Server>>, request: Request) -> Response {
    if !allowed_headers(request.headers(), &server.authority) {
        return failure(StatusCode::FORBIDDEN);
    }
    let Some(token) = request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| value.len() == 64)
        .map(str::to_owned)
    else {
        return failure(StatusCode::UNAUTHORIZED);
    };
    {
        let session = server.session.lock().await;
        if session.expired() || token.as_bytes().ct_eq(session.token.as_bytes()).unwrap_u8() != 1 {
            return failure(StatusCode::UNAUTHORIZED);
        }
    }
    {
        let mut rate = server.rate.lock().await;
        if rate.0.elapsed() >= Duration::from_secs(60) {
            *rate = (std::time::Instant::now(), 0);
        }
        if rate.1 >= 120 {
            return failure(StatusCode::TOO_MANY_REQUESTS);
        }
        rate.1 += 1;
    }
    let Ok(permit) = server.request.clone().try_acquire_owned() else {
        return failure(StatusCode::SERVICE_UNAVAILABLE);
    };
    let path = server.config_path.clone();
    let config = tokio::task::spawn_blocking(move || {
        let config = load_config(&path)?;
        config.validate(&path)?;
        Ok::<_, std::io::Error>(config)
    })
    .await;
    let Ok(Ok(config)) = config else {
        return failure(StatusCode::SERVICE_UNAVAILABLE);
    };
    let Ok(Ok(bytes)) = tokio::time::timeout(
        Duration::from_secs(3),
        to_bytes(request.into_body(), BODY_LIMIT),
    )
    .await
    else {
        return failure(StatusCode::PAYLOAD_TOO_LARGE);
    };
    let response = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        // Failed requests never cancel the browser's copy of a download.
        let queue_full = match std::fs::read_dir(&config.inbox_dir) {
            Ok(entries) => entries.take(QUEUE_LIMIT).count() >= QUEUE_LIMIT,
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        };
        if queue_full {
            return HostResponse {
                ok: false,
                request_id: None,
                error: Some("Browser queue is full".into()),
            };
        }
        process_firefox_message(&config, &bytes)
    })
    .await;
    match response {
        Ok(response) => (
            [
                ("Content-Type", "application/json"),
                ("Cache-Control", "no-store"),
            ],
            serde_json::to_string(&response).expect("bridge response serializes"),
        )
            .into_response(),
        Err(_) => failure(StatusCode::SERVICE_UNAVAILABLE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn automatic_session_queueing_and_restart() {
        let root = tempfile::tempdir().unwrap();
        let config_path = root.path().join("native-bridge.json");
        let inbox = root.path().join("inbox");
        let token = "ab".repeat(32);
        let write_config = |token: &str| {
            std::fs::write(
                &config_path,
                serde_json::to_vec(&serde_json::json!({
                    "token": token, "inbox_dir": inbox,
                }))
                .unwrap(),
            )
            .unwrap();
        };
        write_config(&token);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        assert!(address.ip().is_loopback());
        let server = tokio::spawn(serve(listener, config_path.clone()));
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let endpoint = format!("http://{address}/v1/message");
        let request = || {
            client
                .post(&endpoint)
                .header("Content-Type", "application/json")
                .header("X-QuiverDL-Connector", "1")
        };
        let bootstrap = || {
            client
                .post(format!("http://{address}/v1/session"))
                .header("Content-Type", "application/json")
                .header("X-QuiverDL-Connector", "1")
                .body("{}")
        };
        for origin in ["https://evil.test", "null"] {
            assert_eq!(
                bootstrap()
                    .header("Origin", origin)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            client
                .post(format!("http://{address}/v1/session"))
                .header("Content-Type", "application/json")
                .body("{}")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        let boot = bootstrap().send().await.unwrap();
        assert!(!boot.headers().contains_key("access-control-allow-origin"));
        let boot: serde_json::Value = serde_json::from_str(&boot.text().await.unwrap()).unwrap();
        let token = boot["token"].as_str().unwrap().to_owned();
        assert_eq!(token.len(), 64);
        let ping = r#"{"version":1,"action":"ping"}"#;
        assert_eq!(
            request().body(ping).send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request()
                .bearer_auth("cd".repeat(32))
                .body(ping)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        for origin in ["https://evil.test", "null", "http://127.0.0.1"] {
            assert_eq!(
                request()
                    .bearer_auth(&token)
                    .header("Origin", origin)
                    .body(ping)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            request()
                .bearer_auth(&token)
                .header("Host", "evil.test")
                .body(ping)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        let response = request()
            .bearer_auth(&token)
            .header("Origin", "moz-extension://fixture")
            .body(ping)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            !response
                .headers()
                .contains_key("access-control-allow-origin")
        );
        assert!(!inbox.exists());
        assert_eq!(
            request()
                .bearer_auth(&token)
                .body("x".repeat(BODY_LIMIT + 1))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        for url in ["file:///C:/test", "javascript:alert(1)"] {
            let response = request()
                .bearer_auth(&token)
                .body(
                    serde_json::json!({
                        "version": 1, "action": "enqueue", "url": url
                    })
                    .to_string(),
                )
                .send()
                .await
                .unwrap();
            let body: serde_json::Value =
                serde_json::from_str(&response.text().await.unwrap()).unwrap();
            assert_eq!(body["ok"], false);
        }
        assert!(!inbox.exists());
        let response = request()
            .bearer_auth(&token)
            .body(
                serde_json::json!({
                    "version": 1, "action": "enqueue", "url": "https://example.test/file",
                    "automatic": true, "suggestedFilename": "../file.zip"
                })
                .to_string(),
            )
            .send()
            .await
            .unwrap();
        let body: serde_json::Value =
            serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_eq!(body["ok"], true);
        let id = body["requestId"].as_str().unwrap();
        let saved = std::fs::read_to_string(inbox.join(format!("{id}.json"))).unwrap();
        assert!(!saved.contains(&token));
        let saved: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(saved["automatic"], true);
        assert_eq!(saved["url"], "https://example.test/file");
        // Session credentials are independent of the private native config.
        write_config(&"cd".repeat(32));
        assert_eq!(
            request()
                .bearer_auth(&token)
                .body(ping)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        // Exhaust the per-minute limit without mutating the inbox.
        let mut limited = false;
        for _ in 0..121 {
            if request()
                .bearer_auth(&token)
                .body(ping)
                .send()
                .await
                .unwrap()
                .status()
                == StatusCode::TOO_MANY_REQUESTS
            {
                limited = true;
                break;
            }
        }
        assert!(limited);
        let preflight = client
            .request(reqwest::Method::OPTIONS, &endpoint)
            .header("Origin", "https://evil.test")
            .header("Access-Control-Request-Method", "POST")
            .send()
            .await
            .unwrap();
        assert!(!preflight.status().is_success());
        assert!(
            !preflight
                .headers()
                .contains_key("access-control-allow-origin")
        );
        server.abort();
        let _ = server.await;
        let listener = TcpListener::bind(address).await.unwrap();
        let restarted = tokio::spawn(serve(listener, config_path));
        assert_eq!(
            request()
                .bearer_auth(&token)
                .body(ping)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let boot: serde_json::Value =
            serde_json::from_str(&bootstrap().send().await.unwrap().text().await.unwrap()).unwrap();
        let refreshed = boot["token"].as_str().unwrap();
        assert_ne!(token, refreshed);
        assert_eq!(
            request()
                .bearer_auth(refreshed)
                .body(ping)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        restarted.abort();
    }
}
