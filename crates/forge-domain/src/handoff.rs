//! Immediate canonical stage handoff, independent of asynchronous summaries.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    Actor, ArtifactId, DomainError, EvidenceObject, OutcomeKey, ProjectId, StageId, TaskId,
    Timestamp,
    evidence::{invalid, validate_reference, validate_uuid},
};

/// Authority state frozen when a handoff cites an Artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffArtifactAcceptance {
    Submitted,
    Accepted,
    Rejected,
}

/// Canonical Artifact identity plus its acceptance state at handoff time.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HandoffArtifactReference {
    pub artifact_id: ArtifactId,
    pub acceptance: HandoffArtifactAcceptance,
    /// Required for an accepted/rejected verdict, absent for submission only.
    pub acceptance_id: Option<Uuid>,
}

impl HandoffArtifactReference {
    pub(crate) fn validate(&self) -> Result<(), DomainError> {
        self.artifact_id.validate_v7("handoff.artifact_id")?;
        if let Some(id) = self.acceptance_id {
            validate_uuid(id, "handoff.acceptance_id")?;
        }
        if (self.acceptance == HandoffArtifactAcceptance::Submitted) != self.acceptance_id.is_none()
        {
            return Err(invalid(
                "handoff.artifact_acceptance",
                "acceptance verdict requires its canonical record",
            ));
        }
        Ok(())
    }
}

/// Execution scope that produced the handoff.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HandoffProducer {
    Run { run_id: Uuid },
    ResolutionAssignment { assignment_id: Uuid },
    Human,
}

/// An interrupted attempt cannot claim a successful Pipeline outcome.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HandoffOutcome {
    Completed { outcome: OutcomeKey },
    Interrupted,
}

/// Immutable authoritative stage record. Its public input can be cloned, but
/// callers cannot obtain mutable access to an already issued handoff.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct TaskHandoff(TaskHandoffInput);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TaskHandoffInput {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub actor: Actor,
    pub producer: HandoffProducer,
    pub outcome: HandoffOutcome,
    pub source_stage_id: StageId,
    pub target_stage_id: StageId,
    pub artifacts: Vec<HandoffArtifactReference>,
    pub evidence: Vec<EvidenceObject>,
    pub incident_id: Option<Uuid>,
    pub work_surface_id: Option<Uuid>,
    pub last_observed_state: Option<String>,
    pub created_at: Timestamp,
}

impl TaskHandoff {
    /// Rejects cross-scope evidence and interrupted handoffs that move a stage.
    pub fn new(input: TaskHandoffInput) -> Result<Self, DomainError> {
        validate_uuid(input.id, "handoff.id")?;
        input.project_id.validate_v7("handoff.project_id")?;
        input.task_id.validate_v7("handoff.task_id")?;
        input.actor.validate_snapshot()?;
        input.source_stage_id.validate()?;
        input.target_stage_id.validate()?;
        match input.producer {
            HandoffProducer::Run { run_id } => validate_uuid(run_id, "handoff.run_id")?,
            HandoffProducer::ResolutionAssignment { assignment_id } => {
                validate_uuid(assignment_id, "handoff.assignment_id")?
            }
            HandoffProducer::Human => {}
        }
        for (id, field) in [
            (input.incident_id, "handoff.incident_id"),
            (input.work_surface_id, "handoff.work_surface_id"),
        ] {
            if let Some(id) = id {
                validate_uuid(id, field)?;
            }
        }
        if input.outcome == HandoffOutcome::Interrupted
            && (input.incident_id.is_none() || input.source_stage_id != input.target_stage_id)
        {
            return Err(invalid(
                "handoff.interrupted",
                "requires an incident and preserves the current stage",
            ));
        }
        if let HandoffOutcome::Completed { outcome } = &input.outcome {
            outcome.validate()?;
        }
        if let Some(state) = &input.last_observed_state {
            validate_reference(state, "handoff.last_observed_state")?;
        }
        let mut artifacts = std::collections::BTreeSet::new();
        for artifact in &input.artifacts {
            artifact.validate()?;
            if !artifacts.insert(artifact.artifact_id) {
                return Err(invalid("handoff.artifacts", "must not contain duplicates"));
            }
        }
        for evidence in &input.evidence {
            let scope = evidence.data().scope;
            if scope.project_id != input.project_id
                || scope.task_id != input.task_id
                || matches!(input.producer, HandoffProducer::Run { run_id } if run_id != scope.run_id)
            {
                return Err(invalid(
                    "handoff.evidence",
                    "must belong to this producer's project and task",
                ));
            }
        }
        Ok(Self(input))
    }

    pub fn id(&self) -> Uuid {
        self.0.id
    }

    pub fn data(&self) -> &TaskHandoffInput {
        &self.0
    }
}

impl<'de> Deserialize<'de> for TaskHandoff {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(TaskHandoffInput::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
