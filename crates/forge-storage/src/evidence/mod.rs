//! Bounded local evidence collection and an S3-compatible upload adapter.
//! Canonical metadata publication and Task decisions remain Core-owned.

mod redaction;
mod spool;
mod stream;
mod upload;

pub use redaction::EvidenceRedaction;
pub use spool::{EvidenceSpool, PendingEvidence, SpoolLimits};
pub use stream::{EvidenceCollector, SpoolAppend, SpoolIncident};
pub use upload::{ObjectEvidenceStore, S3EvidenceConfig};

/// Stable adapter failures: provider URLs, credentials and raw output are never
/// incorporated into error text or debug output.
#[derive(Debug, thiserror::Error)]
pub enum EvidenceError {
    #[error("evidence configuration is invalid")]
    InvalidConfiguration,
    #[error("evidence scope or immutable metadata is invalid")]
    InvalidMetadata,
    #[error("evidence spool must be an owner-only directory without symlinks")]
    UnsafeSpool,
    #[error("evidence spool is already in use")]
    SpoolBusy,
    #[error("evidence spool I/O failed")]
    SpoolIo,
    #[error("evidence spool capacity was exhausted")]
    SpoolCapacity,
    #[error("evidence content does not match its immutable receipt")]
    IntegrityMismatch,
    #[error("evidence object storage is unavailable")]
    ObjectStoreUnavailable,
}

fn digest(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests;
