//! Pure, provider-neutral domain rules for Forge.
//!
//! This crate owns value types and invariants that must hold independently of
//! persistence, transport, scheduling, or a particular execution runtime.

#![forbid(unsafe_code)]

pub mod admission;
mod artifact;
mod assignment;
pub mod candidate_review;
pub mod communication;
mod context;
mod employee;
mod error;
mod event;
mod evidence;
pub mod execution_profile;
pub mod finding;
pub mod git;
pub mod git_delivery;
pub mod git_integration;
mod handoff;
mod ids;
mod management;
mod project;
pub mod project_hook;
mod repository;
pub mod resolution;
pub mod runtime;
mod task;

#[cfg(test)]
mod candidate_review_tests;
#[cfg(test)]
mod context_tests;

pub use artifact::{
    Artifact, ArtifactBody, ArtifactKind, ArtifactLink, ArtifactProducer,
    MAX_INLINE_JSON_BODY_BYTES, NewArtifact,
};
pub use assignment::{
    CommunicationAssignmentRef, ExecutionAssignment, HookAssignmentRef, ResolutionAssignmentRef,
    TaskStageAssignment,
};
pub use context::{ContextSnapshot, ContextSnapshotInput};
pub use employee::{
    Employee, EmployeeAmendment, EmployeeRole, EmployeeState, StageEligibility,
    StageEligibilityTarget,
};
pub use error::DomainError;
pub use event::{
    Actor, ActorKind, AggregateRef, AggregateType, DomainEvent, DomainEventInput, DomainEventKind,
};
pub use evidence::{
    EvidenceLocation, EvidenceObject, EvidenceObjectInput, EvidenceScope, EvidenceStream,
};
pub use execution_profile::{
    CapabilityProfile, CredentialBinding, CredentialDeliveryMode, ExecutionProfile,
    ExecutionProfileError, ExecutionProfileInput, RuntimeCapability, TransportEngine,
};
pub use handoff::{
    HandoffArtifactAcceptance, HandoffArtifactReference, HandoffOutcome, HandoffProducer,
    TaskHandoff, TaskHandoffInput,
};
pub use ids::{
    ActorId, ArtifactId, CommandId, EmployeeId, EventId, PipelineId, PipelineVersionId, ProjectId,
    ProjectLocalSequence, StageId, StageVisit, TaskId, TaskKey, TaskKeyParseError, TaskRevision,
    Timestamp, WaitConditionId,
};
pub use management::{
    ConstraintBlockReason, ConstraintEndReason, ExecutionStopMode, NextRunConstraintState,
    NextRunEmployeeConstraint, ResumeRejection, ScheduledResumeState, TaskResumeSchedule,
};
pub use project::{Project, ProjectExecutionGate};
pub use project_hook::{HookExecutionResult, HookOutcomes, HookVerdict, ProjectHookVersion};
pub use repository::{ProjectRepository, TaskGitBinding};
pub use task::{
    ArtifactRequirement, ArtifactRequirementScope, CancellationData, CancellationReason,
    CancellationReasonCatalog, CancellationReasonId, CancellationRequest, CompletionData,
    CompletionSource, DependencyCondition, ExecutorKind, ForgeReference, LifecycleStatus, NewTask,
    OutcomeKey, Pipeline, PipelineStage, PipelineTransition, PipelineTransitionTarget,
    PipelineVersion, PipelineVersionInput, PriorityLevel, PriorityLevelId, PriorityScheme,
    PropertyChoice, PropertyKey, PropertyType, PropertyValue, ResumeLifecycleStatus,
    StageOutcomeSubmission, StageTransitionEffect, StageWorkspaceKind, StageWorkspaceRequirements,
    Task, TaskDependency, TaskDependencyGraph, TaskKind, TaskPipelineBinding, TaskProperties,
    TaskPropertyDefinition, TaskPropertySchema, TaskScope, TaskSource, TaskSpec, TaskSpecInput,
    TaskWaitCondition, TaskWaitKind, TaskWaitKindId, TaskWorkSurface, TerminalData,
};
