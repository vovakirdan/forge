//! Empty stopped project with real pipeline choices, never executable work.
use anyhow::{Context, Result, ensure};
use forge_domain::{PipelineVersionId, ProjectExecutionGate};
use forge_protocol::wire::CommandName;
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::{Value, json};

pub async fn seed(harness: &M0Harness) -> Result<Value> {
    let name = "Create task acceptance";
    let project = harness.create_project(name).await?;
    let mut definition = single_stage_pipeline();
    definition["name"] = json!("Create delivery or analysis");
    definition["task_kinds"] = json!(["delivery", "analysis"]);
    let older = harness.create_pipeline(project, definition.clone()).await?;
    let pipeline_id = harness
        .store
        .load_pipeline_version(older)
        .await?
        .context("older version")?
        .pipeline_id();
    let catalog = harness
        .store
        .load_pipeline(pipeline_id)
        .await?
        .context("pipeline")?;
    definition
        .as_object_mut()
        .context("definition object")?
        .remove("name");
    let latest: PipelineVersionId = harness.execute(project, CommandName::PublishPipelineVersion,
        json!({"pipeline_id":pipeline_id,"expected_pipeline_revision":catalog.revision(),"definition":definition,"make_default":false}))
        .await?.resource.context("published version")?.id.parse()?;
    let mut deleted_definition = single_stage_pipeline();
    deleted_definition["name"] = json!("Deleted create pipeline");
    let deleted = harness.create_pipeline(project, deleted_definition).await?;
    let deleted_pipeline = harness
        .store
        .load_pipeline_version(deleted)
        .await?
        .context("deleted version")?
        .pipeline_id();
    let catalog = harness
        .store
        .load_pipeline(deleted_pipeline)
        .await?
        .context("deleted pipeline")?;
    harness
        .execute(
            project,
            CommandName::DeletePipeline,
            json!({"pipeline_id":deleted_pipeline,"expected_pipeline_revision":catalog.revision()}),
        )
        .await?;
    let mut delivery_definition = single_stage_pipeline();
    delivery_definition["name"] = json!("Delivery only create pipeline");
    let delivery_only = harness
        .create_pipeline(project, delivery_definition)
        .await?;
    let foreign = harness
        .create_project("Foreign create pipeline acceptance")
        .await?;
    let foreign_version = harness
        .create_pipeline(foreign, single_stage_pipeline())
        .await?;
    for id in [project, foreign] {
        ensure!(
            harness.store.list_tasks(id).await?.is_empty(),
            "create fixture must be empty"
        );
        ensure!(
            harness.store.list_employees(id).await?.is_empty(),
            "create fixture cannot employ workers"
        );
        ensure!(
            harness.store.list_runs_for_project(id).await?.is_empty(),
            "create fixture cannot run work"
        );
        ensure!(
            harness
                .store
                .load_project(id)
                .await?
                .context("create project")?
                .execution_gate()
                == ProjectExecutionGate::Stopped,
            "create fixture must remain stopped"
        );
    }
    Ok(json!({"id":project,"name":name,"pipeline_id":pipeline_id,
        "older_version_id":older,"latest_version_id":latest,
        "deleted_pipeline_id":deleted_pipeline,"deleted_version_id":deleted,
        "delivery_only_version_id":delivery_only,
        "foreign_project_id":foreign,"foreign_version_id":foreign_version}))
}
