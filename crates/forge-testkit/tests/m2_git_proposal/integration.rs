use super::*;
use forge_domain::{
    ProjectId, TaskId, Timestamp,
    git::{GitIntegrationIntent, GitObjectId, LocalGitPath},
    git_integration::{IntegrationCode, IntegrationPhase, IntegrationRequest, IntegrationResult},
};
use forge_testkit::m0::ManualSupervisor;

pub(super) fn pipeline(mode: &str) -> Value {
    let missing = mode == "integration_missing_review";
    let mut stages = vec![
        json!({"id":"work","name":"Implementation","executor_kind":"employee","outcomes":["completed"],"workspace":{"kind":"git","access":"read_write"}}),
        json!({"id":"publish_here","name":"Owner-defined integration","executor_kind":"system","outcomes":["landed","nothing","refresh"],
            "system_action":{"kind":"git_integration","outcomes":{"applied":"landed","no_changes":"nothing","stale_base":"refresh"},"required_review_stages":if missing {vec!["assessment"]}else{vec![]}}}),
    ];
    let mut transitions = vec![
        json!({"from_stage_id":"work","outcome":"completed","target":{"kind":"stage","stage_id":"publish_here"},"artifact_requirements":[{"kind":"stage_evidence","minimum_count":1}]}),
        json!({"from_stage_id":"publish_here","outcome":"landed","target":{"kind":"done"}}),
        json!({"from_stage_id":"publish_here","outcome":"nothing","target":{"kind":"done"}}),
        json!({"from_stage_id":"publish_here","outcome":"refresh","target":{"kind":"stage","stage_id":"work"}}),
    ];
    if missing {
        stages[0]["outcomes"] = json!(["completed", "examine"]);
        stages.push(json!({"id":"assessment","name":"Optional route but required integration gate","executor_kind":"employee","outcomes":["yes"],"workspace":{"kind":"git","access":"read_only"},"acceptance_policy":{"kind":"candidate_review","verdicts":{"yes":"accepted"}}}));
        transitions.push(json!({"from_stage_id":"work","outcome":"examine","target":{"kind":"stage","stage_id":"assessment"}}));
        transitions.push(json!({"from_stage_id":"assessment","outcome":"yes","target":{"kind":"stage","stage_id":"publish_here"}}));
    }
    json!({"name":"Explicit local integration","task_kinds":["delivery"],"entry_stage_id":"work","max_stage_visits":8,"stages":stages,"transitions":transitions})
}
async fn tick(harness: &M0Harness) -> Result<()> {
    // The binary runs maintenance in its own Tokio task. Match that boundary
    // instead of nesting the whole controller under every scenario poll frame.
    let core = harness.core.clone();
    tokio::spawn(async move {
        core.watchdog_tick(
            Timestamp::now_utc(),
            forge_core::WatchdogDeadlines::default(),
        )
        .await
    })
    .await??;
    Ok(())
}
fn reply(
    request: &IntegrationRequest,
    code: IntegrationCode,
    intent: Option<GitIntegrationIntent>,
) -> IntegrationResult {
    IntegrationResult {
        message_id: Uuid::now_v7(),
        command_id: request.command_id,
        operation_id: request.operation.id,
        fence: request.operation.fence,
        host_id: request.host_id.clone(),
        boot_id: request.boot_id.clone(),
        code,
        intent,
    }
}
async fn accepted(supervisor: &mut ManualSupervisor, result: IntegrationResult) -> Result<()> {
    let ack = supervisor.send_git_integration(result).await?;
    assert_eq!(
        ack.disposition,
        AcknowledgementDisposition::Accepted as i32,
        "{ack:?}"
    );
    Ok(())
}

