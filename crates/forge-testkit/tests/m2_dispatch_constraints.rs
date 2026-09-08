//! Real scheduler/PG/gRPC, with manual observations and no model execution.
use anyhow::{Context, Result};
use forge_domain::{
    ConstraintBlockReason, EmployeeId, LifecycleStatus, NextRunConstraintState, TaskId,
};
use forge_protocol::{supervisor::v1::RunEventKind, wire::CommandName};
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::json;

async fn employee(
    harness: &M0Harness,
    project: forge_domain::ProjectId,
    name: &str,
) -> Result<EmployeeId> {
    harness.create_employee(project, name).await?;
    Ok(harness
        .store
        .list_employees(project)
        .await?
        .into_iter()
        .find(|stored| stored.employee.name() == name)
        .context("employee")?
        .employee
        .id())
}

async fn pin(
    harness: &M0Harness,
    project: forge_domain::ProjectId,
    task: TaskId,
    employee: EmployeeId,
) -> Result<()> {
    let revision = harness
        .store
        .load_task(task)
        .await?
        .context("task")?
        .task
        .revision()
        .get();
    harness
        .execute(
            project,
            CommandName::SetNextRunEmployee,
            json!({"task_id":task,"expected_task_revision":revision,"employee_id":employee}),
        )
        .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; manual Supervisor, no provider"]
async fn pinned_busy_employee_does_not_starve_other_tasks_and_consumes_exactly_once() -> Result<()>
{
    let harness = M0Harness::start().await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    let project = harness.create_project("Next Employee capacity").await?;
    let bob = employee(&harness, project, "Bob").await?;
    let zoe = employee(&harness, project, "Zoe").await?;
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let first = harness
        .create_task(project, pipeline, "Bob current")
        .await?;
    let second = harness.create_task(project, pipeline, "Bob next").await?;
    let independent = harness
        .create_task(project, pipeline, "Zoe independent")
        .await?;
    for (task, employee) in [(first, bob), (second, bob), (independent, zoe)] {
        harness.approve_task(project, task).await?;
        pin(&harness, project, task, employee).await?;
    }
    harness.start_project(project).await?;
    supervisor.next_provision_for_task(first).await?;
    supervisor.next_provision_for_task(independent).await?;
    let mut runs = harness.store.list_runs(first).await?;
    runs.extend(harness.store.list_runs(independent).await?);
    assert!(harness.store.list_runs(second).await?.is_empty());
    assert_eq!(runs.len(), 2);
    assert!(
        runs.iter()
            .any(|run| run.task_id() == Some(first) && run.employee_id == Some(bob))
    );
    assert!(
        runs.iter()
            .any(|run| run.task_id() == Some(independent) && run.employee_id == Some(zoe))
    );
    let mut tx = harness.store.begin().await?;
    assert_eq!(
        tx.load_task_dispatch_constraint(project, second)
            .await?
            .context("pending")?
            .state,
        NextRunConstraintState::Pending
    );
    tx.commit().await?;
    let run = runs
        .into_iter()
        .find(|run| run.task_id() == Some(first))
        .context("first Run")?;
    let task = harness.store.load_task(first).await?.context("task")?.task;
    harness
        .execute(
            project,
            CommandName::PauseTask,
        json!({"task_id":first,"expected_task_revision":task.revision().get(),"mode":"graceful"}),
        )
        .await?;
    supervisor.next_stop_for_run(&run.id.to_string()).await?;
    supervisor
        .send_observation_kind(&run, run.lease_fencing_token, 1, RunEventKind::Stopped)
        .await?;
    harness.core.dispatch_available(project).await?;
    supervisor.next_provision_for_task(second).await?;
    let second_runs = harness.wait_for_run_count(second, 1).await?;
    assert_eq!(second_runs[0].employee_id, Some(bob));
    let consumed: serde_json::Value = sqlx::query_scalar(
        "SELECT canonical_snapshot FROM task_next_run_constraints WHERE task_id=$1",
    )
    .bind(second.as_uuid())
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(
        consumed["state"],
        json!({"status":"consumed","run_id":second_runs[0].id})
    );
    assert_eq!(harness.core.dispatch_available(project).await?, 0);
    harness.stop_project(project).await?;
    harness.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local PostgreSQL and NATS; manual Supervisor, no provider"]
async fn disabled_assignee_is_a_visible_hold_without_implicit_fallback_or_resume() -> Result<()> {
    let harness = M0Harness::start().await?;
    let mut supervisor = harness.attach_manual_supervisor().await?;
    let project = harness.create_project("Explicit assignment hold").await?;
    let bob = employee(&harness, project, "Bob").await?;
    let zoe = employee(&harness, project, "Zoe").await?;
    let version = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let held = harness
        .create_task(project, version, "Pinned to Bob")
        .await?;
    let other = harness
        .create_task(project, version, "Ordinary work")
        .await?;
    harness.approve_task(project, held).await?;
    pin(&harness, project, held, bob).await?;
    harness.approve_task(project, other).await?;
    harness
        .execute(
            project,
            CommandName::DisableEmployee,
            json!({"employee_id":bob,"expected_employee_revision":1}),
        )
        .await?;
    harness.start_project(project).await?;
    supervisor.next_provision_for_task(other).await?;
    let runs = harness.store.list_runs(other).await?;
    assert!(harness.store.list_runs(held).await?.is_empty());
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].employee_id, Some(zoe));
    let mut tx = harness.store.begin().await?;
    assert_eq!(
        tx.load_task_dispatch_constraint(project, held)
            .await?
            .context("hold")?
            .state,
        NextRunConstraintState::Blocked {
            reason: ConstraintBlockReason::EmployeeDisabled
        }
    );
    tx.commit().await?;
    let task = harness.store.load_task(held).await?.context("Task")?.task;
    assert_eq!(
        task.lifecycle(),
        LifecycleStatus::Ready,
        "admission hold is not Task pause"
    );
    harness
        .execute(
            project,
            CommandName::EnableEmployee,
            json!({"employee_id":bob,"expected_employee_revision":2}),
        )
        .await?;
    assert_eq!(
        harness.core.dispatch_available(project).await?,
        0,
        "catalog change does not clear an explicit hold"
    );
    harness
        .execute(
            project,
            CommandName::ClearNextRunEmployee,
            json!({"task_id":held,"expected_task_revision":task.revision().get()}),
        )
        .await?;
    supervisor.next_provision_for_task(held).await?;
    assert_eq!(
        harness.wait_for_run_count(held, 1).await?[0].require_employee_id()?,
        bob
    );
    harness.stop_project(project).await?;
    harness.shutdown().await;
    Ok(())
}
