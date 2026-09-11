//! Post-commit wake failures and replay use the real Core/gRPC boundary, not a manual dispatch.
use super::*;
use forge_storage::{IntegrationState, RunDesiredState, RunObservedState};

pub(super) async fn finish(
    harness: &M0Harness,
    supervisor: &mut ManualSupervisor,
    project: ProjectId,
    task: TaskId,
    prepare: &IntegrationRequest,
    mode: &str,
) -> Result<()> {
    super::super::source_policy::before_stale(harness, project, task, mode).await?;
    let runs = harness.store.list_runs(task).await?;
    assert_eq!(runs.len(), 1);
    assert!(
        runs.iter()
            .all(|run| run.observed_state == RunObservedState::Stopped),
        "The last Run must already be stopped before integration queues rework"
    );
    let failed_dispatch = matches!(
        mode,
        "integration_stale_dispatch_failure" | "integration_stale_replay_stopped"
    );
    if failed_dispatch {
        install_dispatch_fault(harness).await?;
    }
    if mode == "integration_stale_stopped" {
        harness.stop_project(project).await?;
    }
    let result = reply(prepare, IntegrationCode::StaleBase, None);
    accepted(supervisor, result.clone()).await?;
    if mode == "integration_stale_stopped" {
        return assert_stopped_result(harness, supervisor, project, task, result).await;
    }
    if mode == "integration_pinned_stale" {
        return super::super::source_policy::assert_held(harness, project, task).await;
    }
    let current = harness.store.load_task(task).await?.context("Task")?.task;
    assert_eq!(current.lifecycle(), LifecycleStatus::InProgress);
    assert_eq!(
        current.current_stage_id().context("stage")?.as_str(),
        "work"
    );
    if failed_dispatch {
        return recover_failed_dispatch(
            harness,
            supervisor,
            project,
            task,
            result,
            mode == "integration_stale_replay_stopped",
        )
        .await;
    }
    // There are no live Runs left to supply another terminal observation or wake.
    let next = supervisor.next_provision_for_task(task).await?;
    if mode == "integration_future_source" {
        let next: Value = serde_json::from_str(&next.run_spec_json)?;
        assert_eq!(next["source_request"]["policy_revision"], 3);
        assert_eq!(next["source_request"]["policy"]["mode"], "latest_target");
        let first = &runs[0];
        assert_eq!(first.run_spec["source_request"]["policy_revision"], 1);
        assert!(harness.store.run_diagnostics(first.id).await?["git_source"].is_object());
    }
    Ok(())
}

async fn install_dispatch_fault(harness: &M0Harness) -> Result<()> {
    let schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(&harness.pool)
        .await?;
    anyhow::ensure!(
        schema.strip_prefix("forge_test_").is_some_and(|suffix| {
            suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
        }),
        "failure injection requires this harness's private test schema"
    );
    // DDL only in the generated fixture schema. Canonical integration state is
    // still changed exclusively by the gRPC receipt below.
    sqlx::query("ALTER TABLE runs ADD CONSTRAINT integration_rework_dispatch_fault CHECK (attempt_number < 2)")
        .execute(&harness.pool).await?;
    Ok(())
}

