use crate::{
    command_tests::{PROJECT, TASK, receipt},
    create_task::CreateTask,
};
use serde_json::{Value, json};

pub(crate) fn create_request() -> Value {
    json!({"project_id":PROJECT,"expected_revision":3,"payload":{
        "title":"New draft", "description":"", "kind":"delivery",
        "pipeline_version_id":TASK, "priority":"normal", "properties":{}
    }})
}

#[test]
fn create_accepts_only_supported_draft_shape_and_canonical_text_bounds() {
    for kind in ["delivery", "analysis"] {
        let mut value = create_request();
        value["payload"]["kind"] = json!(kind);
        value["payload"]["title"] = json!("🚀".repeat(240));
        value["payload"]["description"] = json!("🚀".repeat(50_000));
        value["payload"]["definition_of_done"] = json!("🚀".repeat(20_000));
        assert!(CreateTask::parse(&serde_json::to_vec(&value).unwrap()).is_ok());
    }
    assert!(CreateTask::parse(&serde_json::to_vec(&create_request()).unwrap()).is_ok());
    let mut typed = create_request();
    typed["payload"]["properties"] = json!({"risk":{"type":"text","value":"high"}});
    assert!(CreateTask::parse(&serde_json::to_vec(&typed).unwrap()).is_ok());
    for (pointer, replacement) in [
        ("/actor", json!("human")),
        ("/expected_revision", json!(0)),
        ("/expected_revision", json!(9_007_199_254_740_991_u64)),
        ("/project_id", json!("invalid")),
        ("/payload/task_id", json!(TASK)),
        ("/payload/pipeline_id", json!(TASK)),
        ("/payload/pipeline_version_id", json!("invalid")),
        ("/payload/title", json!(" ")),
        ("/payload/title", json!("x".repeat(241))),
        ("/payload/description", json!("x".repeat(50_001))),
        ("/payload/description", Value::Null),
        ("/payload/definition_of_done", json!(" ")),
        ("/payload/definition_of_done", json!("x".repeat(20_001))),
        ("/payload/kind", json!("bug")),
        ("/payload/priority", json!("HIGH")),
        ("/payload/properties", json!({"risk":{"type":"text"}})),
        ("/payload/properties", Value::Null),
        ("/payload/properties", json!([])),
    ] {
        let mut value = create_request();
        let (parent, field) = pointer.rsplit_once('/').unwrap();
        value.pointer_mut(parent).unwrap()[field] = replacement;
        assert!(
            CreateTask::parse(&serde_json::to_vec(&value).unwrap()).is_err(),
            "{pointer}"
        );
    }
    let duplicate = serde_json::to_string(&create_request()).unwrap().replacen(
        "\"title\":\"New draft\"",
        "\"title\":\"New draft\",\"title\":\"Other\"",
        1,
    );
    assert!(CreateTask::parse(duplicate.as_bytes()).is_err());
    let mut positional_payload = create_request();
    positional_payload["payload"] = json!(["New draft", "", null, "delivery", TASK, "normal", {}]);
    assert!(CreateTask::parse(&serde_json::to_vec(&positional_payload).unwrap()).is_err());
    assert!(
        CreateTask::parse(
            &serde_json::to_vec(&json!([PROJECT, 3, create_request()["payload"]])).unwrap()
        )
        .is_err()
    );
}

#[test]
fn create_receipt_validates_generated_uuid_instead_of_existing_task_match() {
    let command = CreateTask::parse(&serde_json::to_vec(&create_request()).unwrap()).unwrap();
    for status in ["applied", "replayed"] {
        let mut value = receipt(status);
        value["resource"]["id"] = json!(uuid::Uuid::now_v7());
        assert!(
            command
                .validate_receipt(&serde_json::to_vec(&value).unwrap())
                .is_ok()
        );
    }
    for (pointer, replacement) in [
        ("/resource/id", json!("bad")),
        ("/resource/kind", json!("project")),
        ("/status", json!("pending")),
        ("/project_revision", json!(5)),
        ("/event_ids", json!([])),
    ] {
        let mut value = receipt("applied");
        *value.pointer_mut(pointer).unwrap() = replacement;
        assert!(
            command
                .validate_receipt(&serde_json::to_vec(&value).unwrap())
                .is_err()
        );
    }
}

#[tokio::test]
async fn create_route_forwards_only_validated_body_key_and_fixed_path() {
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
    let bytes = serde_json::to_vec(&create_request()).unwrap();
    let request = Request::builder()
        .method("POST")
        .uri("/api/commands/create_task")
        .header("host", &state.host)
        .header("origin", &state.origin)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .header("idempotency-key", "create-key")
        .header("x-actor", "private-browser")
        .body(Body::from(bytes.clone()))
        .unwrap();
    let result = handle(State(Arc::new(state)), request).await;
    assert_eq!(result.status(), 200);
    assert_eq!(to_bytes(result.into_body(), 64 * 1024).await.unwrap(), body);
    let forwarded = server.await.unwrap();
    let text = std::str::from_utf8(&forwarded).unwrap();
    assert!(text.starts_with("POST /v1/commands/create_task HTTP/1.1\r\n"));
    assert!(text.contains("idempotency-key: create-key\r\n"));
    assert!(
        !text.contains("private-browser") && !text.contains("Bearer") && !text.contains("origin:")
    );
    assert!(forwarded.ends_with(&bytes));
}

#[tokio::test]
async fn create_route_requires_session_origin_exact_path_and_bounded_input() {
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
        ("method", 404),
        ("query", 404),
        ("malformed", 400),
    ] {
        let body = match case {
            "oversize" => vec![b' '; BODY_LIMIT + 1],
            "malformed" => b"{}".to_vec(),
            _ => serde_json::to_vec(&create_request()).unwrap(),
        };
        let mut request = Request::builder()
            .method(if case == "method" { "PUT" } else { "POST" })
            .uri(if case == "query" {
                "/api/commands/create_task?actor=human"
            } else {
                "/api/commands/create_task"
            })
            .header("host", &state.host)
            .header("origin", &state.origin)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .header("idempotency-key", "create-key")
            .body(Body::from(body))
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
