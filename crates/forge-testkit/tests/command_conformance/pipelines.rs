//! Catalog, Task binding and immutable graph semantics use both production stores.
use super::{
    atomicity::assert_faults,
    fixture::{Backend, BackendKind, Fixture},
};
use anyhow::{Context, Result};
use forge_application::{CommandError, CommandTransaction, RepositoryError};
use forge_domain::{LifecycleStatus, Pipeline, PipelineId, PipelineVersionId, TaskId};
use forge_protocol::wire::{CommandName, CommandStatus};
use forge_storage::PostgresStore;
use serde_json::{Value, json};

fn definition(instructions: &str) -> Value {
    json!({"task_kinds":["delivery"],"entry_stage_id":"work",
        "stages":[{"id":"work","name":"Work","executor_kind":"human","outcomes":["done"],
            "instructions":instructions,"workspace":{"kind":"any","access":"read_only"}}],
        "transitions":[{"from_stage_id":"work","outcome":"done","target":{"kind":"done"}}]})
}

async fn catalog(fixture: &Fixture, id: PipelineId) -> Result<Pipeline> {
    match &fixture.backend {
        Backend::Memory(store) => store
            .snapshot()
            .await
            .pipelines
            .get(&id)
            .cloned()
            .context("pipeline"),
        Backend::Postgres(pool) => PostgresStore::from_pool(pool.clone())
            .load_pipeline(id)
            .await?
            .context("pipeline"),
    }
}

async fn create(fixture: &Fixture, name: &str) -> Result<(PipelineId, PipelineVersionId)> {
    let mut payload = definition("Initial instructions");
    payload["name"] = json!(name);
    let version = fixture.pipeline(payload).await?;
    let id = fixture.snapshot().await?.versions[&version].pipeline_id();
    Ok((id, version))
}

async fn new_task(fixture: &Fixture, pipeline: PipelineId, title: &str) -> Result<TaskId> {
    Ok(fixture
        .execute(
            CommandName::CreateTask,
            json!({"title":title,"definition_of_done":"Report the outcome",
        "kind":"delivery","pipeline_id":pipeline,"priority":"normal"}),
        )
        .await?
        .resource
        .context("task")?
        .id
        .parse()?)
}

pub async fn versioning_preserves_existing_tasks(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let (id, first) = create(&fixture, "Versioned pipeline").await?;
    let old_task = new_task(&fixture, id, "Old draft stays pinned").await?;
    let original_graph = fixture.snapshot().await?.versions[&first].clone();
    let amendment=fixture.envelope(CommandName::PublishPipelineVersion,
        fixture.snapshot().await?.projects[&fixture.project_id].revision(),
        json!({"pipeline_id":id,"expected_pipeline_revision":1,"definition":definition("New instructions")}),"publish-v2");
    assert_faults(&fixture, &amendment).await?;
    let published = fixture.execute_as(&amendment, &fixture.context).await?;
    let second: PipelineVersionId = published.resource.as_ref().context("version")?.id.parse()?;
    let current = catalog(&fixture, id).await?;
    assert_eq!(
        (
            current.revision(),
            current.latest_version(),
            current.default_version_id()
        ),
        (2, 2, first)
    );
    fixture
        .execute(
            CommandName::SetPipelineDefaultVersion,
            json!({"pipeline_id":id,"expected_pipeline_revision":2,"pipeline_version_id":second}),
        )
        .await?;
    let new = new_task(&fixture, id, "New draft selects new default").await?;
    assert_eq!(
        fixture
            .task(old_task)
            .await?
            .pipeline()
            .pipeline_version_id(),
        first
    );
    assert_eq!(
        fixture.task(new).await?.pipeline().pipeline_version_id(),
        second
    );
    fixture
        .execute(
            CommandName::SetPipelineDefaultVersion,
            json!({"pipeline_id":id,"expected_pipeline_revision":3,"pipeline_version_id":first}),
        )
        .await?;
    let third = fixture
        .execute(
            CommandName::PublishPipelineVersion,
            json!({"pipeline_id":id,"expected_pipeline_revision":4,
        "definition":definition("Third instructions"),"make_default":true}),
        )
        .await?
        .resource
        .context("third version")?
        .id
        .parse::<PipelineVersionId>()?;
    assert_eq!(fixture.snapshot().await?.versions[&third].version(), 3);
    assert_eq!(catalog(&fixture, id).await?.default_version_id(), third);
    fixture
        .execute(
            CommandName::DeletePipeline,
            json!({"pipeline_id":id,"expected_pipeline_revision":5}),
        )
        .await?;
    let frozen = fixture.snapshot().await?;
    assert_eq!(frozen.versions[&first], original_graph);
    assert_eq!(
        frozen.versions[&second]
            .stages()
            .next()
            .context("stage")?
            .instructions(),
        "New instructions"
    );
    assert_eq!(frozen.versions.len(), 3);
    let replay = fixture.execute_as(&amendment, &fixture.context).await?;
    assert_eq!(replay.status, CommandStatus::Replayed);
    assert_eq!(replay.event_ids, published.event_ids);
    assert_eq!(frozen.raw, fixture.snapshot().await?.raw);
    fixture
        .task_command(CommandName::ApproveTask, old_task, json!({}))
        .await?;
    let task = fixture.task(old_task).await?;
    let wait = task.wait_conditions().next().context("manual wait")?.id();
    fixture
        .task_command(
            CommandName::SubmitExternalStageOutcome,
            old_task,
            json!({"stage_id":"work","outcome":"done","wait_condition_id":wait}),
        )
        .await?;
    assert_eq!(
        fixture.task(old_task).await?.lifecycle(),
        LifecycleStatus::Done
    );
    let revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
    for selector in [
        json!({"pipeline_id":id}),
        json!({"pipeline_version_id":first}),
    ] {
        let mut payload =
            json!({"title":"Forbidden new Task","kind":"delivery","priority":"normal"});
        payload
            .as_object_mut()
            .context("payload")?
            .extend(selector.as_object().context("selector")?.clone());
        let envelope = fixture.envelope(
            CommandName::CreateTask,
            revision,
            payload,
            "new-after-delete",
        );
        fixture
            .assert_unchanged_after(&envelope, &fixture.context)
            .await?;
    }
    fixture.snapshot().await?.assert_audit_atomic();
    Ok(())
}

