//! Shared secret handling for Core and provider adapters.
//!
//! This crate does not own canonical persistence: Core stores sealed records and
//! commits credential updates using a compare-and-swap transaction.

#![forbid(unsafe_code)]

pub mod adapter;
mod codex_auth;
mod master_key;
mod private_file;
mod secret;

pub use codex_auth::{
    AuthSnapshotLease, AuthWriteback, AuthWritebackConflict, ManagedAuthSnapshot,
    enroll_codex_auth, evaluate_auth_writeback,
};
pub use master_key::{MasterKeyBinding, MasterKeySource};
pub use private_file::PrivateMaterialization;
pub use secret::{SealedSecret, SecretBytes, SecretScope, SecretStore, SecretStoreError};
