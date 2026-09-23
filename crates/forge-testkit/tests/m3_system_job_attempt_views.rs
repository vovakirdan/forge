//! Bounded browser projection of durable SystemJob attempts, with synthetic execution.
#[path = "m3_system_job_ownership/support.rs"]
mod support;

use anyhow::{Context, Result};
use forge_domain::runtime::RunScope;
use forge_storage::EnvironmentReport;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn digest(value: &Value) -> String {
    Sha256::digest(serde_json::to_vec(value).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; synthetic result and physical observations"]
async fn attempt_read_shows_accepted_then_materialized_coordinates_without_private_payload()
-> Result<()> {
    let mut fixture = support::setup().await?;
    assert!(fixture.settings.policy.enabled);
    let event: Uuid = sqlx::query_scalar("SELECT id FROM event_log WHERE project_id=$1 AND event_type='employee_created' ORDER BY project_sequence LIMIT 1")
        .bind(fixture.project.as_uuid())
        .fetch_one(&fixture.h.pool)
        .await?;
    let references = json!([{"kind":"event","event_id":event}]);
    fixture.spec.input.context =
        json!({"source_refs":references,"employee":{"id":fixture.employee,"revision":1}});
    fixture.spec.input.source_digest = digest(&fixture.spec.input.context);
    let result = json!({"source_digest":fixture.spec.input.source_digest,"entries":[{
        "subject":{"kind":"employee_memory_entry","employee_id":fixture.employee,"task_id":null},
        "markdown":"Private provider output must never appear in the attempt read.",
        "source_refs":references
    }]});
    let run = support::admit(&fixture, &fixture.spec).await?;
    assert!(
        fixture
            .h
            .store
            .system_job_exists(fixture.project, fixture.spec.assignment.job_id)
            .await?
    );
    assert!(
        !fixture
            .h
            .store
            .system_job_attempt_cursor_exists(
                fixture.project,
                fixture.spec.assignment.job_id,
                Uuid::now_v7()
            )
            .await?
    );
    let before = fixture
        .h
        .store
        .system_job_attempt_page(fixture.project, fixture.spec.assignment.job_id, None, 2)
        .await?;
    assert_eq!(before.len(), 1);
    assert_eq!(before[0]["run_id"], json!(run.id));
    assert_eq!(before[0]["result_accepted"], false);
    assert_eq!(before[0]["materialized_entries"], json!([]));

    let mut tx = fixture.h.store.begin().await?;
    tx.lock_project(fixture.project).await?.context("Project")?;
    tx.save_system_job_result(
        fixture.spec.assignment.attempt_id,
        &result,
        Uuid::now_v7(),
        &digest(&result),
    )
    .await?;
    tx.request_run_stop(
        run.id,
        run.lease_fencing_token,
        run.environment_epoch,
        false,
    )
    .await?;
    tx.commit().await?;
    let accepted = fixture
        .h
        .store
        .system_job_attempt_page(fixture.project, fixture.spec.assignment.job_id, None, 2)
        .await?;
    assert_eq!(accepted[0]["result_accepted"], true);
    assert_eq!(accepted[0]["materialized_entries"], json!([]));

    let mut tx = fixture.h.store.begin().await?;
    tx.lock_project(fixture.project).await?.context("Project")?;
    tx.request_run_stop(run.id, run.lease_fencing_token, run.environment_epoch, true)
        .await?;
    tx.revoke_run_lease(run.id, run.lease_fencing_token, run.environment_epoch)
        .await?;
    tx.record_environment_report(&EnvironmentReport {
        scope: RunScope {
            run_id: run.id,
            fencing_token: run.lease_fencing_token,
            environment_epoch: run.environment_epoch,
        },
        host_id: "synthetic-host".into(),
        boot_id: "synthetic-boot".into(),
        environment_id: "synthetic-job-view".into(),
        quiescent: true,
        unknown: false,
    })
    .await?;
    tx.commit().await?;
    fixture.h.core.system_job_tick().await?;
    let completed = fixture
        .h
        .store
        .system_job_attempt_page(fixture.project, fixture.spec.assignment.job_id, None, 2)
        .await?;
    assert_eq!(completed[0]["state"], "completed");
    assert_eq!(
        completed[0]["materialized_entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(completed[0]["materialized_entries"][0]["id"].is_string());
    assert!(
        fixture
            .h
            .store
            .system_job_attempt_page(
                fixture.project,
                fixture.spec.assignment.job_id,
                Some(fixture.spec.assignment.attempt_id),
                2,
            )
            .await?
            .is_empty()
    );
    let wire = serde_json::to_string(&completed)?;
    for private in [
        "Private provider output",
        "spec",
        "result_hash",
        "binding",
        "markdown",
    ] {
        assert!(!wire.contains(private), "private field leaked: {private}");
    }
    fixture.h.shutdown().await;
    Ok(())
}
