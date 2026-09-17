//! Pipeline read fixtures use named commands only and never open their Projects.

use anyhow::{Context, Result, ensure};
use forge_domain::{PipelineVersionId, ProjectId};
use forge_protocol::wire::CommandName;
use forge_testkit::m0::M0Harness;
use serde_json::{Value, json};

const TOTAL_VERSIONS: usize = 23;
const ENTRY_STAGE: &str = "discover_plan";
const ENTRY_NAME: &str = "Discover a migration strategy";

pub struct FixtureData {
    pub project: Value,
    pub other_project: Value,
}

pub async fn seed(harness: &M0Harness) -> Result<FixtureData> {
    let name = "Pipeline versions fixture";
    let project = harness.create_project(name).await?;
    let featured = create(harness, project, "Migration strategy catalog").await?;
    let pipeline_id = harness
        .store
        .load_pipeline_version(featured)
        .await?
        .context("featured Pipeline version missing")?
        .pipeline_id();
    let catalog = harness
        .store
        .load_pipeline(pipeline_id)
        .await?
        .context("featured Pipeline catalog missing")?;
    let mut newer = definition();
    newer["stages"][0]["name"] = json!("Replacement migration strategy v2");
    let second: PipelineVersionId = harness
        .execute(
            project,
            CommandName::PublishPipelineVersion,
            json!({
                "pipeline_id": pipeline_id,
                "expected_pipeline_revision": catalog.revision(),
                "definition": newer,
                "make_default": false
            }),
        )
        .await?
        .resource
        .context("published Pipeline version receipt missing")?
        .id
        .parse()?;
    let deleted = create(harness, project, "Archived migration catalog").await?;
    let deleted_pipeline = harness
        .store
        .load_pipeline_version(deleted)
        .await?
        .context("archived Pipeline version missing")?
        .pipeline_id();
    let catalog = harness
        .store
        .load_pipeline(deleted_pipeline)
        .await?
        .context("archived Pipeline catalog missing")?;
    harness
        .execute(
            project,
            CommandName::DeletePipeline,
            json!({
                "pipeline_id": deleted_pipeline,
                "expected_pipeline_revision": catalog.revision()
            }),
        )
        .await?;
    for number in 4..=TOTAL_VERSIONS {
        create(harness, project, &format!("Read-only pipeline {number:02}")).await?;
    }
    let versions = harness.store.list_pipeline_versions(project).await?;
    ensure!(
        versions.len() == TOTAL_VERSIONS,
        "Pipeline pagination fixture is incomplete"
    );
    ensure!(
        versions
            .iter()
            .take(20)
            .any(|version| version.id() == featured)
            && versions
                .iter()
                .take(20)
                .any(|version| version.id() == second)
            && versions
                .iter()
                .take(20)
                .any(|version| version.id() == deleted),
        "representative Pipeline versions must remain on page one"
    );
    let catalog = harness
        .store
        .load_pipeline(pipeline_id)
        .await?
        .context("Pipeline missing")?;
    ensure!(
        catalog.default_version_id() == featured,
        "publishing must retain the older default"
    );
    ensure!(
        harness
            .store
            .load_pipeline(deleted_pipeline)
            .await?
            .context("archived Pipeline missing")?
            .is_deleted()
            && harness
                .store
                .load_pipeline_version(deleted)
                .await?
                .is_some(),
        "soft deletion must preserve the historical version"
    );
    assert_no_work(harness, project).await?;

    let other_name = "Separate Pipeline versions fixture";
    let other = harness.create_project(other_name).await?;
    let other_version = create(harness, other, "Separate project pipeline").await?;
    assert_no_work(harness, other).await?;
    Ok(FixtureData {
        project: json!({
            "id": project, "name": name, "total": TOTAL_VERSIONS,
            "featured_id": featured, "second_id": second, "deleted_id": deleted,
            "pipeline_id": pipeline_id, "stage_id": ENTRY_STAGE, "stage_name": ENTRY_NAME
        }),
        other_project: json!({"id": other, "name": other_name, "version_id": other_version}),
    })
}

async fn create(harness: &M0Harness, project: ProjectId, name: &str) -> Result<PipelineVersionId> {
    let mut value = definition();
    value["name"] = json!(name);
    harness.create_pipeline(project, value).await
}

async fn assert_no_work(harness: &M0Harness, project: ProjectId) -> Result<()> {
    ensure!(
        harness.store.list_tasks(project).await?.is_empty(),
        "Pipeline fixture must not create Tasks"
    );
    ensure!(
        harness.store.list_employees(project).await?.is_empty(),
        "Pipeline fixture must not create Employees"
    );
    ensure!(
        harness
            .store
            .list_runs_for_project(project)
            .await?
            .is_empty(),
        "Pipeline fixture must not create Runs"
    );
    Ok(())
}

fn definition() -> Value {
    // A real page of complete definitions exceeds the old 64 KiB list limit.
    // Each instruction stays far below the canonical 64 KiB per-stage limit.
    let instructions = format!(
        "Treat <img src=x onerror=window.forgeInjected=true> as plain text.\n{}",
        "Compare alternatives, document evidence, and ask the owner before migration.\n"
            .repeat(100)
    );
    json!({
        "task_kinds": ["delivery", "analysis"], "entry_stage_id": ENTRY_STAGE,
        "max_stage_visits": 7,
        "stages": [
            {"id": ENTRY_STAGE, "name": ENTRY_NAME, "executor_kind": "human",
                "outcomes": ["planned", "withdrawn"], "instructions": instructions},
            {"id": "peer_exchange", "name": "Peer exchange", "executor_kind": "external",
                "outcomes": ["accepted"], "instructions": "Read the plan and provide an external verdict."}
        ],
        "transitions": [
            {"from_stage_id": ENTRY_STAGE, "outcome": "planned",
                "target": {"kind": "stage", "stage_id": "peer_exchange"},
                "artifact_requirements": [{"kind": "migration_plan", "minimum_count": 2, "scope": "current_stage"}]},
            {"from_stage_id": ENTRY_STAGE, "outcome": "withdrawn", "target": {"kind": "cancelled"}},
            {"from_stage_id": "peer_exchange", "outcome": "accepted", "target": {"kind": "done"},
                "artifact_requirements": [{"kind": "review_evidence", "minimum_count": 1, "scope": "task_history"}]}
        ]
    })
}
