use std::{fs, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    extract::State,
    http::{Request, StatusCode},
};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::{
    assets::AssetSnapshot,
    auth::SessionStore,
    core_client::CoreClient,
    http::{HttpState, handle},
};

pub(crate) fn state() -> Arc<HttpState> {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("index.html"), "<html></html>").unwrap();
    fs::write(dir.path().join("forge-live-manifest.json"), serde_json::to_vec(&serde_json::json!({
        "format":"forge-live-v1","files":[{"path":"index.html","sha256":format!("{:x}",Sha256::digest(b"<html></html>"))}]
    })).unwrap()).unwrap();
    Arc::new(HttpState {
        host: "127.0.0.1:12345".into(),
        origin: "http://127.0.0.1:12345".into(),
        sessions: Arc::new(SessionStore::default()),
        core: CoreClient {
            socket: dir.path().join("absent-core.sock"),
        },
        assets: AssetSnapshot::load(dir.path()).unwrap(),
        capacity: Semaphore::new(8),
    })
}

fn request(method: &str, path: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:12345")
}

#[tokio::test]
async fn guards_reject_before_absent_core_connection() {
    let state = state();
    let code = state.sessions.issue_code().unwrap();
    let session = state.sessions.exchange(&code.code).unwrap();
    for (host, origin, token, expected) in [
        (
            "evil.invalid",
            None,
            Some(session.token.as_str()),
            StatusCode::FORBIDDEN,
        ),
        (
            "localhost:12345",
            None,
            Some(session.token.as_str()),
            StatusCode::FORBIDDEN,
        ),
        (
            "127.0.0.1:12345",
            Some("null"),
            Some(session.token.as_str()),
            StatusCode::FORBIDDEN,
        ),
        (
            "127.0.0.1:12345",
            Some("https://evil.invalid"),
            Some(session.token.as_str()),
            StatusCode::FORBIDDEN,
        ),
        ("127.0.0.1:12345", None, None, StatusCode::UNAUTHORIZED),
        (
            "127.0.0.1:12345",
            None,
            Some("invalid"),
            StatusCode::UNAUTHORIZED,
        ),
    ] {
        let mut request = Request::builder().uri("/api/health").header("host", host);
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = handle(State(state.clone()), request.body(Body::empty()).unwrap()).await;
        assert_eq!(response.status(), expected);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(
            !response
                .headers()
                .contains_key("access-control-allow-origin")
        );
    }
    let response = handle(
        State(state.clone()),
        request("GET", "/api/health")
            .header("authorization", format!("Bearer {}", session.token))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn exchange_rejects_cross_origin_wrong_content_type_unknown_fields_and_oversize() {
    let state = state();
    for (origin, content_type, body, expected) in [
        (None, "application/json", "{}".into(), StatusCode::FORBIDDEN),
        (
            Some("null"),
            "application/json",
            "{}".into(),
            StatusCode::FORBIDDEN,
        ),
        (
            Some(state.origin.as_str()),
            "text/plain",
            "{}".into(),
            StatusCode::BAD_REQUEST,
        ),
        (
            Some(state.origin.as_str()),
            "application/json",
            "{\"code\":\"x\",\"extra\":1}".into(),
            StatusCode::BAD_REQUEST,
        ),
        (
            Some(state.origin.as_str()),
            "application/json",
            "x".repeat(1025),
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
    ] {
        let mut request =
            request("POST", "/api/auth/exchange").header("content-type", content_type);
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        let response = handle(
            State(state.clone()),
            request.body(Body::from(body)).unwrap(),
        )
        .await;
        assert_eq!(response.status(), expected);
    }
}

#[tokio::test]
async fn exchange_logout_and_copied_token_revocation_follow_http_contract() {
    let state = state();
    let code = state.sessions.issue_code().unwrap();
    let body = serde_json::to_vec(&serde_json::json!({"code":code.code})).unwrap();
    let exchange = || {
        request("POST", "/api/auth/exchange")
            .header("origin", &state.origin)
            .header("content-type", "application/json")
            .body(Body::from(body.clone()))
            .unwrap()
    };
    let response = handle(State(state.clone()), exchange()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
    let session: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let token = session["token"].as_str().unwrap();
    assert_eq!(
        handle(State(state.clone()), exchange()).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let logout = || {
        request("POST", "/api/auth/logout")
            .header("origin", &state.origin)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        handle(State(state.clone()), logout()).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        handle(State(state.clone()), logout()).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(state.sessions.authorize(token).is_err());
}

#[tokio::test]
async fn static_and_route_allowlists_are_closed_and_csp_has_no_inline_escape() {
    let state = state();
    let response = handle(
        State(state.clone()),
        request("GET", "/").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let csp = response.headers()["content-security-policy"]
        .to_str()
        .unwrap();
    assert!(csp.contains("script-src 'self'"));
    assert!(!csp.contains("unsafe-"));
    for (method, path) in [
        ("GET", "/api"),
        ("GET", "/api/projects/bad-id"),
        ("GET", "/v1/health"),
        ("GET", "/_serverFn"),
        ("GET", "/api/projects/../../health"),
        ("GET", "/%2e%2e/index.html"),
        ("POST", "/api/projects/00000000-0000-0000-0000-000000000000"),
        ("OPTIONS", "/api/auth/exchange"),
        ("GET", "/index.html?token=never-accepted"),
        ("GET", "/missing.js"),
    ] {
        assert_eq!(
            handle(
                State(state.clone()),
                request(method, path).body(Body::empty()).unwrap()
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
    }
}

#[tokio::test]
async fn api_capacity_is_bounded_without_waiting_and_duplicate_host_is_rejected() {
    let state = state();
    let _permits = state.capacity.acquire_many(8).await.unwrap();
    assert_eq!(
        handle(
            State(state.clone()),
            request("GET", "/api/health").body(Body::empty()).unwrap()
        )
        .await
        .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        handle(
            State(state.clone()),
            request("GET", "/")
                .header("host", "evil.invalid")
                .body(Body::empty())
                .unwrap()
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn task_routes_reject_invalid_queries_ids_methods_and_bodies_before_core() {
    let state = state();
    let code = state.sessions.issue_code().unwrap();
    let session = state.sessions.exchange(&code.code).unwrap();
    let project = "01988000-0000-7000-8000-000000000001";
    for (method, suffix, expected) in [
        ("GET", "tasks?limit=0", StatusCode::BAD_REQUEST),
        ("GET", "priority-scheme?", StatusCode::NOT_FOUND),
        ("GET", "priority-scheme?limit=1", StatusCode::NOT_FOUND),
        ("GET", "priority-scheme?actor=owner", StatusCode::NOT_FOUND),
        ("GET", "priority-scheme/normal", StatusCode::NOT_FOUND),
        ("POST", "priority-scheme", StatusCode::NOT_FOUND),
        ("DELETE", "priority-scheme", StatusCode::NOT_FOUND),
        ("OPTIONS", "priority-scheme", StatusCode::NOT_FOUND),
        ("GET", "tasks?limit=1&limit=2", StatusCode::BAD_REQUEST),
        ("GET", "tasks?cursor=a&cursor=b", StatusCode::BAD_REQUEST),
        ("GET", "tasks?actor=owner", StatusCode::BAD_REQUEST),
        ("GET", "tasks?cursor=%", StatusCode::BAD_REQUEST),
        ("GET", "tasks?cursor=%FF", StatusCode::BAD_REQUEST),
        (
            "GET",
            "tasks/00000000-0000-0000-0000-000000000000",
            StatusCode::NOT_FOUND,
        ),
        (
            "GET",
            "pipelines/00000000-0000-0000-0000-000000000000",
            StatusCode::NOT_FOUND,
        ),
        (
            "GET",
            "pipelines/01988000-0000-7000-8000-000000000002?limit=20",
            StatusCode::NOT_FOUND,
        ),
        (
            "GET",
            "tasks/01988000-0000-7000-8000-000000000002?cursor=a",
            StatusCode::NOT_FOUND,
        ),
        ("POST", "tasks", StatusCode::NOT_FOUND),
        (
            "DELETE",
            "tasks/01988000-0000-7000-8000-000000000002",
            StatusCode::NOT_FOUND,
        ),
        ("OPTIONS", "tasks", StatusCode::NOT_FOUND),
    ] {
        let response = handle(
            State(state.clone()),
            request(method, &format!("/api/projects/{project}/{suffix}"))
                .header("authorization", format!("Bearer {}", session.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), expected, "{method} {suffix}");
    }
    for suffix in ["tasks?limit=20", "priority-scheme", "cancellation-reasons"] {
        for (header, value) in [("content-length", "1"), ("transfer-encoding", "chunked")] {
            let response = handle(
                State(state.clone()),
                request("GET", &format!("/api/projects/{project}/{suffix}"))
                    .header("authorization", format!("Bearer {}", session.token))
                    .header(header, value)
                    .body(Body::from("x"))
                    .unwrap(),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
    }
}

#[tokio::test]
async fn employee_profile_routes_reject_mutation_and_unbounded_queries_before_core() {
    let state = state();
    let code = state.sessions.issue_code().unwrap();
    let session = state.sessions.exchange(&code.code).unwrap();
    let project = "01988000-0000-7000-8000-000000000001";
    let employee = "01988000-0000-7000-8000-000000000002";
    for (method, suffix, expected) in [
        (
            "GET",
            format!("employees/{employee}?limit=1"),
            StatusCode::NOT_FOUND,
        ),
        (
            "GET",
            format!("employees/{employee}/runs?limit=0"),
            StatusCode::BAD_REQUEST,
        ),
        (
            "GET",
            format!("employees/{employee}/runs?employee_id=x"),
            StatusCode::BAD_REQUEST,
        ),
        (
            "GET",
            format!("employees/{employee}/runs/all"),
            StatusCode::NOT_FOUND,
        ),
        (
            "POST",
            format!("employees/{employee}"),
            StatusCode::NOT_FOUND,
        ),
        (
            "DELETE",
            format!("employees/{employee}"),
            StatusCode::NOT_FOUND,
        ),
        (
            "OPTIONS",
            format!("employees/{employee}/runs"),
            StatusCode::NOT_FOUND,
        ),
    ] {
        let response = handle(
            State(state.clone()),
            request(method, &format!("/api/projects/{project}/{suffix}"))
                .header("authorization", format!("Bearer {}", session.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), expected, "{method} {suffix}");
    }
}

#[tokio::test]
async fn run_routes_reject_unscoped_reads_commands_queries_and_body_before_core() {
    let state = state();
    let code = state.sessions.issue_code().unwrap();
    let session = state.sessions.exchange(&code.code).unwrap();
    let project = "01988000-0000-7000-8000-000000000001";
    let run = "01988000-0000-7000-8000-000000000002";
    for (method, suffix, expected) in [
        ("GET", "runs?limit=101".into(), StatusCode::BAD_REQUEST),
        (
            "GET",
            "runs?cursor=a&cursor=b".into(),
            StatusCode::BAD_REQUEST,
        ),
        ("GET", "runs?task_id=x".into(), StatusCode::BAD_REQUEST),
        ("GET", "runs?purpose=hook".into(), StatusCode::BAD_REQUEST),
        ("GET", "runs?cursor=%FF".into(), StatusCode::BAD_REQUEST),
        (
            "GET",
            "runs/00000000-0000-0000-0000-000000000000".into(),
            StatusCode::NOT_FOUND,
        ),
        ("GET", format!("runs/{run}?limit=20"), StatusCode::NOT_FOUND),
        ("GET", format!("runs/{run}/evidence"), StatusCode::NOT_FOUND),
        ("GET", format!("runs/{run}/context"), StatusCode::NOT_FOUND),
        ("POST", format!("runs/{run}/stop"), StatusCode::NOT_FOUND),
        ("POST", "runs".into(), StatusCode::NOT_FOUND),
        ("DELETE", format!("runs/{run}"), StatusCode::NOT_FOUND),
        ("OPTIONS", "runs".into(), StatusCode::NOT_FOUND),
    ] {
        let response = handle(
            State(state.clone()),
            request(method, &format!("/api/projects/{project}/{suffix}"))
                .header("authorization", format!("Bearer {}", session.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), expected, "{method} {suffix}");
    }
    for suffix in ["runs?limit=20".into(), format!("runs/{run}")] {
        for (header, value) in [("content-length", "1"), ("transfer-encoding", "chunked")] {
            let response = handle(
                State(state.clone()),
                request("GET", &format!("/api/projects/{project}/{suffix}"))
                    .header("authorization", format!("Bearer {}", session.token))
                    .header(header, value)
                    .body(Body::from("x"))
                    .unwrap(),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
    }
}

#[tokio::test]
async fn pipeline_list_does_not_open_catalog_mutations_or_unbounded_queries() {
    let state = state();
    let code = state.sessions.issue_code().unwrap();
    let session = state.sessions.exchange(&code.code).unwrap();
    let project = "01988000-0000-7000-8000-000000000001";
    let version = "01988000-0000-7000-8000-000000000002";
    for (method, suffix, expected) in [
        ("GET", "pipelines?limit=101".into(), StatusCode::BAD_REQUEST),
        (
            "GET",
            "pipelines?limit=1&limit=2".into(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "GET",
            "pipelines?cursor=a&cursor=b".into(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "GET",
            "pipelines?cursor=%FF".into(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "GET",
            "pipelines?deleted=false".into(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "GET",
            "pipelines?actor=owner".into(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "GET",
            format!("pipelines/{version}?limit=20"),
            StatusCode::NOT_FOUND,
        ),
        (
            "GET",
            format!("pipelines/{version}/versions"),
            StatusCode::NOT_FOUND,
        ),
        ("POST", "pipelines".into(), StatusCode::NOT_FOUND),
        (
            "POST",
            format!("pipelines/{version}/publish"),
            StatusCode::NOT_FOUND,
        ),
        (
            "DELETE",
            format!("pipelines/{version}"),
            StatusCode::NOT_FOUND,
        ),
        ("OPTIONS", "pipelines".into(), StatusCode::NOT_FOUND),
    ] {
        let response = handle(
            State(state.clone()),
            request(method, &format!("/api/projects/{project}/{suffix}"))
                .header("authorization", format!("Bearer {}", session.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), expected, "{method} {suffix}");
    }
    for suffix in ["pipelines?limit=20".into(), format!("pipelines/{version}")] {
        for (header, value) in [("content-length", "1"), ("transfer-encoding", "chunked")] {
            let response = handle(
                State(state.clone()),
                request("GET", &format!("/api/projects/{project}/{suffix}"))
                    .header("authorization", format!("Bearer {}", session.token))
                    .header(header, value)
                    .body(Body::from("x"))
                    .unwrap(),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
    }
}
