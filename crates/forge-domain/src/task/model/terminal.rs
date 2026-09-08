use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    Actor, ArtifactId, CancellationReasonId, PipelineId, PipelineVersionId, PriorityLevelId,
    ProjectId, StageId, TaskId, TaskKey, Timestamp,
};

use super::{TaskKind, TaskSource, TaskSpec};

/// Pipeline version selected for a Task and the stage at which it begins.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TaskPipelineBinding {
    pipeline_id: PipelineId,
    pipeline_version_id: PipelineVersionId,
    entry_stage_id: StageId,
}

impl TaskPipelineBinding {
    /// Binds a Task to one immutable Pipeline version and its entry stage.
    #[must_use]
    pub const fn new(
        pipeline_id: PipelineId,
        pipeline_version_id: PipelineVersionId,
        entry_stage_id: StageId,
    ) -> Self {
        Self {
            pipeline_id,
            pipeline_version_id,
            entry_stage_id,
        }
    }

    /// Returns the logical Pipeline identity.
    #[must_use]
    pub const fn pipeline_id(&self) -> PipelineId {
        self.pipeline_id
    }

    /// Returns the immutable version selected at Task creation.
    #[must_use]
    pub const fn pipeline_version_id(&self) -> PipelineVersionId {
        self.pipeline_version_id
    }

    /// Returns the stage that becomes current at approval.
    #[must_use]
    pub fn entry_stage_id(&self) -> &StageId {
        &self.entry_stage_id
    }

    pub(crate) fn validate_snapshot(&self) -> Result<(), crate::DomainError> {
        self.pipeline_id.validate_v7("task.pipeline.pipeline_id")?;
        self.pipeline_version_id
            .validate_v7("task.pipeline.pipeline_version_id")?;
        self.entry_stage_id.validate()
    }
}

/// Task-owned source binding. Provisioning is separate from canonical registration.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskWorkSurface {
    /// The Task has no provisioned work surface in M0.
    #[default]
    None,
    /// An explicit immutable Project repository and persistent private Task surface.
    Git(crate::TaskGitBinding),
}

/// Input used to create one draft Task.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewTask {
    /// Global Task identity, typically UUIDv7.
    pub id: TaskId,
    /// Project that owns the Task.
    pub project_id: ProjectId,
    /// Human-readable Project-local key.
    pub key: TaskKey,
    /// Work category.
    pub kind: TaskKind,
    /// Mutable draft intent.
    pub spec: TaskSpec,
    /// Project priority level selected for scheduling.
    pub priority_level_id: PriorityLevelId,
    /// Pipeline version selected for this Task.
    pub pipeline: TaskPipelineBinding,
    /// Origin of the Task's intent.
    pub source: TaskSource,
    /// Actor creating the Task.
    pub created_by: Actor,
    /// Authoritative creation timestamp.
    pub created_at: Timestamp,
}

/// Which scope produced a terminal completion submission.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionSource {
    /// Completion submitted from an active Pipeline stage.
    StageOutcome,
    /// Completion submitted from a human or external waiting stage.
    ExternalOutcome,
}

/// Accepted terminal-success data retained on a completed Task.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompletionData {
    source: CompletionSource,
    outcome_artifact_ids: BTreeSet<ArtifactId>,
    completed_by: Actor,
    completed_at: Timestamp,
}

impl CompletionData {
    /// Creates immutable terminal-success data.
    #[must_use]
    pub fn new(
        source: CompletionSource,
        outcome_artifact_ids: impl IntoIterator<Item = ArtifactId>,
        completed_by: Actor,
        completed_at: Timestamp,
    ) -> Self {
        Self {
            source,
            outcome_artifact_ids: outcome_artifact_ids.into_iter().collect(),
            completed_by,
            completed_at,
        }
    }

    /// Returns the source that supplied the terminal outcome.
    #[must_use]
    pub const fn source(&self) -> CompletionSource {
        self.source
    }

    /// Returns outcome Artifact IDs in deterministic order.
    pub fn outcome_artifact_ids(&self) -> impl Iterator<Item = ArtifactId> + '_ {
        self.outcome_artifact_ids.iter().copied()
    }

    /// Returns the terminal actor.
    #[must_use]
    pub const fn completed_by(&self) -> Actor {
        self.completed_by
    }

    /// Returns the terminal timestamp.
    #[must_use]
    pub const fn completed_at(&self) -> Timestamp {
        self.completed_at
    }

    pub(crate) fn validate_snapshot(&self) -> Result<(), crate::DomainError> {
        self.completed_by.validate_snapshot()?;
        for artifact_id in &self.outcome_artifact_ids {
            artifact_id.validate_v7("task.completion.outcome_artifact_id")?;
        }
        Ok(())
    }
}

/// Input for a cancellation request. `reason_id` is deliberately optional at
/// the command boundary so the domain can reject omissions explicitly.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CancellationRequest {
    /// Project catalog reason supplied by the caller.
    pub reason_id: Option<CancellationReasonId>,
    /// Optional contextual note.
    pub note: Option<String>,
    /// Actor requesting cancellation.
    pub cancelled_by: Actor,
    /// Authoritative cancellation time.
    pub cancelled_at: Timestamp,
}

/// Accepted terminal-cancellation data retained on a cancelled Task.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CancellationData {
    pub(super) reason_id: CancellationReasonId,
    pub(super) note: Option<String>,
    pub(super) cancelled_by: Actor,
    pub(super) cancelled_at: Timestamp,
}

impl CancellationData {
    /// Returns the required Project catalog reason.
    #[must_use]
    pub fn reason_id(&self) -> &CancellationReasonId {
        &self.reason_id
    }

    /// Returns optional contextual text.
    #[must_use]
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// Returns the actor that cancelled the Task.
    #[must_use]
    pub const fn cancelled_by(&self) -> Actor {
        self.cancelled_by
    }

    /// Returns the terminal cancellation timestamp.
    #[must_use]
    pub const fn cancelled_at(&self) -> Timestamp {
        self.cancelled_at
    }

    pub(crate) fn validate_snapshot(&self) -> Result<(), crate::DomainError> {
        self.reason_id.validate()?;
        self.cancelled_by.validate_snapshot()?;
        if self
            .note
            .as_deref()
            .is_some_and(|note| note.trim().is_empty())
        {
            return Err(crate::DomainError::InvalidValue {
                field: "task.cancellation_note",
                reason: "must not be blank when present".to_owned(),
            });
        }
        Ok(())
    }
}

/// Terminal data, present if and only if the Task is done or cancelled.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalData {
    /// Successful terminal data.
    Completed(CompletionData),
    /// Cancelled terminal data.
    Cancelled(CancellationData),
}

impl TerminalData {
    pub(crate) fn validate_snapshot(&self) -> Result<(), crate::DomainError> {
        match self {
            Self::Completed(data) => data.validate_snapshot(),
            Self::Cancelled(data) => data.validate_snapshot(),
        }
    }
}