pub(super) async fn finish(
    harness: &M0Harness,
    supervisor: &mut ManualSupervisor,
    project: ProjectId,
    task: TaskId,
    mode: &str,
) -> Result<()> {
    tick(harness).await?;
    assert_eq!(
        harness.store.list_runs(task).await?.len(),
        1,
        "System integration never fabricates an Employee Run"
    );
    if mode == "integration_missing_review" {
        let stored = harness.store.load_task(task).await?.context("Task")?;
        assert_eq!(stored.task.lifecycle(), LifecycleStatus::Waiting);
        let mut tx = harness.store.begin().await?;
        let operation = tx
            .integration_for_task(project, task)
            .await?
            .context("held operation")?;
        assert_eq!(operation.state, forge_storage::IntegrationState::Held);
        assert!(operation.command.is_none());
        tx.commit().await?;
        return Ok(());
    }
    let mut prepare = supervisor.next_git_integration().await?;
    assert_eq!(prepare.phase, IntegrationPhase::Prepare);
    if matches!(
        mode,
        "integration_retry_unprepared" | "integration_unknown_unprepared"
    ) && !retry_unprepared(harness, supervisor, project, task, &mut prepare, mode).await?
    {
        return Ok(());
    }
    if mode == "integration_stale" {
        accepted(
            supervisor,
            reply(&prepare, IntegrationCode::StaleBase, None),
        )
        .await?;
        assert_eq!(
            harness
                .store
                .load_task(task)
                .await?
                .context("Task")?
                .task
                .current_stage_id()
                .context("stage")?
                .as_str(),
            "work"
        );
        harness.core.dispatch_available(project).await?;
        supervisor.next_provision_for_task(task).await?;
        return Ok(());
    }
    let intent = intent(&prepare)?;
    let mut apply = prepare_apply(harness, supervisor, project, &prepare, &intent).await?;
    if mode == "integration_retry_prepared" {
        apply = retry_prepared(harness, supervisor, project, task, &apply, &intent).await?;
    }
    observe_applied(harness, supervisor, project, task, &apply, &intent, mode).await?;
    assert_eq!(
        harness
            .store
            .load_task(task)
            .await?
            .context("Task")?
            .task
            .lifecycle(),
        LifecycleStatus::Done
    );
    assert_eq!(harness.store.list_runs(task).await?.len(), 1);
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM artifacts WHERE task_id=$1 AND kind='integration_result'",
    )
    .bind(task.as_uuid())
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(
        count, 1,
        "result replay must not duplicate System artifacts"
    );
    Ok(())
}

