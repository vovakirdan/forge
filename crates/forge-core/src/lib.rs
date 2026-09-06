//! Canonical command, scheduling, and local transport composition for Forge.
//!
//! The Core is the only writer of Forge state. HTTP, the local Supervisor
//! stream, and background dispatch all enter through typed methods here; none
//! of them writes a canonical table directly.

#![forbid(unsafe_code)]

mod auth_writeback;
mod canonical_clock;
mod command;
mod config;
mod credentials;
mod dependency_waits;
mod dispatch;
pub(crate) mod egress;
mod error;
mod event;
mod event_projection;
mod evidence_collection;
mod external_outcome;
mod gateway;
mod handoff;
mod http;
mod inference;
pub mod observability;
mod outbox;
mod pipeline_access;
mod preparation_failure;
mod proxy_credentials;
mod recovery;
mod run_control;
mod runtime_preparation;
mod runtime_report;
mod scheduler;
mod secret_cleanup;
mod secret_guard;
mod supervisor;
mod task_commands;
mod task_control;
mod task_support;
mod watchdog;

pub use config::{CoreConfig, RuntimePaths};
pub use error::CoreError;
pub use event_projection::event_envelope_from_stored_event;
pub use gateway::GatewayHandle;
pub use http::router;
pub use outbox::{OutboxDrainReport, OutboxError, OutboxPublisher, OutboxPublisherConfig};
pub use supervisor::{SupervisorHub, SupervisorService};
pub use watchdog::{WatchdogDeadlines, WatchdogReport};

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
    pub(crate) fake_runtime_enabled: bool,
    pub(crate) secret_store: Option<Arc<forge_provider_common::SecretStore>>,
    pub(crate) execution: Option<Arc<runtime_preparation::ExecutionRuntime>>,
    pub(crate) inference: Option<Arc<proxy_credentials::InferenceProxy>>,
    pub(crate) evidence: Option<Arc<evidence_collection::EvidenceRuntime>>,
    pub(crate) observability: Arc<observability::Observability>,
}

impl CoreService {
    /// Creates a Core service over an already schema-validated Store.
    #[must_use]
    pub fn new(store: PostgresStore, actors: CoreActors, supervisor: Arc<SupervisorHub>) -> Self {
        Self {
            store,
            actors,
            supervisor,
            fake_runtime_enabled: false,
            secret_store: None,
            execution: None,
            inference: None,
            evidence: None,
            observability: Arc::new(observability::Observability::default()),
        }
    }

    /// Explicit simulator opt-in for M0 tests/demos. Production never falls back to it.
    #[must_use]
    pub fn with_fake_runtime(mut self) -> Self {
        self.fake_runtime_enabled = true;
        self
    }

    /// Installs an already initialized key provider; no implicit key generation.
    #[must_use]
    pub fn with_secret_store(mut self, store: forge_provider_common::SecretStore) -> Self {
        self.secret_store = Some(Arc::new(store));
        self
    }

    /// Configures the shared local Supervisor execution root, never a Task path.
    pub fn with_execution_root(mut self, root: std::path::PathBuf) -> Result<Self, CoreError> {
        self.execution = Some(Arc::new(runtime_preparation::ExecutionRuntime::new(
            root.clone(),
        )?));
        self.evidence = Some(Arc::new(evidence_collection::EvidenceRuntime::new(
            &root.join("spool"),
            None,
        )?));
        Ok(self)
    }

    /// Explicit local LiteLLM administration endpoint and private master-key file.
    pub fn with_inference_proxy(
        mut self,
        endpoint: &str,
        master_key: &std::path::Path,
    ) -> Result<Self, CoreError> {
        self.inference = Some(Arc::new(proxy_credentials::InferenceProxy::new(
            endpoint, master_key,
        )?));
        Ok(self)
    }

    /// Returns the storage boundary only to Core-owned local transports.
    #[must_use]
    pub(crate) fn store(&self) -> &PostgresStore {
        &self.store
    }
}
