//! Versioned wire contracts shared by Forge Core, Supervisor, and CLI.
//!
//! This crate deliberately does not depend on Forge domain types. It defines
//! transport envelopes only; Core validates wire values before they become
//! canonical state.

#![forbid(unsafe_code)]

/// Canonical Linux-local Unix-socket directory convention.
pub mod local_paths;

/// HTTP/JSON and SSE wire envelopes.
pub mod wire;

/// Generated Protobuf contract for the authenticated local Supervisor stream.
pub mod supervisor {
    /// Version one of the Supervisor control contract.
    pub mod v1 {
        tonic::include_proto!("forge.supervisor.v1");
    }
}

/// Binary Protobuf descriptor set for reflection-free contract tests.
pub const SUPERVISOR_V1_FILE_DESCRIPTOR_SET: &[u8] =
    tonic::include_file_descriptor_set!("forge_supervisor_v1_descriptor");
