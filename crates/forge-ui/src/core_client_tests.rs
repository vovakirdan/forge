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
    core_client::CoreClient,
    http::ApiError,
    read_target::{CORE_BODY_LIMIT, DETAIL_BODY_LIMIT, ReadTarget},
};

fn health() -> ReadTarget {
    ReadTarget::parse("/api/health", None).unwrap()
}

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
    assert_eq!(client.read(&health()).await.unwrap(), body.as_slice());
    let request = String::from_utf8(server.await.unwrap()).unwrap();
    assert!(request.starts_with("GET /v1/health HTTP/1.1\r\n"));
    assert!(!request.to_lowercase().contains("authorization"));
    assert!(!request.to_lowercase().contains("actor"));
}

#[tokio::test]
async fn upstream_redirect_and_malformed_json_are_rejected() {
    let cases = [
        b"HTTP/1.1 302 Found\r\nLocation: http://evil.invalid\r\nContent-Length: 0\r\n\r\n"
            .to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 4\r\n\r\noops"
            .to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
    ];
    for response in cases {
        let (_dir, client, server) = fake_core(response, Duration::ZERO).await;
        assert!(matches!(
            client.read(&health()).await,
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
        client.read(&health()).await,
        Err(ApiError::Unavailable)
    ));
    assert!(!server.is_finished());
    server.abort();
}

#[tokio::test]
async fn unauthorized_or_cross_origin_browser_never_connects_to_existing_core_socket() {
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
    let code = state.sessions.issue_code().unwrap();
    let session = state.sessions.exchange(&code.code).unwrap();
    for path in [
        "/api/health",
        "/api/projects/01988000-0000-7000-8000-000000000001/tasks?limit=20",
        "/api/projects/01988000-0000-7000-8000-000000000001/tasks/01988000-0000-7000-8000-000000000002",
        "/api/projects/01988000-0000-7000-8000-000000000001/pipelines?limit=20",
        "/api/projects/01988000-0000-7000-8000-000000000001/pipelines/01988000-0000-7000-8000-000000000002",
        "/api/projects/01988000-0000-7000-8000-000000000001/runs?limit=20",
        "/api/projects/01988000-0000-7000-8000-000000000001/runs/01988000-0000-7000-8000-000000000002",
    ] {
        for (host, origin, token, expected) in [
            (
                "127.0.0.1:12345",
                "http://127.0.0.1:12345",
                None,
                axum::http::StatusCode::UNAUTHORIZED,
            ),
            (
                "127.0.0.1:12345",
                "http://127.0.0.1:12345",
                Some("invalid"),
                axum::http::StatusCode::UNAUTHORIZED,
            ),
            (
                "evil.invalid",
                "http://127.0.0.1:12345",
                Some(session.token.as_str()),
                axum::http::StatusCode::FORBIDDEN,
            ),
            (
                "127.0.0.1:12345",
                "https://evil.invalid",
                Some(session.token.as_str()),
                axum::http::StatusCode::FORBIDDEN,
            ),
        ] {
            let mut request = axum::http::Request::builder()
                .uri(path)
                .header("host", host)
                .header("origin", origin);
            if let Some(token) = token {
                request = request.header("authorization", format!("Bearer {token}"));
            }
            let response = super::http::handle(
                axum::extract::State(state.clone()),
                request.body(axum::body::Body::empty()).unwrap(),
            )
            .await;
            assert_eq!(response.status(), expected);
        }
    }
    tokio::task::yield_now().await;
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn stalled_upstream_is_cancelled_by_total_deadline() {
    let (_dir, client, server) = fake_core(Vec::new(), Duration::from_secs(60)).await;
    let start = tokio::time::Instant::now();
    assert!(matches!(
        client.read(&health()).await,
        Err(ApiError::Timeout)
    ));
    assert!(start.elapsed() < Duration::from_secs(7));
    server.abort();
}

fn response(status: u16, body: &[u8]) -> Vec<u8> {
    [
        format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes(),
        body.to_vec(),
    ].concat()
}

fn scoped_targets() -> [ReadTarget; 6] {
    let project = "01988000-0000-7000-8000-000000000001";
    let item = "01988000-0000-7000-8000-000000000002";
    [
        ReadTarget::parse(&format!("/api/projects/{project}/tasks"), None).unwrap(),
        ReadTarget::parse(&format!("/api/projects/{project}/tasks/{item}"), None).unwrap(),
        ReadTarget::parse(&format!("/api/projects/{project}/pipelines"), None).unwrap(),
        ReadTarget::parse(&format!("/api/projects/{project}/pipelines/{item}"), None).unwrap(),
        ReadTarget::parse(&format!("/api/projects/{project}/runs"), None).unwrap(),
        ReadTarget::parse(&format!("/api/projects/{project}/runs/{item}"), None).unwrap(),
    ]
}

#[tokio::test]
async fn each_route_accepts_its_exact_body_bound() {
    for target in [health()].into_iter().chain(scoped_targets()) {
        let body = format!("{{\"value\":\"{}\"}}", "x".repeat(target.body_limit - 12));
        assert_eq!(body.len(), target.body_limit);
        let (_dir, client, server) =
            fake_core(response(200, body.as_bytes()), Duration::ZERO).await;
        assert_eq!(client.read(&target).await.unwrap().len(), target.body_limit);
        server.await.unwrap();
    }
}

#[tokio::test]
async fn declared_and_chunked_oversize_fail_with_explicit_error_even_for_error_statuses() {
    for target in [health()].into_iter().chain(scoped_targets()) {
        assert!(matches!(
            target.body_limit,
            CORE_BODY_LIMIT | DETAIL_BODY_LIMIT
        ));
        for status in [200, 404, 409, 503] {
            // Header-only response proves rejection does not require reading the declared body.
            let declared = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", target.body_limit + 1).into_bytes();
            let chunk = vec![b'x'; target.body_limit];
            let chunked = [
                format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n", chunk.len()).into_bytes(),
                chunk,
                b"\r\n1\r\nx\r\n0\r\n\r\n".to_vec(),
            ].concat();
            for response in [declared, chunked] {
                let (_dir, client, server) = fake_core(response, Duration::ZERO).await;
                assert!(matches!(
                    client.read(&target).await,
                    Err(ApiError::ResponseTooLarge)
                ));
                server.await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn cursor_conflict_mapping_is_exact_and_only_for_paginated_lists() {
    for target in scoped_targets() {
        for body in [
            br#"{"error":{"code":"cursor_invalid","message":"private Core details"}}"#.as_slice(),
            br#"{"error":{"code":"other_conflict"}}"#.as_slice(),
            br#"{"code":"cursor_invalid"}"#.as_slice(),
            b"not json".as_slice(),
        ] {
            let (_dir, client, server) = fake_core(response(409, body), Duration::ZERO).await;
            let result = client.read(&target).await;
            if target.cursor_conflict && body.starts_with(br#"{"error":{"code":"cursor_invalid""#) {
                assert!(matches!(result, Err(ApiError::CursorInvalid)));
            } else {
                assert!(matches!(result, Err(ApiError::BadGateway)));
            }
            server.await.unwrap();
        }
    }
}

async fn browser_read(path: &str, upstream: Vec<u8>) -> (axum::response::Response, Vec<u8>) {
    let (_dir, client, server) = fake_core(upstream, Duration::ZERO).await;
    let mut state = Arc::try_unwrap(super::http_tests::state()).ok().unwrap();
    state.core = client;
    let code = state.sessions.issue_code().unwrap();
    let session = state.sessions.exchange(&code.code).unwrap();
    let response = super::http::handle(
        axum::extract::State(Arc::new(state)),
        axum::http::Request::builder()
            .uri(path)
            .header("host", "127.0.0.1:12345")
            .header("origin", "http://127.0.0.1:12345")
            .header("authorization", format!("Bearer {}", session.token))
            .header("x-forge-actor", "untrusted-browser")
            .header("cookie", "untrusted=browser")
            .body(axum::body::Body::empty())
            .unwrap(),
    )
    .await;
    (response, server.await.unwrap())
}

#[tokio::test]
async fn browser_scoped_reads_forward_only_scoped_path_and_safe_query() {
    for project in [
        "01988000-0000-7000-8000-000000000001",
        "01988000-0000-7000-8000-000000000003",
    ] {
        for suffix in [
            "tasks?cursor=a%26actor%3Downer%2Bb&limit=2",
            "tasks/01988000-0000-7000-8000-000000000002",
            "pipelines?cursor=a%26actor%3Downer%2Bb&limit=2",
            "pipelines/01988000-0000-7000-8000-000000000002",
            "runs?cursor=a%26actor%3Downer%2Bb&limit=2",
            "runs/01988000-0000-7000-8000-000000000002",
        ] {
            let path = format!("/api/projects/{project}/{suffix}");
            let (result, request) = browser_read(&path, response(200, b"{}")).await;
            assert_eq!(result.status(), axum::http::StatusCode::OK);
            let request = String::from_utf8(request).unwrap();
            let expected_suffix = if let Some((resource, _)) = suffix.split_once('?') {
                format!("{resource}?limit=2&cursor=a%26actor%3Downer%2Bb")
            } else {
                suffix.to_owned()
            };
            assert_eq!(
                request,
                format!(
                    "GET /v1/projects/{project}/{expected_suffix} HTTP/1.1\r\nhost: localhost\r\naccept: application/json\r\n\r\n"
                )
            );
        }
    }
}

#[tokio::test]
async fn browser_preserves_scoped_not_found_and_sanitizes_cursor_and_size_errors() {
    for path in [
        "/api/projects/01988000-0000-7000-8000-000000000001/tasks?limit=20",
        "/api/projects/01988000-0000-7000-8000-000000000001/runs?limit=20",
        "/api/projects/01988000-0000-7000-8000-000000000001/pipelines?limit=20",
    ] {
        let (base_path, query) = path.split_once('?').unwrap();
        let bound = ReadTarget::parse(base_path, Some(query))
            .unwrap()
            .body_limit;
        for (upstream, status, expected) in [
            (
                response(404, br#"{"error":"private missing-task details"}"#),
                axum::http::StatusCode::NOT_FOUND,
                serde_json::json!({"error":"not found"}),
            ),
            (
                response(
                    409,
                    br#"{"error":{"code":"cursor_invalid","message":"private cursor"}}"#,
                ),
                axum::http::StatusCode::CONFLICT,
                serde_json::json!({"error":"pagination cursor is no longer valid","code":"cursor_invalid"}),
            ),
            (
                response(
                    409,
                    br#"{"error":{"code":"unknown","message":"private error"}}"#,
                ),
                axum::http::StatusCode::BAD_GATEWAY,
                serde_json::json!({"error":"invalid Core response"}),
            ),
            (
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", bound + 1).into_bytes(),
                axum::http::StatusCode::BAD_GATEWAY,
                serde_json::json!({"error":"response exceeds interface limit","code":"response_too_large"}),
            ),
        ] {
            let (result, _) = browser_read(path, upstream).await;
            assert_eq!(result.status(), status);
            let bytes = axum::body::to_bytes(result.into_body(), 1024)
                .await
                .unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
                expected
            );
        }
    }
}

#[tokio::test]
async fn pipeline_list_accepts_more_than_summary_bound_without_relaxing_other_lists() {
    let body = serde_json::to_vec(&serde_json::json!({
        "items": [{"instructions": "x".repeat(CORE_BODY_LIMIT)}]
    }))
    .unwrap();
    assert!(body.len() > CORE_BODY_LIMIT && body.len() < DETAIL_BODY_LIMIT);
    for (resource, expected) in [
        ("pipelines", axum::http::StatusCode::OK),
        ("tasks", axum::http::StatusCode::BAD_GATEWAY),
        ("runs", axum::http::StatusCode::BAD_GATEWAY),
    ] {
        // Transport-only JSON; the browser/Core fixture separately validates actual definitions.
        let path =
            format!("/api/projects/01988000-0000-7000-8000-000000000001/{resource}?limit=20");
        let (result, _) = browser_read(&path, response(200, &body)).await;
        assert_eq!(result.status(), expected, "{resource}");
        let bytes = axum::body::to_bytes(result.into_body(), DETAIL_BODY_LIMIT)
            .await
            .unwrap();
        if resource == "pipelines" {
            assert_eq!(bytes.as_ref(), body.as_slice());
        } else {
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["code"],
                "response_too_large"
            );
        }
    }
}
