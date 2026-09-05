//! Immutable Pipeline graph definitions and deterministic transition rules.

mod catalog;
mod definition;
mod engine;
mod validation;
mod version;

#[cfg(test)]
mod tests;

pub use catalog::Pipeline;
pub use definition::{
    ArtifactRequirement, ArtifactRequirementScope, ExecutorKind, OutcomeKey, PipelineStage,
    PipelineTransition, PipelineTransitionTarget,
};
pub use engine::{StageOutcomeSubmission, StageTransitionEffect};
pub use version::{PipelineVersion, PipelineVersionInput};
