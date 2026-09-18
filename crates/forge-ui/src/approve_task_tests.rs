use crate::{
    approve_task::ApproveTask,
    command::AmendDraft,
    command_tests::{PROJECT, TASK, receipt, request},
};
use serde_json::{Value, json};

pub(crate) fn approval() -> Value {
    json!({"project_id":PROJECT,"expected_revision":3,"payload":{"task_id":TASK,"expected_task_revision":1}})
}

#[test]
fn amend_dod_accepts_clear_and_text_but_rejects_blank_oversize_wrong_type() {
    for dod in [Value::Null, json!("Acceptance"), json!("🚀".repeat(20_000))] {
        let mut value = request();
        value["payload"]["patch"] = json!({"definition_of_done":dod});
        assert!(AmendDraft::parse(&serde_json::to_vec(&value).unwrap()).is_ok());
    }
    for dod in [
        json!(""),
        json!(" "),
        json!("🚀".repeat(20_001)),
        json!(42),
        json!({}),
    ] {
        let mut value = request();
        value["payload"]["patch"] = json!({"definition_of_done":dod});
        assert!(AmendDraft::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    let mut positional = request();
    positional["payload"]["patch"] = json!([null, "Title", "Description"]);
    assert!(AmendDraft::parse(&serde_json::to_vec(&positional).unwrap()).is_err());
}

#[test]
fn approval_only_accepts_exact_identity_revision_intent_and_correlated_receipt() {
    let command = ApproveTask::parse(&serde_json::to_vec(&approval()).unwrap()).unwrap();
    for status in ["applied", "replayed"] {
        assert!(
            command
                .validate_receipt(&serde_json::to_vec(&receipt(status)).unwrap())
                .is_ok()
        );
    }
    for (pointer, replacement) in [
        ("/actor", json!("human")),
        ("/project_id", json!("invalid")),
        ("/expected_revision", json!(0)),
        ("/expected_revision", json!(9_007_199_254_740_991_u64)),
        ("/payload/task_id", json!("invalid")),
        ("/payload/expected_task_revision", json!(0)),
        ("/payload/lifecycle", json!("ready")),
        ("/payload/expected_task_revision", json!(1.5)),
    ] {
        let mut value = approval();
        let (parent, field) = pointer.rsplit_once('/').unwrap();
        value.pointer_mut(parent).unwrap()[field] = replacement;
        assert!(
            ApproveTask::parse(&serde_json::to_vec(&value).unwrap()).is_err(),
            "{pointer}"
        );
    }
    let mut wrong = receipt("applied");
    wrong["resource"]["id"] = json!(PROJECT);
    assert!(
        command
            .validate_receipt(&serde_json::to_vec(&wrong).unwrap())
            .is_err()
    );
    assert!(
        ApproveTask::parse(
            &serde_json::to_vec(&json!([PROJECT,3,{"task_id":TASK,"expected_task_revision":1}]))
                .unwrap()
        )
        .is_err()
    );
}

#[tokio::test]
async fn approval_route_forwards_intent_and_key_without_browser_authority() {
    use crate::{
        command_client_tests::{fake_core, response},
        http::handle,
        http_tests::state,
    };
    use axum::{
        body::{Body, to_bytes},
        extract::State,
        http::Request,
    };
    use std::{sync::Arc, time::Duration};
    let body = serde_json::to_vec(&receipt("applied")).unwrap();
    let (_dir, client, server) = fake_core(response(200, &body), Duration::ZERO).await;
    let mut state = Arc::try_unwrap(state()).ok().unwrap();
    state.core = client;
    let code = state.sessions.issue_code().unwrap();
    let token = state.sessions.exchange(&code.code).unwrap().token;
    let bytes = serde_json::to_vec(&approval()).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri("/api/commands/approve_task")
        .header("host", &state.host)
        .header("origin", &state.origin)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .header("idempotency-key", "approve-key")
        .header("x-actor", "private-browser")
        .body(Body::from(bytes.clone()))
        .unwrap();
    let result = handle(State(Arc::new(state)), request).await;
    assert_eq!(result.status(), 200);
    assert_eq!(to_bytes(result.into_body(), 64 * 1024).await.unwrap(), body);
    let forwarded = server.await.unwrap();
    let text = std::str::from_utf8(&forwarded).unwrap();
    assert!(text.starts_with("POST /v1/commands/approve_task HTTP/1.1\r\n"));
    assert!(text.contains("idempotency-key: approve-key\r\n"));
    assert!(
        !text.contains("private-browser") && !text.contains("Bearer") && !text.contains("origin:")
    );
    assert!(forwarded.ends_with(&bytes));
}

#[tokio::test]
async fn approval_guards_block_unsafe_requests_before_core() {
    use crate::{command::BODY_LIMIT, http::handle, http_tests::state};
    use axum::{body::Body, extract::State, http::Request};
    let state = state();
    let code = state.sessions.issue_code().unwrap();
    let token = state.sessions.exchange(&code.code).unwrap().token;
    for (case, status) in [
        ("host", 403),
        ("origin", 403),
        ("authorization", 401),
        ("content-type", 400),
        ("idempotency-key", 400),
        ("oversize", 413),
        ("query", 404),
        ("method", 404),
    ] {
        let bytes = if case == "oversize" {
            vec![b' '; BODY_LIMIT + 1]
        } else {
            serde_json::to_vec(&approval()).unwrap()
        };
        let mut request = Request::builder()
            .method(if case == "method" { "PUT" } else { "POST" })
            .uri(if case == "query" {
                "/api/commands/approve_task?actor=human"
            } else {
                "/api/commands/approve_task"
            })
            .header("host", &state.host)
            .header("origin", &state.origin)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .header("idempotency-key", "approve-key")
            .body(Body::from(bytes))
            .unwrap();
        request.headers_mut().remove(case);
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
