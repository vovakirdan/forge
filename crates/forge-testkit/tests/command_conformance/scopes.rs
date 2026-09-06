use anyhow::Result;
use forge_application::CommandContext;
use forge_domain::ProjectId;
use forge_protocol::wire::CommandName;
use forge_testkit::m0::single_stage_pipeline;
use serde_json::json;

use super::{
    commands::reject,
    fixture::{BackendKind, Fixture},
};

pub async fn catalog_uniqueness_and_project_scope(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let employee =
        json!({"name":"Scoped employee","role":"worker","stage_eligibility":{"mode":"any"}});
    fixture
        .execute(CommandName::CreateEmployee, employee.clone())
        .await?;
    reject(&fixture, CommandName::CreateEmployee, employee.clone()).await?;
    let pipeline = fixture.pipeline(single_stage_pipeline()).await?;
    reject(
        &fixture,
        CommandName::CreatePipeline,
        single_stage_pipeline(),
    )
    .await?;
    let task = fixture.create_task(pipeline, "Local task").await?;

    // Share persistence, but use a separately trusted project context.
    let mut other = fixture.clone();
    other.project_id = ProjectId::new();
    other.context = CommandContext::local_human(
        other.project_id,
        fixture.context.actor,
        fixture.context.core_actor,
    );
    other
        .execute(CommandName::CreateProject, json!({"name":"Other project"}))
        .await?;
    other.execute(CommandName::CreateEmployee, employee).await?;
    let foreign_pipeline = other.pipeline(single_stage_pipeline()).await?;
    let foreign_task = other.create_task(foreign_pipeline, "Foreign task").await?;
    reject(
        &fixture,
        CommandName::CreateTask,
        json!({"title":"Wrong pipeline","kind":"delivery",
        "priority":"normal","pipeline_version_id":foreign_pipeline}),
    )
    .await?;
    reject(&fixture,CommandName::SetTaskPriority,json!({"task_id":foreign_task,
        "expected_task_revision":other.task(foreign_task).await?.revision().get(),"priority":"high"})).await?;
    reject(
        &fixture,
        CommandName::CreateDependency,
        super::commands::edge(task, foreign_task),
    )
    .await?;
    fixture.snapshot().await?.assert_audit_atomic();
    Ok(())
}

pub async fn mutated_typed_intent_cannot_diverge_from_fingerprinted_json(
    kind: BackendKind,
) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let original = fixture.envelope(
        CommandName::StartProjectExecution,
        1,
        json!({"reason":"Original"}),
        "integrity",
    );
    let different = fixture.envelope(
        CommandName::StartProjectExecution,
        1,
        json!({"reason":"Modified"}),
        "integrity",
    );
    let mut modified = original.clone();
    modified.payload = different.payload;
    fixture
        .assert_unchanged_after(&modified, &fixture.context)
        .await?;
    let mut modified = original.clone();
    modified.name = CommandName::StopProjectExecution;
    fixture
        .assert_unchanged_after(&modified, &fixture.context)
        .await?;
    fixture.execute_as(&original, &fixture.context).await?;
    Ok(())
}
