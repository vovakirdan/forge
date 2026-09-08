//! Ignored M0 acceptance tests against real local PostgreSQL, NATS, and gRPC.
//!
//! The command path calls public `CoreService::execute_command` directly: its
//! named-command envelope is the canonical Core boundary, while this suite
//! still drives the real Supervisor gRPC transport. HTTP/CLI transport
//! conformance belongs to the API and CLI test suites.

use std::{collections::BTreeSet, time::Duration};

use anyhow::{Context, Result, bail};
use forge_domain::{LifecycleStatus, ProjectId, Task, TaskWaitKind};
use forge_protocol::supervisor::v1::{
    AcknowledgementDisposition, CoreAcknowledgement, RunEventKind,
};
use forge_storage::{RunDesiredState, RunObservedState};
use forge_testkit::m0::{
    M0Harness, four_stage_pipeline, human_retry_pipeline, retry_pipeline, single_stage_pipeline,
};
use tokio::time::timeout;
use uuid::Uuid;

#[path = "milestone0_acceptance/terminal_interruption.rs"]
mod terminal_interruption;

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; run `just test-integration`"]
async fn m0_multistage_task_keeps_one_identity_and_attaches_stage_evidence() -> Result<()> {
    let mut harness = M0Harness::start().await?;
    harness.attach_fake_supervisor().await?;
    let project = harness.create_project("M0 multi-stage").await?;
    let pipeline = harness
        .create_pipeline(project, four_stage_pipeline())
        .await?;
    harness.create_employee(project, "M0 worker").await?;
    let task = harness
        .create_task(project, pipeline, "deliver across employee stages")
        .await?;
    harness.approve_task(project, task).await?;
    harness.start_project(project).await?;

    harness
        .wait_for_lifecycle(task, LifecycleStatus::Done)
        .await?;
    let runs = harness.wait_for_run_count(task, 4).await?;
    let artifacts = harness.store.list_artifacts_for_task(task).await?;
    let stages = runs
        .iter()
        .map(|run| {
            run.require_task_stage()
                .expect("TaskStage fixture")
                .stage_id
                .to_string()
        })
        .collect::<BTreeSet<_>>();
    let event_types = event_types(&harness, project).await?;

    assert_eq!(artifacts.len(), 3, "current stages attached evidence");
    assert_eq!(
        stages,
        BTreeSet::from([
            "implementation".to_owned(),
            "verification".to_owned(),
            "review".to_owned(),
            "integration".to_owned(),
        ])
    );
    assert_events(
        &event_types,
        [
            "task_started",
            "artifact_created",
            "task_stage_advanced",
            "task_completed",
        ],
    );

    harness.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; run `just test-integration`"]
async fn m0_capacity_one_leaves_the_second_task_durably_queued() -> Result<()> {
    let harness = M0Harness::start().await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    let project = harness.create_project("M0 capacity").await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    harness.create_employee(project, "only worker").await?;
    let first = harness.create_task(project, pipeline, "first task").await?;
    let second = harness
        .create_task(project, pipeline, "second task")
        .await?;
    harness.approve_task(project, first).await?;
    harness.approve_task(project, second).await?;
    harness.start_project(project).await?;

    let provision = supervisor.next_provision_for_task(first).await?;
    assert_eq!(provision.task_id, first.as_uuid().to_string());
    harness
        .wait_for_lifecycle(first, LifecycleStatus::InProgress)
        .await?;
    harness
        .wait_for_lifecycle(second, LifecycleStatus::Ready)
        .await?;
    assert_eq!(harness.wait_for_run_count(first, 1).await?.len(), 1);
    assert_eq!(harness.queued_count(project).await?, 1);
    assert!(
        timeout(
            Duration::from_millis(250),
            supervisor.next_provision_for_task(second),
        )
        .await
        .is_err(),
        "the only Employee must not receive a concurrent second Run"
    );

    drop(supervisor);
    harness.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; run `just test-integration`"]
async fn m0_dependency_wait_is_released_only_after_the_blocker_completes() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let harness = M0Harness::start().await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    let project = harness.create_project("M0 dependency").await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    harness
        .create_employee(project, "dependency worker")
        .await?;
    let blocker = harness.create_task(project, pipeline, "blocker").await?;
    let blocked = harness.create_task(project, pipeline, "blocked").await?;
    harness.create_dependency(project, blocker, blocked).await?;
    harness.approve_task(project, blocked).await?;
    harness.wait_for_task(blocked, has_dependency_wait).await?;
    harness.approve_task(project, blocker).await?;
    harness.start_project(project).await?;

    drive_employee_stage(&harness, &mut supervisor, blocker, "completed").await?;
    drive_employee_stage(&harness, &mut supervisor, blocked, "completed").await?;

    harness
        .wait_for_lifecycle(blocker, LifecycleStatus::Done)
        .await?;
    harness
        .wait_for_lifecycle(blocked, LifecycleStatus::Done)
        .await?;
    assert_eq!(harness.store.list_runs(blocker).await?.len(), 1);
    assert_eq!(harness.store.list_runs(blocked).await?.len(), 1);
    assert_events(
        &event_types(&harness, project).await?,
        ["task_waiting", "task_resumed", "task_completed"],
    );

    drop(supervisor);
    harness.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; run `just test-integration`"]
async fn m0_project_stop_pauses_active_work_without_claiming_the_next_task() -> Result<()> {
    let harness = M0Harness::start().await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    let project = harness.create_project("M0 project stop").await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    harness.create_employee(project, "stop worker").await?;
    let active = harness
        .create_task(project, pipeline, "active task")
        .await?;
    let queued = harness
        .create_task(project, pipeline, "queued task")
        .await?;
    harness.approve_task(project, active).await?;
    harness.approve_task(project, queued).await?;
    harness.start_project(project).await?;

    let provision = supervisor.next_provision_for_task(active).await?;
    let active_run = harness.wait_for_run_count(active, 1).await?.remove(0);
    assert_eq!(provision.run_id, active_run.id.to_string());
    harness.stop_project(project).await?;
    let stop = supervisor
        .next_stop_for_run(&active_run.id.to_string())
        .await?;
    assert_eq!(stop.run_id, active_run.id.to_string());
    assert_eq!(stop.lease_fencing_token, active_run.lease_fencing_token);
    assert_eq!(stop.environment_epoch, active_run.environment_epoch);
    assert_accepted(
        "late running observation after stop",
        supervisor
            .send_observation_kind(
                &active_run,
                active_run.lease_fencing_token,
                1,
                RunEventKind::Running,
            )
            .await?,
    )?;
    let stopped_run = harness
        .store
        .load_run(active_run.id)
        .await?
        .context("stopped Run is missing")?;
    assert_eq!(stopped_run.desired_state, RunDesiredState::StopRequested);
    let (_, late_submission) = supervisor.submit_stage_evidence(&active_run, 2).await?;
    assert_ne!(
        late_submission.disposition,
        AcknowledgementDisposition::Accepted as i32,
        "a Run with committed stop intent must reject executor work"
    );
    harness.wait_for_task(active, has_project_stop_wait).await?;
    harness
        .wait_for_lifecycle(queued, LifecycleStatus::Ready)
        .await?;
    assert_eq!(harness.store.list_runs_for_project(project).await?.len(), 1);
    assert_eq!(harness.queued_count(project).await?, 1);
    assert!(
        timeout(
            Duration::from_millis(250),
            supervisor.next_provision_for_task(queued),
        )
        .await
        .is_err(),
        "closed Project gate must prevent new Run provisioning"
    );

    drop(supervisor);
    harness.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; run `just test-integration`"]
async fn m0_priority_change_does_not_duplicate_a_leased_queue_entry() -> Result<()> {
    let harness = M0Harness::start().await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    let project = harness.create_project("M0 leased priority").await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    harness.create_employee(project, "priority worker").await?;
    let leased = harness
        .create_task(project, pipeline, "leased task")
        .await?;
    harness.approve_task(project, leased).await?;
    harness.start_project(project).await?;
    supervisor.next_provision_for_task(leased).await?;
    harness.wait_for_run_count(leased, 1).await?;
    assert_eq!(harness.queue_entry_count_for_task(leased).await?, 1);

    harness.set_task_priority(project, leased, "high").await?;
    assert_eq!(
        harness.queue_entry_count_for_task(leased).await?,
        1,
        "priority updates must not add a replacement queue entry while a Run owns the lease"
    );

    let ready_project = harness.create_project("M0 ready priority").await?;
    let ready_pipeline = harness
        .create_pipeline(ready_project, single_stage_pipeline())
        .await?;
    harness
        .create_employee(ready_project, "ready priority worker")
        .await?;
    let normal = harness
        .create_task(ready_project, ready_pipeline, "normal task")
        .await?;
    let elevated = harness
        .create_task(ready_project, ready_pipeline, "elevated task")
        .await?;
    harness.approve_task(ready_project, normal).await?;
    harness.approve_task(ready_project, elevated).await?;
    harness
        .set_task_priority(ready_project, elevated, "high")
        .await?;
    harness.start_project(ready_project).await?;
    let provision = supervisor.next_provision_for_task(elevated).await?;
    assert_eq!(provision.task_id, elevated.as_uuid().to_string());

    drop(supervisor);
    harness.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; run `just test-integration`"]
async fn m0_stale_fence_message_is_ignored_without_mutating_the_run() -> Result<()> {
    let harness = M0Harness::start().await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    let project = harness.create_project("M0 stale fence").await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    harness.create_employee(project, "fence worker").await?;
    let task = harness
        .create_task(project, pipeline, "fenced task")
        .await?;
    harness.approve_task(project, task).await?;
    harness.start_project(project).await?;

    let _ = supervisor.next_provision_for_task(task).await?;
    let run = harness.wait_for_run_count(task, 1).await?.remove(0);
    let acknowledgement = supervisor
        .send_observation(&run, run.lease_fencing_token.saturating_add(1), 1)
        .await?;
    let persisted = harness.store.load_run(run.id).await?.expect("Run persists");
    let ignored_audit = harness
        .store
        .list_events(project, None, 1_000)
        .await?
        .into_iter()
        .find(|event| event.event_type == "run_observation_ignored")
        .context("stale fence observation must leave a durable audit Event")?;

    assert_eq!(
        acknowledgement.disposition,
        AcknowledgementDisposition::IgnoredStale as i32
    );
    assert_eq!(
        acknowledgement.reason_code,
        "stale_or_duplicate_run_message"
    );
    assert_eq!(persisted.last_sequence, 0);
    assert_eq!(persisted.observed_state, RunObservedState::Unknown);
    assert_eq!(
        ignored_audit.payload["reason_code"],
        "stale_fence_or_environment_epoch"
    );
    assert_eq!(ignored_audit.payload["run_id"], run.id.to_string());
    assert_eq!(
        ignored_audit.payload["expected_lease_fencing_token"],
        run.lease_fencing_token
    );
    let audit_outbox_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM outbox WHERE event_id = $1)")
            .bind(ignored_audit.id.as_uuid())
            .fetch_one(&harness.pool)
            .await?;
    assert!(
        audit_outbox_exists,
        "stale-observation audit must enter outbox atomically"
    );

    drop(supervisor);
    harness.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; run `just test-integration`"]
async fn m0_retry_exhaustion_waits_for_management_then_can_be_cancelled() -> Result<()> {
    let harness = M0Harness::start().await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    let project = harness.create_project("M0 retry exhaustion").await?;
    let pipeline = harness.create_pipeline(project, retry_pipeline()).await?;
    harness.create_employee(project, "retry worker").await?;
    let task = harness.create_task(project, pipeline, "retry task").await?;
    harness.approve_task(project, task).await?;
    harness.start_project(project).await?;

    drive_employee_stage(&harness, &mut supervisor, task, "again").await?;
    drive_employee_stage(&harness, &mut supervisor, task, "again").await?;

    harness
        .wait_for_task(task, has_retry_exhausted_wait)
        .await?;
    assert_eq!(harness.wait_for_run_count(task, 2).await?.len(), 2);
    assert_events(
        &event_types(&harness, project).await?,
        ["task_retry_exhausted", "task_waiting"],
    );
    harness.cancel_task(project, task).await?;
    harness
        .wait_for_lifecycle(task, LifecycleStatus::Cancelled)
        .await?;

    drop(supervisor);
    harness.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; run `just test-integration`"]
async fn m0_human_retry_exhaustion_emits_the_same_wait_audit() -> Result<()> {
    let harness = M0Harness::start().await?;
    let project = harness.create_project("M0 human retry exhaustion").await?;
    let pipeline = harness
        .create_pipeline(project, human_retry_pipeline())
        .await?;
    let task = harness
        .create_task(project, pipeline, "human retry task")
        .await?;
    harness.approve_task(project, task).await?;

    harness
        .submit_external_stage_outcome(project, task, "again")
        .await?;
    harness
        .submit_external_stage_outcome(project, task, "again")
        .await?;

    harness
        .wait_for_task(task, has_retry_exhausted_wait)
        .await?;
    assert_events(
        &event_types(&harness, project).await?,
        ["task_retry_exhausted", "task_waiting"],
    );

    harness.shutdown().await;
    Ok(())
}

async fn drive_employee_stage(
    harness: &M0Harness,
    supervisor: &mut forge_testkit::m0::ManualSupervisor,
    task: forge_domain::TaskId,
    outcome: &str,
) -> Result<()> {
    let provision = supervisor.next_provision_for_task(task).await?;
    let run_id = Uuid::parse_str(&provision.run_id).context("parse fixture Run identity")?;
    let run = harness
        .store
        .load_run(run_id)
        .await?
        .context("fixture Run is missing")?;
    assert_eq!(run.require_task_id()?, task);
    assert_accepted(
        "provisioning observation",
        supervisor
            .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Provisioning)
            .await?,
    )?;
    assert_accepted(
        "running observation",
        supervisor
            .send_observation_kind(&run, run.lease_fencing_token, 2, RunEventKind::Running)
            .await?,
    )?;
    let (evidence_message_id, acknowledgement) = supervisor.submit_stage_evidence(&run, 3).await?;
    assert_accepted("stage evidence", acknowledgement)?;
    assert_accepted(
        "stage outcome",
        supervisor
            .submit_stage_outcome(&run, 4, outcome, vec![evidence_message_id])
            .await?,
    )?;
    assert_accepted(
        "stopped observation",
        supervisor
            .send_observation_kind(&run, run.lease_fencing_token, 5, RunEventKind::Stopped)
            .await?,
    )
}

fn assert_accepted(label: &str, acknowledgement: CoreAcknowledgement) -> Result<()> {
    if acknowledgement.disposition != AcknowledgementDisposition::Accepted as i32 {
        bail!("Core rejected {label}: {acknowledgement:?}")
    }
    Ok(())
}

async fn event_types(harness: &M0Harness, project: ProjectId) -> Result<BTreeSet<String>> {
    Ok(harness
        .store
        .list_events(project, None, 1_000)
        .await?
        .into_iter()
        .map(|event| event.event_type)
        .collect())
}

fn assert_events<const N: usize>(actual: &BTreeSet<String>, expected: [&str; N]) {
    for event_type in expected {
        assert!(
            actual.contains(event_type),
            "missing event {event_type}; actual: {actual:?}"
        );
    }
}

fn has_dependency_wait(task: &Task) -> bool {
    task.lifecycle() == LifecycleStatus::Waiting
        && task
            .wait_conditions()
            .any(|condition| condition.kind() == &TaskWaitKind::Dependency)
}

fn has_project_stop_wait(task: &Task) -> bool {
    task.lifecycle() == LifecycleStatus::Waiting
        && task.wait_conditions().any(|condition| {
            condition.kind() == &TaskWaitKind::ManualPause
                && condition.detail() == Some("project_execution_stopped")
        })
}

fn has_retry_exhausted_wait(task: &Task) -> bool {
    task.lifecycle() == LifecycleStatus::Waiting
        && task
            .wait_conditions()
            .any(|condition| condition.kind() == &TaskWaitKind::RetryExhausted)
}

fn has_interrupted_wait(task: &Task) -> bool {
    task.lifecycle() == LifecycleStatus::Waiting
        && task
            .wait_conditions()
            .any(|condition| condition.kind() == &TaskWaitKind::Interrupted)
}
