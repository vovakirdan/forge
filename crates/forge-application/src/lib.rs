//! Typed named-command boundary for the Forge Core.
//!
//! This crate parses untrusted HTTP command bodies into validated application
//! intent. It deliberately owns neither SQL transactions nor HTTP handlers.

#![forbid(unsafe_code)]

mod artifact_submission;
mod command;
mod communication;
pub mod engine;
mod error;
mod finding;
pub use finding::{
    FindingTriageDecision, PromoteFindingCommand, ReportFindingCommand, TriageFindingCommand,
};
mod hook_input;
mod payload;
mod pipeline_input;
pub use hook_input::ConfigureProjectHookInput;

pub use artifact_submission::{ArtifactInput, ExternalStageOutcomeCommand};
pub use command::{CommandEnvelope, CommandPayload, IdempotencyKey, RaiseEscalationInput};
pub use communication::{
    OpenEmployeeThreadCommand, SendEmployeeMessageCommand, WaiveMessageRequirementCommand,
};
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
pub use pipeline_input::{
    CreatePipelineCommand, PipelineDefinitionInput, PipelineStageInput, PipelineTransitionInput,
};
