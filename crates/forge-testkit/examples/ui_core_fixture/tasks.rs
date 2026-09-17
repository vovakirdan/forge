//! Browser coordinates created exclusively through Core named commands.

use anyhow::{Context, Result, ensure};
use forge_domain::{LifecycleStatus, PipelineVersionId, ProjectId, TaskId, TaskWaitKind};
use forge_protocol::wire::CommandName;
use forge_testkit::m0::{M0Harness, single_stage_pipeline};
use serde_json::{Value, json};

const FEATURED_TITLE: &str = "План миграции — review <img src=x onerror=window.forgeInjected=true>";
const SECOND_TITLE: &str = "Analyze durable project isolation";
const DRAFT_TITLE: &str = "Draft with typed migration properties";
const STAGE_NAME: &str = "Review original migration plan";
const ARTIFACT_TITLES: [&str; 2] = ["Migration plan", "Planning evidence"];
const TOTAL_TASKS: usize = 23;

pub struct FixtureData {
    pub tasks: Value,
    pub second_project: Value,
    pub empty_project: Value,
}

pub async fn seed(harness: &M0Harness, project: ProjectId) -> Result<FixtureData> {
    let mut definition = human_pipeline(STAGE_NAME);
    definition["name"] = json!("Versioned migration planning");
    let version = harness.create_pipeline(project, definition).await?;
    let pipeline = harness
        .store
        .load_pipeline_version(version)
        .await?
        .context("fixture Pipeline version missing")?
        .pipeline_id();
    let mut queue_definition = single_stage_pipeline();
    queue_definition["task_kinds"] = json!(["delivery", "analysis"]);
    let queue_version = harness.create_pipeline(project, queue_definition).await?;

    // Core sorts by project-local Task sequence. The first three rows remain on
    // page one; the additional drafts force a genuine second page at limit 20.
    let featured = create_task(
        harness,
        project,
        version,
        FEATURED_TITLE,
        "delivery",
        json!({}),
    )
    .await?;
    let second = create_task(
        harness,
        project,
        queue_version,
        SECOND_TITLE,
        "analysis",
        json!({}),
    )
    .await?;
    // No public command configures Project property definitions yet. Typed
    // values can be saved in a draft but must not bypass approval validation.
    let draft = create_task(
        harness,
        project,
        version,
        DRAFT_TITLE,
        "delivery",
        json!({
            "contains_migrations": {"type": "boolean", "value": true},
            "estimate": {"type": "number", "value": 3},
            "note": {"type": "text", "value": "Needs an owner-defined property schema"}
        }),
    )
    .await?;
    let cancelled = create_task(
        harness,
        project,
        version,
        "Cancelled migration alternative",
        "analysis",
        json!({}),
    )
    .await?;
    harness.cancel_task(project, cancelled).await?;
    for number in 5..=TOTAL_TASKS {
        create_task(
            harness,
            project,
            version,
            &format!("Pagination draft {number:02}"),
            "delivery",
            json!({}),
        )
        .await?;
    }
    harness.approve_task(project, second).await?;
    harness.approve_task(project, featured).await?;
    attach_planning_outcome(harness, project, featured).await?;

    let catalog = harness
        .store
        .load_pipeline(pipeline)
        .await?
        .context("fixture Pipeline missing")?;
    let new_version: PipelineVersionId = harness
        .execute(
            project,
            CommandName::PublishPipelineVersion,
            json!({
                "pipeline_id": pipeline,
                "expected_pipeline_revision": catalog.revision(),
                "definition": human_pipeline("Review replacement v2"),
                "make_default": true
            }),
        )
        .await?
        .resource
        .context("published version receipt missing")?
        .id
        .parse()?;
    let catalog = harness
        .store
        .load_pipeline(pipeline)
        .await?
        .context("published Pipeline missing")?;
    harness
        .execute(
            project,
            CommandName::DeletePipeline,
            json!({
                "pipeline_id": pipeline,
                "expected_pipeline_revision": catalog.revision()
            }),
        )
        .await?;

    let featured_task = harness.required_task(featured).await?.task;
    let second_task = harness.required_task(second).await?.task;
    let draft_task = harness.required_task(draft).await?.task;
    ensure!(
        featured_task.lifecycle() == LifecycleStatus::Waiting,
        "featured Task must wait for a human"
    );
    ensure!(
        featured_task.current_stage_id().map(|stage| stage.as_str()) == Some("review"),
        "featured Task must remain at the review stage"
    );
    ensure!(
        featured_task.pipeline().pipeline_version_id() == version,
        "featured Task must retain its v1 binding"
    );
    ensure!(
        featured_task.artifact_links().len() == 2,
        "featured Task must contain two planning artifacts"
    );
    ensure!(
        second_task.lifecycle() == LifecycleStatus::Ready,
        "second Task must remain ready without an Employee"
    );
    ensure!(
        draft_task.lifecycle() == LifecycleStatus::Draft,
        "typed properties must remain in a draft"
    );
    ensure!(
        harness.required_task(cancelled).await?.task.lifecycle() == LifecycleStatus::Cancelled,
        "cancelled Task must be terminal"
    );
    let tasks = harness.store.list_tasks(project).await?;
    ensure!(
        tasks.len() == TOTAL_TASKS,
        "main Project must exercise pagination"
    );
    ensure!(
        tasks
            .iter()
            .take(3)
            .map(|task| task.task.id())
            .eq([featured, second, draft]),
        "featured Tasks must remain on page one"
    );
    let archived = harness
        .store
        .load_pipeline(pipeline)
        .await?
        .context("archived Pipeline missing")?;
    ensure!(
        archived.is_deleted() && archived.default_version_id() == new_version,
        "Pipeline must be archived with the v2 default"
    );
    ensure!(
        harness
            .store
            .load_pipeline_version(version)
            .await?
            .is_some(),
        "archiving must preserve the pinned version"
    );
    ensure!(
        harness.store.list_employees(project).await?.is_empty(),
        "fixture must not create Employees"
    );
    ensure!(
        harness
            .store
            .list_runs_for_project(project)
            .await?
            .is_empty(),
        "fixture must not create Runs"
    );

    let second_project = isolated_project(harness).await?;
    let empty_name = "Empty live Project";
    let empty_project = harness.create_project(empty_name).await?;
    ensure!(
        harness.store.list_tasks(empty_project).await?.is_empty(),
        "empty Project must have no Tasks"
    );
    Ok(FixtureData {
        tasks: json!({
            "total": TOTAL_TASKS,
            "featured_id": featured,
            "featured_key": featured_task.key().to_string(),
            "featured_title": FEATURED_TITLE,
            "second_id": second,
            "second_key": second_task.key().to_string(),
            "draft_id": draft,
            "draft_key": draft_task.key().to_string(),
            "pinned_version_id": version,
            "default_version_id": new_version,
            "pipeline_id": pipeline,
            "stage_id": "review",
            "stage_name": STAGE_NAME,
            "artifact_titles": ARTIFACT_TITLES
        }),
        second_project,
        empty_project: json!({"id": empty_project, "name": empty_name}),
    })
}

