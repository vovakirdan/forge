//! Canonical command, scheduling, and local transport composition for Forge.
//!
//! The Core is the only writer of Forge state. HTTP, the local Supervisor
//! stream, and background dispatch all enter through typed methods here; none
//! of them writes a canonical table directly.

#![forbid(unsafe_code)]

mod command;
mod config;
mod dependency_waits;
mod dispatch;
mod error;
mod event;
mod event_projection;
mod external_outcome;
mod http;
mod outbox;
mod pipeline_access;
mod recovery;
mod run_control;
mod scheduler;
mod supervisor;
mod task_commands;
mod task_control;
mod task_support;

pub use config::{CoreConfig, RuntimePaths};
pub use error::CoreError;
pub use event_projection::event_envelope_from_stored_event;
pub use http::router;
pub use outbox::{OutboxDrainReport, OutboxError, OutboxPublisher, OutboxPublisherConfig};
pub use supervisor::{SupervisorHub, SupervisorService};

use std::sync::Arc;

use forge_domain::{Actor, ActorId};
use forge_storage::PostgresStore;

/// Core-owned identities used for auditable local M0 actions.
#[derive(Clone, Copy, Debug)]
pub struct CoreActors {
    /// Local human/CLI actor used before M0 gains user authentication.
    pub human: Actor,
    /// Deterministic Core actor used by scheduler-owned work.
    pub core: Actor,
}

impl CoreActors {
    /// Builds stable process-local actor identities from explicit UUIDv7 values.
    #[must_use]
    pub const fn new(human: ActorId, core: ActorId) -> Self {
        Self {
            human: Actor::human(human),
            core: Actor::core(core),
        }
    }
}

/// The shared stateful application service behind every Core transport.
#[derive(Clone)]
pub struct CoreService {
    pub(crate) store: PostgresStore,
    pub(crate) actors: CoreActors,
    pub(crate) supervisor: Arc<SupervisorHub>,
}

impl CoreService {
    /// Creates a Core service over an already schema-validated Store.
    #[must_use]
    pub fn new(store: PostgresStore, actors: CoreActors, supervisor: Arc<SupervisorHub>) -> Self {
        Self {
            store,
            actors,
            supervisor,
        }
    }

    /// Returns the storage boundary only to Core-owned local transports.
    #[must_use]
    pub(crate) fn store(&self) -> &PostgresStore {
        &self.store
    }
}
