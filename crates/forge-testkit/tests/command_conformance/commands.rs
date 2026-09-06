use std::collections::BTreeSet;

use anyhow::{Context, Result};
use forge_domain::{LifecycleStatus, ProjectExecutionGate, TaskId, TaskWaitKind};
use forge_protocol::wire::CommandName;
use forge_testkit::m0::{human_retry_pipeline, single_stage_pipeline};
use serde_json::{Value, json};
use uuid::Uuid;

use super::fixture::{BackendKind, Fixture};

pub async fn all_m0_commands(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let pipeline = fixture.pipeline(single_stage_pipeline()).await?;
    fixture
        .execute(
            CommandName::CreateEmployee,
            json!({
                "name":"Conformance worker","role":"m0_worker","stage_eligibility":{"mode":"any"}
            }),
        )
        .await?;
    let blocker = fixture.create_task(pipeline, "Dependency blocker").await?;
    let task = fixture.create_task(pipeline, "Command table").await?;
    fixture
        .task_command(
            CommandName::AmendDraft,
            task,
            json!({
                "patch":{"title":"Amended command table", "priority":"high"}
            }),
        )
        .await?;
    assert_eq!(
        fixture.task(task).await?.spec().title(),
        "Amended command table"
    );
    fixture
        .execute(CommandName::CreateDependency, edge(blocker, task))
        .await?;
    fixture
        .task_command(CommandName::ApproveTask, task, json!({}))
        .await?;
    let waiting = fixture.task(task).await?;
    assert_eq!(waiting.lifecycle(), LifecycleStatus::Waiting);
    let wait = waiting
        .wait_conditions()
        .find(|wait| wait.kind() == &TaskWaitKind::Dependency)
        .context("dependency wait")?
        .id();
    fixture
        .task_command(
            CommandName::ResumeTask,
            task,
            json!({"wait_condition_id":wait}),
        )
        .await?;
    // Resolving a wait cannot delete the canonical dependency gate itself.
    assert_eq!(fixture.snapshot().await?.rows("task_dependencies").len(), 1);
    fixture
        .execute(
            CommandName::RemoveDependency,
            json!({"blocker_task_id":blocker,"blocked_task_id":task}),
        )
        .await?;
    fixture
        .task_command(
            CommandName::SetTaskPriority,
            task,
            json!({"priority":"low"}),
        )
        .await?;
    assert_eq!(
        fixture.task(task).await?.priority_level_id().as_str(),
        "low"
    );
    fixture
        .execute(
            CommandName::StartProjectExecution,
            json!({"reason":"conformance"}),
        )
        .await?;
    assert_eq!(
        fixture.snapshot().await?.projects[&fixture.project_id].execution_gate(),
        ProjectExecutionGate::Open
    );
    fixture
        .execute(
            CommandName::StopProjectExecution,
            json!({"reason":"operator stop"}),
        )
        .await?;
    assert_eq!(
        fixture.snapshot().await?.projects[&fixture.project_id].execution_gate(),
        ProjectExecutionGate::Stopped
    );
    fixture
        .task_command(
            CommandName::CancelTask,
            task,
            json!({"cancellation_reason_key":"unspecified"}),
        )
        .await?;
    assert_eq!(
        fixture.task(task).await?.lifecycle(),
        LifecycleStatus::Cancelled
    );

    let human = fixture.pipeline(human_evidence_pipeline()).await?;
    let review = fixture.create_task(human, "Human evidence").await?;
    fixture
        .task_command(CommandName::ApproveTask, review, json!({}))
        .await?;
    complete_human(&fixture, review, "accepted").await?;
    assert_eq!(
        fixture.task(review).await?.lifecycle(),
        LifecycleStatus::Done
    );
    let snapshot = fixture.snapshot().await?;
    assert_eq!(snapshot.rows("artifacts").len(), 1);
    snapshot.assert_audit_atomic();
    let actual = snapshot
        .rows("idempotency_keys")
        .iter()
        .map(|record| record["command_name"].as_str().expect("command name"))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual,
        BTreeSet::from([
            "create_project",
            "create_pipeline",
            "create_employee",
            "create_task",
            "amend_draft",
            "approve_task",
            "cancel_task",
            "resume_task",
            "set_task_priority",
            "create_dependency",
            "remove_dependency",
            "start_project_execution",
            "stop_project_execution",
            "submit_external_stage_outcome"
        ])
    );
    Ok(())
}

