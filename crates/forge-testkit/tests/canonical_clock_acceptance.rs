//! Canonical mutations remain monotonic when host wall time falls behind durable state.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use forge_application::CommandEnvelope;
use forge_domain::{LifecycleStatus, Project, ProjectId, TaskId, TaskWaitKind, Timestamp};
use forge_protocol::{
    supervisor::v1::{AcknowledgementDisposition, CoreAcknowledgement, RunEventKind},
    wire::{CommandName, CommandRequest},
};
use forge_storage::StorageTransaction;
use forge_testkit::m0::{M0Harness, ManualSupervisor, single_stage_pipeline};
use serde_json::json;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; deterministic held-lock wall-clock rollback"]
async fn command_uses_the_project_clock_committed_while_waiting_for_its_lock() -> Result<()> {
    let harness = M0Harness::start().await?;
    let project_id = harness.create_project("Clock rollback behind lock").await?;
    let pipeline = harness
        .create_pipeline(project_id, single_stage_pipeline())
        .await?;
    harness.create_employee(project_id, "Clock worker").await?;
    let task_id = harness
        .create_task(project_id, pipeline, "Approve after a clock rollback")
        .await?;
    let original_task = harness.required_task(task_id).await?.task;
    let mut writer = harness.store.begin().await?;
    let mut project = writer.lock_project(project_id).await?.context("Project")?;
    let project_revision = project.revision();
    let future = future_timestamp();
    let envelope = CommandEnvelope::parse(
        CommandName::ApproveTask,
        CommandRequest {
            project_id: project_id.as_uuid().to_string(),
            expected_revision: project_revision + 1,
            payload: json!({
                "task_id": task_id.as_uuid().to_string(),
                "expected_task_revision": original_task.revision().get() + 1
            })
            .as_object()
            .context("command payload")?
            .clone(),
        },
        Uuid::now_v7().to_string(),
    )?;
    let command = harness.core.execute_command(envelope);
    tokio::pin!(command);
    tokio::select! {
        result = &mut command => bail!("command bypassed held Project lock: {result:?}"),
        () = sleep(Duration::from_millis(50)) => {},
    }
    // Fault injection only: model an earlier writer committing before a host
    // clock rollback, without changing the host clock or weakening domain checks.
    advance_task(&mut writer, &project, task_id, future).await?;
    project.record_child_mutation(future)?;
    writer.update_project(&project, project_revision).await?;
    writer.commit().await?;

    timeout(Duration::from_secs(10), &mut command).await??;
    assert!(
        Timestamp::now_utc() < future,
        "rollback fixture must still hold"
    );
    let task = harness.required_task(task_id).await?.task;
    assert_eq!(task.lifecycle(), LifecycleStatus::Ready);
    assert!(task.updated_at() >= future);
    assert_project_time(&harness, project_id, task.updated_at()).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; dependency, dispatch and observations after clock rollback"]
async fn draft_dependency_and_execution_keep_canonical_time_ahead_of_wall_time() -> Result<()> {
    let harness = M0Harness::start().await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    let project_id = harness
        .create_project("Clock rollback across execution")
        .await?;
    let pipeline = harness
        .create_pipeline(project_id, single_stage_pipeline())
        .await?;
    harness
        .create_employee(project_id, "Dependency worker")
        .await?;
    let blocker = harness.create_task(project_id, pipeline, "Blocker").await?;
    let blocked = harness.create_task(project_id, pipeline, "Blocked").await?;
    let first_sequence = harness
        .store
        .list_events(project_id, None, 1000)
        .await?
        .last()
        .context("setup events")?
        .project_sequence;
    let future = future_timestamp();
    let mut writer = harness.store.begin().await?;
    let mut project = writer.lock_project(project_id).await?.context("Project")?;
    let project_revision = project.revision();
    // Test-only persisted-state fault injection, as in the held-lock regression.
    advance_task(&mut writer, &project, blocker, future).await?;
    advance_task(&mut writer, &project, blocked, future).await?;
    project.record_child_mutation(future)?;
    writer.update_project(&project, project_revision).await?;
    writer.commit().await?;

    harness
        .create_dependency(project_id, blocker, blocked)
        .await?;
    assert_eq!(
        harness.required_task(blocked).await?.task.lifecycle(),
        LifecycleStatus::Draft
    );
    assert_project_time(&harness, project_id, future).await?;
    harness.approve_task(project_id, blocked).await?;
    let waiting = harness.required_task(blocked).await?.task;
    assert_eq!(waiting.lifecycle(), LifecycleStatus::Waiting);
    assert!(
        waiting
            .wait_conditions()
            .any(|wait| wait.kind() == &TaskWaitKind::Dependency)
    );
    assert!(waiting.updated_at() >= future);
    harness.approve_task(project_id, blocker).await?;
    let immediately_eligible: bool = sqlx::query_scalar(
        "SELECT eligible_at <= clock_timestamp() + INTERVAL '5 seconds' FROM queue_entries WHERE task_id=$1 AND queue_state='queued'",
    )
    .bind(blocker.as_uuid())
    .fetch_one(&harness.pool)
    .await?;
    assert!(
        immediately_eligible,
        "canonical future time must not delay dispatch"
    );
    harness.start_project(project_id).await?;
    for task_id in [blocker, blocked] {
        complete_stage(&harness, &mut supervisor, task_id, future).await?;
        let task = harness.required_task(task_id).await?.task;
        assert_eq!(task.lifecycle(), LifecycleStatus::Done);
        assert!(task.updated_at() >= future);
        assert_project_time(&harness, project_id, task.updated_at()).await?;
    }
    harness.stop_project(project_id).await?;
    assert!(
        Timestamp::now_utc() < future,
        "rollback fixture must still hold"
    );
    let events = harness
        .store
        .list_events(project_id, Some(first_sequence), 1000)
        .await?;
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "task_dependency_created")
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "task_completed")
    );
    assert!(events.iter().all(|event| event.occurred_at >= future));
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0].occurred_at <= pair[1].occurred_at)
    );
    Ok(())
}