async fn recover_failed_dispatch(
    harness: &M0Harness,
    supervisor: &mut ManualSupervisor,
    project: ProjectId,
    task: TaskId,
    result: IntegrationResult,
    stop_before_replay: bool,
) -> Result<()> {
    assert_eq!(harness.store.list_runs(task).await?.len(), 1);
    let queue: Vec<(String, i32)> = sqlx::query_as(
        "SELECT queue_state,attempt_number FROM queue_entries WHERE task_id=$1 AND queue_state IN ('queued','leased')",
    ).bind(task.as_uuid()).fetch_all(&harness.pool).await?;
    assert_eq!(queue, vec![("queued".into(), 2)]);
    let (leases, active, orphaned): (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*),count(*) FILTER(WHERE l.lease_state='active'),count(*) FILTER(WHERE NOT EXISTS(SELECT 1 FROM runs r WHERE r.lease_id=l.id)) FROM leases l WHERE l.task_id=$1",
    ).bind(task.as_uuid()).fetch_one(&harness.pool).await?;
    assert_eq!((leases, active, orphaned), (1, 0, 0));
    let reservations: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_environment_reservations WHERE task_id=$1 AND released_at IS NULL",
    ).bind(task.as_uuid()).fetch_one(&harness.pool).await?;
    assert_eq!(reservations, 0);
    let mut tx = harness.store.begin().await?;
    let integration = tx
        .load_git_integration(project, result.operation_id)
        .await?
        .context("committed integration")?;
    assert_eq!(integration.state, IntegrationState::Completed);
    assert_eq!(integration.result, Some(result.clone()));
    assert_eq!(
        tx.integration_receipt(result.command_id)
            .await?
            .context("receipt")?
            .1,
        Some(result.clone())
    );
    tx.commit().await?;
    let effects = integration_effects(harness, task, &result).await?;
    assert_eq!(effects, (1, 1, 1));

    // Repair only the fixture fault: a Core management command would itself
    // dispatch and conceal a missing wake in the identical-receipt path.
    sqlx::query("ALTER TABLE runs DROP CONSTRAINT integration_rework_dispatch_fault")
        .execute(&harness.pool)
        .await?;
    if stop_before_replay {
        harness.stop_project(project).await?;
        let before = canonical_state(harness, project, task).await?;
        accepted(supervisor, result).await?;
        assert_eq!(
            canonical_state(harness, project, task).await?,
            before,
            "Replay must not wake already-queued rework after the Project was stopped"
        );
        let enabled: bool =
            sqlx::query_scalar("SELECT execution_enabled FROM projects WHERE id=$1")
                .bind(project.as_uuid())
                .fetch_one(&harness.pool)
                .await?;
        assert!(!enabled);
        assert_eq!(harness.store.list_runs(task).await?.len(), 1);
        return Ok(());
    }
    let before = canonical_state(harness, project, task).await?;
    let mut conflicting = result.clone();
    conflicting.code = IntegrationCode::NoChanges;
    let rejected = supervisor.send_git_integration(conflicting).await?;
    assert_eq!(
        rejected.disposition,
        AcknowledgementDisposition::Rejected as i32
    );
    assert_eq!(rejected.reason_code, "idempotency_conflict");
    assert_eq!(
        canonical_state(harness, project, task).await?,
        before,
        "A conflicting receipt must not mutate state or wake the now-dispatchable queue"
    );

    accepted(supervisor, result.clone()).await?;
    let provision = supervisor.next_provision_for_task(task).await?;
    let runs = harness.store.list_runs(task).await?;
    assert_eq!(runs.len(), 2);
    let next = runs
        .iter()
        .find(|run| run.id.to_string() == provision.run_id)
        .context("rework Run")?;
    assert_eq!(next.attempt_number, 2);
    assert_eq!(next.require_task_stage()?.stage_id.as_str(), "work");
    assert_eq!(next.desired_state, RunDesiredState::ProvisionRequested);
    assert_eq!(next.observed_state, RunObservedState::Unknown);
    assert_eq!(integration_effects(harness, task, &result).await?, effects);
    let before = canonical_state(harness, project, task).await?;
    accepted(supervisor, result).await?;
    assert_eq!(
        canonical_state(harness, project, task).await?,
        before,
        "A third identical receipt must not duplicate the Run, artifacts, handoff or stage events"
    );
    Ok(())
}

async fn integration_effects(
    harness: &M0Harness,
    task: TaskId,
    result: &IntegrationResult,
) -> Result<(i64, i64, i64)> {
    Ok(sqlx::query_as(
        "SELECT (SELECT count(*) FROM artifacts WHERE task_id=$1 AND kind='integration_result'),(SELECT count(*) FROM event_log WHERE aggregate_id=$1 AND event_type='task_stage_advanced' AND command_id=$2),(SELECT count(*) FROM task_handoffs WHERE task_id=$1 AND integration_id=$2)",
    ).bind(task.as_uuid()).bind(result.operation_id).fetch_one(&harness.pool).await?)
}

