//! Explicit public read models derived from canonical domain snapshots.

use forge_domain::{
    Actor, ActorKind as DomainActorKind, Artifact, ArtifactBody, ArtifactLink, ArtifactRequirement,
    Employee, PipelineStage, PipelineTransition, PipelineTransitionTarget, PipelineVersion,
    Project, Task, TaskWaitCondition, TaskWaitKind, Timestamp,
};
use forge_protocol::wire::{ActorKind, ActorReference};
use forge_storage::{
    AdmissionResourceSnapshot, EmployeeOperationalCounts, PipelineCatalogItem, RunProjection,
    StoredArtifact,
};
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
pub(crate) struct ProjectTaskPropertySchemaView {
    project_id: String,
    project_revision: u64,
    schema: forge_domain::TaskPropertySchema,
}

#[derive(Serialize)]
pub(crate) struct ProjectResourcesView {
    project_id: String,
    policy: AdmissionPolicyView,
    occupancy: AdmissionOccupancyView,
    observed_at: String,
}

#[derive(Serialize)]
struct AdmissionPolicyView {
    revision: u64,
    host_max_runs: u16,
    project_max_runs: u16,
    credential_account_max_runs: u16,
    updated_at: String,
}

#[derive(Serialize)]
struct AdmissionOccupancyView {
    host_runs: u64,
    project_runs: u64,
    credential_account_runs: Option<u64>,
}

#[derive(Serialize)]
pub(crate) struct EmployeeSummaryView {
    id: String,
    name: String,
    role: String,
    state: forge_domain::EmployeeState,
    revision: u64,
    max_concurrent_runs: u16,
}

#[derive(Serialize)]
pub(crate) struct EmployeeProfileView {
    #[serde(flatten)]
    summary: EmployeeSummaryView,
    project_id: String,
    stage_eligibility: forge_domain::StageEligibility,
    created_at: String,
    updated_at: String,
}

#[derive(Serialize)]
pub(crate) struct EmployeeOperationsView {
    employee_id: String,
    max_concurrent_runs: u16,
    occupied_slots: u64,
    observed_running_runs: u64,
    runtime_binding_configured: bool,
    availability: &'static str,
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
    work_surface_kind: &'static str,
    artifacts: Vec<ArtifactView>,
    wait_conditions: Vec<WaitConditionView>,
    cancellation: Option<CancellationView>,
}

#[derive(Serialize)]
struct CancellationView {
    reason_id: String,
    note: Option<String>,
    cancelled_by: ActorReference,
    cancelled_at: String,
}

#[derive(Serialize)]
pub(crate) struct ArtifactView {
    id: String,
    kind: String,
    title: String,
    metadata: Value,
    body: Value,
    created_at: String,
    producer: forge_domain::ArtifactProducer,
    source_stage_id: Option<String>,
    source_stage_visit: Option<u64>,
    submitted_by: ActorReference,
    submitted_at: String,
    employee_id: Option<String>,
    accepted_as_outcome: bool,
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
    catalog_revision: u64,
    default_version_id: String,
    latest_version: u32,
    deleted_at: Option<String>,
    task_kinds: Vec<forge_domain::TaskKind>,
    entry_stage_id: String,
    max_stage_visits: Option<u32>,
    stages: Vec<PipelineStageView>,
    transitions: Vec<PipelineTransitionView>,
}

#[derive(Serialize)]
pub(crate) struct PipelineCatalogView {
    id: String,
    name: String,
    revision: u64,
    default_version_id: Option<String>,
    latest_version_id: Option<String>,
    latest_version: Option<u32>,
    deleted_at: Option<String>,
    pinned_task_count: u64,
}

#[derive(Serialize)]
pub(crate) struct PipelineStageView {
    id: String,
    name: String,
    executor_kind: forge_domain::ExecutorKind,
    outcomes: Vec<String>,
    instructions: String,
    workspace: Option<forge_domain::StageWorkspaceRequirements>,
    acceptance_policy: Option<forge_domain::candidate_review::StageAcceptancePolicy>,
    system_action: Option<forge_domain::git_integration::SystemStageAction>,
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
    task_id: Option<String>,
    assignment: forge_domain::ExecutionAssignment,
    employee_id: Option<String>,
    stage_id: Option<String>,
    attempt: u32,
    desired_state: forge_storage::RunDesiredState,
    observed_state: forge_storage::RunObservedState,
    lease_fencing_token: u64,
    environment_epoch: u64,
    last_observed_sequence: u64,
    run_spec_version: u16,
}

