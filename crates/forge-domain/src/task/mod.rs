//! Task lifecycle, typed Project metadata, priority, and terminal data.

mod dependency;
mod lifecycle;
mod model;
mod pipeline;
mod priority;
mod properties;

pub use dependency::{DependencyCondition, TaskDependency, TaskDependencyGraph};
pub use lifecycle::{
    LifecycleStatus, ResumeLifecycleStatus, TaskWaitCondition, TaskWaitKind, TaskWaitKindId,
};
pub use model::{
    CancellationData, CancellationRequest, CompletionData, CompletionSource, NewTask, Task,
    TaskKind, TaskPipelineBinding, TaskScope, TaskSource, TaskSpec, TaskSpecInput, TaskWorkSurface,
    TerminalData,
};
pub use pipeline::{
    ArtifactRequirement, ArtifactRequirementScope, ExecutorKind, OutcomeKey, Pipeline,
    PipelineStage, PipelineTransition, PipelineTransitionTarget, PipelineVersion,
    PipelineVersionInput, StageOutcomeSubmission, StageTransitionEffect, StageWorkspaceKind,
    StageWorkspaceRequirements,
};
pub use priority::{
    CancellationReason, CancellationReasonCatalog, CancellationReasonId, PriorityLevel,
    PriorityLevelId, PriorityScheme,
};
pub use properties::{
    ForgeReference, PropertyChoice, PropertyKey, PropertyType, PropertyValue, TaskProperties,
    TaskPropertyDefinition, TaskPropertySchema,
};