fn future_timestamp() -> Timestamp {
    Timestamp::from_offset_date_time(
        (Timestamp::now_utc().as_offset_date_time() + Duration::from_secs(3600))
            .replace_nanosecond(0)
            .expect("zero nanoseconds is valid"),
    )
}

async fn advance_task(
    writer: &mut StorageTransaction<'_>,
    project: &Project,
    task_id: TaskId,
    changed_at: Timestamp,
) -> Result<()> {
    let stored = writer.lock_task(task_id).await?.context("locked Task")?;
    let mut task = stored.task;
    let revision = task.revision().get();
    task.set_priority(
        project.priority_scheme(),
        task.priority_level_id().clone(),
        changed_at,
    )?;
    writer
        .update_task(&task, stored.persistence, revision)
        .await?;
    Ok(())
}

async fn assert_project_time(
    harness: &M0Harness,
    project_id: ProjectId,
    floor: Timestamp,
) -> Result<()> {
    let project = harness
        .store
        .load_project(project_id)
        .await?
        .context("Project")?;
    assert!(
        project.updated_at() >= floor,
        "Project cannot lag its canonical child mutation"
    );
    Ok(())
}

async fn complete_stage(
    harness: &M0Harness,
    supervisor: &mut ManualSupervisor,
    task_id: TaskId,
    floor: Timestamp,
) -> Result<()> {
    supervisor.next_provision_for_task(task_id).await?;
    let run = harness.wait_for_run_count(task_id, 1).await?.remove(0);
    let dispatched = harness.required_task(task_id).await?.task;
    assert_eq!(dispatched.lifecycle(), LifecycleStatus::InProgress);
    assert!(dispatched.updated_at() >= floor);
    let wall_clock_lease: bool = sqlx::query_scalar(
        "SELECT expires_at BETWEEN clock_timestamp() + INTERVAL '4 minutes' AND clock_timestamp() + INTERVAL '6 minutes' FROM leases WHERE id=$1",
    )
    .bind(run.lease_id)
    .fetch_one(&harness.pool)
    .await?;
    assert!(
        wall_clock_lease,
        "lease expiry must use real elapsed wall time"
    );
    accepted(
        supervisor
            .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Provisioning)
            .await?,
    )?;
    accepted(
        supervisor
            .send_observation_kind(&run, run.lease_fencing_token, 2, RunEventKind::Running)
            .await?,
    )?;
    assert_wall_clock_liveness(harness, run.id).await?;
    accepted(
        supervisor
            .send_observation_kind(&run, run.lease_fencing_token, 3, RunEventKind::Heartbeat)
            .await?,
    )?;
    assert_wall_clock_liveness(harness, run.id).await?;
    let (evidence_id, ack) = supervisor.submit_stage_evidence(&run, 4).await?;
    accepted(ack)?;
    accepted(
        supervisor
            .submit_stage_outcome(&run, 5, "completed", vec![evidence_id])
            .await?,
    )?;
    accepted(
        supervisor
            .send_observation_kind(&run, run.lease_fencing_token, 6, RunEventKind::Stopped)
            .await?,
    )?;
    Ok(())
}

async fn assert_wall_clock_liveness(harness: &M0Harness, run_id: Uuid) -> Result<()> {
    let wall_clock_liveness: bool = sqlx::query_scalar(
        "SELECT liveness_observed_at BETWEEN clock_timestamp() - INTERVAL '1 minute' AND clock_timestamp() + INTERVAL '5 seconds' FROM runs WHERE id=$1",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert!(
        wall_clock_liveness,
        "liveness must not inherit canonical future time"
    );
    Ok(())
}

fn accepted(ack: CoreAcknowledgement) -> Result<()> {
    if ack.disposition != AcknowledgementDisposition::Accepted as i32 {
        bail!("Core rejected clock-regression observation/submission: {ack:?}");
    }
    Ok(())
}
