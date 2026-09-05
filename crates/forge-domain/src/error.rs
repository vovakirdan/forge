use thiserror::Error;

/// A deterministic refusal of a Forge domain operation.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum DomainError {
    /// A string value cannot serve as a stable domain identifier.
    #[error("invalid {field}: {reason}")]
    InvalidIdentifier {
        /// Name of the rejected field.
        field: &'static str,
        /// Reason that the value was rejected.
        reason: String,
    },

    /// A value violates a local domain invariant.
    #[error("invalid {field}: {reason}")]
    InvalidValue {
        /// Name of the rejected field.
        field: &'static str,
        /// Reason that the value was rejected.
        reason: String,
    },

    /// A lifecycle transition is not defined by the Task contract.
    #[error("invalid lifecycle transition from {from} to {to}")]
    InvalidLifecycleTransition {
        /// Source lifecycle status.
        from: String,
        /// Requested lifecycle status.
        to: String,
    },

    /// A terminal Task cannot be mutated.
    #[error("task is terminal ({status})")]
    TerminalTask {
        /// Terminal lifecycle status.
        status: String,
    },

    /// A required cancellation reason was not supplied.
    #[error("cancellation requires a reason")]
    CancellationReasonRequired,

    /// A cancellation reason is not active in the Project catalog.
    #[error("cancellation reason is not active: {reason_id}")]
    UnknownCancellationReason {
        /// Stable Project-defined reason identifier.
        reason_id: String,
    },

    /// A supplied Artifact does not belong to the Task's Project.
    #[error("artifact belongs to a different project")]
    CrossProjectArtifact,

    /// An Artifact was attached to a Task more than once.
    #[error("artifact is already attached to this task")]
    DuplicateArtifactLink,

    /// A terminal outcome names an Artifact that is not attached to the Task.
    #[error("outcome references an artifact that is not attached to the task")]
    UnknownOutcomeArtifact,

    /// An analysis Task lacks its mandatory analysis-result Artifact.
    #[error("analysis task completion requires an analysis_result artifact")]
    MissingAnalysisResult,

    /// A Task property is absent, unknown, or does not match its schema.
    #[error("invalid task property {key}: {reason}")]
    InvalidProperty {
        /// Stable property key.
        key: String,
        /// Reason that the property was rejected.
        reason: String,
    },

    /// A conditional wait cannot be resolved with the requested operation.
    #[error("invalid task wait operation: {reason}")]
    InvalidWait {
        /// Reason that the operation was rejected.
        reason: String,
    },

    /// A Pipeline version does not describe a valid executable graph.
    #[error("invalid pipeline: {reason}")]
    InvalidPipeline {
        /// Human-readable explanation of the violated graph invariant.
        reason: String,
    },

    /// A stage outcome is not available from the Task's pinned Pipeline stage.
    #[error("invalid stage outcome: {reason}")]
    InvalidStageOutcome {
        /// Human-readable explanation of the rejected stage transition.
        reason: String,
    },

    /// A bounded Pipeline retry/re-entry policy has no remaining stage visits.
    #[error("pipeline stage-visit limit reached: maximum {maximum}")]
    StageVisitLimitExceeded {
        /// Immutable Pipeline-wide cap on stage entries for one Task.
        maximum: u32,
    },

    /// A command expected a different mutable Project revision.
    #[error("stale project revision: expected {expected}, current {actual}")]
    StaleProjectRevision {
        /// Revision supplied by the command.
        expected: u64,
        /// Revision held by the Project.
        actual: u64,
    },

    /// A proposed Task dependency would create a directed cycle.
    #[error("task dependency cycle detected")]
    DependencyCycle,

    /// A revision cannot be incremented further.
    #[error("task revision overflow")]
    RevisionOverflow,

    /// A mutation timestamp predates the Task's current revision.
    #[error("mutation timestamp cannot precede the current task timestamp")]
    NonMonotonicTimestamp,
}
