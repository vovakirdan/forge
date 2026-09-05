//! Explicit public read models derived from canonical domain snapshots.

use forge_domain::{
    Actor, ActorKind as DomainActorKind, Artifact, ArtifactBody, ArtifactRequirement,
    PipelineStage, PipelineTransition, PipelineTransitionTarget, PipelineVersion, Project, Task,
    TaskWaitCondition, TaskWaitKind, Timestamp,
};
use forge_protocol::wire::{ActorKind, ActorReference};
use forge_storage::{RunProjection, StoredArtifact};
use serde::Serialize;
use serde_json::Value;
use time::format_description::well_known::Rfc3339;

use super::reads::{PipelineVersionRead, TaskRead};
use crate::CoreError;

#[derive(Serialize)]
pub(crate) struct HealthView {
    status: &'static str,
    api_version: &'static str,
}

#[derive(Serialize)]
pub(crate) struct ProjectView {
    id: String,
    revision: u64,
    name: String,
    execution_gate: forge_domain::ProjectExecutionGate,
}

#[derive(Serialize)]
pub(crate) struct TaskSummaryView {
    id: String,
    key: String,
    revision: u64,
    title: String,
    kind: forge_domain::TaskKind,
    lifecycle: forge_domain::LifecycleStatus,
    current_stage_id: Option<String>,
    priority: String,
    pipeline_version_id: String,
    updated_at: String,
}

#[derive(Serialize)]
pub(crate) struct TaskDetailView {
    #[serde(flatten)]
    summary: TaskSummaryView,
    description: String,
    definition_of_done: Option<String>,
    properties: Value,
    artifacts: Vec<ArtifactView>,
    wait_conditions: Vec<WaitConditionView>,
}

#[derive(Serialize)]
pub(crate) struct ArtifactView {
    id: String,
    kind: String,
    title: String,
    metadata: Value,
    body: Value,
    created_at: String,
}

#[derive(Serialize)]
pub(crate) struct WaitConditionView {
    id: String,
    kind: String,
    detail: Option<String>,
    source_stage_id: Option<String>,
    created_by: ActorReference,
    created_at: String,
}

#[derive(Serialize)]
pub(crate) struct PipelineVersionView {
    id: String,
    pipeline_id: String,
    version: u32,
    name: String,
    task_kinds: Vec<forge_domain::TaskKind>,
    entry_stage_id: String,
    max_stage_visits: Option<u32>,
    stages: Vec<PipelineStageView>,
    transitions: Vec<PipelineTransitionView>,
}

