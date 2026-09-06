//! Typed named-command boundary for the Forge Core.
//!
//! This crate parses untrusted HTTP command bodies into validated application
//! intent. It deliberately owns neither SQL transactions nor HTTP handlers.

#![forbid(unsafe_code)]

mod artifact_submission;
mod command;
pub mod engine;
mod error;
mod payload;
mod pipeline_input;

pub use artifact_submission::{ArtifactInput, ExternalStageOutcomeCommand};
pub use command::{CommandEnvelope, CommandPayload, IdempotencyKey};
pub use engine::ports;
pub use engine::{
    Clock, CommandContext, CommandError, CommandTransaction, PreparedCommand, PreparedCommandState,
    RepositoryError, SystemClock, command_fingerprint, execute_in_transaction,
    execute_prepared_in_transaction, finish_command, prepare_command,
};
pub use error::ApplicationError;
pub use payload::{
    CreateEmployeeCommand, CreateProjectCommand, CreateTaskCommand, DraftTaskPatch,
    TaskDraftContext,
};
pub use pipeline_input::{CreatePipelineCommand, PipelineStageInput, PipelineTransitionInput};