#[derive(Serialize)]
pub(crate) struct RunDetailView {
    #[serde(flatten)]
    pub(crate) run: RunView,
    pub(crate) diagnostics: Value,
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

pub(crate) fn project_task_property_schema_view(
    project: &Project,
) -> ProjectTaskPropertySchemaView {
    ProjectTaskPropertySchemaView {
        project_id: project.id().to_string(),
        project_revision: project.revision(),
        schema: project.property_schema().clone(),
    }
}

pub(crate) fn project_resources_view(
    project_id: forge_domain::ProjectId,
    snapshot: AdmissionResourceSnapshot,
) -> Result<ProjectResourcesView, CoreError> {
    Ok(ProjectResourcesView {
        project_id: project_id.to_string(),
        policy: AdmissionPolicyView {
            revision: snapshot.policy_revision,
            host_max_runs: snapshot.limits.host_max_runs,
            project_max_runs: snapshot.limits.project_max_runs,
            credential_account_max_runs: snapshot.limits.credential_account_max_runs,
            updated_at: timestamp(Timestamp::from_offset_date_time(snapshot.policy_updated_at))?,
        },
        occupancy: AdmissionOccupancyView {
            host_runs: snapshot.host_occupied_runs,
            project_runs: snapshot.project_occupied_runs,
            credential_account_runs: None,
        },
        observed_at: timestamp(Timestamp::from_offset_date_time(snapshot.observed_at))?,
    })
}

pub(crate) fn pipeline_catalog_view(
    item: PipelineCatalogItem,
) -> Result<PipelineCatalogView, CoreError> {
    Ok(PipelineCatalogView {
        id: item.id.to_string(),
        name: item.name,
        revision: item.revision,
        default_version_id: item.default_version_id.map(|id| id.to_string()),
        latest_version_id: item.latest_version_id.map(|id| id.to_string()),
        latest_version: item.latest_version,
        deleted_at: item
            .deleted_at
            .map(|value| timestamp(Timestamp::from_offset_date_time(value)))
            .transpose()?,
        pinned_task_count: item.pinned_task_count,
    })
}

pub(crate) fn employee_summary_view(employee: &Employee) -> EmployeeSummaryView {
    EmployeeSummaryView {
        id: employee.id().to_string(),
        name: employee.name().to_owned(),
        role: employee.role().as_str().to_owned(),
        state: employee.state(),
        revision: employee.revision(),
        max_concurrent_runs: employee.max_concurrent_runs(),
    }
}

pub(crate) fn employee_profile_view(employee: &Employee) -> Result<EmployeeProfileView, CoreError> {
    Ok(EmployeeProfileView {
        summary: employee_summary_view(employee),
        project_id: employee.project_id().to_string(),
        stage_eligibility: employee.stage_eligibility().clone(),
        created_at: timestamp(employee.created_at())?,
        updated_at: timestamp(employee.updated_at())?,
    })
}

pub(crate) fn employee_operations_view(
    employee: &Employee,
    counts: EmployeeOperationalCounts,
) -> EmployeeOperationsView {
    EmployeeOperationsView {
        employee_id: employee.id().to_string(),
        max_concurrent_runs: employee.max_concurrent_runs(),
        occupied_slots: counts.occupied_slots,
        observed_running_runs: counts.observed_running_runs,
        runtime_binding_configured: counts.runtime_binding_configured,
        availability: "unknown",
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
    let accepted_outcomes = match task.terminal_data() {
        Some(forge_domain::TerminalData::Completed(data)) => {
            data.outcome_artifact_ids()
                .collect::<std::collections::HashSet<_>>()
        }
        _ => std::collections::HashSet::new(),
    };
    let artifacts = read
        .artifacts
        .iter()
        .map(|stored| {
            let link = task
                .artifact_links()
                .iter()
                .find(|link| link.artifact_id() == stored.artifact.id())
                .ok_or(CoreError::InvalidTransport {
                    field: "task.artifact_links",
                    reason: "stored artifact has no Task link".into(),
                })?;
            artifact_view(
                stored,
                link,
                accepted_outcomes.contains(&stored.artifact.id()),
            )
        })
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
        work_surface_kind: match task.work_surface() {
            forge_domain::TaskWorkSurface::None => "none",
            forge_domain::TaskWorkSurface::Git(_) => "git",
        },
        artifacts,
        wait_conditions,
        cancellation: match task.terminal_data() {
            Some(forge_domain::TerminalData::Cancelled(data)) => Some(CancellationView {
                reason_id: data.reason_id().as_str().to_owned(),
                note: data.note().map(ToOwned::to_owned),
                cancelled_by: actor_reference(data.cancelled_by()),
                cancelled_at: timestamp(data.cancelled_at())?,
            }),
            _ => None,
        },
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
        catalog_revision: read.pipeline.revision(),
        default_version_id: read.pipeline.default_version_id().to_string(),
        latest_version: read.pipeline.latest_version(),
        deleted_at: read.pipeline.deleted_at().map(timestamp).transpose()?,
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
        task_id: run.task_id().map(|id| id.to_string()),
        assignment: run.assignment.clone(),
        employee_id: run.employee_id.map(|id| id.to_string()),
        stage_id: run
            .assignment
            .task_stage()
            .map(|owner| owner.stage_id.to_string()),
        attempt: run.attempt_number,
        desired_state: run.desired_state,
        observed_state: run.observed_state,
        lease_fencing_token: run.lease_fencing_token,
        environment_epoch: run.environment_epoch,
        last_observed_sequence: run.last_sequence,
        run_spec_version: run.run_spec_version,
    }
}

fn artifact_view(
    stored: &StoredArtifact,
    link: &ArtifactLink,
    accepted_as_outcome: bool,
) -> Result<ArtifactView, CoreError> {
    let artifact = &stored.artifact;
    Ok(ArtifactView {
        id: artifact.id().to_string(),
        kind: artifact.kind().as_str().to_owned(),
        title: artifact.title().to_owned(),
        metadata: artifact.metadata().clone(),
        body: artifact_body(artifact),
        created_at: timestamp(artifact.created_at())?,
        producer: link.producer(),
        source_stage_id: link.source_stage_id().map(ToString::to_string),
        source_stage_visit: link.source_stage_visit().map(|visit| visit.get()),
        submitted_by: actor_reference(link.submitted_by()),
        submitted_at: timestamp(link.submitted_at())?,
        employee_id: link.employee_id().map(|id| id.to_string()),
        accepted_as_outcome,
    })
}

fn artifact_body(artifact: &Artifact) -> Value {
    match artifact.body() {
        ArtifactBody::InlineJson { value } => value.clone(),
        ArtifactBody::ObjectReference {
            media_type,
            content_digest,
            ..
        } => serde_json::json!({
            "storage": "object_reference",
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
        instructions: stage.instructions().to_owned(),
        workspace: stage.workspace().copied(),
        acceptance_policy: stage.acceptance_policy().cloned(),
        system_action: stage.system_action().cloned(),
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

pub(super) fn timestamp(value: Timestamp) -> Result<String, CoreError> {
    value
        .as_offset_date_time()
        .format(&Rfc3339)
        .map_err(|_| CoreError::InvalidTransport {
            field: "timestamp",
            reason: "cannot be encoded as RFC 3339".to_owned(),
        })
}

#[cfg(test)]
#[path = "views/run_tests.rs"]
mod run_tests;

#[cfg(test)]
mod tests {
    use forge_domain::{
        Actor, ActorId, Artifact, ArtifactBody, ArtifactId, ArtifactKind, EmployeeId, NewArtifact,
        Project, ProjectId, Timestamp,
    };
    use forge_storage::EmployeeOperationalCounts;

    use super::project_view;

    #[test]
    fn project_view_exposes_only_its_control_projection() {
        let project =
            Project::new(ProjectId::new(), "M0", Timestamp::now_utc()).expect("valid fixture");
        let actual = serde_json::to_value(project_view(&project)).expect("view serializes");

        assert_eq!(actual["execution_gate"], "stopped");
    }

    #[test]
    fn artifact_read_omits_object_storage_key() {
        let artifact = Artifact::new(
            ArtifactId::new(),
            NewArtifact {
                project_id: ProjectId::new(),
                kind: ArtifactKind::new(ArtifactKind::PLAN).expect("kind"),
                title: "Stored plan".to_owned(),
                body: ArtifactBody::ObjectReference {
                    object_key: "private/storage/key".to_owned(),
                    media_type: Some("application/json".to_owned()),
                    content_digest: Some("sha256:abc".to_owned()),
                },
                metadata: serde_json::json!({}),
                created_by: Actor::human(ActorId::new()),
                created_at: Timestamp::now_utc(),
            },
        )
        .expect("artifact");
        let body = super::artifact_body(&artifact);
        assert_eq!(body["storage"], "object_reference");
        assert_eq!(body["media_type"], "application/json");
        assert!(body.get("object_key").is_none());
    }

    #[test]
    fn task_property_schema_view_preserves_typed_definitions() {
        let project =
            Project::new(ProjectId::new(), "Schema", Timestamp::now_utc()).expect("project");
        let mut snapshot = serde_json::to_value(project).expect("project snapshot");
        snapshot["property_schema"] = serde_json::json!({
            "definitions": {
                "impact": {
                    "key": "impact", "display_name": "Impact", "property_type": "enum",
                    "required": true, "default_value": {"type":"enum","value":"medium"},
                    "allowed_choices": ["high", "medium"]
                }
            }
        });
        let project: Project = serde_json::from_value(snapshot).expect("typed schema snapshot");
        let actual = serde_json::to_value(super::project_task_property_schema_view(&project))
            .expect("view serializes");
        assert_eq!(actual["project_revision"], 1);
        assert_eq!(
            actual["schema"]["definitions"]["impact"],
            serde_json::json!({
                "key":"impact", "display_name":"Impact", "property_type":"enum",
                "required":true, "default_value":{"type":"enum","value":"medium"},
                "allowed_choices":["high","medium"]
            })
        );
    }

    #[test]
    fn employee_operations_view_keeps_observation_distinct_from_availability() {
        let input: forge_application::CreateEmployeeCommand =
            serde_json::from_value(serde_json::json!({
                "name": "Bob", "role": "developer", "stage_eligibility": {"mode": "any"}
            }))
            .expect("valid fixture");
        let employee = input
            .build(
                EmployeeId::new(),
                ProjectId::new(),
                Actor::human(ActorId::new()),
                Timestamp::now_utc(),
            )
            .expect("employee");
        let actual = serde_json::to_value(super::employee_operations_view(
            &employee,
            EmployeeOperationalCounts {
                occupied_slots: 1,
                observed_running_runs: 0,
                runtime_binding_configured: true,
            },
        ))
        .expect("view serializes");
        assert_eq!(actual["occupied_slots"], 1);
        assert_eq!(actual["observed_running_runs"], 0);
        assert_eq!(actual["runtime_binding_configured"], true);
        assert_eq!(actual["availability"], "unknown");
        assert_eq!(actual.as_object().expect("object").len(), 6);
    }

    #[test]
    fn pipeline_view_exposes_pinned_instructions_and_catalog_without_rewriting_graph() {
        let now = Timestamp::now_utc();
        let input:forge_application::CreatePipelineCommand=serde_json::from_value(serde_json::json!({
            "name":"Review","task_kinds":["delivery"],"entry_stage_id":"review",
            "stages":[{"id":"review","name":"Review","executor_kind":"employee","outcomes":["done"],
                "instructions":"Inspect the accepted revision","workspace":{"kind":"git","access":"read_only"}}],
            "transitions":[{"from_stage_id":"review","outcome":"done","target":{"kind":"done"}}]
        })).unwrap();
        let (mut pipeline, version) = input
            .build(
                forge_domain::PipelineId::new(),
                forge_domain::PipelineVersionId::new(),
                ProjectId::new(),
                forge_domain::Actor::human(forge_domain::ActorId::new()),
                now,
            )
            .unwrap();
        pipeline.soft_delete(now).unwrap();
        let id = version.id().to_string();
        let actual = serde_json::to_value(
            super::pipeline_version_view(super::PipelineVersionRead { pipeline, version }).unwrap(),
        )
        .unwrap();
        assert_eq!(actual["catalog_revision"], 2);
        assert_eq!(actual["latest_version"], 1);
        assert_eq!(actual["default_version_id"], id);
        assert!(actual["deleted_at"].is_string());
        assert_eq!(
            actual["stages"][0]["instructions"],
            "Inspect the accepted revision"
        );
        assert_eq!(
            actual["stages"][0]["workspace"],
            serde_json::json!({"kind":"git","access":"read_only"})
        );
    }
}
