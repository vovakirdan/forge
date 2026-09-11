//! Path-free receipt for the exact source made available to one writer Run.
use super::{
    GitBranchRef, GitInitialRevision, GitObjectFormat, GitSourceRequest, GitValueError,
    TaskGitSourcePolicy,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitSourceDescriptor {
    pub schema_version: u16,
    pub repository_id: uuid::Uuid,
    pub target_ref: GitBranchRef,
    pub policy_revision: u64,
    pub policy: TaskGitSourcePolicy,
    pub selected_revision: GitInitialRevision,
    pub object_format: GitObjectFormat,
    pub bundle_file: Option<String>,
    pub bundle_sha256: Option<String>,
    pub bundle_bytes: Option<u64>,
}

impl GitSourceDescriptor {
    pub fn validate_request(&self, request: &GitSourceRequest) -> Result<(), GitValueError> {
        request.validate()?;
        if self.schema_version != 1
            || self.repository_id != request.repository_id
            || self.target_ref != request.target_ref
            || self.policy_revision != request.policy_revision
            || self.policy != request.policy
            || self.object_format != request.initial_revision.object_format()
            || self.object_format != self.selected_revision.object_format()
        {
            return Err(GitValueError::SourcePolicy);
        }
        match &self.selected_revision {
            GitInitialRevision::Commit(commit) => {
                if self.bundle_file.as_deref() != Some("source.bundle")
                    || self.bundle_bytes.is_none_or(|size| size == 0)
                    || self.bundle_sha256.as_ref().is_none_or(|sha| {
                        sha.len() != 64 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit())
                    })
                    || matches!(&self.policy, TaskGitSourcePolicy::PinnedCommit { commit: pinned } if pinned != commit)
                {
                    return Err(GitValueError::SourcePolicy);
                }
            }
            GitInitialRevision::Unborn { .. } => {
                if !matches!(request.initial_revision, GitInitialRevision::Unborn { .. })
                    || !matches!(self.policy, TaskGitSourcePolicy::LatestTarget)
                    || self.bundle_file.is_some()
                    || self.bundle_bytes.is_some()
                    || self.bundle_sha256.is_some()
                {
                    return Err(GitValueError::SourcePolicy);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{GitObjectId, LocalGitPath};
    use super::*;

    fn request() -> GitSourceRequest {
        GitSourceRequest {
            repository_id: uuid::Uuid::now_v7(),
            source: LocalGitPath::new("/tmp/source").unwrap(),
            target_ref: GitBranchRef::new("refs/heads/main").unwrap(),
            initial_revision: GitInitialRevision::Unborn {
                object_format: GitObjectFormat::Sha1,
            },
            policy_revision: 1,
            policy: TaskGitSourcePolicy::LatestTarget,
        }
    }
    fn descriptor(request: &GitSourceRequest) -> GitSourceDescriptor {
        GitSourceDescriptor {
            schema_version: 1,
            repository_id: request.repository_id,
            target_ref: request.target_ref.clone(),
            policy_revision: request.policy_revision,
            policy: request.policy.clone(),
            selected_revision: request.initial_revision.clone(),
            object_format: GitObjectFormat::Sha1,
            bundle_file: None,
            bundle_bytes: None,
            bundle_sha256: None,
        }
    }
    #[test]
    fn unborn_receipt_has_no_bundle_and_cannot_replace_established_history() {
        let mut request = request();
        let receipt = descriptor(&request);
        assert!(receipt.validate_request(&request).is_ok());
        request.initial_revision = GitObjectId::new("a".repeat(40)).unwrap().into();
        assert!(receipt.validate_request(&request).is_err());
    }
    #[test]
    fn receipts_reject_foreign_policy_pin_and_host_bundle_paths() {
        let request = request();
        let mut receipt = descriptor(&request);
        receipt.selected_revision = GitObjectId::new("a".repeat(40)).unwrap().into();
        receipt.bundle_file = Some("source.bundle".into());
        receipt.bundle_sha256 = Some("b".repeat(64));
        receipt.bundle_bytes = Some(1);
        assert!(receipt.validate_request(&request).is_ok());
        let mut foreign = receipt.clone();
        foreign.repository_id = uuid::Uuid::now_v7();
        assert!(foreign.validate_request(&request).is_err());
        let mut foreign = receipt.clone();
        foreign.policy_revision += 1;
        assert!(foreign.validate_request(&request).is_err());
        let mut foreign = receipt.clone();
        foreign.bundle_file = Some("/host/source.bundle".into());
        assert!(foreign.validate_request(&request).is_err());
        let mut request = request;
        request.policy = TaskGitSourcePolicy::PinnedCommit {
            commit: GitObjectId::new("c".repeat(40)).unwrap(),
        };
        receipt.policy = request.policy.clone();
        assert!(receipt.validate_request(&request).is_err());
    }
}
