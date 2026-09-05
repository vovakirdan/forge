//! Local path and process configuration for the Linux-first M0 daemon.

use std::path::PathBuf;

use forge_protocol::local_paths::default_runtime_directory;

/// Paths owned by the current user's Core process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimePaths {
    /// Local HTTP/JSON and SSE socket used by CLI and future UI clients.
    pub api_socket: PathBuf,
    /// Authenticated gRPC socket used only by the local Supervisor.
    pub supervisor_socket: PathBuf,
}

impl RuntimePaths {
    /// Resolves owner-only sockets from XDG runtime/state conventions.
    #[must_use]
    pub fn from_environment() -> Self {
        let directory = default_runtime_directory();
        Self {
            api_socket: directory.join("api.sock"),
            supervisor_socket: directory.join("core.sock"),
        }
    }
}

/// Process configuration that intentionally excludes secret material.
#[derive(Clone)]
pub struct CoreConfig {
    /// PostgreSQL connection URL supplied by the local operator environment.
    pub database_url: String,
    /// Optional NATS endpoint for the outbox publisher.
    pub nats_url: Option<String>,
    /// Local owner-only transport locations.
    pub runtime_paths: RuntimePaths,
}

impl std::fmt::Debug for CoreConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CoreConfig")
            .field("database_url", &"<redacted>")
            .field("nats_url", &self.nats_url.as_ref().map(|_| "<redacted>"))
            .field("runtime_paths", &self.runtime_paths)
            .finish()
    }
}

impl CoreConfig {
    /// Constructs Core configuration from already validated process arguments.
    #[must_use]
    pub fn new(
        database_url: String,
        nats_url: Option<String>,
        runtime_paths: RuntimePaths,
    ) -> Self {
        Self {
            database_url,
            nats_url,
            runtime_paths,
        }
    }
}
