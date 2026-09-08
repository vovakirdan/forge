//! A Git outcome is a proposal until the writer is quiescent and Core accepts it.
use crate::{
    ArtifactId, DomainError, EmployeeId, OutcomeKey, PipelineVersionId, ProjectId, StageId, TaskId,
    Timestamp,
    evidence::{invalid, validate_uuid},
    git::{GitCandidate, GitObjectId},
    runtime::RunScope,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

/// Immutable intent captured while an authenticated writer still owns its Run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitStageProposal {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub pipeline_version_id: PipelineVersionId,
    pub stage_id: StageId,
    pub stage_visit: u64,
    pub task_revision: u64,
    pub employee_id: EmployeeId,
    pub scope: RunScope,
    pub surface_id: Uuid,
    pub outcome: OutcomeKey,
    pub artifact_ids: BTreeSet<ArtifactId>,
    pub note: Option<String>,
    pub cancellation_reason_id: Option<crate::CancellationReasonId>,
    pub expected_commit: GitObjectId,
    pub submitted_at: Timestamp,
}
impl GitStageProposal {
    pub fn validate(&self) -> Result<(), DomainError> {
        for (id, field) in [
            (self.id, "git_proposal.id"),
            (self.scope.run_id, "git_proposal.run_id"),
            (self.surface_id, "git_proposal.surface_id"),
        ] {
            validate_uuid(id, field)?;
        }
        self.project_id.validate_v7("git_proposal.project_id")?;
        self.task_id.validate_v7("git_proposal.task_id")?;
        self.pipeline_version_id
            .validate_v7("git_proposal.pipeline_version_id")?;
        self.employee_id.validate_v7("git_proposal.employee_id")?;
        self.stage_id.validate()?;
        self.outcome.validate()?;
        if self.stage_visit == 0
            || self.task_revision == 0
            || self.scope.fencing_token == 0
            || self.scope.environment_epoch == 0
            || self.artifact_ids.len() > 64
            || self.note.as_ref().is_some_and(|note| note.len() > 16384)
        {
            return Err(invalid(
                "git_proposal",
                "invalid revision, scope or evidence bounds",
            ));
        }
        for artifact in &self.artifact_ids {
            artifact.validate_v7("git_proposal.artifact_id")?;
        }
        Ok(())
    }

    /// A positive filesystem observation grants no Pipeline transition by itself.
    pub fn validate_observed_candidate(&self, candidate: &GitCandidate) -> Result<(), DomainError> {
        self.validate()?;
        if candidate.commit != self.expected_commit
            || candidate.tree.as_str().len() != candidate.commit.as_str().len()
        {
            return Err(invalid(
                "git_proposal.candidate",
                "does not match the explicitly submitted commit",
            ));
        }
        Ok(())
    }
}

/// Operational progress; `NeedsAttention` retains the proposal and all work.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitProposalState {
    AwaitingQuiescence,
    Inspecting,
    NeedsAttention,
    Accepted,
    Superseded,
}

/// Core-attributed writers, not the Git author field or last Employee alone.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskGitCandidate {
    pub proposal_id: Uuid,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub surface_id: Uuid,
    pub candidate: GitCandidate,
    pub contributors: BTreeSet<EmployeeId>,
    #[serde(default)]
    pub artifact_ids: BTreeSet<ArtifactId>,
    pub verified_at: Timestamp,
}
impl TaskGitCandidate {
    pub fn validate(&self) -> Result<(), DomainError> {
        validate_uuid(self.proposal_id, "candidate.proposal_id")?;
        validate_uuid(self.surface_id, "candidate.surface_id")?;
        self.project_id.validate_v7("candidate.project_id")?;
        self.task_id.validate_v7("candidate.task_id")?;
        if self.contributors.is_empty()
            || self.artifact_ids.len() > 64
            || self.candidate.commit.as_str().len() != self.candidate.tree.as_str().len()
        {
            return Err(invalid(
                "candidate",
                "requires provenance and consistent Git object format",
            ));
        }
        for employee in &self.contributors {
            employee.validate_v7("candidate.contributor")?;
        }
        for artifact in &self.artifact_ids {
            artifact.validate_v7("candidate.artifact_id")?;
        }
        Ok(())
    }
    pub fn permits_independent_reviewer(&self, employee: EmployeeId) -> bool {
        !self.contributors.contains(&employee)
    }
}
