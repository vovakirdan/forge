//! Project-owned Task property schema reads are scoped and read-only.

use anyhow::Result;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use forge_domain::ProjectId;
use forge_protocol::wire::CommandStatus;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::fixture::{Backend, BackendKind, Fixture};

#[tokio::test]
#[ignore = "requires explicit local PostgreSQL; validates Project schema read"]
async fn task_property_schema_http_returns_canonical_default_and_scopes_project() -> Result<()> {
    let fixture = Fixture::create(BackendKind::Postgres).await?;
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!("PostgreSQL test")
    };
    let router = forge_core::router(fixture.core(pool));
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/projects/{}/task-property-schema",
                    fixture.project_id
                ))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await?)?;
    assert_eq!(body["project_id"], json!(fixture.project_id));
    assert_eq!(body["project_revision"], 1);
    assert_eq!(body["schema"], json!({"definitions":{}}));
    assert_eq!(body.as_object().expect("object").len(), 3);

    let foreign = router
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/projects/{}/task-property-schema",
                    ProjectId::new()
                ))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit local PostgreSQL; validates schema command and readback"]
async fn task_property_schema_http_command_reads_back_canonical_configuration() -> Result<()> {
    let fixture = Fixture::create(BackendKind::Postgres).await?;
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!("PostgreSQL test")
    };
    let router = forge_core::router(fixture.core(pool));
    let body = json!({
        "project_id":fixture.project_id,
        "expected_revision":1,
        "payload":{"schema":{"definitions":{"impact":{
            "key":"impact","display_name":"Impact","property_type":"enum",
            "required":true,"default_value":{"type":"enum","value":"medium"},
            "allowed_choices":["high","medium"]
        }}}}
    });
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/commands/configure_task_property_schema")
                .header("content-type", "application/json")
                .header("idempotency-key", "schema-http-test")
                .body(Body::from(body.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let receipt: forge_protocol::wire::CommandReceipt =
        serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await?)?;
    assert_eq!(receipt.status, CommandStatus::Applied);
    assert_eq!(receipt.project_revision, 2);

    let response = router
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/projects/{}/task-property-schema",
                    fixture.project_id
                ))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let read: Value = serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await?)?;
    assert_eq!(read["project_revision"], 2);
    assert_eq!(read["schema"], body["payload"]["schema"]);
    Ok(())
}
