use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
};

use crate::{
    core_client::{CORE_BODY_LIMIT, CoreClient},
    http::ApiError,
};

async fn fake_core(
    response: Vec<u8>,
    delay: Duration,
) -> (
    tempfile::TempDir,
    CoreClient,
    tokio::task::JoinHandle<Vec<u8>>,
) {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let path = dir.path().join("core.sock");
    let listener = UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let byte = socket.read_u8().await.unwrap();
            request.push(byte);
            if request.ends_with(b"\r\n\r\n") {
                break;
            }
            assert!(request.len() < 4096);
        }
        tokio::time::sleep(delay).await;
        let _ = socket.write_all(&response).await;
        request
    });
    (dir, CoreClient { socket: path }, task)
}

#[tokio::test]
async fn upstream_request_is_fixed_and_never_carries_browser_authority() {
    let body = b"{\"status\":\"ready\",\"api_version\":\"v1\"}";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    let (_dir, client, server) =
        fake_core([response, body.to_vec()].concat(), Duration::ZERO).await;
    assert_eq!(client.read("/v1/health").await.unwrap(), body.as_slice());
    let request = String::from_utf8(server.await.unwrap()).unwrap();
    assert!(request.starts_with("GET /v1/health HTTP/1.1\r\n"));
    assert!(!request.to_lowercase().contains("authorization"));
    assert!(!request.to_lowercase().contains("actor"));
}

#[tokio::test]
async fn upstream_chunked_and_declared_oversize_redirect_and_malformed_json_are_rejected() {
    let large = vec![b'x'; CORE_BODY_LIMIT + 1];
    let cases = [
        [format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",large.len()).into_bytes(),large.clone()].concat(),
        [format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",large.len()).into_bytes(),large,b"\r\n0\r\n\r\n".to_vec()].concat(),
        b"HTTP/1.1 302 Found\r\nLocation: http://evil.invalid\r\nContent-Length: 0\r\n\r\n".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 4\r\n\r\noops".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
    ];
    for response in cases {
        let (_dir, client, server) = fake_core(response, Duration::ZERO).await;
        assert!(matches!(
            client.read("/v1/health").await,
            Err(ApiError::BadGateway)
        ));
        let _ = server.await.unwrap();
    }
}

#[tokio::test]
async fn unsafe_socket_modes_are_rejected_before_connection() {
    let (_dir, client, server) = fake_core(Vec::new(), Duration::ZERO).await;
    fs::set_permissions(&client.socket, fs::Permissions::from_mode(0o666)).unwrap();
    assert!(matches!(
        client.read("/v1/health").await,
        Err(ApiError::Unavailable)
    ));
    assert!(!server.is_finished());
    server.abort();
}

#[tokio::test]
async fn unauthorized_browser_does_not_connect_to_existing_core_socket() {
    let hits = Arc::new(AtomicUsize::new(0));
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let path = dir.path().join("core.sock");
    let listener = UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let observed = hits.clone();
    let server = tokio::spawn(async move {
        loop {
            if listener.accept().await.is_ok() {
                observed.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
    let mut state = Arc::try_unwrap(super::http_tests::state()).ok().unwrap();
    state.core.socket = path;
    let state = Arc::new(state);
    let response = super::http::handle(
        axum::extract::State(state),
        axum::http::Request::builder()
            .uri("/api/health")
            .header("host", "127.0.0.1:12345")
            .body(axum::body::Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    tokio::task::yield_now().await;
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn stalled_upstream_is_cancelled_by_total_deadline() {
    let (_dir, client, server) = fake_core(Vec::new(), Duration::from_secs(60)).await;
    let start = tokio::time::Instant::now();
    assert!(matches!(
        client.read("/v1/health").await,
        Err(ApiError::Timeout)
    ));
    assert!(start.elapsed() < Duration::from_secs(7));
    server.abort();
}
