//! Core owns capture reservations, Artifact publication and future input bindings.

mod commands;
mod materialize;
mod processing;

use crate::CoreError;

fn invalid(reason: &str) -> CoreError {
    CoreError::InvalidTransport {
        field: "file_snapshot",
        reason: reason.into(),
    }
}
