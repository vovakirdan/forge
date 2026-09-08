//! Shared stage/result metadata; a skipped Hook has no invented execution fields.
use forge_domain::{
    PipelineVersionId, ProjectHookVersion, ProjectId, StageId, TaskId, git::GitCandidate,
};
use forge_storage::{HookSkipSpec, StoredHookInvocation};
use uuid::Uuid;

pub(super) struct HookCompletion {
    pub invocation_id: Uuid,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub pipeline_version_id: PipelineVersionId,
    pub stage_id: StageId,
    pub stage_visit: u64,
    pub hook: ProjectHookVersion,
    pub run_id: Option<Uuid>,
    pub candidate_proposal_id: Option<Uuid>,
    pub candidate: Option<GitCandidate>,
    pub surface_id: Option<Uuid>,
}
impl From<&StoredHookInvocation> for HookCompletion {
    fn from(stored: &StoredHookInvocation) -> Self {
        let spec = &stored.spec;
        let source = &spec.assignment;
        Self {
            invocation_id: source.invocation_id,
            project_id: spec.project_id,
            task_id: source.task_id,
            pipeline_version_id: source.pipeline_version_id,
            stage_id: source.stage_id.clone(),
            stage_visit: source.stage_visit,
            hook: spec.hook.clone(),
            run_id: stored.run_id,
            candidate_proposal_id: Some(source.candidate_proposal_id),
            candidate: Some(spec.candidate.clone()),
            surface_id: Some(spec.binding.surface_id),
        }
    }
}
impl From<&HookSkipSpec> for HookCompletion {
    fn from(spec: &HookSkipSpec) -> Self {
        Self {
            invocation_id: spec.invocation_id,
            project_id: spec.project_id,
            task_id: spec.task_id,
            pipeline_version_id: spec.pipeline_version_id,
            stage_id: spec.stage_id.clone(),
            stage_visit: spec.stage_visit,
            hook: spec.hook.clone(),
            run_id: None,
            candidate_proposal_id: None,
            candidate: None,
            surface_id: None,
        }
    }
}
