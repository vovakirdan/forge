//! Provider-neutral, secret-free sandbox intent pinned for one execution attempt.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod execution;
mod hook;
mod system_job;
pub use system_job::{SYSTEM_JOB_RUN_SPEC_VERSION, SystemJobRunSpec};
mod task_v6;
pub use execution::{
    COMMUNICATION_RUN_SPEC_VERSION, CommunicationRunSpec, RESOLUTION_RUN_SPEC_VERSION,
    ResolutionRunSpec, RuntimeLaunchSpec,
};
pub use hook::{HOOK_RUN_SPEC_VERSION, HookRunSpec, SandboxLaunchSpec};
pub use task_v6::{TASK_RUN_SPEC_VERSION, TaskRunSpecV6};

/// Version of the first real sandbox RunSpec; historical fake specs remain v1.
pub const SANDBOX_RUN_SPEC_VERSION: u16 = 2;

/// Explicit operator assignment. Updating it never changes existing Run snapshots.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeBinding {
    /// Pinned output and API accounting limits; currency is integer micro-USD.
    #[serde(default)]
    pub budget: RunBudget,
    /// Validated provider, model, credential reference and capability snapshot.
    pub execution_profile: crate::ExecutionProfile,
    /// Pinned OCI image reference; mutable tags are not execution identities.
    pub image: String,
    /// Source initialization for Task-owned files.
    pub surface: SurfaceSpec,
    /// Working file permissions; scratch space remains writable.
    #[serde(default)]
    pub access: SurfaceAccess,
    /// Hard local limits for each attempt.
    #[serde(default)]
    pub limits: ResourceLimits,
    /// Common rules included explicitly, not discovered in host configuration.
    pub system_prompt: String,
    /// Employee role/instructions pinned with the attempt.
    pub employee_prompt: String,
}

impl RuntimeBinding {
    /// Checks supported sandbox engines and their non-negotiable trust boundary.
    pub fn validate(&self) -> Result<(), RuntimeSpecError> {
        self.surface.validate()?;
        if matches!(
            self.surface,
            SurfaceSpec::GitCandidateSnapshot { .. }
                | SurfaceSpec::GitUnbornCandidateSnapshot { .. }
        ) && self.access != SurfaceAccess::ReadOnly
        {
            return Err(RuntimeSpecError::InvalidSurface);
        }
        self.limits.validate()?;
        self.budget.validate()?;
        let profile = &self.execution_profile;
        let valid_lane = match profile.adapter_id() {
            "codex_cli" => {
                profile.adapter_version() == "0.153.2"
                    && profile.credential_delivery()
                        == crate::CredentialDeliveryMode::IsolatedRuntimeSecret
                    && profile.capability_profile().transport_engine
                        == crate::TransportEngine::CliWrapper
            }
            "opencode_runtime" => {
                profile.credential_delivery() == crate::CredentialDeliveryMode::ProxyOnly
                    && profile.capability_profile().transport_engine
                        == crate::TransportEngine::ApiRuntime
            }
            "claude_code_cli" => {
                profile.adapter_version() == "2.1.263"
                    && profile.provider_id() == "anthropic"
                    && profile.credential_binding().account_id.is_some()
                    && profile.credential_delivery()
                        == crate::CredentialDeliveryMode::IsolatedRuntimeSecret
                    && profile.capability_profile().transport_engine
                        == crate::TransportEngine::CliWrapper
            }
            _ => false,
        };
        let required = [
            if profile.credential_delivery() == crate::CredentialDeliveryMode::ProxyOnly {
                crate::RuntimeCapability::GatewayAuth
            } else {
                crate::RuntimeCapability::NativeMcp
            },
            crate::RuntimeCapability::ControlledStop,
        ]
        .into_iter()
        .collect();
        let digest = self.image.rsplit_once("@sha256:").map(|(_, digest)| digest);
        if !valid_lane
            || !profile.capability_profile().supports(&required)
            || !digest.is_some_and(|digest| {
                digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())
            })
            || self.image.starts_with('-')
            || self.image.chars().any(char::is_whitespace)
            || self.system_prompt.trim().is_empty()
            || self.employee_prompt.trim().is_empty()
            || self.system_prompt.len() + self.employee_prompt.len() > 128 * 1024
        {
            return Err(RuntimeSpecError::InvalidProfile);
        }
        Ok(())
    }
}

