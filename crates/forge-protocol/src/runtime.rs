//! Private runner materialization contract, separate from canonical RunSpec.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Non-secret launch instructions supplied only by trusted Core composition.
/// Prompt bytes and credential bytes are separate private files, never fields.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerInvocation {
    /// Immutable adapter identity checked against the RunSpec.
    pub adapter_id: String,
    /// Executable inside the pinned image, never a host executable.
    pub program: String,
    /// Argument vector; no shell expansion is performed.
    pub args: Vec<String>,
    /// Explicit non-secret environment; inherited environment is cleared.
    pub env: BTreeMap<String, String>,
    /// Configuration files relative to the private runtime directory.
    pub managed_files: Vec<RunnerManagedFile>,
    /// Optional auth copy into writable runtime state for CLI refresh.
    pub credential_files: Vec<RunnerCredentialFile>,
    /// Maximum redacted stdout plus stderr bytes persisted by the wrapper.
    pub max_output_bytes: u64,
    /// Grace before environment force stop, in seconds.
    pub stop_grace_seconds: u32,
    /// Explicit opt-in. Omission preserves historical immutable one-shot bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_input: Option<RuntimeInputConfig>,
}

/// Pinned exact scope for the sandbox-local native input bridge.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeInputConfig {
    pub run_id: String,
    pub fencing_token: u64,
    pub environment_epoch: u64,
}

/// Managed configuration content, explicitly excluding credentials and prompts.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerManagedFile {
    pub relative_path: String,
    pub contents: String,
}

/// Paths within the fixed private mount layout, never arbitrary host paths.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerCredentialFile {
    pub source: String,
    pub target: String,
    pub writeback: bool,
}

/// Durable wrapper exit evidence. Exit zero is not a stage outcome submission.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerExit {
    pub exit_code: Option<i32>,
    pub output_incomplete: bool,
    pub stop_requested: bool,
}
