//! Production transport and postcommit behavior around the shared command engine.

use std::{
    io::Write,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use forge_application::Clock;
use forge_domain::{
    AggregateRef, CommandId, DomainEvent, DomainEventInput, DomainEventKind, EventId, Timestamp,
    runtime::BootRecoveryPolicy,
};
use forge_protocol::wire::{CommandName, CommandReceipt, CommandStatus, EVENT_SCHEMA_VERSION};
use forge_storage::{IdempotencyRecord, PostgresStore};
use serde_json::{Value, json};
use tower::ServiceExt;
use tracing::instrument::WithSubscriber;
use uuid::Uuid;

use super::{
    active_runs::running_task,
    compatibility::{RECOVERY_HASH, legacy_receipt},
    fixture::{Backend, BackendKind, Fixture},
};

#[tokio::test]
#[ignore = "requires explicit local PostgreSQL; validates HTTP production wiring"]
async fn http_commands_use_the_shared_clock_and_keep_wire_errors() -> Result<()> {
    let fixture = Fixture::new(BackendKind::Postgres).await?;
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!("PostgreSQL fixture")
    };
    let core = fixture
        .core(pool)
        .with_command_clock(Arc::new(fixture.clock.clone()));
    let router = forge_core::router(core);
    let (status, receipt) = post_command(
        &router,
        &fixture,
        "create_project",
        0,
        json!({"name":"HTTP shared engine"}),
        "http-create",
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let parsed: CommandReceipt = serde_json::from_value(receipt.clone())?;
    assert_eq!(parsed.status, CommandStatus::Applied);
    assert_eq!(parsed.project_revision, 1);
    assert_eq!(receipt.as_object().context("receipt object")?.len(), 5);
    assert_eq!(
        receipt["resource"],
        json!({"kind":"project", "id":fixture.project_id})
    );
    let snapshot = fixture.snapshot().await?;
    snapshot.assert_audit_atomic();
    assert_eq!(snapshot.rows("idempotency_keys")[0]["receipt"], receipt);
    let occurred: time::OffsetDateTime =
        sqlx::query_scalar("SELECT occurred_at FROM event_log WHERE id=$1")
            .bind(parsed.event_ids[0].parse::<Uuid>()?)
            .fetch_one(pool)
            .await?;
    assert_eq!(
        Timestamp::from_offset_date_time(occurred),
        fixture.clock.now()
    );
    assert_eq!(
        snapshot.projects[&fixture.project_id].updated_at(),
        fixture.clock.now()
    );

    let (status, replay) = post_command(
        &router,
        &fixture,
        "create_project",
        0,
        json!({"name":"HTTP shared engine"}),
        "http-create",
    )
    .await?;
    let mut expected = receipt;
    expected["status"] = json!("replayed");
    assert_eq!((status, replay), (StatusCode::OK, expected));
    for (name, payload, key, code) in [
        (
            "start_project_execution",
            json!({}),
            "http-stale",
            "stale_revision",
        ),
        (
            "create_project",
            json!({"name":"Different intent"}),
            "http-create",
            "idempotency_conflict",
        ),
    ] {
        let (status, error) = post_command(&router, &fixture, name, 0, payload, key).await?;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error["error"]["code"], code);
        assert!(error["error"]["request_id"].as_str().is_some());
    }
    assert_eq!(fixture.snapshot().await?.raw, snapshot.raw);
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit local PostgreSQL; validates actual postcommit delivery failure"]
async fn failed_stop_delivery_does_not_reject_the_committed_command() -> Result<()> {
    let setup = running_task(BackendKind::Postgres).await?;
    let fixture = &setup.fixture;
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!("PostgreSQL fixture")
    };
    // This fixture contains a real fenced Lease/Run, but no Supervisor transport.
    // Its pending stop must reach send() and fail, not short-circuit empty dispatch.
    let core = fixture
        .core(pool)
        .with_fake_runtime()
        .with_command_clock(Arc::new(fixture.clock.clone()));
    let before = fixture.snapshot().await?;
    let envelope = fixture.envelope(
        CommandName::StopProjectExecution,
        before.projects[&fixture.project_id].revision(),
        json!({}),
        "offline-stop",
    );
    let logs = CapturedLogs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let receipt = core
        .execute_command(envelope.clone())
        .with_subscriber(subscriber)
        .await?;
    let captured = String::from_utf8(logs.0.lock().expect("captured logs").clone())?;
    assert!(
        captured.lines().any(|line| {
            let entry: Value = serde_json::from_str(line).expect("structured log");
            entry["fields"]["message"] == "deferred Supervisor stop delivery failed"
                && entry["fields"]["error"] == "local Supervisor is unavailable"
                && entry["fields"]["project_id"] == fixture.project_id.to_string()
        }),
        "the actual postcommit send must fail: {captured}"
    );
    assert_eq!(receipt.status, CommandStatus::Applied);
    let after = fixture.snapshot().await?;
    after.assert_audit_atomic();
    assert_eq!(after.rows("leases"), before.rows("leases"));
    assert_eq!(
        after.rows("run_environment_reservations"),
        before.rows("run_environment_reservations")
    );
    let run = after
        .rows("runs")
        .iter()
        .find(|run| run["id"] == setup.run.id.to_string())
        .context("stopped Run")?;
    assert_eq!(run["desired_state"], "stop_requested");
    assert_eq!(run["lease_fencing_token"], setup.run.lease_fencing_token);
    assert_eq!(run["environment_epoch"], setup.run.environment_epoch);
    let record = after
        .rows("idempotency_keys")
        .iter()
        .find(|row| row["idempotency_key"] == "offline-stop")
        .context("accepted receipt")?;
    assert_eq!(record["receipt"], serde_json::to_value(&receipt)?);
    assert!(
        after
            .rows("event_log")
            .iter()
            .any(|event| event["command_id"] == receipt.command_id
                && event["event_type"] == "run_stop_requested")
    );
    let replay = core.execute_command(envelope).await?;
    let mut expected = serde_json::to_value(receipt)?;
    expected["status"] = json!("replayed");
    assert_eq!(serde_json::to_value(replay)?, expected);
    assert_eq!(fixture.snapshot().await?.raw, after.raw);
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit local PostgreSQL; reads an inserted pre-extraction M1 receipt"]
async fn core_replays_a_persisted_legacy_m1_receipt_after_later_revisions() -> Result<()> {
    let fixture = Fixture::create(BackendKind::Postgres).await?;
    for _ in 0..6 {
        fixture
            .execute(CommandName::StartProjectExecution, json!({}))
            .await?;
    }
    let Backend::Postgres(pool) = &fixture.backend else {
        unreachable!("PostgreSQL fixture")
    };
    let store = PostgresStore::from_pool(pool.clone());
    let legacy = legacy_receipt();
    let receipt: CommandReceipt = serde_json::from_value(legacy.clone())?;
    let command_id: CommandId = receipt.command_id.parse()?;
    let event_id: EventId = receipt.event_ids[0].parse()?;
    let mut transaction = store.begin().await?;
    let mut project = transaction
        .lock_project(fixture.project_id)
        .await?
        .context("Project")?;
    assert_eq!(project.revision(), 7);
    project.record_child_mutation(fixture.clock.now())?;
    transaction.update_project(&project, 7).await?;
    transaction
        .configure_boot_recovery_policy(project.id(), BootRecoveryPolicy::ManualHold)
        .await?;
    transaction
        .append_event_and_outbox(&DomainEvent::new(
            event_id,
            DomainEventInput {
                project_id: project.id(),
                aggregate: AggregateRef::Project(project.id()),
                aggregate_revision: 8,
                kind: DomainEventKind::BootRecoveryPolicyConfigured,
                actor: fixture.context.actor,
                command_id,
                reason: None,
                occurred_at: fixture.clock.now(),
                schema_version: EVENT_SCHEMA_VERSION,
                payload: json!({"policy":"manual_hold"}),
            },
        )?)
        .await?;
    // Insert the literal historical JSON/hash, never a newly generated command receipt.
    transaction
        .insert_idempotency(&IdempotencyRecord {
            project_id: project.id(),
            key: "persisted-legacy-recovery".into(),
            command_id,
            command_name: "configure_boot_recovery_policy".into(),
            expected_revision: Some(7),
            request_hash: RECOVERY_HASH.into(),
            actor: serde_json::to_value(fixture.context.actor)?,
            receipt: legacy.clone(),
            event_id: Some(event_id),
            response_revision: Some(8),
        })
        .await?;
    transaction.commit().await?;
    fixture
        .execute(CommandName::StopProjectExecution, json!({}))
        .await?;
    let before = fixture.snapshot().await?;
    before.assert_audit_atomic();
    assert!(before.projects[&fixture.project_id].revision() > receipt.project_revision);
    let core = fixture.core(pool);
    let replay = core
        .execute_command(fixture.envelope(
            CommandName::ConfigureBootRecoveryPolicy,
            7,
            json!({"policy":"manual_hold"}),
            "persisted-legacy-recovery",
        ))
        .await?;
    let mut expected = legacy;
    expected["status"] = json!("replayed");
    assert_eq!(serde_json::to_value(replay)?, expected);
    assert_eq!(fixture.snapshot().await?.raw, before.raw);
    Ok(())
}

async fn post_command(
    router: &Router,
    fixture: &Fixture,
    name: &str,
    revision: u64,
    payload: Value,
    key: &str,
) -> Result<(StatusCode, Value)> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/commands/{name}"))
                .header("content-type", "application/json")
                .header("idempotency-key", key)
                .body(Body::from(serde_json::to_vec(&json!({
                    "project_id":fixture.project_id, "expected_revision":revision, "payload":payload
                }))?))?,
        )
        .await?;
    assert!(response.headers().contains_key("x-request-id"));
    let status = response.status();
    let body = serde_json::from_slice(&to_bytes(response.into_body(), 16 * 1024).await?)?;
    Ok((status, body))
}

#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl Write for CapturedLogs {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("captured logs")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
