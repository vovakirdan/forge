//! Pipeline-defined candidate assessment, not automatic semantic verification.
use crate::{
    ArtifactId, DomainError, EmployeeId, ExecutorKind, OutcomeKey, PipelineStage,
    PipelineVersionId, ProjectId, StageId, StageWorkspaceKind, StageWorkspaceRequirements, TaskId,
    Timestamp,
    evidence::{invalid, validate_uuid},
    git::GitCandidate,
    runtime::{RunScope, SurfaceAccess},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Meaning assigned by the Pipeline to one of its named stage outcomes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateVerdict {
    Accepted,
    Rejected,
    Inconclusive,
}

/// Optional read-only Employee assessment of an explicitly pinned Git revision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StageAcceptancePolicy {
    CandidateReview {
        /// Independence is explicit; omission conservatively forbids every writer.
        #[serde(default = "independent_default")]
        independent: bool,
        /// Every declared outcome must be classified; names are Project-defined.
        verdicts: BTreeMap<OutcomeKey, CandidateVerdict>,
    },
}
fn independent_default() -> bool {
    true
}
impl StageAcceptancePolicy {
    pub fn validate_task_surface(
        &self,
        surface: &crate::TaskWorkSurface,
    ) -> Result<(), DomainError> {
        if !matches!(surface, crate::TaskWorkSurface::Git(_)) {
            return Err(invalid(
                "stage.acceptance_policy",
                "candidate review requires a Task-owned Git binding regardless of independence policy",
            ));
        }
        Ok(())
    }
    pub fn requires_independence(&self) -> bool {
        match self {
            Self::CandidateReview { independent, .. } => *independent,
        }
    }
    pub fn verdict(&self, outcome: &OutcomeKey) -> Option<CandidateVerdict> {
        match self {
            Self::CandidateReview { verdicts, .. } => verdicts.get(outcome).copied(),
        }
    }
    pub fn validate_for(&self, stage: &PipelineStage) -> Result<(), DomainError> {
        if stage.executor_kind() != ExecutorKind::Employee
            || stage.workspace()
                != Some(&StageWorkspaceRequirements {
                    kind: StageWorkspaceKind::Git,
                    access: SurfaceAccess::ReadOnly,
                })
        {
            return Err(invalid(
                "stage.acceptance_policy",
                "candidate review requires an Employee stage with Git read-only workspace",
            ));
        }
        let Self::CandidateReview { verdicts, .. } = self;
        let declared: BTreeSet<_> = stage
            .transitions()
            .map(|transition| transition.outcome())
            .collect();
        if verdicts.keys().collect::<BTreeSet<_>>() != declared {
            return Err(invalid(
                "stage.acceptance_policy",
                "verdicts must classify every declared Pipeline outcome exactly",
            ));
        }
        for outcome in verdicts.keys() {
            outcome.validate()?;
        }
        Ok(())
    }
}

/// Immutable assessment provenance. Verdict does not claim objective truth.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateReviewRecord {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub pipeline_version_id: PipelineVersionId,
    pub stage_id: StageId,
    pub stage_visit: u64,
    pub employee_id: EmployeeId,
    pub scope: RunScope,
    pub candidate_proposal_id: Uuid,
    pub candidate: GitCandidate,
    pub subject_artifact_ids: BTreeSet<ArtifactId>,
    pub verdict_artifact_ids: BTreeSet<ArtifactId>,
    pub outcome: OutcomeKey,
    pub verdict: CandidateVerdict,
    pub recorded_at: Timestamp,
}
impl CandidateReviewRecord {
    pub fn validate(&self) -> Result<(), DomainError> {
        for (id, field) in [
            (self.id, "review.id"),
            (self.scope.run_id, "review.run_id"),
            (self.candidate_proposal_id, "review.proposal_id"),
        ] {
            validate_uuid(id, field)?;
        }
        self.project_id.validate_v7("review.project_id")?;
        self.task_id.validate_v7("review.task_id")?;
        self.employee_id.validate_v7("review.employee_id")?;
        self.pipeline_version_id
            .validate_v7("review.pipeline_version_id")?;
        self.stage_id.validate()?;
        self.outcome.validate()?;
        if self.stage_visit == 0
            || self.scope.fencing_token == 0
            || self.scope.environment_epoch == 0
            || self.candidate.commit.as_str().len() != self.candidate.tree.as_str().len()
            || self.subject_artifact_ids.len() > 64
            || self.verdict_artifact_ids.len() > 64
        {
            return Err(invalid("review", "invalid scope or evidence bounds"));
        }
        for artifact in self
            .subject_artifact_ids
            .iter()
            .chain(&self.verdict_artifact_ids)
        {
            artifact.validate_v7("review.artifact_id")?;
        }
        Ok(())
    }
}