fn intent(prepare: &IntegrationRequest) -> Result<GitIntegrationIntent> {
    Ok(GitIntegrationIntent {
        operation_id: prepare.operation.id,
        target_repository: LocalGitPath::new(
            prepare
                .operation
                .binding
                .source
                .as_path()
                .join(".git")
                .to_string_lossy()
                .into_owned(),
        )?,
        target_ref: prepare.operation.binding.target_ref.clone(),
        expected_target: GitObjectId::new("a".repeat(40))?,
        candidate: prepare.operation.candidate.clone(),
        merge_commit: GitObjectId::new("d".repeat(40))?,
    })
}
async fn prepare_apply(
    harness: &M0Harness,
    supervisor: &mut ManualSupervisor,
    project: ProjectId,
    prepare: &IntegrationRequest,
    intent: &GitIntegrationIntent,
) -> Result<IntegrationRequest> {
    accepted(
        supervisor,
        reply(prepare, IntegrationCode::Prepared, Some(intent.clone())),
    )
    .await?;
    let mut tx = harness.store.begin().await?;
    assert_eq!(
        tx.load_git_integration(project, prepare.operation.id)
            .await?
            .context("operation")?
            .intent,
        Some(intent.clone()),
        "exact T/H/M is durable BEFORE apply delivery"
    );
    tx.commit().await?;
    tick(harness).await?;
    let apply = supervisor.next_git_integration().await?;
    assert_eq!(apply.phase, IntegrationPhase::Apply);
    assert_eq!(apply.intent, Some(intent.clone()));
    Ok(apply)
}
async fn retry_unprepared(
    harness: &M0Harness,
    supervisor: &mut ManualSupervisor,
    project: ProjectId,
    task: TaskId,
    prepare: &mut IntegrationRequest,
    mode: &str,
) -> Result<bool> {
    accepted(supervisor, reply(prepare, IntegrationCode::Refused, None)).await?;
    resume_and_resolve(
        harness,
        project,
        task,
        prepare.operation.id,
        CommandName::RetryGitIntegration,
    )
    .await?;
    tick(harness).await?;
    let reconcile = supervisor.next_git_integration().await?;
    assert_eq!(reconcile.phase, IntegrationPhase::Reconcile);
    assert!(reconcile.intent.is_none());
    let code = if mode == "integration_unknown_unprepared" {
        IntegrationCode::Unknown
    } else {
        IntegrationCode::NoPreparation
    };
    accepted(supervisor, reply(&reconcile, code, None)).await?;
    let mut tx = harness.store.begin().await?;
    let prior = tx
        .load_git_integration(project, prepare.operation.id)
        .await?
        .context("prior operation")?;
    tx.commit().await?;
    if code == IntegrationCode::Unknown {
        assert_eq!(prior.state, forge_storage::IntegrationState::Held);
        return Ok(false);
    }
    assert_eq!(prior.state, forge_storage::IntegrationState::Retired);
    tick(harness).await?;
    let replacement = supervisor.next_git_integration().await?;
    assert_eq!(replacement.phase, IntegrationPhase::Prepare);
    assert_ne!(replacement.operation.id, prepare.operation.id);
    assert!(replacement.operation.fence > prepare.operation.fence);
    *prepare = replacement;
    Ok(true)
}
async fn retry_prepared(
    harness: &M0Harness,
    supervisor: &mut ManualSupervisor,
    project: ProjectId,
    task: TaskId,
    apply: &IntegrationRequest,
    intent: &GitIntegrationIntent,
) -> Result<IntegrationRequest> {
    accepted(supervisor, reply(apply, IntegrationCode::Unavailable, None)).await?;
    resume_and_resolve(
        harness,
        project,
        task,
        apply.operation.id,
        CommandName::RetryGitIntegration,
    )
    .await?;
    tick(harness).await?;
    let reconcile = supervisor.next_git_integration().await?;
    assert_eq!(reconcile.phase, IntegrationPhase::Reconcile);
    assert_eq!(reconcile.intent, Some(intent.clone()));
    let invalid = supervisor
        .send_git_integration(reply(&reconcile, IntegrationCode::NoPreparation, None))
        .await?;
    assert_ne!(
        invalid.disposition,
        AcknowledgementDisposition::Accepted as i32,
        "a durable apply permit can never become no preparation"
    );
    accepted(
        supervisor,
        reply(&reconcile, IntegrationCode::Retryable, Some(intent.clone())),
    )
    .await?;
    tick(harness).await?;
    let retry = supervisor.next_git_integration().await?;
    assert_eq!(retry.phase, IntegrationPhase::Apply);
    assert_eq!(retry.operation, apply.operation);
    assert_eq!(retry.intent, apply.intent);
    assert_ne!(retry.command_id, apply.command_id);
    Ok(retry)
}
async fn observe_applied(
    harness: &M0Harness,
    supervisor: &mut ManualSupervisor,
    project: ProjectId,
    task: TaskId,
    apply: &IntegrationRequest,
    intent: &GitIntegrationIntent,
    mode: &str,
) -> Result<()> {
    if mode == "integration_stop" {
        harness.stop_project(project).await?;
    }
    if mode == "integration_restart" {
        sqlx::query("UPDATE git_integrations SET core_instance=$2 WHERE id=$1")
            .bind(apply.operation.id)
            .bind(Uuid::now_v7())
            .execute(&harness.pool)
            .await?;
    }
    let applied = reply(apply, IntegrationCode::Applied, Some(intent.clone()));
    accepted(supervisor, applied.clone()).await?;
    accepted(supervisor, applied).await?;
    if matches!(mode, "integration_stop" | "integration_restart") {
        if mode == "integration_stop" {
            harness.start_project(project).await?;
        }
        accept_held(
            harness,
            supervisor,
            project,
            task,
            apply.operation.id,
            intent,
        )
        .await?;
    }
    Ok(())
}
async fn accept_held(
    harness: &M0Harness,
    supervisor: &mut ManualSupervisor,
    project: ProjectId,
    task: TaskId,
    operation: Uuid,
    intent: &GitIntegrationIntent,
) -> Result<()> {
    let held = harness
        .store
        .load_task(task)
        .await?
        .context("held Task")?
        .task;
    assert_eq!(held.lifecycle(), LifecycleStatus::Waiting);
    let wait = held
        .wait_conditions()
        .next()
        .context("integration wait")?
        .id();
    harness.execute(project,CommandName::ResumeTask,json!({"task_id":task,"expected_task_revision":held.revision().get(),"wait_condition_id":wait})).await?;
    tick(harness).await?;
    let mut tx = harness.store.begin().await?;
    assert_eq!(
        tx.integration_for_task(project, task)
            .await?
            .context("retained held action")?
            .state,
        forge_storage::IntegrationState::Held
    );
    tx.commit().await?;
    let current = harness.store.load_task(task).await?.context("Task")?.task;
    harness.execute(project,CommandName::AcceptGitIntegrationResult,json!({"operation_id":operation,"expected_task_revision":current.revision().get(),"reason":"Explicitly accept recorded physical result"})).await?;
    tick(harness).await?;
    let reconcile = supervisor.next_git_integration().await?;
    assert_eq!(
        reconcile.phase,
        IntegrationPhase::Reconcile,
        "accept must never repeat CAS"
    );
    accepted(
        supervisor,
        reply(&reconcile, IntegrationCode::Applied, Some(intent.clone())),
    )
    .await?;
    Ok(())
}

