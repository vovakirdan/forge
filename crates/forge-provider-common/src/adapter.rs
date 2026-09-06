//! Provider-neutral preparations and observations. No process spawning or
//! canonical Task transitions occur at this boundary.

use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::SecretBytes;

/// Nonsecret runtime configuration. Relative paths are below the private runtime
/// directory, never TaskWorkSurface. The Supervisor enforces this boundary.
#[derive(Clone)]
pub struct ManagedRuntimeFile {
    pub relative_path: String,
    pub contents: String,
}

impl fmt::Debug for ManagedRuntimeFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManagedRuntimeFile")
            .field("relative_path", &self.relative_path)
            .field("contents", &"[REDACTED]")
            .finish()
    }
}

/// Allowlisted copy from a scoped secret mount into a private writable home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialFileCopy {
    pub source: String,
    pub target: String,
    pub writeback: bool,
}

/// Instructions to a container-local wrapper, not a host shell command.
/// It is intentionally not serializable: stdin may contain sensitive context.
#[derive(Debug)]
pub struct PreparedInvocation {
    pub program: String,
    pub args: Vec<String>,
    /// Must be installed with env_clear: never merge with the host environment.
    pub env: BTreeMap<String, String>,
    pub stdin: SecretBytes,
    pub managed_files: Vec<ManagedRuntimeFile>,
    pub credential_files: Vec<CredentialFileCopy>,
    pub stop: StopStrategy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopStrategy {
    /// Send SIGINT to the sandbox process group; force-stop the environment only
    /// when its configured grace deadline expires. Exit still requires evidence.
    SigintThenEnvironmentKill,
    RuntimeApiAbortThenEnvironmentKill,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AdapterError {
    #[error("runtime version does not match the pinned adapter version")]
    VersionMismatch,
    #[error("execution profile is incompatible with this adapter")]
    ProfileMismatch,
    #[error("profile declares a capability not supported by this adapter")]
    UnsupportedCapability,
    #[error("runtime paths or relay endpoints violate the managed sandbox layout")]
    InvalidEnvironment,
    #[error("runtime output is malformed or exceeds its bounded event size")]
    InvalidEvent,
    #[error("runtime usage contains invalid token counters")]
    InvalidUsage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeToolKind {
    Shell,
    FileChange,
    Mcp,
    WebSearch,
    Plan,
    Collaboration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeActivityPhase {
    Started,
    Updated,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeFailureKind {
    AuthRefreshReused,
    AuthExpired,
    AuthInvalidated,
    AuthRequired,
    RateLimited,
    ProviderUnavailable,
    RuntimeError,
}

/// Missing reported counters stay unknown, not zero. Cost is not inferred.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: Option<u64>,
    pub cache_write_input_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
}

#[derive(Debug)]
pub enum RuntimeObservation {
    ThreadStarted,
    TurnStarted,
    /// A completed provider turn is NOT an accepted Task/stage outcome.
    TurnCompleted {
        usage: Option<RuntimeUsage>,
    },
    AssistantOutput {
        text: SecretBytes,
    },
    ToolActivity {
        kind: RuntimeToolKind,
        phase: RuntimeActivityPhase,
        succeeded: Option<bool>,
    },
    Failure {
        kind: RuntimeFailureKind,
    },
    /// Hidden reasoning and unrecognized future events are never copied into
    /// normalized output. A caller must not persist their raw body as fallback.
    Ignored,
}