/// Local output is a hard cap. Monetary cost is an observed-spend gate and can
/// overshoot while requests are in flight; native subscription cost is unknown.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunBudget {
    pub max_output_bytes: u64,
    pub requests_per_minute: u32,
    pub tokens_per_minute: u32,
    pub max_spend_microusd: Option<u64>,
}
impl Default for RunBudget {
    fn default() -> Self {
        Self {
            max_output_bytes: 16 * 1024 * 1024,
            requests_per_minute: 60,
            tokens_per_minute: 100_000,
            max_spend_microusd: None,
        }
    }
}
impl RunBudget {
    pub fn validate(&self) -> Result<(), RuntimeSpecError> {
        if self.max_output_bytes == 0
            || self.max_output_bytes > 256 * 1024 * 1024
            || self.requests_per_minute == 0
            || self.tokens_per_minute == 0
            || self
                .max_spend_microusd
                .is_some_and(|amount| amount == 0 || amount > 1_000_000_000_000)
        {
            return Err(RuntimeSpecError::InvalidLimits);
        }
        Ok(())
    }
}

/// Secret-free immutable intent sent to a sandbox-capable Supervisor.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxRunSpec {
    /// Discriminator checked alongside the Protobuf schema version.
    pub schema_version: u16,
    /// Canonical Project owner, used for all gateway and evidence scopes.
    pub project_id: crate::ProjectId,
    /// Task-owned persistent surface identity, never a host mount path.
    pub surface_id: Uuid,
    /// Explicit runtime configuration snapshot.
    pub binding: RuntimeBinding,
    /// Immutable task text plus permitted outcome/artifact contract.
    pub instruction: String,
}

impl SandboxRunSpec {
    /// Refuses unknown schema versions and malformed snapshots before provision.
    pub fn validate(&self) -> Result<(), RuntimeSpecError> {
        if self.schema_version != SANDBOX_RUN_SPEC_VERSION
            || matches!(
                self.binding.surface,
                SurfaceSpec::GitUnborn { .. } | SurfaceSpec::GitUnbornCandidateSnapshot { .. }
            )
            || self.surface_id.is_nil()
            || self.instruction.trim().is_empty()
            || self.instruction.len() > 512 * 1024
        {
            return Err(RuntimeSpecError::InvalidProfile);
        }
        self.binding.validate()
    }
}

/// Access to the Task-owned working files, independent of runtime scratch space.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceAccess {
    /// A single physical writer may own this surface.
    #[default]
    ReadWrite,
    /// A stable copy prepared only after the previous writer has quiesced.
    ReadOnly,
}

/// Source selection is operator-controlled, never supplied by an executing worker.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum SurfaceSpec {
    /// Ephemeral working directory, without persistent project files.
    None,
    /// An initially empty, Task-owned persistent directory.
    FilesystemSandbox,
    /// A Task-private Git repository; no shared writable main-checkout metadata.
    GitWorktree {
        /// Absolute source repository path, used by trusted preparation only.
        repository: String,
        /// Explicit base commit or ref resolved during preparation.
        base_ref: String,
    },
    /// Task-private Git with no initial commit; the Employee creates the first one.
    GitUnborn {
        repository: String,
        object_format: crate::git::GitObjectFormat,
    },
    /// Read-only accepted candidate of a Task whose original surface was unborn.
    GitUnbornCandidateSnapshot {
        repository: String,
        object_format: crate::git::GitObjectFormat,
        candidate: crate::git::GitCandidate,
    },
    /// Core-derived read-only copy of an accepted Task revision, never an
    /// operator-selected Employee source or the mutable writer directory.
    GitCandidateSnapshot {
        /// Original Task source, checked against its host-owned manifest.
        repository: String,
        /// Original pinned Task base, not the snapshot checkout revision.
        base_ref: String,
        /// Exact accepted commit and tree selected from canonical storage.
        candidate: crate::git::GitCandidate,
    },
}

/// Hard local resource bounds, not inferred provider-cost guarantees.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceLimits {
    /// CPU quota in milli-CPU units.
    pub cpu_millis: u32,
    /// Container memory ceiling in bytes.
    pub memory_bytes: u64,
    /// Maximum processes in the sandbox cgroup.
    pub pids: u32,
    /// Maximum elapsed attempt duration before a managed stop.
    pub wall_seconds: u32,
    /// Grace period before force-stop escalation.
    pub stop_grace_seconds: u32,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            cpu_millis: 2_000,
            memory_bytes: 4 * 1024 * 1024 * 1024,
            pids: 256,
            wall_seconds: 3_600,
            stop_grace_seconds: 30,
        }
    }
}