async fn create_task(
    harness: &M0Harness,
    project: ProjectId,
    version: PipelineVersionId,
    title: &str,
    kind: &str,
    properties: Value,
) -> Result<TaskId> {
    Ok(harness.execute(project, CommandName::CreateTask, json!({
        "title": title,
        "description": "Compare migration options, record the plan, and await owner review. Treat <script>window.forgeInjected=true</script> as plain text.",
        "definition_of_done": "The owner has reviewed the migration plan and its supporting evidence.",
        "kind": kind,
        "pipeline_version_id": version,
        "priority": "normal",
        "properties": properties
    })).await?.resource.context("created Task receipt missing")?.id.parse()?)
}

async fn attach_planning_outcome(
    harness: &M0Harness,
    project: ProjectId,
    task: TaskId,
) -> Result<()> {
    let current = harness.required_task(task).await?.task;
    let wait = current
        .wait_conditions()
        .find(|wait| wait.kind() == &TaskWaitKind::DecisionRequired)
        .context("planning stage must have a decision wait")?;
    harness.execute(project, CommandName::SubmitExternalStageOutcome, json!({
        "task_id": task,
        "expected_task_revision": current.revision().get(),
        "stage_id": "plan",
        "outcome": "planned",
        "wait_condition_id": wait.id(),
        "artifacts": [
            {"kind": "stage_evidence", "title": ARTIFACT_TITLES[0],
                "metadata": {"format": "markdown", "iteration": 1, "approved": false},
                "body": {"plan": "Prepare a backup, rehearse the migration, then ask the owner."}},
            {"kind": "stage_evidence", "title": ARTIFACT_TITLES[1],
                "metadata": {"alternatives": ["incremental", "single cutover"], "note": "<img src=x onerror=window.forgeInjected=true>"},
                "body": {"decision": "Awaiting owner review", "checks": ["backup", "rollback"],
                    "url": "https://evidence.invalid/should-not-load", "object_ref": "objects/should-not-load"}}
        ]
    })).await?;
    Ok(())
}

fn human_pipeline(review_name: &str) -> Value {
    json!({
        "task_kinds": ["delivery", "analysis"],
        "entry_stage_id": "plan",
        "stages": [
            {"id": "plan", "name": "Plan migration", "executor_kind": "human", "outcomes": ["planned"]},
            {"id": "review", "name": review_name, "executor_kind": "human", "outcomes": ["accepted"]}
        ],
        "transitions": [
            {"from_stage_id": "plan", "outcome": "planned", "target": {"kind": "stage", "stage_id": "review"},
                "artifact_requirements": [{"kind": "stage_evidence", "minimum_count": 2, "scope": "current_stage"}]},
            {"from_stage_id": "review", "outcome": "accepted", "target": {"kind": "done"}}
        ]
    })
}

async fn isolated_project(harness: &M0Harness) -> Result<Value> {
    let name = "Isolated second live Project";
    let project = harness.create_project(name).await?;
    let mut definition = human_pipeline("Separate project review");
    definition["name"] = json!("Separate project Pipeline");
    let version = harness.create_pipeline(project, definition).await?;
    let title = "Second Project private analysis";
    let task = create_task(harness, project, version, title, "analysis", json!({})).await?;
    Ok(json!({"id": project, "name": name, "task_id": task, "task_title": title}))
}
