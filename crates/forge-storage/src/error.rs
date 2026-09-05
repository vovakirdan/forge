//! Typed failures at Forge's PostgreSQL persistence boundary.

use thiserror::Error;

use crate::MigrationError;

/// A failure that callers can classify without parsing a database error string.
#[derive(Debug, Error)]
pub enum StorageError {
    /// PostgreSQL rejected or could not execute an operation.
    #[error("Forge PostgreSQL operation failed")]
    Database(#[from] sqlx::Error),

    /// The database is reachable but does not meet Forge's explicit schema contract.
    #[error(transparent)]
    Migration(#[from] MigrationError),

    /// A durable JSON snapshot could not be encoded or decoded.
    #[error("invalid {aggregate} canonical snapshot: {source}")]
    Snapshot {
        /// Aggregate whose snapshot was invalid.
        aggregate: &'static str,
        /// Serialization failure details; values themselves are intentionally omitted.
        #[source]
        source: serde_json::Error,
    },

    /// A JSON snapshot decoded successfully but violates aggregate invariants.
    #[error("invalid {aggregate} canonical snapshot invariant: {source}")]
    SnapshotInvariant {
        /// Aggregate whose persisted state was structurally invalid.
        aggregate: &'static str,
        /// The domain invariant that rejected the decoded state.
        #[source]
        source: forge_domain::DomainError,
    },

    /// A JSON column required an object or array but received another JSON shape.
    #[error("invalid storage JSON for {field}: expected {expected}")]
    InvalidJsonShape {
        /// Safe logical field name.
        field: &'static str,
        /// Required JSON shape.
        expected: &'static str,
    },

    /// A domain integer cannot fit into PostgreSQL's signed integer representation.
    #[error("{field} is outside PostgreSQL signed integer range")]
    IntegerOutOfRange {
        /// Safe logical field name.
        field: &'static str,
    },

    /// A required durable aggregate does not exist in its Project scope.
    #[error("{aggregate} was not found")]
    NotFound {
        /// Aggregate category safe to expose to a local operator.
        aggregate: &'static str,
    },

    /// A write was rejected because the projection revision changed first.
    #[error("stale {aggregate} revision")]
    StaleRevision {
        /// Aggregate category safe to expose to a local operator.
        aggregate: &'static str,
    },

    /// An ExecutorSubmission Artifact receipt was already mapped to immutable evidence.
    #[error("executor artifact receipt is already mapped")]
    ExecutorArtifactReceiptAlreadyMapped,

    /// An input violates a storage-only structural invariant.
    #[error("invalid storage input: {reason}")]
    InvalidInput {
        /// Safe structural explanation.
        reason: String,
    },
}
