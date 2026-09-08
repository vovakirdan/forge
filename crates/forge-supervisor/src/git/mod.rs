//! Bounded local Git operations, not a scheduler or a source of domain authority.
//!
//! Core owns candidate approval, writer quiescence, durable integration intent,
//! and the terminal outcome. Preparation writes only a fresh private directory.
//! Applying an intent is a separately authorized effect. Repository owners must
//! not concurrently switch/check out the target in an external worktree: Git's
//! ref lock cannot provide that exclusion against the owner's shell.
//!
//! M2 fails closed for private repositories with configured filters (including
//! clean/process filters) or submodule candidates. Their host-side execution and
//! nested-source contracts are not implemented; employee config is never stripped.

use std::time::Duration;

use forge_domain::git::GitValueError;
use thiserror::Error;

mod candidate;
mod command;
mod integration;
mod repository;

pub use integration::GitIntegrationRequest;

#[cfg(test)]
mod tests;

/// Local Git execution with bounded subprocess time and diagnostic output.
#[derive(Clone, Debug)]
pub struct GitBackend {
    timeout: Duration,
}

impl Default for GitBackend {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
        }
    }
}

impl GitBackend {
    /// Overrides the per-command timeout, bounded to one millisecond..five minutes.
    pub fn with_timeout(timeout: Duration) -> Result<Self, GitBackendError> {
        if timeout < Duration::from_millis(1) || timeout > Duration::from_secs(300) {
            return Err(GitBackendError::InvalidTimeout);
        }
        Ok(Self { timeout })
    }
}

/// Typed operational failures. Git stderr, paths, and repository content are omitted.
#[derive(Debug, Error)]
pub enum GitBackendError {
    /// Domain input was malformed before any Git subprocess was started.
    #[error(transparent)]
    InvalidValue(#[from] GitValueError),
    /// Timeout configuration is outside the supported positive bound.
    #[error("invalid Git command timeout")]
    InvalidTimeout,
    /// A path is noncanonical, an unexpected symlink, or escapes the private root.
    #[error("unsafe local Git repository path")]
    UnsafePath,
    /// A fresh snapshot/staging directory must never overwrite retained work.
    #[error("Git destination already exists")]
    DestinationExists,
    /// The caller must have positive evidence that the writer is gone.
    #[error("Git candidate writer is not confirmed quiescent")]
    WriterActive,
    /// Committed HEAD differs from the proposed revision.
    #[error("Git candidate HEAD changed")]
    HeadChanged,
    /// Uncommitted or untracked files must be addressed by an Employee, not auto-added.
    #[error("Git candidate workspace is not clean")]
    DirtyCandidate,
    /// The commit's actual tree does not match the accepted candidate.
    #[error("Git candidate does not match the pinned commit")]
    CandidateMismatch,
    /// Worker-owned filters could otherwise execute programs in host-side Git.
    #[error("Git private repository config contains external filters")]
    ExternalFilter,
    /// Nested repositories require a separate source/isolation contract.
    #[error("Git candidate contains unsupported submodules")]
    SubmodulesUnsupported,
    /// No distinct candidate exists to make a two-parent merge.
    #[error("Git candidate has no changes to integrate")]
    NoChanges,
    /// The target is not an ancestor of the accepted candidate.
    #[error("Git candidate needs an explicit merge of the current target")]
    StaleBase,
    /// Includes main, linked, locked, and missing-but-registered worktrees.
    #[error("Git integration target is checked out in a worktree")]
    TargetCheckedOut,
    /// A symbolic target could redirect CAS outside the authorized ref.
    #[error("Git integration target is symbolic")]
    SymbolicTarget,
    /// Another Forge process owns this physical repository/branch operation.
    #[error("Git integration target is busy")]
    IntegrationBusy,
    /// The caller's durable intent does not match objects or repository identity.
    #[error("Git integration intent does not match prepared objects")]
    InvalidIntent,
    /// Git did not finish within its configured per-command deadline.
    #[error("Git command timed out")]
    Timeout,
    /// Subprocess output exceeded the fixed diagnostic/protocol budget.
    #[error("Git command output exceeded its bound")]
    OutputLimit,
    /// Git failed; only the operation name and exit code are exposed.
    #[error("Git {operation} failed (exit {exit_code:?})")]
    Command {
        /// Static operation label, never caller input.
        operation: &'static str,
        /// Exit code, absent when terminated by a signal.
        exit_code: Option<i32>,
    },
    /// Local filesystem/subprocess I/O failed without exposing content.
    #[error("Git backend I/O failed")]
    Io(#[from] std::io::Error),
}
