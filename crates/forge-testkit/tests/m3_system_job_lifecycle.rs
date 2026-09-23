//! Canonical lifecycle fault injection; no model output is represented as live inference.
#[path = "m3_system_job_ownership/support.rs"]
mod support;

use anyhow::{Context, Result};
use forge_domain::{
    EmployeeId, EvidenceLocation, EvidenceObject, EvidenceObjectInput, EvidenceScope,
    EvidenceStream, ProjectId, Timestamp, runtime::RunScope, system_job::SystemJobKind,
};
use forge_storage::{EnvironmentReport, RunProjection};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn digest(value: &Value) -> String {
    Sha256::digest(serde_json::to_vec(value).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

async fn input(f: &mut support::Fixture) -> Result<Value> {
    assert!(f.settings.policy.enabled);
    let event:Uuid=sqlx::query_scalar("SELECT id FROM event_log WHERE project_id=$1 AND event_type='employee_created' ORDER BY project_sequence LIMIT 1")
        .bind(f.project.as_uuid()).fetch_one(&f.h.pool).await?;
    let refs = json!([{"kind":"event","event_id":event}]);
    f.spec.input.context = json!({"source_refs":refs,"employee":{"id":f.employee,"revision":1}});
    f.spec.input.source_digest = digest(&f.spec.input.context);
    Ok(
        json!({"source_digest":f.spec.input.source_digest,"entries":[{"subject":{"kind":"employee_memory_entry","employee_id":f.employee,"task_id":null},"markdown":"The worker read its role and the supplied project rules.","source_refs":refs}]}),
    )
}
async fn submit(f: &support::Fixture, run: &RunProjection, result: Value) -> Result<()> {
    let mut tx = f.h.store.begin().await?;
    tx.lock_project(f.project).await?.context("Project")?;
    let scope = RunScope {
        run_id: run.id,
        fencing_token: run.lease_fencing_token,
        environment_epoch: run.environment_epoch,
    };
    assert!(tx.validate_gateway_scope(&scope).await?.is_some());
    tx.save_system_job_result(
        f.spec.assignment.attempt_id,
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
    Ok(())
}
async fn quiesce(f: &support::Fixture, run: &RunProjection) -> Result<()> {
    let mut tx = f.h.store.begin().await?;
    tx.lock_project(f.project).await?.context("Project")?;
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
        environment_id: "synthetic-job".into(),
        quiescent: true,
        unknown: false,
    })
    .await?;
    tx.commit().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; synthetic result and physical observations"]
async fn accepted_onboarding_survives_stop_after_submission_and_materializes_once() -> Result<()> {
    let mut f = support::setup().await?;
    let result = input(&mut f).await?;
    let run = support::admit(&f, &f.spec).await?;
    let receipt = EvidenceObject::new(EvidenceObjectInput {
        id: Uuid::now_v7(),
        scope: EvidenceScope {
            project_id: f.project,
            task_id: None,
            run_id: run.id,
        },
        stream: EvidenceStream::Diagnostic,
        sequence: 1,
        sha256: "a".repeat(64),
        size_bytes: 8,
        redaction_policy_reference: "policy/v1".into(),
        location: EvidenceLocation::PendingUpload,
        created_at: Timestamp::now_utc(),
    })?;
    let mut evidence_tx = f.h.store.begin().await?;
    assert!(evidence_tx.record_evidence_object(&receipt, false).await?);
    evidence_tx.commit().await?;
    assert_eq!(
        f.h.store
            .run_evidence_page(f.project, run.id, None, 2)
            .await?
            .len(),
        1,
        "ownerless SystemJob Run retains its technical evidence receipt"
    );
    submit(&f, &run, result).await?;
    f.h.core.system_job_tick().await?;
    assert!(
        f.h.store
            .visible_derived_memory(f.project, Some(f.employee), None, 100)
            .await?
            .is_empty(),
        "result alone is not physical completion"
    );
    f.h.stop_project(f.project).await?;
    quiesce(&f, &run).await?;
    f.h.core.system_job_tick().await?;
    let entries =
        f.h.store
            .visible_derived_memory(f.project, Some(f.employee), None, 100)
            .await?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].created_by_job_id, f.spec.assignment.job_id);
    let status = f.h.store.system_job_status(f.project).await?;
    assert_eq!(status["jobs"][0]["state"], "completed");
    assert_eq!(status["onboarding"][0]["state"], "completed");
    assert_eq!(
        status["onboarding"][0]["receipt"]["source_digest"],
        f.spec.input.source_digest
    );
    assert!(
        f.h.store
            .visible_derived_memory(f.project, None, None, 100)
            .await?
            .is_empty(),
        "onboarding is personal"
    );
    f.h.core.system_job_tick().await?;
    assert_eq!(
        f.h.store
            .visible_derived_memory(f.project, Some(f.employee), None, 100)
            .await?,
        entries
    );
    let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM derived_memory_entries WHERE project_id=$1),(SELECT count(*) FROM event_log WHERE project_id=$1 AND event_type='derived_memory_changed')")
        .bind(f.project.as_uuid()).fetch_one(&f.h.pool).await?;
    assert_eq!(counts, (1, 1));
    f.h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn newer_coalesced_sources_fence_old_results_without_losing_retry_input() -> Result<()> {
    let mut f = support::setup().await?;
    let pipeline =
        f.h.create_pipeline(f.project, forge_testkit::m0::single_stage_pipeline())
            .await?;
    let task =
        f.h.create_task(f.project, pipeline, "Late evidence")
            .await?;
    let mut result = input(&mut f).await?;
    let through: i64 =
        sqlx::query_scalar("SELECT max(project_sequence) FROM event_log WHERE project_id=$1")
            .bind(f.project.as_uuid())
            .fetch_one(&f.h.pool)
            .await?;
    sqlx::query("UPDATE system_jobs SET kind='summarization',target_employee_id=NULL,source_task_id=$2,covered_sequence=$3 WHERE id=$1")
        .bind(f.spec.assignment.job_id).bind(task.as_uuid()).bind(through).execute(&f.h.pool).await?;
    f.spec.assignment.kind = SystemJobKind::Summarization;
    f.spec.input.source_task_id = Some(task);
    f.spec.input.target_employee_id = None;
    f.spec.input.covered_sequence = through as u64;
    result["entries"][0]["subject"] = json!({"kind":"task_summary","task_id":task});
    let run = support::admit(&f, &f.spec).await?;
    submit(&f, &run, result).await?;
    let mut tx = f.h.store.begin().await?;
    tx.lock_project(f.project).await?;
    let same = tx
        .enqueue_task_summary(f.project, task, through as u64 + 1, 0)
        .await?;
    assert_eq!(same, f.spec.assignment.job_id);
    tx.commit().await?;
    quiesce(&f, &run).await?;
    f.h.core.system_job_tick().await?;
    assert!(
        f.h.store
            .visible_derived_memory(f.project, Some(f.employee), None, 100)
            .await?
            .is_empty()
    );
    let job = f.h.store.system_jobs(f.project).await?.remove(0);
    assert_eq!(job.state, "pending");
    assert_eq!(job.generation, 2);
    assert_eq!(job.covered_sequence, through as u64 + 1);
    let result_retained: bool = sqlx::query_scalar(
        "SELECT state='superseded' AND result IS NOT NULL FROM system_job_attempts WHERE id=$1",
    )
    .bind(f.spec.assignment.attempt_id)
    .fetch_one(&f.h.pool)
    .await?;
    assert!(result_retained);
    f.h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference"]
async fn onboarding_is_an_explicit_admission_gate_not_employee_capacity() -> Result<()> {
    let h = forge_testkit::m0::M0Harness::start_configured(Ok).await?;
    let project = h.create_project("New employee onboarding gate").await?;
    let receipt = h
        .execute(
            project,
            forge_protocol::wire::CommandName::CreateEmployee,
            json!({"name":"New arrival","role":"writer","stage_eligibility":{"mode":"any"}}),
        )
        .await?;
    let employee: EmployeeId = h
        .store
        .list_employees(project)
        .await?
        .remove(0)
        .employee
        .id();
    assert!(!receipt.event_ids.is_empty());
    let mut tx = h.store.begin().await?;
    assert!(!tx.onboarding_allowed(project, employee).await?);
    assert!(tx.lock_employee_capacity(project, employee).await?);
    tx.commit().await?;
    h.execute(project,forge_protocol::wire::CommandName::SkipEmployeeOnboarding,json!({"employee_id":employee,"reason":"Operator explicitly supplied the project orientation outside Forge"})).await?;
    let mut tx = h.store.begin().await?;
    assert!(tx.onboarding_allowed(project, employee).await?);
    assert!(!tx.onboarding_allowed(ProjectId::new(), employee).await?);
    tx.commit().await?;
    assert_eq!(
        h.store.system_job_status(project).await?["onboarding"][0]["state"],
        "skipped"
    );
    assert!(
        h.store.system_jobs(project).await?.is_empty(),
        "skip fabricates neither a job nor completion"
    );
    h.shutdown().await;
    Ok(())
}
