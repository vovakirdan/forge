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

impl crate::CoreService {
    /// Explicit dedicated loopback AgentMemory service; no implicit personal daemon.
    pub fn with_agentmemory(
        mut self,
        endpoint: &str,
        credential: forge_provider_common::SecretBytes,
        timeout: std::time::Duration,
    ) -> Result<Self, crate::CoreError> {
        let client = crate::memory_projection::agentmemory::AgentMemoryClient::new(
            endpoint, credential, timeout,
        )
        .map_err(|_| crate::CoreError::InvalidTransport {
            field: "agentmemory",
            reason: "invalid dedicated AgentMemory configuration".into(),
        })?;
        self.agentmemory = Some(std::sync::Arc::new(client));
        Ok(self)
    }

    /// Owner-only key file, bounded before parsing and never included in diagnostics.
    pub fn with_agentmemory_secret_file(
        self,
        endpoint: &str,
        file: &std::path::Path,
        timeout: std::time::Duration,
    ) -> Result<Self, crate::CoreError> {
        let credential = forge_provider_common::PrivateMaterialization::open(file)
            .and_then(|file| file.read(4096))
            .map_err(|_| crate::CoreError::InvalidTransport {
                field: "agentmemory",
                reason: "cannot read owner-only AgentMemory credential".into(),
            })?;
        let bytes = credential.expose();
        let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
        let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
        self.with_agentmemory(
            endpoint,
            forge_provider_common::SecretBytes::new(bytes.to_vec()),
            timeout,
        )
    }

    /// Explicit local-operator startup configuration. Changes while any Run has
    /// logical or uncertain physical ownership fail closed; same values are safe.
    pub async fn with_admission_limits(
        self,
        limits: forge_domain::admission::AdmissionLimits,
    ) -> Result<Self, crate::CoreError> {
        self.store.configure_local_admission(limits).await?;
        Ok(self)
    }
}
