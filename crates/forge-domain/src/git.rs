//! Validated Git values shared by Core's durable intent and execution adapters.

use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Invalid input at the Git domain boundary; values are omitted from diagnostics.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum GitValueError {
    /// Object IDs must be complete, nonzero SHA-1 or SHA-256 hexadecimal values.
    #[error("invalid complete Git object id")]
    ObjectId,
    /// Only explicit, valid refs under refs/heads are accepted as targets.
    #[error("invalid explicit Git branch ref")]
    BranchRef,
    /// Local repository paths are absolute, non-root, normalized UTF-8 paths.
    #[error("invalid absolute local Git path")]
    LocalPath,
    /// The intent must identify one operation and use one object hash format.
    #[error("invalid Git integration intent")]
    IntegrationIntent,
}

/// Complete immutable object identity, never a symbolic ref or revision expression.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GitObjectId(String);

impl GitObjectId {
    /// Validates a full nonzero SHA-1 or SHA-256 ID and normalizes hex case.
    pub fn new(value: impl Into<String>) -> Result<Self, GitValueError> {
        let value = value.into();
        if !matches!(value.len(), 40 | 64)
            || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
            || value.bytes().all(|byte| byte == b'0')
        {
            return Err(GitValueError::ObjectId);
        }
        Ok(Self(value.to_ascii_lowercase()))
    }

    /// Returns the complete lowercase object ID.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Explicit target branch, kept separate from commit IDs and arbitrary refs.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GitBranchRef(String);

impl GitBranchRef {
    /// Accepts a full refs/heads name using Git's non-pattern ref restrictions.
    pub fn new(value: impl Into<String>) -> Result<Self, GitValueError> {
        let value = value.into();
        if !value.starts_with("refs/heads/")
            || value.len() > 1024
            || value.ends_with('.')
            || value.contains("..")
            || value.contains("@{")
            || value
                .bytes()
                .any(|byte| byte <= b' ' || byte == 127 || b"~^:?*[\\".contains(&byte))
            || value
                .split('/')
                .any(|part| part.is_empty() || part.starts_with('.') || part.ends_with(".lock"))
        {
            return Err(GitValueError::BranchRef);
        }
        Ok(Self(value))
    }

    /// Returns the fully qualified branch ref.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Lexically validated local path; the filesystem adapter also checks canonicality.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct LocalGitPath(String);

impl LocalGitPath {
    /// Rejects root, relative paths, traversal components, and NUL bytes.
    pub fn new(value: impl Into<String>) -> Result<Self, GitValueError> {
        let value = value.into();
        let path = Path::new(&value);
        if !path.is_absolute()
            || path.parent().is_none()
            || value.len() > 4096
            || value.bytes().any(|byte| byte < 32 || byte == 127)
            || value
                .split('/')
                .skip(1)
                .any(|part| matches!(part, "" | "." | ".."))
            || path
                .components()
                .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
        {
            return Err(GitValueError::LocalPath);
        }
        Ok(Self(value))
    }

    /// Returns the local filesystem path, without performing I/O.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }
}

macro_rules! string_conversion {
    ($name:ident) => {
        impl TryFrom<String> for $name {
            type Error = GitValueError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

string_conversion!(GitObjectId);
string_conversion!(GitBranchRef);
string_conversion!(LocalGitPath);

/// A committed candidate and its exact tree; this value alone grants no approval.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitCandidate {
    /// Commit inspected after the caller confirmed writer quiescence.
    pub commit: GitObjectId,
    /// Tree read from that commit, not reconstructed from mutable workspace files.
    pub tree: GitObjectId,
}

/// Exact integration input which Core persists before allowing a target mutation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitIntegrationIntent {
    /// Stable operation identity, reused by reconciliation and retry.
    pub operation_id: Uuid,
    /// Canonical Git common directory of the explicitly connected target.
    pub target_repository: LocalGitPath,
    /// Explicit target branch; never HEAD or a symbolic alias.
    pub target_ref: GitBranchRef,
    /// Target used as merge parent one and compare-and-swap expectation.
    pub expected_target: GitObjectId,
    /// Approved candidate used as merge parent two and sole source of its tree.
    pub candidate: GitCandidate,
    /// Exact prepared merge; retries never regenerate commit metadata.
    pub merge_commit: GitObjectId,
}

impl GitIntegrationIntent {
    /// Checks cross-field identity constraints; the adapter also verifies objects.
    pub fn validate(&self) -> Result<(), GitValueError> {
        let width = self.expected_target.as_str().len();
        if self.operation_id.is_nil()
            || self.expected_target == self.candidate.commit
            || self.merge_commit == self.expected_target
            || self.merge_commit == self.candidate.commit
            || [
                &self.candidate.commit,
                &self.candidate.tree,
                &self.merge_commit,
            ]
            .iter()
            .any(|object| object.as_str().len() != width)
        {
            return Err(GitValueError::IntegrationIntent);
        }
        Ok(())
    }

    /// Private retention marker; its existence alone never proves completion.
    #[must_use]
    pub fn receipt_ref(&self) -> String {
        format!("refs/forge/integrations/{}", self.operation_id)
    }
}

/// Observed result of reconciling an intent against the actual target history.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitIntegrationState {
    /// The exact merge is current or is retained in the target's ancestry.
    Applied,
    /// Expected target is unchanged and there is no contradictory receipt.
    Retryable,
    /// Target or receipt disagrees; Core must hold or apply a configured outcome.
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_ids_reject_revision_expressions_and_zero() {
        for value in ["HEAD", "main^{commit}", &"0".repeat(40), &"a".repeat(39)] {
            assert!(GitObjectId::new(value).is_err());
        }
        assert_eq!(
            GitObjectId::new("A".repeat(64)).unwrap().as_str(),
            "a".repeat(64)
        );
    }

    #[test]
    fn branch_refs_reject_ambiguous_or_injected_names() {
        for value in [
            "main",
            "refs/heads/",
            "refs/heads/a..b",
            "refs/heads/a.lock",
            "refs/heads/a\ncreate refs/heads/b",
            "refs/heads/a@{1}",
        ] {
            assert!(GitBranchRef::new(value).is_err(), "{value}");
        }
        assert!(GitBranchRef::new("refs/heads/team/release-1").is_ok());
    }

    #[test]
    fn serde_cannot_bypass_git_value_validation() {
        assert!(serde_json::from_str::<GitObjectId>("\"HEAD\"").is_err());
        assert!(serde_json::from_str::<GitBranchRef>("\"--force\"").is_err());
        for value in ["/", "relative/repo", "/tmp/../repo", "/tmp/./repo"] {
            assert!(LocalGitPath::new(value).is_err(), "{value}");
        }
    }
}