async fn resume_and_resolve(
    harness: &M0Harness,
    project: ProjectId,
    task: TaskId,
    operation: Uuid,
    command: CommandName,
) -> Result<()> {
    let held = harness
        .store
        .load_task(task)
        .await?
        .context("held Task")?
        .task;
    let wait = held
        .wait_conditions()
        .next()
        .context("integration wait")?
        .id();
    harness.execute(project,CommandName::ResumeTask,json!({"task_id":task,"expected_task_revision":held.revision().get(),"wait_condition_id":wait})).await?;
    let current = harness.store.load_task(task).await?.context("Task")?.task;
    harness.execute(project,command,json!({"operation_id":operation,"expected_task_revision":current.revision().get(),"reason":"Operator repaired the explicitly configured local target"})).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires PG/NATS; synthetic Integration peer, no model"]
async fn integration_persists_intent_before_apply_and_completes_without_employee_run() -> Result<()>
{
    scenario("integration_applied").await
}
#[tokio::test]
#[ignore = "requires PG/NATS; synthetic Integration peer, no model"]
async fn integration_stop_retains_applied_fact_and_requires_named_accept_with_fresh_reconcile()
-> Result<()> {
    scenario("integration_stop").await
}
#[tokio::test]
#[ignore = "requires PG/NATS; synthetic previous-Core ownership, no model"]
async fn integration_previous_core_owner_cannot_silently_apply_domain_outcome() -> Result<()> {
    scenario("integration_restart").await
}
#[tokio::test]
#[ignore = "requires PG/NATS; synthetic Integration peer, no model"]
async fn integration_stale_base_returns_same_task_to_declared_employee_stage() -> Result<()> {
    scenario("integration_stale").await
}
#[tokio::test]
#[ignore = "requires PG/NATS; no model"]
async fn integration_does_not_infer_acceptance_when_required_review_route_was_skipped() -> Result<()>
{
    scenario("integration_missing_review").await
}
#[tokio::test]
#[ignore = "requires PG/NATS; synthetic Integration peer, no model"]
async fn integration_retries_only_the_same_prepared_merge_after_fresh_read_only_reconcile()
-> Result<()> {
    scenario("integration_retry_prepared").await
}
#[tokio::test]
#[ignore = "requires PG/NATS; synthetic Integration peer, no model"]
async fn integration_explicit_retry_can_retire_proven_unprepared_attempt_and_fence_replacement()
-> Result<()> {
    scenario("integration_retry_unprepared").await
}
#[tokio::test]
#[ignore = "requires PG/NATS; synthetic Integration peer, no model"]
async fn integration_unknown_without_intent_is_not_no_preparation() -> Result<()> {
    scenario("integration_unknown_unprepared").await
}
