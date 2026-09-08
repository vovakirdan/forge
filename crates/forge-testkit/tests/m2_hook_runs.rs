//! Real PostgreSQL/Core/Git identities, controlled Supervisor reports; no inference.
#[path = "m2_hook_runs/fairness.rs"]
mod fairness;
#[path = "m2_hook_runs/history.rs"]
mod history;
#[path = "m2_hook_runs/ownership.rs"]
mod ownership;
#[path = "m2_hook_runs/ownership_restart.rs"]
mod ownership_restart;
#[path = "m2_hook_runs/policy.rs"]
mod policy;
#[path = "m2_communication_runs/support.rs"]
#[allow(dead_code)] // Only synthetic enrollment and Gateway helpers are reused.
mod provider_fixture;
#[path = "m2_hook_runs/support.rs"]
mod support;

use anyhow::{Context, Result};
use forge_domain::{HookExecutionResult, HookVerdict, LifecycleStatus, runtime::HookRunSpec};
use forge_protocol::{
    supervisor::v1::{AcknowledgementDisposition, RunEventKind},
    wire::CommandName,
};
use serde_json::{Value, json};
use uuid::Uuid;

async fn hook_run(f: &mut support::Fixture) -> Result<(forge_storage::RunProjection, HookRunSpec)> {
    let provision = f.supervisor.next_provision().await?;
    assert_eq!(provision.run_spec_version, 5);
    assert!(provision.employee_id.is_empty());
    assert!(provision.task_id.is_empty());
    assert!(provision.stage_id.is_empty());
    let run =
        f.h.store
            .load_run(provision.run_id.parse()?)
            .await?
            .context("Hook Run")?;
    assert!(run.employee_id.is_none());
    assert!(
        run.require_task_id().is_err(),
        "Hook context is not a Task writer lease"
    );
    assert!(run.run_spec.get("execution_profile").is_none());
    assert!(run.run_spec.get("credential_binding").is_none());
    assert!(!f.root.join("grants").join(run.id.to_string()).exists());
    assert!(!f.root.join("gateways").join(run.id.to_string()).exists());
    let spec: HookRunSpec = serde_json::from_value(run.run_spec.clone())?;
    assert_eq!(spec.assignment.task_id, f.task);
    assert_eq!(spec.candidate, f.candidate);
    assert_eq!(spec.hook.id, f.hook);
    Ok((run, spec))
}

fn result(spec: &HookRunSpec, verdict: HookVerdict) -> HookExecutionResult {
    HookExecutionResult {
        invocation_id: spec.assignment.invocation_id,
        candidate_proposal_id: spec.assignment.candidate_proposal_id,
        candidate: spec.candidate.clone(),
        verdict,
        exit_code: match verdict {
            HookVerdict::Passed => Some(0),
            HookVerdict::Failed => Some(1),
            _ => None,
        },
        output_incomplete: false,
    }
}

async fn gate(f: &support::Fixture, candidate: Option<Uuid>) -> Result<bool> {
    let task = f.h.store.load_task(f.task).await?.context("Task")?.task;
    let version =
        f.h.store
            .load_pipeline_version(task.pipeline().pipeline_version_id())
            .await?
            .context("Pipeline")?;
    let mut tx = f.h.store.begin().await?;
    tx.lock_project(f.project).await?.context("Project")?;
    Ok(tx
        .required_hooks_satisfied(&task, &version, candidate)
        .await?)
}

