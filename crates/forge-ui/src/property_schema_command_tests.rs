use axum::{
    body::{Body, to_bytes},
    extract::State,
    http::{Request, StatusCode},
};
use serde_json::json;
use std::{sync::Arc, time::Duration};

use crate::{
    command::CommandTarget,
    command_client_tests::{fake_core, response},
    http::handle,
    http_tests::state,
    property_schema_command::PropertySchemaCommand,
};

const PROJECT: &str = "01988000-0000-7000-8000-000000000001";

fn request() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "project_id":PROJECT,
        "expected_revision":3,
        "payload":{"schema":{"definitions":{"impact":{
            "key":"impact","display_name":"Impact","property_type":"enum",
            "required":true,"default_value":{"type":"enum","value":"medium"},
            "allowed_choices":["high","medium"]
        }}}}
    }))
    .unwrap()
}

fn receipt() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "command_id":"01988000-0000-7000-8000-000000000003",
        "status":"applied","project_revision":4,
        "event_ids":["01988000-0000-7000-8000-000000000004"],
        "resource":{"kind":"project","id":PROJECT}
    }))
    .unwrap()
}

#[test]
fn exact_schema_shape_and_project_receipt() {
    let target = CommandTarget::from_browser_path("/api/commands/configure_task_property_schema");
    assert!(matches!(
        target,
        Some(CommandTarget::ConfigureTaskPropertySchema)
    ));
    assert!(
        CommandTarget::from_browser_path("/api/commands/configure_task_property_schema/").is_none()
    );
    let command = PropertySchemaCommand::parse(&request()).unwrap();
    command.validate_receipt(&receipt()).unwrap();
    let mut wrong: serde_json::Value = serde_json::from_slice(&request()).unwrap();
    wrong["payload"]["schema"]["definitions"]["impact"]["key"] = json!("other");
    assert!(PropertySchemaCommand::parse(&serde_json::to_vec(&wrong).unwrap()).is_err());
    wrong["payload"]["schema"]["definitions"]["impact"]["key"] = json!("impact");
    wrong["payload"]["schema"]["definitions"]["impact"]["source_file"] = json!("/tmp/secret");
    assert!(PropertySchemaCommand::parse(&serde_json::to_vec(&wrong).unwrap()).is_err());
    let mut wrong_receipt: serde_json::Value = serde_json::from_slice(&receipt()).unwrap();
    wrong_receipt["resource"]["kind"] = json!("task");
    assert!(
        command
            .validate_receipt(&serde_json::to_vec(&wrong_receipt).unwrap())
            .is_err()
    );
}

#[tokio::test]
async fn owner_route_forwards_validated_schema_command_without_browser_actor() {
    let body = receipt();
    let (_dir, client, server) = fake_core(response(200, &body), Duration::ZERO).await;
    let mut gateway = Arc::try_unwrap(state()).ok().unwrap();
    gateway.core = client;
    let gateway = Arc::new(gateway);
    let code = gateway.sessions.issue_code().unwrap();
    let token = gateway.sessions.exchange(&code.code).unwrap().token;
    let input = request();
    let request = Request::builder()
        .method("POST")
        .uri("/api/commands/configure_task_property_schema")
        .header("host", &gateway.host)
        .header("origin", &gateway.origin)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .header("idempotency-key", "schema-key")
        .header("x-actor", "forged-browser-actor")
        .body(Body::from(input.clone()))
        .unwrap();
    let result = handle(State(gateway), request).await;
    assert_eq!(result.status(), StatusCode::OK);
    assert_eq!(to_bytes(result.into_body(), 65_536).await.unwrap(), body);
    let forwarded = server.await.unwrap();
    let text = std::str::from_utf8(&forwarded).unwrap();
    assert!(text.contains("/v1/commands/configure_task_property_schema"));
    assert!(forwarded.ends_with(&input));
    assert!(!text.contains("forged-browser-actor"));
}