pub async fn negative_domain_cases(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let pipeline = fixture.pipeline(single_stage_pipeline()).await?;
    let blocker = fixture.create_task(pipeline, "Blocker").await?;
    let blocked = fixture.create_task(pipeline, "Blocked").await?;
    fixture
        .execute(CommandName::CreateDependency, edge(blocker, blocked))
        .await?;
    reject(
        &fixture,
        CommandName::CreateDependency,
        edge(blocked, blocker),
    )
    .await?;
    reject(
        &fixture,
        CommandName::CreateDependency,
        edge(blocked, blocked),
    )
    .await?;
    fixture
        .task_command(CommandName::ApproveTask, blocked, json!({}))
        .await?;
    assert_eq!(
        fixture.task(blocked).await?.lifecycle(),
        LifecycleStatus::Waiting
    );
    assert!(fixture.snapshot().await?.rows("queue_entries").is_empty());
    reject_task(
        &fixture,
        CommandName::AmendDraft,
        blocked,
        json!({"patch":{"title":"Too late"}}),
    )
    .await?;
    reject_task(
        &fixture,
        CommandName::CancelTask,
        blocked,
        json!({"cancellation_reason_key":"unknown"}),
    )
    .await?;
    reject_task(
        &fixture,
        CommandName::SetTaskPriority,
        blocked,
        json!({"priority":"unknown"}),
    )
    .await?;
    reject_task(
        &fixture,
        CommandName::ResumeTask,
        blocked,
        json!({"wait_condition_id":Uuid::now_v7()}),
    )
    .await?;
    fixture
        .task_command(
            CommandName::CancelTask,
            blocker,
            json!({"cancellation_reason_key":"unspecified"}),
        )
        .await?;
    reject_task(
        &fixture,
        CommandName::SetTaskPriority,
        blocker,
        json!({"priority":"high"}),
    )
    .await?;
    assert_eq!(
        fixture.task(blocked).await?.lifecycle(),
        LifecycleStatus::Waiting,
        "cancelled blocker does not satisfy a done dependency"
    );

    let human = fixture.pipeline(human_evidence_pipeline()).await?;
    let task = fixture.create_task(human, "Evidence requirements").await?;
    fixture
        .task_command(CommandName::ApproveTask, task, json!({}))
        .await?;
    let wait = fixture
        .task(task)
        .await?
        .wait_conditions()
        .next()
        .context("stage wait")?
        .id();
    reject_task(
        &fixture,
        CommandName::ResumeTask,
        task,
        json!({"wait_condition_id":wait}),
    )
    .await?;
    for (stage, outcome, evidence) in [
        ("review", "unknown", true),
        ("other", "accepted", true),
        ("review", "accepted", false),
    ] {
        let artifacts = if evidence {
            vec![evidence_artifact()]
        } else {
            Vec::new()
        };
        reject_task(&fixture, CommandName::SubmitExternalStageOutcome, task,
            json!({"stage_id":stage,"outcome":outcome,"wait_condition_id":wait,"artifacts":artifacts})).await?;
    }
    complete_human(&fixture, task, "accepted").await?;
    reject_task(
        &fixture,
        CommandName::CancelTask,
        task,
        json!({"cancellation_reason_key":"unspecified"}),
    )
    .await?;

    let retry = fixture.pipeline(human_retry_pipeline()).await?;
    let retry_task = fixture.create_task(retry, "Finite retry").await?;
    fixture
        .task_command(CommandName::ApproveTask, retry_task, json!({}))
        .await?;
    for _ in 0..2 {
        let wait = fixture
            .task(retry_task)
            .await?
            .wait_conditions()
            .next()
            .context("retry stage wait")?
            .id();
        fixture
            .task_command(
                CommandName::SubmitExternalStageOutcome,
                retry_task,
                json!({"stage_id":"decision","outcome":"again","wait_condition_id":wait}),
            )
            .await?;
    }
    assert!(
        fixture
            .task(retry_task)
            .await?
            .wait_conditions()
            .any(|wait| wait.kind() == &TaskWaitKind::RetryExhausted)
    );
    fixture.snapshot().await?.assert_audit_atomic();
    Ok(())
}

pub async fn reject(fixture: &Fixture, name: CommandName, payload: Value) -> Result<()> {
    let revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
    let envelope = fixture.envelope(name, revision, payload, &Uuid::now_v7().to_string());
    fixture
        .assert_unchanged_after(&envelope, &fixture.context)
        .await?;
    Ok(())
}

async fn reject_task(
    fixture: &Fixture,
    name: CommandName,
    task: TaskId,
    mut extra: Value,
) -> Result<()> {
    extra["task_id"] = json!(task);
    extra["expected_task_revision"] = json!(fixture.task(task).await?.revision().get());
    reject(fixture, name, extra).await
}

pub fn edge(blocker: TaskId, blocked: TaskId) -> Value {
    json!({"blocker_task_id":blocker,"blocked_task_id":blocked,"required_condition":"task_done"})
}

pub fn human_evidence_pipeline() -> Value {
    json!({
        "name":"Human evidence conformance","task_kinds":["delivery"],"entry_stage_id":"review",
        "stages":[{"id":"review","name":"Review","executor_kind":"human","outcomes":["accepted"]}],
        "transitions":[{"from_stage_id":"review","outcome":"accepted","target":{"kind":"done"},
            "artifact_requirements":[{"kind":"stage_evidence","minimum_count":1,"scope":"current_stage"}]}]
    })
}

pub fn evidence_artifact() -> Value {
    json!({"kind":"stage_evidence","title":"Human review","metadata":{},"body":{"review":"accepted"}})
}

pub async fn complete_human(fixture: &Fixture, task: TaskId, outcome: &str) -> Result<()> {
    let wait = fixture
        .task(task)
        .await?
        .wait_conditions()
        .next()
        .context("stage wait")?
        .id();
    fixture.task_command(CommandName::SubmitExternalStageOutcome, task,
        json!({"stage_id":"review","outcome":outcome,"wait_condition_id":wait,"artifacts":[evidence_artifact()]})).await?;
    Ok(())
}