async fn execution_case(
    mode: &str,
    verdict: Option<HookVerdict>,
    expected: LifecycleStatus,
) -> Result<()> {
    let mut f = Box::pin(support::setup(mode)).await?;
    let (run, spec) = hook_run(&mut f).await?;
    if mode != "advisory" {
        assert!(!gate(&f, None).await?, "running is not passed");
    }
    if mode == "stop_late" {
        f.h.stop_project(f.project).await?;
    }
    let mut reported = verdict.map(|verdict| result(&spec, verdict));
    if mode == "wrong_candidate" {
        reported.as_mut().context("report")?.candidate_proposal_id = Uuid::now_v7();
    }
    let ack = f
        .supervisor
        .send_hook_result(&run, 1, reported.as_ref())
        .await?;
    assert_eq!(
        ack.disposition,
        AcknowledgementDisposition::Accepted as i32,
        "{ack:?}"
    );
    support::tick(&f.h).await?;
    let task = f.h.store.load_task(f.task).await?.context("Task")?.task;
    if expected == LifecycleStatus::Ready {
        assert_eq!(task.current_stage_id().context("stage")?.as_str(), "work");
        assert!(matches!(
            task.lifecycle(),
            LifecycleStatus::Ready | LifecycleStatus::InProgress
        ));
    } else {
        assert_eq!(task.lifecycle(), expected, "mode {mode}");
    }
    if expected == LifecycleStatus::Done {
        assert!(gate(&f, None).await?);
    } else {
        assert!(!gate(&f, None).await?);
    }
    assert!(
        f.h.store
            .list_artifacts_for_task(f.task)
            .await?
            .iter()
            .any(|artifact| artifact.artifact.kind().as_str() == "hook_result")
    );
    drop(f.supervisor);
    f.h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference or Hook process"]
async fn hook_passes_same_task_without_employee_or_provider_authority() -> Result<()> {
    execution_case("passed", Some(HookVerdict::Passed), LifecycleStatus::Done).await
}
#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference or Hook process"]
async fn advisory_hook_failure_can_follow_configured_done_route() -> Result<()> {
    execution_case("advisory", Some(HookVerdict::Failed), LifecycleStatus::Done).await
}
#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference or Hook process"]
async fn required_hook_failure_returns_same_task_to_work() -> Result<()> {
    execution_case(
        "required_fail",
        Some(HookVerdict::Failed),
        LifecycleStatus::Ready,
    )
    .await
}
#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference or Hook process"]
async fn missing_hook_result_holds_instead_of_fabricating_pass() -> Result<()> {
    execution_case("missing", None, LifecycleStatus::Waiting).await
}
#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference or Hook process"]
async fn project_stop_blocks_late_hook_pass() -> Result<()> {
    execution_case(
        "stop_late",
        Some(HookVerdict::Passed),
        LifecycleStatus::Waiting,
    )
    .await
}
#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference or Hook process"]
async fn hook_result_cannot_claim_another_candidate() -> Result<()> {
    execution_case(
        "wrong_candidate",
        Some(HookVerdict::Passed),
        LifecycleStatus::Waiting,
    )
    .await
}
#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference or Hook process"]
async fn nonapplicable_hook_records_skip_without_run() -> Result<()> {
    let f = Box::pin(support::setup("skipped")).await?;
    assert_eq!(
        f.h.store
            .load_task(f.task)
            .await?
            .context("Task")?
            .task
            .lifecycle(),
        LifecycleStatus::Done
    );
    assert!(gate(&f, None).await?);
    let runs = f.h.store.list_runs_for_project(f.project).await?;
    assert_eq!(runs.len(), 1, "only the writer ran");
    let artifacts = f.h.store.list_artifacts_for_task(f.task).await?;
    assert!(
        artifacts
            .iter()
            .any(|artifact| artifact.artifact.kind().as_str() == "hook_result")
    );
    drop(f.supervisor);
    f.h.shutdown().await;
    Ok(())
}
#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference or Hook process"]
async fn writer_cannot_bypass_required_hook_with_direct_done_route() -> Result<()> {
    let f = Box::pin(support::setup("missing_required")).await?;
    assert_ne!(
        f.h.store
            .load_task(f.task)
            .await?
            .context("Task")?
            .task
            .lifecycle(),
        LifecycleStatus::Done
    );
    assert!(!gate(&f, None).await?);
    drop(f.supervisor);
    f.h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference or Hook process"]
async fn nonapplicable_hook_without_git_needs_no_run_or_candidate() -> Result<()> {
    let (h, project, _employee, _root) = provider_fixture::fixture().await?;
    let hook:Uuid=h.execute(project,CommandName::ConfigureProjectHook,json!({"name":"Only for analysis","image":format!("localhost/synthetic@sha256:{}","0".repeat(64)),"command":["/bin/true"],"workdir":".","limits":{"cpu_millis":1000,"memory_bytes":134217728,"pids":32,"wall_seconds":60,"stop_grace_seconds":1},"max_output_bytes":4096,"applicable_task_kinds":["analysis"],"required":true})).await?.resource.context("hook")?.id.parse()?;
    let mut pipeline = support::pipeline("skipped", hook);
    pipeline["stages"][0]["executor_kind"] = json!("human");
    pipeline["stages"][0]
        .as_object_mut()
        .context("stage")?
        .remove("workspace");
    pipeline["transitions"][0]
        .as_object_mut()
        .context("transition")?
        .remove("artifact_requirements");
    let version = h.create_pipeline(project, pipeline).await?;
    let task = h
        .create_task(project, version, "A text-only task has no repository")
        .await?;
    h.approve_task(project, task).await?;
    let mut supervisor = h.attach_manual_supervisor().await?;
    supervisor.reconcile_empty().await?;
    h.start_project(project).await?;
    h.submit_external_stage_outcome(project, task, "completed")
        .await?;
    support::tick(&h).await?;
    assert_eq!(
        h.store
            .load_task(task)
            .await?
            .context("Task")?
            .task
            .lifecycle(),
        LifecycleStatus::Done
    );
    assert!(h.store.list_runs_for_project(project).await?.is_empty());
    let row: (Option<Uuid>, Option<Uuid>, Value) = sqlx::query_as(
        "SELECT run_id,candidate_proposal_id,result FROM hook_invocations WHERE task_id=$1",
    )
    .bind(task.as_uuid())
    .fetch_one(&h.pool)
    .await?;
    assert!(row.0.is_none() && row.1.is_none());
    assert_eq!(row.2["verdict"], "skipped");
    drop(supervisor);
    h.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL/NATS; no provider inference or Hook process"]
async fn human_done_cannot_bypass_an_applicable_required_hook() -> Result<()> {
    let (h, project, _employee, _root) = provider_fixture::fixture().await?;
    let hook:Uuid=h.execute(project,CommandName::ConfigureProjectHook,json!({"name":"Required delivery check","image":format!("localhost/synthetic@sha256:{}","0".repeat(64)),"command":["/bin/true"],"workdir":".","limits":{"cpu_millis":1000,"memory_bytes":134217728,"pids":32,"wall_seconds":60,"stop_grace_seconds":1},"max_output_bytes":4096,"applicable_task_kinds":["delivery"],"required":true})).await?.resource.context("hook")?.id.parse()?;
    let mut pipeline = support::pipeline("missing_required", hook);
    pipeline["stages"][0]["executor_kind"] = json!("human");
    pipeline["stages"][0]
        .as_object_mut()
        .context("stage")?
        .remove("workspace");
    pipeline["transitions"][0]
        .as_object_mut()
        .context("transition")?
        .remove("artifact_requirements");
    let version = h.create_pipeline(project, pipeline).await?;
    let task = h
        .create_task(
            project,
            version,
            "Human command still respects configured acceptance",
        )
        .await?;
    h.approve_task(project, task).await?;
    let before = h.store.load_task(task).await?.context("Task")?.task;
    assert!(
        h.submit_external_stage_outcome(project, task, "completed")
            .await
            .is_err()
    );
    assert_eq!(before, h.store.load_task(task).await?.context("Task")?.task);
    assert!(h.store.list_runs_for_project(project).await?.is_empty());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM task_handoffs WHERE task_id=$1")
        .bind(task.as_uuid())
        .fetch_one(&h.pool)
        .await?;
    assert_eq!(
        count, 0,
        "rejected transition creates no successful handoff"
    );
    h.shutdown().await;
    Ok(())
}
