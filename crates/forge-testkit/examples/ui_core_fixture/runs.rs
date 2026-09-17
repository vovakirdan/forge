//! Real Run projections created by named commands and the deterministic M0 runtime.

use std::time::Duration;

use anyhow::{Context, Result, ensure};
use forge_domain::{LifecycleStatus, ProjectId};
use forge_storage::{RunObservedState, RunProjection};
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::{Value, json};
use tokio::time::{sleep, timeout};

pub struct FixtureData {
    pub project: Value,
    pub other_project: Value,
}

pub async fn seed(harness: &M0Harness) -> Result<FixtureData> {
    let name = "Run history fixture";
    let project = harness.create_project(name).await?;
    let runs = completed_runs(harness, project, 23).await?;
    let featured = runs.first().context("Run fixture must have a first page")?;
    let second = runs
        .get(1)
        .context("Run fixture must allow selection changes")?;
    let owner = featured.require_task_stage()?;

    let other_name = "Separate Run history fixture";
    let other_project = harness.create_project(other_name).await?;
    let other_runs = completed_runs(harness, other_project, 1).await?;
    let other = other_runs
        .first()
        .context("separate Project must have a Run")?;
    Ok(FixtureData {
        project: json!({
            "id": project, "name": name, "total": runs.len(),
            "featured_id": featured.id, "second_id": second.id,
            "featured_task_id": owner.task_id,
            "employee_id": featured.require_employee_id()?, "stage_id": owner.stage_id
        }),
        other_project: json!({"id": other_project, "name": other_name, "run_id": other.id}),
    })
}

async fn completed_runs(
    harness: &M0Harness,
    project: ProjectId,
    count: usize,
) -> Result<Vec<RunProjection>> {
    let pipeline = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    // This helper issues CreateEmployee and an explicit audited onboarding skip;
    // neither credentials nor provider/runtime bindings are configured.
    harness
        .create_employee(project, "Deterministic M0 fixture worker")
        .await?;
    let mut tasks = Vec::with_capacity(count);
    for index in 1..=count {
        let task = harness
            .create_task(project, pipeline, &format!("Run fixture Task {index:02}"))
            .await?;
        harness.approve_task(project, task).await?;
        tasks.push(task);
    }
    harness.start_project(project).await?;
    for task in tasks {
        harness
            .wait_for_lifecycle(task, LifecycleStatus::Done)
            .await?;
        ensure!(
            harness.wait_for_run_count(task, 1).await?.len() == 1,
            "fixture Task must produce exactly one Run"
        );
    }
    // Submission can complete a Task before its runtime has stopped. Publish
    // fixture coordinates only after the real observed state reaches Stopped.
    timeout(Duration::from_secs(10), async {
        loop {
            let runs = harness.store.list_runs_for_project(project).await?;
            ensure!(
                runs.len() <= count,
                "fixture produced unexpected extra Runs"
            );
            if runs.len() == count
                && runs
                    .iter()
                    .all(|run| run.observed_state == RunObservedState::Stopped)
            {
                return Ok(runs);
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("fixture Runs did not stop before readiness")?
}