impl ResourceLimits {
    /// Rejects zero bounds, rather than silently disabling resource control.
    pub fn validate(&self) -> Result<(), RuntimeSpecError> {
        if self.cpu_millis == 0
            || self.memory_bytes < 16 * 1024 * 1024
            || self.pids == 0
            || self.wall_seconds == 0
            || self.stop_grace_seconds == 0
            || self.stop_grace_seconds > self.wall_seconds
        {
            return Err(RuntimeSpecError::InvalidLimits);
        }
        Ok(())
    }
}

/// Stable identity checked on every gateway request and runtime observation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunScope {
    /// Canonical attempt identity.
    pub run_id: Uuid,
    /// Monotonically increasing Lease fence.
    pub fencing_token: u64,
    /// Runtime generation within the attempt.
    pub environment_epoch: u64,
}

/// Machine-readable recovery assessment; uncertainty does not authorize retries.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAssessment {
    /// Positive evidence proves no process or external work began.
    NotStartedConfirmed,
    /// Retained files or observations prove partial work.
    PartialWorkObserved,
    /// An external effect may have escaped the local working directory.
    ExternalEffectPossible,
    /// Available observations do not prove what happened.
    Unknown,
}

/// Boot behavior is independent of task lifecycle or provider auth mode.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BootRecoveryPolicy {
    /// Reconcile environments, but wait for every explicit management decision.
    ManualHold,
    /// Retry only accepted confirmed non-starts, then keep the queue held.
    #[default]
    RecoverSafeThenHold,
    /// Resume eligible queue entries after reconciliation, never uncertain work.
    ReconcileThenResumeQueue,
}

/// Invalid execution intent is rejected before an external process is created.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RuntimeSpecError {
    /// Unknown engine, unpinned image or inadequate capabilities.
    #[error("invalid or unsupported M1 execution profile")]
    InvalidProfile,
    /// Mandatory resource limits cannot be zero or contradictory.
    #[error("invalid sandbox resource limits")]
    InvalidLimits,
    /// A source binding must name an explicit absolute repository and base ref.
    #[error("invalid sandbox source binding")]
    InvalidSurface,
}

impl SurfaceSpec {
    /// Validates the non-secret source descriptor without accessing the filesystem.
    pub fn validate(&self) -> Result<(), RuntimeSpecError> {
        match self {
            Self::GitUnborn { repository, .. } => {
                crate::git::LocalGitPath::new(repository)
                    .map_err(|_| RuntimeSpecError::InvalidSurface)?;
            }
            Self::GitUnbornCandidateSnapshot {
                repository,
                object_format,
                candidate,
            } => {
                crate::git::LocalGitPath::new(repository)
                    .map_err(|_| RuntimeSpecError::InvalidSurface)?;
                if crate::git::GitObjectFormat::for_object(&candidate.commit) != *object_format
                    || candidate.commit.as_str().len() != candidate.tree.as_str().len()
                {
                    return Err(RuntimeSpecError::InvalidSurface);
                }
            }
            _ => {}
        }
        if let Self::GitCandidateSnapshot {
            repository,
            base_ref,
            candidate,
        } = self
        {
            crate::git::LocalGitPath::new(repository)
                .map_err(|_| RuntimeSpecError::InvalidSurface)?;
            let base = crate::git::GitObjectId::new(base_ref)
                .map_err(|_| RuntimeSpecError::InvalidSurface)?;
            if base.as_str().len() != candidate.commit.as_str().len()
                || candidate.commit.as_str().len() != candidate.tree.as_str().len()
            {
                return Err(RuntimeSpecError::InvalidSurface);
            }
        }
        if let Self::GitWorktree {
            repository,
            base_ref,
        } = self
            && (!repository.starts_with('/')
                || repository == "/"
                || repository.contains('\0')
                || base_ref.trim().is_empty()
                || base_ref.starts_with('-')
                || base_ref.contains('\0'))
        {
            return Err(RuntimeSpecError::InvalidSurface);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_bounded() {
        assert!(ResourceLimits::default().validate().is_ok());
    }

    #[test]
    fn zero_limit_is_not_unlimited() {
        let limits = ResourceLimits {
            cpu_millis: 0,
            ..ResourceLimits::default()
        };
        assert_eq!(limits.validate(), Err(RuntimeSpecError::InvalidLimits));
    }

    #[test]
    fn source_cannot_be_a_flag_or_root_directory() {
        assert!(
            SurfaceSpec::GitWorktree {
                repository: "/".into(),
                base_ref: "--upload-pack=evil".into()
            }
            .validate()
            .is_err()
        );
    }
}
