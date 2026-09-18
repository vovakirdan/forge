use std::{sync::Arc, time::Duration};

use axum::{
    body::{Body, to_bytes},
    extract::State,
    http::{Request, StatusCode},
};
use serde_json::json;

use crate::{
    command::BODY_LIMIT,
    command_client_tests::{fake_core, response},
    command_tests::{priority_request, receipt},
    http::handle,
    http_tests::state,
};

#[tokio::test]
async fn priority_post_preserves_body_and_key_and_filters_browser_authority() {
    for status in ["applied", "replayed"] {
        let body = serde_json::to_vec(&receipt(status)).unwrap();
        let (_dir, client, server) = fake_core(response(200, &body), Duration::ZERO).await;
        let mut state = Arc::try_unwrap(state()).ok().unwrap();
        state.core = client;
        let code = state.sessions.issue_code().unwrap();
        let token = state.sessions.exchange(&code.code).unwrap().token;
        let mut bytes = serde_json::to_vec(&priority_request()).unwrap();
        bytes.resize(BODY_LIMIT, b' ');
        let request = Request::builder()
            .method("POST")
            .uri("/api/commands/set_task_priority")
            .header("host", &state.host)
            .header("origin", &state.origin)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .header("idempotency-key", "priority-key")
            .header("x-actor", "private-browser-authority")
            .body(Body::from(bytes.clone()))
            .unwrap();
        let received = handle(State(Arc::new(state)), request).await;
        assert_eq!(received.status(), StatusCode::OK);
        assert_eq!(received.headers()["cache-control"], "no-store");
        assert_eq!(
            to_bytes(received.into_body(), 64 * 1024).await.unwrap(),
            body
        );
        let forwarded = server.await.unwrap();
        let text = std::str::from_utf8(&forwarded).unwrap();
        assert!(text.starts_with("POST /v1/commands/set_task_priority HTTP/1.1\r\n"));
        assert!(text.contains("idempotency-key: priority-key\r\n"));
        assert!(
            !text.contains("private-browser")
                && !text.contains("Bearer")
                && !text.contains("origin:")
        );
        assert!(forwarded.ends_with(&bytes));
    }
}

#[tokio::test]
async fn priority_guards_reject_unsafe_requests_before_upstream() {
    let state = state();
    let code = state.sessions.issue_code().unwrap();
    let token = state.sessions.exchange(&code.code).unwrap().token;
    for (case, status) in [
        ("host", 403),
        ("origin", 403),
        ("authorization", 401),
        ("content-type", 400),
        ("idempotency-key", 400),
        ("query", 404),
        ("method", 404),
        ("oversize", 413),
        ("malformed", 400),
        ("key-too-long", 400),
    ] {
        let body = match case {
            "oversize" => vec![b' '; BODY_LIMIT + 1],
            "malformed" => b"{}".to_vec(),
            _ => serde_json::to_vec(&priority_request()).unwrap(),
        };
        let mut request = Request::builder()
            .method(if case == "method" { "PUT" } else { "POST" })
            .uri(if case == "query" {
                "/api/commands/set_task_priority?actor=owner"
            } else {
                "/api/commands/set_task_priority"
            })
            .header("host", &state.host)
            .header("origin", &state.origin)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .header("idempotency-key", "priority-key")
            .body(Body::from(body))
            .unwrap();
        if case == "key-too-long" {
            request
                .headers_mut()
                .insert("idempotency-key", "x".repeat(129).parse().unwrap());
        } else {
            request.headers_mut().remove(case);
        }
        assert_eq!(
            handle(State(state.clone()), request)
                .await
                .status()
                .as_u16(),
            status,
            "{case}"
        );
    }
}

#[tokio::test]
async fn priority_transport_uses_same_typed_refusal_and_ambiguous_failure_contract() {
    for (status, code, expected) in [
        (400, "invalid_request", 400),
        (409, "stale_revision", 409),
        (409, "idempotency_conflict", 409),
        (422, "validation_failed", 422),
        (404, "not_found", 404),
        (403, "forbidden", 403),
        (503, "unavailable", 503),
        (409, "invalid_request", 502),
        (500, "internal", 502),
    ] {
        let body = serde_json::to_vec(&json!({"error":{"code":code,"message":"private-upstream"}}))
            .unwrap();
        let (_dir, client, server) = fake_core(response(status, &body), Duration::ZERO).await;
        let mut state = Arc::try_unwrap(state()).ok().unwrap();
        state.core = client;
        let code = state.sessions.issue_code().unwrap();
        let token = state.sessions.exchange(&code.code).unwrap().token;
        let request = Request::builder()
            .method("POST")
            .uri("/api/commands/set_task_priority")
            .header("host", &state.host)
            .header("origin", &state.origin)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .header("idempotency-key", "priority-key")
            .body(Body::from(serde_json::to_vec(&priority_request()).unwrap()))
            .unwrap();
        let received = handle(State(Arc::new(state)), request).await;
        assert_eq!(received.status().as_u16(), expected);
        assert!(
            !std::str::from_utf8(&to_bytes(received.into_body(), 1024).await.unwrap())
                .unwrap()
                .contains("private-upstream")
        );
        server.await.unwrap();
    }
}
