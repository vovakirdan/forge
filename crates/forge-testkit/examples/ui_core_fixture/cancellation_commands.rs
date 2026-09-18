use anyhow::{Context, Result};
use forge_domain::{ProjectId, TaskId};
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::{Value, json};

pub async fn seed(harness: &M0Harness) -> Result<Value> {
    let mut value = super::commands::seed(harness, "Cancellation command acceptance").await?;
    let project: ProjectId = value["id"].as_str().context("project")?.parse()?;
    let first: TaskId = value["drafts"][0]["id"]
        .as_str()
        .context("draft")?
        .parse()?;
    let version = harness
        .required_task(first)
        .await?
        .task
        .pipeline()
        .pipeline_version_id();
    let ready = harness
        .create_task(project, version, "Cancel ready work")
        .await?;
    harness.approve_task(project, ready).await?;
    let mut definition = single_stage_pipeline();
    definition["name"] = json!("Cancellation human wait");
    definition["stages"][0]["executor_kind"] = json!("human");
    let version = harness.create_pipeline(project, definition).await?;
    let waiting = harness
        .create_task(project, version, "Cancel waiting work")
        .await?;
    harness.approve_task(project, waiting).await?;
    value["ready_id"] = json!(ready);
    value["waiting_id"] = json!(waiting);
    Ok(value)
}
