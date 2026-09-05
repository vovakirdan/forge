//! Ignored public HTTP-over-UDS transport acceptance for local M0.

use anyhow::{Context, Result};
use forge_cli::LocalClient;
use forge_protocol::wire::{CommandRequest, CommandStatus};
use forge_testkit::m0::{LocalHttpApi, M0Harness};
use serde_json::{Map, Value, json};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; run `just test-integration`"]
async fn m0_http_uds_command_idempotency_and_sse_last_event_id_reconnect() -> Result<()> {
    let harness = M0Harness::start().await?;
    let api = LocalHttpApi::start(harness.core.clone())?;
    let client = LocalClient::new(api.socket().to_path_buf());
    let project_id = Uuid::now_v7().to_string();
    let create_project = CommandRequest {
        project_id: project_id.clone(),
        expected_revision: 0,
        payload: payload(json!({ "name": "M0 public transport" }))?,
    };
    let idempotency_key = Uuid::now_v7().to_string();

    let applied = client
        .execute_command("create_project", &create_project, &idempotency_key)
        .await?;
    let replayed = client
        .execute_command("create_project", &create_project, &idempotency_key)
        .await?;

    assert_eq!(applied.status, CommandStatus::Applied);
    assert_eq!(replayed.status, CommandStatus::Replayed);
    assert_eq!(replayed.command_id, applied.command_id);
    assert_eq!(replayed.project_revision, applied.project_revision);
    assert_eq!(replayed.event_ids, applied.event_ids);

    let initial_events = client.watch_events(&project_id, None).await?;
    let last_event_id = initial_events
        .last()
        .context("create_project must be visible through public SSE")?
        .project_sequence;
    let employee = client
        .execute_command(
            "create_employee",
            &CommandRequest {
                project_id: project_id.clone(),
                expected_revision: applied.project_revision,
                payload: payload(json!({
                    "name": "M0 public transport employee",
                    "role": "m0_worker",
                    "stage_eligibility": { "mode": "any" }
                }))?,
            },
            &Uuid::now_v7().to_string(),
        )
        .await?;
    let reconnected_events = client
        .watch_events_after_last_event_id(&project_id, last_event_id)
        .await?;

    assert_eq!(employee.status, CommandStatus::Applied);
    assert_eq!(reconnected_events.len(), 1);
    assert!(
        reconnected_events
            .iter()
            .all(|event| event.project_sequence > last_event_id),
        "Last-Event-ID must resume strictly after the last observed sequence"
    );
    assert_eq!(reconnected_events[0].event_id, employee.event_ids[0]);

    api.shutdown().await;
    harness.shutdown().await;
    Ok(())
}

fn payload(value: Value) -> Result<Map<String, Value>> {
    value
        .as_object()
        .cloned()
        .context("transport fixture payload must be an object")
}