async fn assert_stopped_result(
    harness: &M0Harness,
    supervisor: &mut ManualSupervisor,
    project: ProjectId,
    task: TaskId,
    result: IntegrationResult,
) -> Result<()> {
    let held = harness
        .store
        .load_task(task)
        .await?
        .context("held Task")?
        .task;
    assert_eq!(held.lifecycle(), LifecycleStatus::Waiting);
    assert_eq!(
        held.current_stage_id().context("stage")?.as_str(),
        "publish_here"
    );
    let mut tx = harness.store.begin().await?;
    let integration = tx
        .load_git_integration(project, result.operation_id)
        .await?
        .context("held integration")?;
    assert_eq!(integration.state, IntegrationState::Held);
    assert_eq!(integration.result, Some(result.clone()));
    assert!(!tx.integration_dispatch_open(project, task).await?);
    tx.commit().await?;
    assert_eq!(harness.store.list_runs(task).await?.len(), 1);
    let queued: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM queue_entries WHERE task_id=$1 AND queue_state IN ('queued','leased')",
    ).bind(task.as_uuid()).fetch_one(&harness.pool).await?;
    assert_eq!(queued, 0);
    assert_eq!(
        integration_effects(harness, task, &result).await?,
        (0, 0, 0)
    );
    let before = canonical_state(harness, project, task).await?;
    accepted(supervisor, result).await?;
    assert_eq!(
        canonical_state(harness, project, task).await?,
        before,
        "A replay must retain the fact without bypassing the stopped Project gate"
    );
    Ok(())
}

async fn canonical_state(harness: &M0Harness, project: ProjectId, task: TaskId) -> Result<Value> {
    Ok(sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
            'project',(SELECT canonical_snapshot FROM projects WHERE id=$2),
            'task',(SELECT canonical_snapshot FROM tasks WHERE id=$1),
            'runs',(SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM runs r WHERE r.task_id=$1),
            'leases',(SELECT jsonb_agg(to_jsonb(l) ORDER BY l.id) FROM leases l WHERE l.task_id=$1),
            'queue',(SELECT jsonb_agg(to_jsonb(q) ORDER BY q.id) FROM queue_entries q WHERE q.task_id=$1),
            'integrations',(SELECT jsonb_agg(to_jsonb(i) ORDER BY i.id) FROM git_integrations i WHERE i.task_id=$1),
            'receipts',(SELECT jsonb_agg(to_jsonb(r) ORDER BY r.command_id) FROM git_integration_receipts r JOIN git_integrations i ON i.id=r.operation_id WHERE i.task_id=$1),
            'artifacts',(SELECT jsonb_agg(to_jsonb(a) ORDER BY a.id) FROM artifacts a WHERE a.task_id=$1),
            'handoffs',(SELECT jsonb_agg(to_jsonb(h) ORDER BY h.id) FROM task_handoffs h WHERE h.task_id=$1),
            'events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY e.project_sequence) FROM event_log e WHERE e.project_id=$2))"#,
    ).bind(task.as_uuid()).bind(project.as_uuid()).fetch_one(&harness.pool).await?)
}

#[tokio::test]
#[ignore = "requires PG/NATS; isolated schema fault injection and synthetic peer, no model"]
async fn integration_stale_receipt_survives_dispatch_failure_and_replay_wakes_exactly_once()
-> Result<()> {
    scenario("integration_stale_dispatch_failure").await
}

#[tokio::test]
#[ignore = "requires PG/NATS; synthetic Integration peer, no model"]
async fn integration_stale_receipt_cannot_wake_work_in_a_stopped_project() -> Result<()> {
    scenario("integration_stale_stopped").await
}

#[tokio::test]
#[ignore = "requires PG/NATS; isolated schema fault injection and synthetic peer, no model"]
async fn integration_receipt_replay_cannot_dispatch_queued_rework_after_project_stop() -> Result<()>
{
    scenario("integration_stale_replay_stopped").await
}
