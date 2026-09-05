//! Typed named-command boundary for the Forge Core.
//!
//! This crate parses untrusted HTTP command bodies into validated application
//! intent. It deliberately owns neither SQL transactions nor HTTP handlers.

#![forbid(unsafe_code)]

mod artifact_submission;
mod command;
mod error;
mod payload;
mod pipeline_input;

pub use artifact_submission::{ArtifactInput, ExternalStageOutcomeCommand};
pub use command::{CommandEnvelope, CommandPayload, IdempotencyKey};
pub use error::ApplicationError;
pub use payload::{
    CreateEmployeeCommand, CreateProjectCommand, CreateTaskCommand, DraftTaskPatch,
    TaskDraftContext,
};
pub use pipeline_input::{CreatePipelineCommand, PipelineStageInput, PipelineTransitionInput};
