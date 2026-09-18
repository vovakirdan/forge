use anyhow::{Context, Result, ensure};
use forge_domain::{ProjectId, TaskId};
use forge_protocol::wire::CommandName;
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::{Value, json};

pub async fn seed(harness: &M0Harness) -> Result<Value> {
    let mut value = super::commands::seed(harness, "Approval command acceptance").await?;
    let project: ProjectId = value["id"].as_str().context("project")?.parse()?;
    let first: TaskId = value["drafts"][0]["id"]
        .as_str()
        .context("first draft")?
        .parse()?;
    let version = harness
        .required_task(first)
        .await?
        .task
        .pipeline()
        .pipeline_version_id();
    value["pipeline_version_id"] = json!(version);
    let missing = harness
        .execute(
            project,
            CommandName::CreateTask,
            json!({
                "title":"Approval needs Definition of Done","description":"", "kind":"delivery",
                "pipeline_version_id":version,"priority":"normal","properties":{}
            }),
        )
        .await?
        .resource
        .context("without dod")?
        .id;
    value["without_dod_id"] = json!(missing);
    for executor in ["human", "external"] {
        let mut definition = single_stage_pipeline();
        definition["name"] = json!(format!("Approval {executor}"));
        definition["stages"][0]["executor_kind"] = json!(executor);
        let pin = harness.create_pipeline(project, definition).await?;
        let task = harness
            .create_task(project, pin, &format!("Approval {executor} stage"))
            .await?;
        value[format!("{executor}_id")] = json!(task);
    }
    let blocker = harness
        .create_task(project, version, "Approval dependency blocker")
        .await?;
    let blocked = harness
        .create_task(project, version, "Approval blocked by other Task")
        .await?;
    harness.execute(project,CommandName::CreateDependency,json!({"blocker_task_id":blocker,"blocked_task_id":blocked,"required_condition":"task_done"})).await?;
    value["blocker_id"] = json!(blocker);
    value["dependency_id"] = json!(blocked);
    let mut definition = single_stage_pipeline();
    definition["name"] = json!("Approval archived pin");
    let deleted_version = harness.create_pipeline(project, definition).await?;
    let pinned = harness
        .create_task(project, deleted_version, "Approval keeps archived pin")
        .await?;
    let pipeline = harness
        .store
        .load_pipeline_version(deleted_version)
        .await?
        .context("version")?
        .pipeline_id();
    let revision = harness
        .store
        .load_pipeline(pipeline)
        .await?
        .context("pipeline")?
        .revision();
    harness
        .execute(
            project,
            CommandName::DeletePipeline,
            json!({"pipeline_id":pipeline,"expected_pipeline_revision":revision}),
        )
        .await?;
    value["deleted_pipeline_task_id"] = json!(pinned);
    ensure!(
        harness
            .store
            .list_runs_for_project(project)
            .await?
            .is_empty(),
        "approval fixture must never start work"
    );
    ensure!(
        harness.store.list_employees(project).await?.is_empty(),
        "approval fixture has no Employees"
    );
    Ok(value)
}