pub async fn refusals_and_repository_cas(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let (id, first) = create(&fixture, "Managed").await?;
    let (_, foreign) = create(&fixture, "Other").await?;
    let mut stale = catalog(&fixture, id).await?;
    let revision = fixture.snapshot().await?.projects[&fixture.project_id].revision();
    for (key, payload) in [
        (
            "stale",
            json!({"pipeline_id":id,"expected_pipeline_revision":2,"pipeline_version_id":first}),
        ),
        (
            "wrong-pipeline",
            json!({"pipeline_id":id,"expected_pipeline_revision":1,"pipeline_version_id":foreign}),
        ),
    ] {
        let envelope = fixture.envelope(
            CommandName::SetPipelineDefaultVersion,
            revision,
            payload,
            key,
        );
        fixture
            .assert_unchanged_after(&envelope, &fixture.context)
            .await?;
    }
    let mut invalid = definition("Invalid graph");
    invalid["entry_stage_id"] = json!("missing");
    let envelope = fixture.envelope(
        CommandName::PublishPipelineVersion,
        revision,
        json!({"pipeline_id":id,"expected_pipeline_revision":1,"definition":invalid}),
        "invalid-graph",
    );
    fixture
        .assert_unchanged_after(&envelope, &fixture.context)
        .await?;
    let valid = fixture.envelope(
        CommandName::SetPipelineDefaultVersion,
        revision,
        json!({"pipeline_id":id,"expected_pipeline_revision":1,"pipeline_version_id":first}),
        "set-default",
    );
    let mut denied = fixture.context.clone();
    denied.capabilities.clear();
    assert!(matches!(
        fixture.assert_unchanged_after(&valid, &denied).await?,
        CommandError::Forbidden
    ));
    assert_faults(&fixture, &valid).await?;
    fixture.execute_as(&valid, &fixture.context).await?;
    stale.soft_delete(forge_domain::Timestamp::now_utc())?;
    let before = fixture.snapshot().await?.raw;
    let result = match &fixture.backend {
        Backend::Memory(store) => store.begin().await.update_pipeline(&stale, 1).await,
        Backend::Postgres(pool) => {
            let store = PostgresStore::from_pool(pool.clone());
            CommandTransaction::update_pipeline(&mut store.begin().await?, &stale, 1).await
        }
    };
    assert!(matches!(
        result,
        Err(RepositoryError::StaleRevision {
            aggregate: "pipeline"
        })
    ));
    assert_eq!(before, fixture.snapshot().await?.raw);
    let delete = fixture.envelope(
        CommandName::DeletePipeline,
        fixture.snapshot().await?.projects[&fixture.project_id].revision(),
        json!({"pipeline_id":id,"expected_pipeline_revision":2}),
        "delete",
    );
    assert_faults(&fixture, &delete).await?;
    fixture.execute_as(&delete, &fixture.context).await?;
    let repeat=fixture.envelope(CommandName::PublishPipelineVersion,fixture.snapshot().await?.projects[&fixture.project_id].revision(),
        json!({"pipeline_id":id,"expected_pipeline_revision":3,"definition":definition("Too late")}),"publish-deleted");
    fixture
        .assert_unchanged_after(&repeat, &fixture.context)
        .await?;
    Ok(())
}
