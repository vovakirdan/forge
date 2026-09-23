use serde_json::json;

use crate::{command::CommandTarget, dependency_command::DependencyCommand};

const PROJECT: &str = "0195105a-73f4-75dd-8e90-84a2d438cfe1";
const BLOCKER: &str = "0195105a-73f4-75dd-8e90-84a2d438cfe2";
const BLOCKED: &str = "0195105a-73f4-75dd-8e90-84a2d438cfe3";

fn request(condition: Option<&str>) -> Vec<u8> {
    let mut payload = json!({"blocker_task_id": BLOCKER, "blocked_task_id": BLOCKED});
    if let Some(condition) = condition {
        payload["required_condition"] = json!(condition);
    }
    json!({"project_id": PROJECT, "expected_revision": 3, "payload": payload})
        .to_string()
        .into_bytes()
}

#[test]
fn dependency_commands_require_exact_payloads_and_distinct_tasks() {
    assert!(
        DependencyCommand::parse(&request(Some("task_done")), CommandTarget::CreateDependency)
            .is_ok()
    );
    assert!(DependencyCommand::parse(&request(None), CommandTarget::RemoveDependency).is_ok());
    assert!(DependencyCommand::parse(&request(None), CommandTarget::CreateDependency).is_err());
    assert!(
        DependencyCommand::parse(&request(Some("task_done")), CommandTarget::RemoveDependency)
            .is_err()
    );
    let self_link = json!({"project_id": PROJECT, "expected_revision": 3, "payload": {
        "blocker_task_id": BLOCKER, "blocked_task_id": BLOCKER, "required_condition": "task_done"
    }});
    assert!(
        DependencyCommand::parse(
            self_link.to_string().as_bytes(),
            CommandTarget::CreateDependency
        )
        .is_err()
    );
}

#[test]
fn dependency_receipt_accepts_change_or_noop_but_requires_correlated_resource() {
    let command =
        DependencyCommand::parse(&request(None), CommandTarget::RemoveDependency).unwrap();
    let receipt = |revision, events: Vec<&str>, kind, id| {
        json!({"command_id": PROJECT, "status": "applied", "project_revision": revision,
            "event_ids": events, "resource": {"kind": kind, "id": id}})
        .to_string()
    };
    assert!(
        command
            .validate_receipt(receipt(4, vec![BLOCKER], "task_dependency", BLOCKED).as_bytes())
            .is_ok()
    );
    assert!(
        command
            .validate_receipt(receipt(3, vec![], "task_dependency", BLOCKED).as_bytes())
            .is_ok()
    );
    assert!(
        command
            .validate_receipt(receipt(3, vec![BLOCKER], "task_dependency", BLOCKED).as_bytes())
            .is_err()
    );
    assert!(
        command
            .validate_receipt(receipt(4, vec![], "task_dependency", BLOCKED).as_bytes())
            .is_err()
    );
    assert!(
        command
            .validate_receipt(receipt(4, vec![BLOCKER], "task", BLOCKED).as_bytes())
            .is_err()
    );
    assert!(
        command
            .validate_receipt(receipt(4, vec![BLOCKER], "task_dependency", BLOCKER).as_bytes())
            .is_err()
    );
}

#[tokio::test]
async fn dependency_routes_forward_only_exact_validated_commands() {
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
    for (action, condition) in [
        ("create_dependency", Some("task_done")),
        ("remove_dependency", None),
    ] {
        let receipt = json!({"command_id": PROJECT, "status": "applied", "project_revision": 4,
            "event_ids": [BLOCKER], "resource": {"kind": "task_dependency", "id": BLOCKED}});
        let response_body = serde_json::to_vec(&receipt).unwrap();
        let (_dir, client, server) = fake_core(response(200, &response_body), Duration::ZERO).await;
        let mut state = Arc::try_unwrap(state()).ok().unwrap();
        state.core = client;
        let code = state.sessions.issue_code().unwrap();
        let token = state.sessions.exchange(&code.code).unwrap().token;
        let bytes = request(condition);
        let browser_request = Request::builder()
            .method("POST")
            .uri(format!("/api/commands/{action}"))
            .header("host", &state.host)
            .header("origin", &state.origin)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .header("idempotency-key", "dependency-key")
            .header("x-actor", "private-browser")
            .body(Body::from(bytes.clone()))
            .unwrap();
        let result = handle(State(Arc::new(state)), browser_request).await;
        assert_eq!(result.status(), 200);
        assert_eq!(
            to_bytes(result.into_body(), 64 * 1024).await.unwrap(),
            response_body
        );
        let forwarded = server.await.unwrap();
        let text = std::str::from_utf8(&forwarded).unwrap();
        assert!(text.starts_with(&format!("POST /v1/commands/{action} HTTP/1.1\r\n")));
        assert!(text.contains("idempotency-key: dependency-key\r\n"));
        assert!(
            !text.contains("private-browser")
                && !text.contains("Bearer")
                && !text.contains("origin:")
        );
        assert!(forwarded.ends_with(&bytes));
    }
}
