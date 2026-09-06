//! Safe classifications at the command repository boundary.
use thiserror::Error;

/// Adapter failure classifications, without backend messages, queries, or credentials.
#[derive(Debug, Error)]
pub enum RepositoryError {
    /// An expected aggregate is absent.
    #[error("{aggregate} was not found")]
    NotFound { aggregate: &'static str },
    /// A persisted revision did not match the transaction's expected revision.
    #[error("stale {aggregate} revision")]
    StaleRevision { aggregate: &'static str },
    /// A backend failure safe to expose without database details.
    #[error("canonical storage is temporarily unavailable")]
    Unavailable,
    /// A bounded, non-secret adapter invariant explanation.
    #[error("invalid storage input: {reason}")]
    InvalidInput { reason: String },
}

/// Canonical command refusals; composition adapters retain their existing HTTP mapping.
#[derive(Debug, Error)]
pub enum CommandError {
    /// The parsed command violates its input contract.
    #[error("invalid command: {0}")]
    Application(#[from] crate::ApplicationError),
    /// The requested transition violates a domain invariant.
    #[error("domain transition rejected: {0}")]
    Domain(#[from] forge_domain::DomainError),
    /// The borrowed transaction could not read or stage an operation.
    #[error("canonical storage rejected the operation: {0}")]
    Repository(#[from] RepositoryError),
    /// A command-scoped target was not found.
    #[error("{aggregate} was not found")]
    NotFound { aggregate: &'static str },
    /// The same Project/key identity was already used for a different canonical hash.
    #[error("idempotency key was reused for a different command")]
    IdempotencyConflict,
    /// A stored or internal transport value cannot be represented by the wire contract.
    #[error("invalid transport value for {field}: {reason}")]
    InvalidTransport { field: &'static str, reason: String },
    /// Trusted scope, capability, or prepared invocation provenance did not match.
    #[error("actor is not authorized for this project command")]
    Forbidden,
    /// One of the four M1 extensions requires Core's production runtime composition.
    #[error("command requires the production runtime extension")]
    UnsupportedCommand,
}
