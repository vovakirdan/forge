use super::{
    commands::{edge, reject},
    fixture::{BackendKind, Fixture},
};
use anyhow::{Context, Result};
use forge_domain::{LifecycleStatus, TaskId};
use forge_protocol::wire::{CommandName, CommandStatus};
use forge_testkit::m0::single_stage_pipeline;
use serde_json::json;

pub async fn dod_edits_and_pipeline_approval(kind: BackendKind) -> Result<()> {
    let fixture = Fixture::create(kind).await?;
    let employee = fixture.pipeline(single_stage_pipeline()).await?;
    let missing:TaskId=fixture.execute(CommandName::CreateTask,json!({
        "title":"Needs DoD","kind":"delivery","pipeline_version_id":employee,"priority":"normal"
    })).await?.resource.context("task")?.id.parse()?;
    reject(
        &fixture,
        CommandName::ApproveTask,
        json!({"task_id":missing,"expected_task_revision":1}),
    )
    .await?;
    fixture
        .task_command(
            CommandName::AmendDraft,
            missing,
            json!({"patch":{"definition_of_done":"Acceptance"}}),
        )
        .await?;
    fixture
        .task_command(
            CommandName::AmendDraft,
            missing,
            json!({"patch":{"title":"Retained DoD"}}),
        )
        .await?;
    assert_eq!(
        fixture.task(missing).await?.spec().definition_of_done(),
        Some("Acceptance")
    );
    let task = fixture.task(missing).await?;
    let clear=fixture.envelope(CommandName::AmendDraft,fixture.snapshot().await?.projects[&fixture.project_id].revision(),json!({
        "task_id":missing,"expected_task_revision":task.revision().get(),"patch":{"definition_of_done":null}
    }),"clear-dod");
    fixture.execute_as(&clear, &fixture.context).await?;
    assert_eq!(
        fixture.task(missing).await?.spec().definition_of_done(),
        None
    );
    let cleared = fixture.snapshot().await?;
    assert_eq!(
        fixture.execute_as(&clear, &fixture.context).await?.status,
        CommandStatus::Replayed
    );
    assert_eq!(fixture.snapshot().await?.raw, cleared.raw);
    reject(&fixture,CommandName::ApproveTask,json!({"task_id":missing,"expected_task_revision":fixture.task(missing).await?.revision().get()})).await?;
    fixture
        .task_command(
            CommandName::AmendDraft,
            missing,
            json!({"patch":{"definition_of_done":"Restored acceptance"}}),
        )
        .await?;
    let blocker = fixture.create_task(employee, "Blocker").await?;
    // Project schema settings are not a public command yet; exercise the domain
    // requirement without rewriting canonical Project snapshots for this test.
    let schema =
        forge_domain::TaskPropertySchema::new([forge_domain::TaskPropertyDefinition::new(
            forge_domain::PropertyKey::new("risk")?,
            "Risk",
            forge_domain::PropertyType::Boolean,
            true,
            None,
            None,
        )?])?;
    let mut scoped = fixture.task(blocker).await?;
    let original = serde_json::to_value(&scoped)?;
    assert!(
        scoped
            .approve(&schema, super::fixture::base_time())
            .is_err()
    );
    assert_eq!(serde_json::to_value(scoped)?, original);
    for executor in ["employee", "human", "external", "dependency", "deleted"] {
        let mut definition = single_stage_pipeline();
        definition["name"] = json!(format!("Approval {executor}"));
        if matches!(executor, "human" | "external") {
            definition["stages"][0]["executor_kind"] = json!(executor);
        }
        let version = fixture.pipeline(definition).await?;
        let task_id = if executor == "employee" {
            missing
        } else {
            fixture.create_task(version, executor).await?
        };
        if executor == "dependency" {
            fixture
                .execute(CommandName::CreateDependency, edge(blocker, task_id))
                .await?;
        }
        if executor == "deleted" {
            let pipeline_id = fixture.snapshot().await?.versions[&version].pipeline_id();
            fixture
                .execute(
                    CommandName::DeletePipeline,
                    json!({"pipeline_id":pipeline_id,"expected_pipeline_revision":1}),
                )
                .await?;
        }
        let before = fixture.snapshot().await?;
        let envelope = fixture.envelope(
            CommandName::ApproveTask,
            before.projects[&fixture.project_id].revision(),
            json!({
                "task_id":task_id,"expected_task_revision":before.tasks[&task_id].revision().get()
            }),
            &format!("approve-{executor}"),
        );
        let receipt = fixture.execute_as(&envelope, &fixture.context).await?;
        let after = fixture.snapshot().await?;
        let task = &after.tasks[&task_id];
        assert_eq!(task.pipeline(), before.tasks[&task_id].pipeline());
        let waiting = matches!(executor, "human" | "external" | "dependency");
        assert_eq!(
            task.lifecycle(),
            if waiting {
                LifecycleStatus::Waiting
            } else {
                LifecycleStatus::Ready
            }
        );
        assert_eq!(task.current_stage_id().context("stage")?.as_str(), "work");
        let queue = after
            .rows("queue_entries")
            .iter()
            .filter(|row| row["task_id"] == json!(task_id) && row["queue_state"] == "queued")
            .count();
        assert_eq!(queue, usize::from(!waiting));
        assert!(after.rows("runs").is_empty());
        assert_eq!(
            receipt.project_revision,
            before.projects[&fixture.project_id].revision() + 1
        );
        assert_eq!(receipt.event_ids.len(), if waiting { 2 } else { 1 });
        assert_eq!(
            fixture
                .execute_as(&envelope, &fixture.context)
                .await?
                .status,
            CommandStatus::Replayed
        );
        assert_eq!(fixture.snapshot().await?.raw, after.raw);
        reject(
            &fixture,
            CommandName::ApproveTask,
            json!({"task_id":task_id,"expected_task_revision":task.revision().get()}),
        )
        .await?;
        after.assert_audit_atomic();
    }
    let mut system = single_stage_pipeline();
    system["name"] = json!("Unsupported system approval");
    system["stages"][0]["executor_kind"] = json!("system");
    let version = fixture.pipeline(system).await?;
    let task = fixture.create_task(version, "Unsupported system").await?;
    reject(
        &fixture,
        CommandName::ApproveTask,
        json!({"task_id":task,"expected_task_revision":1}),
    )
    .await?;
    reject(
        &fixture,
        CommandName::ApproveTask,
        json!({"task_id":blocker,"expected_task_revision":99}),
    )
    .await?;
    reject(
        &fixture,
        CommandName::ApproveTask,
        json!({"task_id":TaskId::new(),"expected_task_revision":1}),
    )
    .await?;
    let mut git_review = single_stage_pipeline();
    git_review["name"] = json!("Approval requires Git surface");
    git_review["stages"][0]["workspace"] = json!({"kind":"git","access":"read_only"});
    git_review["stages"][0]["acceptance_policy"] =
        json!({"kind":"candidate_review","verdicts":{"completed":"accepted"}});
    let version = fixture.pipeline(git_review).await?;
    let task = fixture
        .create_task(version, "Wrong workspace surface")
        .await?;
    reject(
        &fixture,
        CommandName::ApproveTask,
        json!({"task_id":task,"expected_task_revision":1}),
    )
    .await?;
    Ok(())
}
