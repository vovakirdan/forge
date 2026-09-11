//! Shared secret handling for Core and provider adapters.
//!
//! This crate does not own canonical persistence: Core stores sealed records and
//! commits credential updates using a compare-and-swap transaction.

#![forbid(unsafe_code)]

pub mod adapter;
mod codex_auth;
pub mod driver_event;
mod master_key;
pub mod native_event;
pub mod native_input;
mod private_file;
mod secret;
pub mod selected_file;

pub use codex_auth::{
    AuthSnapshotLease, AuthWriteback, AuthWritebackConflict, ManagedAuthSnapshot,
    enroll_codex_auth, evaluate_auth_writeback,
};
pub use master_key::{MasterKeyBinding, MasterKeySource};
pub use private_file::PrivateMaterialization;
pub use secret::{SealedSecret, SecretBytes, SecretScope, SecretStore, SecretStoreError};
