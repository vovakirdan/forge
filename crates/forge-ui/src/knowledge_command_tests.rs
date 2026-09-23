use axum::{
    body::{Body, to_bytes},
    extract::State,
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

use crate::{
    command::CommandTarget,
    command_client_tests::{fake_core, response},
    http::handle,
    http_tests::state,
    knowledge_command::KnowledgeCommand,
};

const PROJECT: &str = "01988000-0000-7000-8000-000000000001";
const PAGE: &str = "01988000-0000-7000-8000-000000000002";

fn request(payload: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"project_id":PROJECT,"expected_revision":3,"payload":payload}))
        .unwrap()
}

#[test]
fn named_payloads_and_receipt_are_exact() {
    let shapes = [
        (
            CommandTarget::AuthorKnowledgePage,
            json!({"page_id":PAGE,"expected_page_revision":0,"kind":"guide","title":"Guide","markdown":"Text","source_refs":[]}),
        ),
        (
            CommandTarget::PublishKnowledgePage,
            json!({"page_id":PAGE,"expected_page_revision":1}),
        ),
        (
            CommandTarget::SupersedeKnowledgePage,
            json!({"page_id":PAGE,"expected_page_revision":2,"title":"Guide","markdown":"Changed"}),
        ),
        (
            CommandTarget::WithdrawKnowledgePage,
            json!({"page_id":PAGE,"expected_page_revision":3}),
        ),
    ];
    let receipt = serde_json::to_vec(&json!({"command_id":"01988000-0000-7000-8000-000000000003","status":"applied","project_revision":4,"event_ids":["01988000-0000-7000-8000-000000000004"],"resource":{"kind":"knowledge_page","id":PAGE}})).unwrap();
    for (target, payload) in shapes {
        let command = KnowledgeCommand::parse(&request(payload), target).unwrap();
        command.validate_receipt(&receipt).unwrap();
    }
    for name in [
        "author_knowledge_page",
        "publish_knowledge_page",
        "supersede_knowledge_page",
        "withdraw_knowledge_page",
    ] {
        assert!(CommandTarget::from_browser_path(&format!("/api/commands/{name}")).is_some());
        assert!(CommandTarget::from_browser_path(&format!("/api/commands/{name}/")).is_none());
    }
}

#[test]
fn wrong_shape_and_receipt_are_rejected() {
    let target = CommandTarget::PublishKnowledgePage;
    for payload in [
        json!({"page_id":PAGE,"expected_page_revision":0}),
        json!({"page_id":PAGE,"expected_page_revision":1,"operation":"withdraw"}),
        json!({"page_id":PROJECT.replace("7000", "4000"),"expected_page_revision":1}),
    ] {
        assert!(KnowledgeCommand::parse(&request(payload), target).is_err());
    }
    let command = KnowledgeCommand::parse(
        &request(json!({"page_id":PAGE,"expected_page_revision":1})),
        target,
    )
    .unwrap();
    for resource in [
        json!({"kind":"task","id":PAGE}),
        json!({"kind":"knowledge_page","id":PROJECT}),
    ] {
        let wrong = serde_json::to_vec(&json!({"command_id":"01988000-0000-7000-8000-000000000003","status":"applied","project_revision":4,"event_ids":["01988000-0000-7000-8000-000000000004"],"resource":resource})).unwrap();
        assert!(command.validate_receipt(&wrong).is_err());
    }
    let author = CommandTarget::AuthorKnowledgePage;
    for payload in [
        json!({"page_id":PAGE,"expected_page_revision":0,"kind":"guide","title":" ","markdown":"Text"}),
        json!({"page_id":PAGE,"expected_page_revision":0,"kind":"guide","title":"Guide","markdown":" "}),
        json!({"page_id":PAGE,"expected_page_revision":0,"kind":"guide","title":"Guide","markdown":"Text","source_refs":[{"kind":"event","event_id":PAGE,"url":"https://invalid"}]}),
    ] {
        assert!(KnowledgeCommand::parse(&request(payload), author).is_err());
    }
}

#[tokio::test]
async fn exact_owner_route_forwards_only_validated_knowledge_command() {
    let body = serde_json::to_vec(&json!({"command_id":"01988000-0000-7000-8000-000000000003","status":"applied","project_revision":4,"event_ids":["01988000-0000-7000-8000-000000000004"],"resource":{"kind":"knowledge_page","id":PAGE}})).unwrap();
    let (_dir, client, server) = fake_core(response(200, &body), Duration::ZERO).await;
    let mut gateway = Arc::try_unwrap(state()).ok().unwrap();
    gateway.core = client;
    let gateway = Arc::new(gateway);
    let code = gateway.sessions.issue_code().unwrap();
    let token = gateway.sessions.exchange(&code.code).unwrap().token;
    let input = request(json!({"page_id":PAGE,"expected_page_revision":1}));
    let request = Request::builder()
        .method("POST")
        .uri("/api/commands/publish_knowledge_page")
        .header("host", &gateway.host)
        .header("origin", &gateway.origin)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .header("idempotency-key", "fixed-key")
        .header("x-actor", "forged-browser-actor")
        .body(Body::from(input.clone()))
        .unwrap();
    let result = handle(State(gateway), request).await;
    assert_eq!(result.status(), StatusCode::OK);
    assert_eq!(to_bytes(result.into_body(), 65_536).await.unwrap(), body);
    let forwarded = server.await.unwrap();
    let text = std::str::from_utf8(&forwarded).unwrap();
    assert!(text.contains("/v1/commands/publish_knowledge_page"));
    assert!(forwarded.ends_with(&input));
    assert!(!text.contains("forged-browser-actor"));
}
