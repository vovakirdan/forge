//! A Task's immutable starting point is independent of its future-Run source policy.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use super::{GitBranchRef, GitObjectId, GitValueError, LocalGitPath};

/// Object format is explicit even before the first Git object exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitObjectFormat {
    Sha1,
    Sha256,
}

impl GitObjectFormat {
    #[must_use]
    pub fn for_object(object: &GitObjectId) -> Self {
        if object.as_str().len() == 40 {
            Self::Sha1
        } else {
            Self::Sha256
        }
    }
}

/// An unborn source has no fabricated commit identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitInitialRevision {
    Commit(GitObjectId),
    Unborn { object_format: GitObjectFormat },
}

impl GitInitialRevision {
    #[must_use]
    pub fn commit(&self) -> Option<&GitObjectId> {
        match self {
            Self::Commit(commit) => Some(commit),
            Self::Unborn { .. } => None,
        }
    }

    #[must_use]
    pub fn object_format(&self) -> GitObjectFormat {
        match self {
            Self::Commit(commit) => GitObjectFormat::for_object(commit),
            Self::Unborn { object_format } => *object_format,
        }
    }
}

impl From<GitObjectId> for GitInitialRevision {
    fn from(commit: GitObjectId) -> Self {
        Self::Commit(commit)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ExplicitRevision {
    Commit { commit: GitObjectId },
    Unborn { object_format: GitObjectFormat },
}

impl Serialize for GitInitialRevision {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            // Preserve existing committed Task snapshot bytes and SQL projections.
            Self::Commit(commit) => commit.serialize(serializer),
            Self::Unborn { object_format } => ExplicitRevision::Unborn {
                object_format: *object_format,
            }
            .serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for GitInitialRevision {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Input {
            Legacy(GitObjectId),
            Explicit(ExplicitRevision),
        }
        Ok(match Input::deserialize(deserializer)? {
            Input::Legacy(commit) | Input::Explicit(ExplicitRevision::Commit { commit }) => {
                Self::Commit(commit)
            }
            Input::Explicit(ExplicitRevision::Unborn { object_format }) => {
                Self::Unborn { object_format }
            }
        })
    }
}

/// This setting affects only future writer Runs; it never resets retained files.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskGitSourcePolicy {
    #[default]
    LatestTarget,
    PinnedCommit {
        commit: GitObjectId,
    },
}

/// Revision one is the immutable default for Tasks without an explicit override.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskGitSourceSetting {
    pub revision: u64,
    pub policy: TaskGitSourcePolicy,
}

impl Default for TaskGitSourceSetting {
    fn default() -> Self {
        Self {
            revision: 1,
            policy: TaskGitSourcePolicy::LatestTarget,
        }
    }
}

impl TaskGitSourceSetting {
    pub fn validate(&self) -> Result<(), GitValueError> {
        if self.revision == 0 {
            Err(GitValueError::SourcePolicy)
        } else {
            Ok(())
        }
    }

    pub fn changed(&self, policy: TaskGitSourcePolicy) -> Result<Self, GitValueError> {
        self.validate()?;
        Ok(Self {
            revision: self
                .revision
                .checked_add(1)
                .ok_or(GitValueError::SourcePolicy)?,
            policy,
        })
    }
}

/// Operator-registered source request frozen in a V6 writer RunSpec.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitSourceRequest {
    pub repository_id: Uuid,
    pub source: LocalGitPath,
    pub target_ref: GitBranchRef,
    pub initial_revision: GitInitialRevision,
    pub policy_revision: u64,
    pub policy: TaskGitSourcePolicy,
}

impl GitSourceRequest {
    pub fn validate(&self) -> Result<(), GitValueError> {
        if self.repository_id.get_version_num() != 7 || self.policy_revision == 0 {
            return Err(GitValueError::SourcePolicy);
        }
        if let TaskGitSourcePolicy::PinnedCommit { commit } = &self.policy
            && GitObjectFormat::for_object(commit) != self.initial_revision.object_format()
        {
            return Err(GitValueError::SourcePolicy);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn initial_revision_reads_legacy_and_explicit_commit_without_fake_unborn_sha() {
        let sha = "a".repeat(40);
        let legacy: GitInitialRevision = serde_json::from_value(json!(sha)).unwrap();
        let explicit: GitInitialRevision =
            serde_json::from_value(json!({"kind":"commit","commit":sha})).unwrap();
        assert_eq!(legacy, explicit);
        assert_eq!(serde_json::to_value(legacy).unwrap(), json!(sha));
        for invalid in [json!(null), json!("0".repeat(40)), json!({"kind":"unborn"})] {
            assert!(serde_json::from_value::<GitInitialRevision>(invalid).is_err());
        }
        let unborn: GitInitialRevision =
            serde_json::from_value(json!({"kind":"unborn","object_format":"sha1"})).unwrap();
        assert_eq!(unborn.commit(), None);
    }

    #[test]
    fn source_policy_revisions_preserve_prior_selection() {
        let initial = TaskGitSourceSetting::default();
        let changed = initial
            .changed(TaskGitSourcePolicy::PinnedCommit {
                commit: GitObjectId::new("a".repeat(40)).unwrap(),
            })
            .unwrap();
        assert_eq!(initial, TaskGitSourceSetting::default());
        assert_eq!(changed.revision, 2);
        assert_eq!(
            changed
                .changed(TaskGitSourcePolicy::LatestTarget)
                .unwrap()
                .revision,
            3
        );
    }
}
