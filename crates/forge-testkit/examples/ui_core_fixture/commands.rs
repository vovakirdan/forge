//! Isolated command fixtures: stopped Project, no Employees or executable work.

use anyhow::{Context, Result, ensure};
use forge_domain::{LifecycleStatus, ProjectExecutionGate, TaskId};
use forge_protocol::wire::CommandName;
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::{Value, json};

pub async fn seed(harness: &M0Harness, name: &str) -> Result<Value> {
    let project = harness.create_project(name).await?;
    let version = harness
        .create_pipeline(project, single_stage_pipeline())
        .await?;
    let mut drafts = Vec::new();
    // Each mutating browser scenario owns a distinct draft; tests do not reset
    // canonical data through SQL or depend on another scenario's edits.
    for number in 1..=12 {
        let task: TaskId = harness
            .execute(
                project,
                CommandName::CreateTask,
                json!({
                    "title": format!("Editable draft {number:02}"),
                    "description": "Original draft description.",
                    "definition_of_done": "Owner confirms the draft text.",
                    "kind": "delivery", "priority": "normal",
                    "pipeline_version_id": version, "properties": {}
                }),
            )
            .await?
            .resource
            .context("draft command fixture receipt missing")?
            .id
            .parse()?;
        let loaded = harness.required_task(task).await?.task;
        ensure!(
            loaded.lifecycle() == LifecycleStatus::Draft,
            "fixture must stay draft"
        );
        drafts.push(json!({"id": task, "key": loaded.key().to_string()}));
    }
    let cancelled = harness
        .create_task(project, version, "Immutable cancelled draft")
        .await?;
    harness.cancel_task(project, cancelled).await?;
    ensure!(
        harness
            .store
            .load_project(project)
            .await?
            .context("command Project missing")?
            .execution_gate()
            == ProjectExecutionGate::Stopped,
        "command fixture must never open execution"
    );
    ensure!(
        harness.store.list_employees(project).await?.is_empty(),
        "no fixture Employees"
    );
    ensure!(
        harness
            .store
            .list_runs_for_project(project)
            .await?
            .is_empty(),
        "no fixture Runs"
    );
    Ok(json!({"id": project, "name": name, "drafts": drafts, "cancelled_id": cancelled}))
}
