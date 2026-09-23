use std::{fs, os::unix::fs::PermissionsExt, sync::Arc, time::Duration};

use axum::{
    body::{Body, to_bytes},
    extract::State,
    http::{Request, StatusCode},
};
use tokio::net::UnixListener;

use crate::{
    command::BODY_LIMIT,
    command_client_tests::{fake_core, response},
    command_tests::{receipt, request},
    http::{HttpState, handle},
    http_tests::state,
};

fn session_token(state: &HttpState) -> String {
    let code = state.sessions.issue_code().unwrap();
    state.sessions.exchange(&code.code).unwrap().token
}

fn authenticated_request(state: &HttpState, token: &str) -> axum::http::request::Builder {
    Request::builder()
        .method("POST")
        .uri("/api/commands/amend_draft")
        .header("host", &state.host)
        .header("origin", &state.origin)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .header("idempotency-key", "synthetic-key")
}

#[tokio::test]
async fn rejected_mutations_never_connect_to_core() {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let socket = dir.path().join("core.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    let mut state = Arc::try_unwrap(state()).ok().unwrap();
    state.core.socket = socket;
    let state = Arc::new(state);
    let token = session_token(&state);
    let body = serde_json::to_vec(&request()).unwrap();
    for (name, value, status) in [
        ("host", None, StatusCode::FORBIDDEN),
        ("host", Some("evil.invalid"), StatusCode::FORBIDDEN),
        ("origin", None, StatusCode::FORBIDDEN),
        ("origin", Some("null"), StatusCode::FORBIDDEN),
        (
            "origin",
            Some("https://evil.invalid"),
            StatusCode::FORBIDDEN,
        ),
        ("authorization", None, StatusCode::UNAUTHORIZED),
        (
            "authorization",
            Some("Bearer invalid"),
            StatusCode::UNAUTHORIZED,
        ),
        ("content-type", None, StatusCode::BAD_REQUEST),
        ("content-type", Some("text/plain"), StatusCode::BAD_REQUEST),
        ("idempotency-key", None, StatusCode::BAD_REQUEST),
        ("idempotency-key", Some("   "), StatusCode::BAD_REQUEST),
    ] {
        let mut request = authenticated_request(&state, &token)
            .body(Body::from(body.clone()))
            .unwrap();
        request.headers_mut().remove(name);
        if let Some(value) = value {
            request.headers_mut().insert(name, value.parse().unwrap());
        }
        assert_eq!(
            handle(State(state.clone()), request).await.status(),
            status,
            "{name}"
        );
    }
    for name in [
        "host",
        "origin",
        "authorization",
        "content-type",
        "idempotency-key",
    ] {
        let mut request = authenticated_request(&state, &token)
            .body(Body::from(body.clone()))
            .unwrap();
        let value = request.headers()[name].clone();
        request.headers_mut().append(name, value);
        assert!(
            !handle(State(state.clone()), request)
                .await
                .status()
                .is_success()
        );
    }
    for (method, path) in [
        ("POST", "/api/commands/resume_task"),
        ("POST", "/v1/commands/amend_draft"),
        ("GET", "/api/commands/amend_draft"),
        ("PUT", "/api/commands/amend_draft"),
        ("POST", "/api/commands/amend_draft?actor=owner"),
        ("POST", "/api/commands/amend_draft/"),
    ] {
        let request = authenticated_request(&state, &token)
            .method(method)
            .uri(path)
            .body(Body::from(body.clone()))
            .unwrap();
        assert_eq!(
            handle(State(state.clone()), request).await.status(),
            StatusCode::NOT_FOUND
        );
    }
    for bytes in [b"not json".to_vec(),b"{}".to_vec(),serde_json::to_vec(&serde_json::json!({
        "project_id":crate::command_tests::PROJECT,"expected_revision":1,
        "payload":{"task_id":crate::command_tests::TASK,"expected_task_revision":1,"patch":{"lifecycle":"ready"}}
    })).unwrap()] {
        let request = authenticated_request(&state, &token).body(Body::from(bytes)).unwrap();
        assert_eq!(handle(State(state.clone()),request).await.status(),StatusCode::BAD_REQUEST);
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn command_request_and_idempotency_limits_are_enforced_before_core() {
    let state = state();
    let token = session_token(&state);
    for (key, body, status) in [
        (
            "x".repeat(128),
            serde_json::to_vec(&request()).unwrap(),
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        (
            "x".repeat(129),
            serde_json::to_vec(&request()).unwrap(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "key".into(),
            vec![b'x'; BODY_LIMIT + 1],
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
    ] {
        let mut request = authenticated_request(&state, &token)
            .body(Body::from(body))
            .unwrap();
        request
            .headers_mut()
            .insert("idempotency-key", key.parse().unwrap());
        assert_eq!(handle(State(state.clone()), request).await.status(), status);
    }
    let request = authenticated_request(&state, &token)
        .header("content-length", BODY_LIMIT + 1)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        handle(State(state), request).await.status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[tokio::test]
async fn command_forwards_only_validated_body_and_key_not_browser_authority_or_other_headers() {
    let body = serde_json::to_vec(&receipt("applied")).unwrap();
    let (_dir, client, server) = fake_core(response(200, &body), Duration::ZERO).await;
    let mut state = Arc::try_unwrap(state()).ok().unwrap();
    state.core = client;
    let state = Arc::new(state);
    let token = session_token(&state);
    let mut bytes = serde_json::to_vec(&request()).unwrap();
    bytes.resize(BODY_LIMIT, b' ');
    let request = authenticated_request(&state, &token)
        .header("cookie", "private-browser-cookie")
        .header("x-actor", "private-browser-actor")
        .header("x-provider-key", "private-provider-key")
        .body(Body::from(bytes.clone()))
        .unwrap();
    let response = handle(State(state), request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(
        to_bytes(response.into_body(), 64 * 1024).await.unwrap(),
        body
    );
    let forwarded = server.await.unwrap();
    let text = std::str::from_utf8(&forwarded).unwrap();
    assert!(!text.contains("private-") && !text.contains("Bearer") && !text.contains("origin:"));
    assert!(forwarded.ends_with(&bytes));
}

#[tokio::test]
async fn exhausted_capacity_rejects_command_before_upstream() {
    let state = state();
    let token = session_token(&state);
    let _permit = state.capacity.acquire_many(8).await.unwrap();
    let request = authenticated_request(&state, &token)
        .body(Body::from(serde_json::to_vec(&request()).unwrap()))
        .unwrap();
    assert_eq!(
        handle(State(state.clone()), request).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn create_employee_is_an_exact_owner_command_with_employee_receipt() {
    let body = serde_json::to_vec(&serde_json::json!({
        "project_id":crate::command_tests::PROJECT,
        "expected_revision":3,
        "payload":{"name":"New teammate","role":"reviewer","stage_eligibility":{"mode":"any"}}
    }))
    .unwrap();
    let employee_receipt = serde_json::to_vec(&serde_json::json!({
        "command_id":"01988000-0000-7000-8000-000000000003",
        "status":"applied","project_revision":4,
        "event_ids":["01988000-0000-7000-8000-000000000004"],
        "resource":{"kind":"employee","id":"01988000-0000-7000-8000-000000000005"}
    }))
    .unwrap();
    let (_dir, client, server) = fake_core(response(200, &employee_receipt), Duration::ZERO).await;
    let mut state = Arc::try_unwrap(state()).ok().unwrap();
    state.core = client;
    let state = Arc::new(state);
    let token = session_token(&state);
    for (method, uri, status) in [
        (
            "GET",
            "/api/commands/create_employee",
            StatusCode::NOT_FOUND,
        ),
        (
            "POST",
            "/api/commands/create_employee/",
            StatusCode::NOT_FOUND,
        ),
        (
            "POST",
            "/api/commands/create_employee?actor=owner",
            StatusCode::NOT_FOUND,
        ),
    ] {
        let request = authenticated_request(&state, &token)
            .method(method)
            .uri(uri)
            .body(Body::from(body.clone()))
            .unwrap();
        assert_eq!(handle(State(state.clone()), request).await.status(), status);
    }
    let mut unauthorized = authenticated_request(&state, &token)
        .uri("/api/commands/create_employee")
        .body(Body::from(body.clone()))
        .unwrap();
    unauthorized.headers_mut().remove("authorization");
    assert_eq!(
        handle(State(state.clone()), unauthorized).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let request = authenticated_request(&state, &token)
        .uri("/api/commands/create_employee")
        .body(Body::from(body.clone()))
        .unwrap();
    let response = handle(State(state), request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), 64 * 1024).await.unwrap(),
        employee_receipt
    );
    let forwarded = server.await.unwrap();
    assert!(
        std::str::from_utf8(&forwarded)
            .unwrap()
            .starts_with("POST /v1/commands/create_employee HTTP/1.1")
    );
    assert!(forwarded.ends_with(&body));
}
