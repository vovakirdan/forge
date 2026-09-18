//! Stopped, Run-free dependency graph created exclusively with canonical commands.

use anyhow::{Context, Result, ensure};
use forge_domain::{ProjectExecutionGate, TaskId, TaskWaitKind};
use forge_protocol::wire::CommandName;
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::{Value, json};

pub async fn seed(harness: &M0Harness) -> Result<Value> {
    let name = "Dependency reading acceptance";
    let project = harness.create_project(name).await?;
    let mut definition = single_stage_pipeline();
    definition["stages"][0]["executor_kind"] = json!("human");
    let version = harness.create_pipeline(project, definition).await?;
    let root = harness
        .create_task(project, version, "Dependency hub")
        .await?;
    let empty = harness
        .create_task(project, version, "Task without dependencies")
        .await?;
    let cancellation_blocker = harness
        .create_task(project, version, "Cancellation prerequisite")
        .await?;
    let cancellation_dependent = harness
        .create_task(project, version, "Cancellation dependent")
        .await?;
    harness.execute(project,CommandName::CreateDependency,json!({"blocker_task_id":cancellation_blocker,"blocked_task_id":cancellation_dependent,"required_condition":"task_done"})).await?;
    let mut first_blocker = None;
    let mut last_blocked = None;
    for number in 0..23 {
        let task = harness
            .create_task(
                project,
                version,
                &format!("Upstream prerequisite {number:02}"),
            )
            .await?;
        harness.execute(project, CommandName::CreateDependency, json!({"blocker_task_id":task,"blocked_task_id":root,"required_condition":"task_done"})).await?;
        match number {
            1 => {
                harness.approve_task(project, task).await?;
                let current = harness.required_task(task).await?.task;
                let wait = current
                    .wait_conditions()
                    .find(|wait| wait.kind() == &TaskWaitKind::DecisionRequired)
                    .context("human wait")?;
                harness.execute(project,CommandName::SubmitExternalStageOutcome,json!({"task_id":task,"expected_task_revision":current.revision().get(),"stage_id":"work","outcome":"completed","wait_condition_id":wait.id(),"artifacts":[{"kind":"stage_evidence","title":"Owner completed prerequisite","metadata":{},"body":{"result":"Complete"}}]})).await?;
            }
            2 => {
                harness.cancel_task(project, task).await?;
            }
            _ => {}
        };
        if number == 0 {
            first_blocker = Some(summary(harness, task).await?);
        }
    }
    for number in 0..23 {
        let task = harness
            .create_task(
                project,
                version,
                &format!("Downstream dependent {number:02}"),
            )
            .await?;
        harness.execute(project, CommandName::CreateDependency, json!({"blocker_task_id":root,"blocked_task_id":task,"required_condition":"task_done"})).await?;
        if number == 22 {
            last_blocked = Some(summary(harness, task).await?);
        }
    }
    ensure!(
        harness
            .store
            .load_project(project)
            .await?
            .context("project")?
            .execution_gate()
            == ProjectExecutionGate::Stopped,
        "fixture must remain stopped"
    );
    ensure!(
        harness.store.list_employees(project).await?.is_empty(),
        "no Employees"
    );
    ensure!(
        harness
            .store
            .list_runs_for_project(project)
            .await?
            .is_empty(),
        "no Runs"
    );
    Ok(
        json!({"id":project,"name":name,"root":summary(harness,root).await?,"empty":summary(harness,empty).await?,"first_blocker":first_blocker.context("first blocker")?,"last_blocked":last_blocked.context("last blocked")?,"cancellation_blocker":summary(harness,cancellation_blocker).await?,"cancellation_dependent":summary(harness,cancellation_dependent).await?}),
    )
}

async fn summary(harness: &M0Harness, task: TaskId) -> Result<Value> {
    let task = harness.required_task(task).await?.task;
    Ok(json!({"id":task.id(),"key":task.key().to_string(),"title":task.spec().title()}))
}
