//! Pure, provider-neutral domain rules for Forge.
//!
//! This crate owns value types and invariants that must hold independently of
//! persistence, transport, scheduling, or a particular execution runtime.

#![forbid(unsafe_code)]

mod artifact;
mod employee;
mod error;
mod event;
mod ids;
mod project;
mod task;

pub use artifact::{
    Artifact, ArtifactBody, ArtifactKind, ArtifactLink, ArtifactProducer,
    MAX_INLINE_JSON_BODY_BYTES, NewArtifact,
};
pub use employee::{
    Employee, EmployeeRole, EmployeeState, StageEligibility, StageEligibilityTarget,
};
pub use error::DomainError;
pub use event::{
    Actor, ActorKind, AggregateRef, AggregateType, DomainEvent, DomainEventInput, DomainEventKind,
};
pub use ids::{
    ActorId, ArtifactId, CommandId, EmployeeId, EventId, PipelineId, PipelineVersionId, ProjectId,
    ProjectLocalSequence, StageId, StageVisit, TaskId, TaskKey, TaskKeyParseError, TaskRevision,
    Timestamp, WaitConditionId,
};
pub use project::{Project, ProjectExecutionGate};
pub use task::{
    ArtifactRequirement, ArtifactRequirementScope, CancellationData, CancellationReason,
    CancellationReasonCatalog, CancellationReasonId, CancellationRequest, CompletionData,
    CompletionSource, DependencyCondition, ExecutorKind, ForgeReference, LifecycleStatus, NewTask,
    OutcomeKey, Pipeline, PipelineStage, PipelineTransition, PipelineTransitionTarget,
    PipelineVersion, PipelineVersionInput, PriorityLevel, PriorityLevelId, PriorityScheme,
    PropertyChoice, PropertyKey, PropertyType, PropertyValue, ResumeLifecycleStatus,
    StageOutcomeSubmission, StageTransitionEffect, Task, TaskDependency, TaskDependencyGraph,
    TaskKind, TaskPipelineBinding, TaskProperties, TaskPropertyDefinition, TaskPropertySchema,
    TaskScope, TaskSource, TaskSpec, TaskSpecInput, TaskWaitCondition, TaskWaitKind,
    TaskWaitKindId, TaskWorkSurface, TerminalData,
};
