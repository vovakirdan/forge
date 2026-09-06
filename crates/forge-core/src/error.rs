//! Safe errors exposed by Core transports and orchestration.

use forge_application::{ApplicationError, CommandError, RepositoryError};
use forge_domain::DomainError;
use forge_storage::StorageError;
use thiserror::Error;

/// Failures from Core's typed command and execution boundary.
#[derive(Debug, Error)]
pub enum CoreError {
    /// The public command envelope could not become typed application intent.
    #[error("invalid command: {0}")]
    Application(#[from] ApplicationError),

    /// A pure domain invariant rejected the proposed state transition.
    #[error("domain transition rejected: {0}")]
    Domain(#[from] DomainError),

    /// Durable canonical storage was unavailable or rejected a write.
    #[error("canonical storage rejected the operation: {0}")]
    Storage(#[from] StorageError),

    /// The shared command adapter rejected a canonical repository operation.
    #[error("canonical storage rejected the operation: {0}")]
    Repository(#[from] RepositoryError),

    /// A command actor or capability does not authorize the requested Project.
    #[error("actor is not authorized for this project command")]
    Forbidden,

    /// A referenced aggregate was absent in the command Project scope.
    #[error("{aggregate} was not found")]
    NotFound {
        /// Safe aggregate category.
        aggregate: &'static str,
    },

    /// An idempotency key was reused with a different semantic request.
    #[error("idempotency key was reused for a different command")]
    IdempotencyConflict,

    /// The local Supervisor control channel is not currently connected.
    #[error("local Supervisor is unavailable")]
    SupervisorUnavailable,

    /// A malformed or unsafe external transport value was rejected.
    #[error("invalid transport value for {field}: {reason}")]
    InvalidTransport {
        /// Safe field label.
        field: &'static str,
        /// Safe structural explanation.
        reason: String,
    },
}

impl From<CommandError> for CoreError {
    fn from(error: CommandError) -> Self {
        match error {
            CommandError::Application(error) => Self::Application(error),
            CommandError::Domain(error) => Self::Domain(error),
            CommandError::Repository(error) => Self::Repository(error),
            CommandError::NotFound { aggregate } => Self::NotFound { aggregate },
            CommandError::IdempotencyConflict => Self::IdempotencyConflict,
            CommandError::InvalidTransport { field, reason } => {
                Self::InvalidTransport { field, reason }
            }
            CommandError::Forbidden => Self::Forbidden,
            CommandError::UnsupportedCommand => Self::InvalidTransport {
                field: "command",
                reason: "requires the production runtime extension".to_owned(),
            },
        }
    }
}
