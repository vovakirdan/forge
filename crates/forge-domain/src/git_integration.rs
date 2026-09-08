//! Explicit local system action and fenced, immutable cross-process intent.
use crate::{
    DomainError, ExecutorKind, OutcomeKey, PipelineStage, PipelineVersion, PipelineVersionId,
    ProjectId, StageId, Task, TaskGitBinding, TaskId, TaskWorkSurface, Timestamp,
    evidence::{invalid, validate_uuid},
    git::{GitCandidate, GitIntegrationIntent},
    runtime::RunScope,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SystemStageAction {
    GitIntegration {
        outcomes: IntegrationOutcomes,
        #[serde(default)]
        required_review_stages: BTreeSet<StageId>,
    },
    ProjectHook {
        hook_version_id: Uuid,
        outcomes: crate::HookOutcomes,
    },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationOutcomes {
    pub applied: OutcomeKey,
    pub no_changes: OutcomeKey,
    pub stale_base: OutcomeKey,
}
impl SystemStageAction {
    pub fn validate_for(&self, stage: &PipelineStage) -> Result<(), DomainError> {
        if stage.executor_kind() != ExecutorKind::System
            || stage.acceptance_policy().is_some()
            || stage.workspace().is_some()
        {
            return Err(invalid(
                "stage.system_action",
                "requires a System stage without Employee workspace or acceptance policy",
            ));
        }
        let mapped = match self {
            Self::GitIntegration {
                outcomes,
                required_review_stages,
            } => {
                if required_review_stages.len() > 64 {
                    return Err(invalid(
                        "stage.system_action",
                        "too many required review stages",
                    ));
                }
                for id in required_review_stages {
                    id.validate()?;
                }
                BTreeSet::from([
                    &outcomes.applied,
                    &outcomes.no_changes,
                    &outcomes.stale_base,
                ])
            }
            Self::ProjectHook {
                hook_version_id,
                outcomes,
            } => {
                validate_uuid(*hook_version_id, "stage.hook_version_id")?;
                BTreeSet::from([
                    &outcomes.passed,
                    &outcomes.failed,
                    &outcomes.timed_out,
                    &outcomes.skipped,
                ])
            }
        };
        if mapped
            != stage
                .transitions()
                .map(|transition| transition.outcome())
                .collect()
        {
            return Err(invalid(
                "stage.system_action",
                "outcomes must classify the complete declared action matrix",
            ));
        }
        for key in mapped {
            key.validate()?;
        }
        Ok(())
    }
    pub fn validate_version(
        &self,
        stage: &PipelineStage,
        version: &PipelineVersion,
    ) -> Result<(), DomainError> {
        self.validate_for(stage)?;
        let Self::GitIntegration {
            outcomes,
            required_review_stages,
        } = self
        else {
            return Ok(());
        };
        let target = stage
            .transition(&outcomes.stale_base)
            .map(|transition| transition.target());
        if !matches!(target,Some(crate::PipelineTransitionTarget::Stage(id)) if version.stage(id).is_some_and(|next|next.executor_kind()==ExecutorKind::Employee))
        {
            return Err(invalid(
                "stage.system_action",
                "stale_base must return the same Task to a declared Employee stage",
            ));
        }
        if required_review_stages.iter().any(|id| {
            version
                .stage(id)
                .is_none_or(|stage| stage.acceptance_policy().is_none())
        }) {
            return Err(invalid(
                "stage.system_action",
                "required review stages must declare candidate_review in this version",
            ));
        }
        Ok(())
    }
    /// Registry-backed policy validation; call after loading the immutable hook pin.
    pub fn validate_hook_version(
        &self,
        stage: &PipelineStage,
        version: &PipelineVersion,
        hook: &crate::ProjectHookVersion,
    ) -> Result<(), DomainError> {
        self.validate_for(stage)?;
        hook.validate_snapshot()?;
        let Self::ProjectHook {
            hook_version_id,
            outcomes,
        } = self
        else {
            return Err(invalid("stage.hook", "not a project hook"));
        };
        if *hook_version_id != hook.id || hook.project_id != version.project_id() {
            return Err(invalid("stage.hook", "hook pin or Project does not match"));
        }
        if hook.required {
            for outcome in [&outcomes.failed, &outcomes.timed_out] {
                let target = stage
                    .transition(outcome)
                    .map(|transition| transition.target());
                if !matches!(target,Some(crate::PipelineTransitionTarget::Stage(id)) if version.stage(id).is_some_and(|next| next.executor_kind()==ExecutorKind::Employee || next.executor_kind().requires_wait()))
                {
                    return Err(invalid(
                        "stage.hook",
                        "required failures must return to Employee work or an explicit wait stage",
                    ));
                }
            }
        }
        Ok(())
    }
    pub fn validate_task(&self, task: &Task) -> Result<(), DomainError> {
        // Hook applicability requires its pinned owner configuration. Core can
        // record a non-applicable skip without inventing a Git candidate.
        if matches!(self, Self::GitIntegration { .. })
            && !matches!(task.work_surface(), TaskWorkSurface::Git(_))
        {
            return Err(invalid(
                "stage.system_action",
                "this System action requires a Task-owned Git repository",
            ));
        }
        Ok(())
    }
}

/// Frozen once before preparation. Writer scope is provenance, not a fabricated Run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitIntegrationOperation {
    pub id: Uuid,
    pub fence: u64,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub pipeline_version_id: PipelineVersionId,
    pub stage_id: StageId,
    pub stage_visit: u64,
    pub task_revision: u64,
    pub candidate_proposal_id: Uuid,
    pub candidate: GitCandidate,
    pub binding: TaskGitBinding,
    pub writer: RunScope,
    pub created_at: Timestamp,
}
impl GitIntegrationOperation {
    pub fn validate(&self) -> Result<(), DomainError> {
        for (id, field) in [
            (self.id, "integration.id"),
            (self.candidate_proposal_id, "integration.proposal"),
            (self.writer.run_id, "integration.writer"),
        ] {
            validate_uuid(id, field)?;
        }
        self.project_id.validate_v7("integration.project")?;
        self.task_id.validate_v7("integration.task")?;
        self.pipeline_version_id
            .validate_v7("integration.pipeline")?;
        self.stage_id.validate()?;
        self.binding.validate_snapshot()?;
        if self.fence == 0
            || self.stage_visit == 0
            || self.task_revision == 0
            || self.writer.fencing_token == 0
            || self.writer.environment_epoch == 0
            || self.candidate.commit.as_str().len() != self.candidate.tree.as_str().len()
        {
            return Err(invalid(
                "integration",
                "invalid fence, revision or candidate",
            ));
        }
        Ok(())
    }
    pub fn matches_task(&self, task: &Task) -> bool {
        task.project_id() == self.project_id
            && task.id() == self.task_id
            && task.pipeline().pipeline_version_id() == self.pipeline_version_id
            && task.current_stage_id() == Some(&self.stage_id)
            && task.current_stage_visit().map(|visit| visit.get()) == Some(self.stage_visit)
            && task.work_surface() == &TaskWorkSurface::Git(self.binding.clone())
            && !task.lifecycle().is_terminal()
    }
    pub fn validate_intent(&self, intent: &GitIntegrationIntent) -> Result<(), DomainError> {
        intent
            .validate()
            .map_err(|_| invalid("integration.intent", "invalid prepared merge"))?;
        if intent.operation_id != self.id
            || intent.candidate != self.candidate
            || intent.target_ref != self.binding.target_ref
        {
            return Err(invalid(
                "integration.intent",
                "prepared merge differs from the frozen action",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationPhase {
    Prepare,
    Apply,
    Reconcile,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationCode {
    Prepared,
    Applied,
    NoChanges,
    StaleBase,
    Retryable,
    NoPreparation,
    Unknown,
    Busy,
    Refused,
    Unavailable,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationRequest {
    pub command_id: Uuid,
    pub operation: GitIntegrationOperation,
    pub phase: IntegrationPhase,
    pub intent: Option<GitIntegrationIntent>,
    pub host_id: String,
    pub boot_id: String,
}
impl IntegrationRequest {
    pub fn validate(&self) -> Result<(), DomainError> {
        self.operation.validate()?;
        validate_uuid(self.command_id, "integration.command")?;
        if [self.host_id.as_str(), self.boot_id.as_str()]
            .iter()
            .any(|id| id.is_empty() || id.len() > 128 || id.chars().any(char::is_control))
            || (self.phase == IntegrationPhase::Prepare && self.intent.is_some())
            || (self.phase == IntegrationPhase::Apply && self.intent.is_none())
        {
            return Err(invalid(
                "integration.request",
                "invalid phase or host scope",
            ));
        }
        if let Some(intent) = &self.intent {
            self.operation.validate_intent(intent)?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationResult {
    pub message_id: Uuid,
    pub command_id: Uuid,
    pub operation_id: Uuid,
    pub fence: u64,
    pub host_id: String,
    pub boot_id: String,
    pub code: IntegrationCode,
    pub intent: Option<GitIntegrationIntent>,
}
#[cfg(test)]
mod tests;
