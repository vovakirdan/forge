use super::fixture::{BackendKind, Fixture};
use anyhow::{Context, Result};
use forge_domain::{LifecycleStatus, PipelineVersionId, TaskId};
use forge_protocol::wire::{CommandName, CommandStatus};
use forge_testkit::m0::single_stage_pipeline;
use serde_json::json;

pub async fn pinned_draft_replays_without_work(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let mut definition = single_stage_pipeline();
    definition["task_kinds"] = json!(["delivery", "analysis"]);
    let older = fixture.pipeline(definition.clone()).await?;
    let pipeline_id = fixture.snapshot().await?.versions[&older].pipeline_id();
    definition
        .as_object_mut()
        .context("definition")?
        .remove("name");
    let latest:PipelineVersionId=fixture.execute(CommandName::PublishPipelineVersion,json!({
        "pipeline_id":pipeline_id,"expected_pipeline_revision":1,"definition":definition,"make_default":true
    })).await?.resource.context("published")?.id.parse()?;
    for (number, (task_kind, version)) in [("delivery", older), ("analysis", latest)]
        .into_iter()
        .enumerate()
    {
        let before = fixture.snapshot().await?;
        let request = fixture.envelope(
            CommandName::CreateTask,
            before.projects[&fixture.project_id].revision(),
            json!({
                "title":format!("Created {task_kind}"),"description":"", "kind":task_kind,
                "pipeline_version_id":version,"priority":"high","properties":{}
            }),
            &format!("create-{number}"),
        );
        let receipt = fixture.execute_as(&request, &fixture.context).await?;
        assert_eq!(receipt.status, CommandStatus::Applied);
        let id: TaskId = receipt
            .resource
            .as_ref()
            .context("created task")?
            .id
            .parse()?;
        let after = fixture.snapshot().await?;
        let task = &after.tasks[&id];
        assert_eq!(task.lifecycle(), LifecycleStatus::Draft);
        assert_eq!(task.pipeline().pipeline_version_id(), version);
        assert_eq!(task.revision().get(), 1);
        assert_eq!(task.spec().definition_of_done(), None);
        assert_eq!(task.current_stage_id(), None);
        assert_eq!(task.priority_level_id().as_str(), "high");
        assert_eq!(after.tasks.len(), before.tasks.len() + 1);
        assert_eq!(
            receipt.project_revision,
            before.projects[&fixture.project_id].revision() + 1
        );
        assert!(
            after
                .rows("event_log")
                .iter()
                .any(|event| event["id"] == receipt.event_ids[0]
                    && event["event_type"] == "task_created")
        );
        for table in ["runs", "queue_entries", "artifacts"] {
            assert_eq!(after.rows(table), before.rows(table), "{table}");
        }
        let replay = fixture.execute_as(&request, &fixture.context).await?;
        assert_eq!(replay.status, CommandStatus::Replayed);
        assert_eq!(replay.resource, receipt.resource);
        assert_eq!(replay.command_id, receipt.command_id);
        assert_eq!(after.raw, fixture.snapshot().await?.raw);
        // Build a distinct request under the original key; never silently create again.
        let changed = fixture.envelope(
            CommandName::CreateTask,
            before.projects[&fixture.project_id].revision(),
            json!({
                "title":"Different", "description":"", "kind":task_kind,
                "pipeline_version_id":version,"priority":"high","properties":{}
            }),
            &format!("create-{number}"),
        );
        fixture
            .assert_unchanged_after(&changed, &fixture.context)
            .await?;
        after.assert_audit_atomic();
    }
    Ok(())
}
