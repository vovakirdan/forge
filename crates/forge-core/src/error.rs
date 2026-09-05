//! Safe errors exposed by Core transports and orchestration.

use forge_application::ApplicationError;
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