#[derive(Serialize)]
pub(crate) struct PipelineStageView {
    id: String,
    name: String,
    executor_kind: forge_domain::ExecutorKind,
    outcomes: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct PipelineTransitionView {
    from_stage_id: String,
    outcome: String,
    target: PipelineTargetView,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    artifact_requirements: Vec<ArtifactRequirementView>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PipelineTargetView {
    Stage { stage_id: String },
    Done,
    Cancelled,
}

#[derive(Serialize)]
pub(crate) struct ArtifactRequirementView {
    kind: String,
    minimum_count: u16,
    scope: forge_domain::ArtifactRequirementScope,
}

#[derive(Serialize)]
pub(crate) struct RunView {
    id: String,
    task_id: String,
    employee_id: String,
    stage_id: String,
    attempt: u32,
    desired_state: forge_storage::RunDesiredState,
    observed_state: forge_storage::RunObservedState,
    lease_fencing_token: u64,
    environment_epoch: u64,
    last_observed_sequence: u64,
    run_spec_version: u16,
}

#[derive(Serialize)]
pub(crate) struct ListView<T> {
    pub(crate) items: Vec<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_cursor: Option<String>,
}

pub(crate) fn health_view() -> HealthView {
    HealthView {
        status: "ready",
        api_version: forge_protocol::wire::API_VERSION,
    }
}

pub(crate) fn project_view(project: &Project) -> ProjectView {
    ProjectView {
        id: project.id().to_string(),
        revision: project.revision(),
        name: project.name().to_owned(),
        execution_gate: project.execution_gate(),
    }
}

pub(crate) fn task_summary_view(task: &Task) -> Result<TaskSummaryView, CoreError> {
    Ok(TaskSummaryView {
        id: task.id().to_string(),
        key: task.key().to_string(),
        revision: task.revision().get(),
        title: task.spec().title().to_owned(),
        kind: task.kind(),
        lifecycle: task.lifecycle(),
        current_stage_id: task.current_stage_id().map(ToString::to_string),
        priority: task.priority_level_id().as_str().to_owned(),
        pipeline_version_id: task.pipeline().pipeline_version_id().to_string(),
        updated_at: timestamp(task.updated_at())?,
    })
}

pub(crate) fn task_detail_view(read: TaskRead) -> Result<TaskDetailView, CoreError> {
    let task = &read.task.task;
    let artifacts = read
        .artifacts
        .iter()
        .map(artifact_view)
        .collect::<Result<Vec<_>, _>>()?;
    let wait_conditions = task
        .wait_conditions()
        .map(wait_condition_view)
        .collect::<Result<Vec<_>, _>>()?;
    let properties = serde_json::to_value(task.spec().properties()).map_err(|error| {
        CoreError::InvalidTransport {
            field: "task.properties",
            reason: error.to_string(),
        }
    })?;
    Ok(TaskDetailView {
        summary: task_summary_view(task)?,
        description: task.spec().description().to_owned(),
        definition_of_done: task.spec().definition_of_done().map(ToOwned::to_owned),
        properties,
        artifacts,
        wait_conditions,
    })
}

pub(crate) fn pipeline_version_view(
    read: PipelineVersionRead,
) -> Result<PipelineVersionView, CoreError> {
    let stages = read
        .version
        .stages()
        .map(pipeline_stage_view)
        .collect::<Vec<_>>();
    let transitions = read
        .version
        .stages()
        .flat_map(|stage| {
            stage
                .transitions()
                .map(move |transition| pipeline_transition_view(stage, transition))
        })
        .collect::<Vec<_>>();
    Ok(PipelineVersionView {
        id: read.version.id().to_string(),
        pipeline_id: read.version.pipeline_id().to_string(),
        version: read.version.version(),
        name: read.pipeline.name().to_owned(),
        task_kinds: supported_task_kinds(&read.version),
        entry_stage_id: read.version.entry_stage_id().to_string(),
        max_stage_visits: read.version.max_stage_visits(),
        stages,
        transitions,
    })
}

pub(crate) fn run_view(run: RunProjection) -> RunView {
    RunView {
        id: run.id.to_string(),
        task_id: run.task_id.to_string(),
        employee_id: run.employee_id.to_string(),
        stage_id: run.stage_id,
        attempt: run.attempt_number,
        desired_state: run.desired_state,
        observed_state: run.observed_state,
        lease_fencing_token: run.lease_fencing_token,
        environment_epoch: run.environment_epoch,
        last_observed_sequence: run.last_sequence,
        run_spec_version: run.run_spec_version,
    }
}

fn artifact_view(stored: &StoredArtifact) -> Result<ArtifactView, CoreError> {
    let artifact = &stored.artifact;
    Ok(ArtifactView {
        id: artifact.id().to_string(),
        kind: artifact.kind().as_str().to_owned(),
        title: artifact.title().to_owned(),
        metadata: artifact.metadata().clone(),
        body: artifact_body(artifact),
        created_at: timestamp(artifact.created_at())?,
    })
}

fn artifact_body(artifact: &Artifact) -> Value {
    match artifact.body() {
        ArtifactBody::InlineJson { value } => value.clone(),
        ArtifactBody::ObjectReference {
            object_key,
            media_type,
            content_digest,
        } => serde_json::json!({
            "storage": "object_reference",
            "object_key": object_key,
            "media_type": media_type,
            "content_digest": content_digest,
        }),
    }
}

fn wait_condition_view(condition: &TaskWaitCondition) -> Result<WaitConditionView, CoreError> {
    Ok(WaitConditionView {
        id: condition.id().to_string(),
        kind: wait_kind(condition.kind()).to_owned(),
        detail: condition.detail().map(ToOwned::to_owned),
        source_stage_id: condition.source_stage_id().map(ToString::to_string),
        created_by: actor_reference(condition.created_by()),
        created_at: timestamp(condition.created_at())?,
    })
}

fn pipeline_stage_view(stage: &PipelineStage) -> PipelineStageView {
    PipelineStageView {
        id: stage.id().to_string(),
        name: stage.display_name().to_owned(),
        executor_kind: stage.executor_kind(),
        outcomes: stage
            .transitions()
            .map(|transition| transition.outcome().as_str().to_owned())
            .collect(),
    }
}

fn pipeline_transition_view(
    stage: &PipelineStage,
    transition: &PipelineTransition,
) -> PipelineTransitionView {
    PipelineTransitionView {
        from_stage_id: stage.id().to_string(),
        outcome: transition.outcome().as_str().to_owned(),
        target: transition_target(transition.target()),
        artifact_requirements: transition
            .artifact_requirements()
            .iter()
            .map(artifact_requirement_view)
            .collect(),
    }
}

fn transition_target(target: &PipelineTransitionTarget) -> PipelineTargetView {
    match target {
        PipelineTransitionTarget::Stage(stage_id) => PipelineTargetView::Stage {
            stage_id: stage_id.to_string(),
        },
        PipelineTransitionTarget::Done => PipelineTargetView::Done,
        PipelineTransitionTarget::Cancelled => PipelineTargetView::Cancelled,
    }
}

fn artifact_requirement_view(requirement: &ArtifactRequirement) -> ArtifactRequirementView {
    ArtifactRequirementView {
        kind: requirement.kind().as_str().to_owned(),
        minimum_count: requirement.minimum_count(),
        scope: requirement.scope(),
    }
}

fn supported_task_kinds(version: &PipelineVersion) -> Vec<forge_domain::TaskKind> {
    [
        forge_domain::TaskKind::Delivery,
        forge_domain::TaskKind::Analysis,
    ]
    .into_iter()
    .filter(|kind| version.supports_task_kind(*kind))
    .collect()
}

fn wait_kind(kind: &TaskWaitKind) -> &str {
    match kind {
        TaskWaitKind::Dependency => "dependency",
        TaskWaitKind::DecisionRequired => "decision_required",
        TaskWaitKind::ManualPause => "manual_pause",
        TaskWaitKind::EscalationPending => "escalation_pending",
        TaskWaitKind::RetryExhausted => "retry_exhausted",
        TaskWaitKind::Interrupted => "interrupted",
        TaskWaitKind::Other(other) => other.as_str(),
    }
}

fn actor_reference(actor: Actor) -> ActorReference {
    let kind = match actor.kind() {
        DomainActorKind::Human => ActorKind::Human,
        DomainActorKind::Employee => ActorKind::Employee,
        DomainActorKind::SystemManager => ActorKind::SystemManager,
        DomainActorKind::Core => ActorKind::Core,
        DomainActorKind::Supervisor => ActorKind::Supervisor,
    };
    ActorReference {
        kind,
        id: actor.id().to_string(),
    }
}

fn timestamp(value: Timestamp) -> Result<String, CoreError> {
    value
        .as_offset_date_time()
        .format(&Rfc3339)
        .map_err(|_| CoreError::InvalidTransport {
            field: "timestamp",
            reason: "cannot be encoded as RFC 3339".to_owned(),
        })
}

#[cfg(test)]
mod tests {
    use forge_domain::{Project, ProjectId, Timestamp};

    use super::project_view;

    #[test]
    fn project_view_exposes_only_its_control_projection() {
        let project =
            Project::new(ProjectId::new(), "M0", Timestamp::now_utc()).expect("valid fixture");
        let actual = serde_json::to_value(project_view(&project)).expect("view serializes");

        assert_eq!(actual["execution_gate"], "stopped");
    }
}
