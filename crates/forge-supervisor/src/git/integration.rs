//! Preparing objects and publishing a target ref are intentionally separate effects.

use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

use forge_domain::{
    Timestamp,
    git::{
        GitBranchRef, GitCandidate, GitIntegrationIntent, GitIntegrationState, GitObjectId,
        LocalGitPath,
    },
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    GitBackend, GitBackendError,
    repository::{canonical_directory, path_text},
};

/// Inputs supplied by Core after it validated candidate acceptance and authority.
pub struct GitIntegrationRequest<'a> {
    /// Explicit connected local repository; never inferred from an Employee config.
    pub target: &'a Path,
    /// Exact protected branch which this operation may update.
    pub branch: &'a GitBranchRef,
    /// Task-private repository containing the pinned candidate.
    pub candidate_source: &'a Path,
    /// Accepted immutable candidate, not a mutable working tree.
    pub candidate: &'a GitCandidate,
    /// New owner-only preparation directory retained for recovery.
    pub staging_directory: &'a Path,
    /// Stable identity to persist with the resulting intent.
    pub operation_id: Uuid,
    /// Canonical commit timestamp, pinned before any retry.
    pub committed_at: Timestamp,
    /// Explicit initial Unborn binding with no established-target history.
    pub allow_initial_publication: bool,
}

impl GitBackend {
    /// Prepares a two-parent merge, or an exact first publication of the candidate.
    ///
    /// Writes only `staging_directory`; no source or target ref is modified. Core
    /// must durably persist the returned intent before calling `apply_integration`.
    pub async fn prepare_integration(
        &self,
        request: GitIntegrationRequest<'_>,
    ) -> Result<GitIntegrationIntent, GitBackendError> {
        let common = self.common_directory(request.target).await?;
        self.ensure_target_available(request.target, request.branch)
            .await?;
        self.private_source(request.candidate_source).await?;
        let expected_target = self
            .read_ref(request.target, request.branch.as_str())
            .await?;
        if expected_target.is_none() && !request.allow_initial_publication {
            return Err(GitBackendError::InvalidIntent);
        }
        if expected_target.as_ref() == Some(&request.candidate.commit) {
            return Err(GitBackendError::NoChanges);
        }
        if request.operation_id.is_nil()
            || request.committed_at.as_offset_date_time().unix_timestamp() < 0
        {
            return Err(GitBackendError::InvalidIntent);
        }
        let repository = self
            .clone_private(request.target, request.staging_directory)
            .await?;
        if let Some(target) = &expected_target {
            self.fetch_object(&repository, request.target, target)
                .await?;
        }
        self.fetch_object(
            &repository,
            request.candidate_source,
            &request.candidate.commit,
        )
        .await?;
        self.ensure_supported_candidate(&repository, request.candidate)
            .await?;
        if let Some(target) = &expected_target
            && !self
                .ancestor(&repository, target, &request.candidate.commit)
                .await?
        {
            return Err(GitBackendError::StaleBase);
        }
        let timestamp = format!(
            "@{} +0000",
            request.committed_at.as_offset_date_time().unix_timestamp()
        );
        let message = format!("Forge integration {}\n", request.operation_id);
        let merge_commit = if let Some(target) = &expected_target {
            let output = self
                .command(
                    &repository,
                    &[
                        "commit-tree",
                        request.candidate.tree.as_str(),
                        "-p",
                        target.as_str(),
                        "-p",
                        request.candidate.commit.as_str(),
                    ],
                    message.as_bytes(),
                    &[
                        ("GIT_AUTHOR_NAME", "Forge Integration"),
                        ("GIT_AUTHOR_EMAIL", "forge@localhost"),
                        ("GIT_COMMITTER_NAME", "Forge Integration"),
                        ("GIT_COMMITTER_EMAIL", "forge@localhost"),
                        ("GIT_AUTHOR_DATE", &timestamp),
                        ("GIT_COMMITTER_DATE", &timestamp),
                    ],
                )
                .await?
                .require_success("prepare merge")?;
            GitObjectId::new(
                String::from_utf8(output)
                    .map_err(|_| GitBackendError::InvalidIntent)?
                    .trim(),
            )?
        } else {
            request.candidate.commit.clone()
        };
        let intent = GitIntegrationIntent {
            operation_id: request.operation_id,
            target_repository: LocalGitPath::new(path_text(&common)?)?,
            target_ref: request.branch.clone(),
            expected_target,
            candidate: request.candidate.clone(),
            merge_commit,
        };
        intent.validate()?;
        self.text(
            &repository,
            &[
                "update-ref",
                &intent.receipt_ref(),
                intent.merge_commit.as_str(),
            ],
            "retain prepared merge",
        )
        .await?;
        Ok(intent)
    }

    /// Publishes a previously persisted intent using a physical-repository/ref lock.
    ///
    /// A checked-out target is rejected even in linked worktrees. Concurrent owner
    /// checkout/worktree operations are unsupported; this lock coordinates Forge,
    /// not the owner's arbitrary shell. Git and PostgreSQL are not one transaction.
    pub async fn apply_integration(
        &self,
        target: &Path,
        staging_directory: &Path,
        intent: &GitIntegrationIntent,
    ) -> Result<GitIntegrationState, GitBackendError> {
        let common = self.intent_repository(target, intent).await?;
        let _lock = integration_lock(&common, &intent.target_ref)?;
        let state = self.reconcile_integration(target, intent).await?;
        if state != GitIntegrationState::Retryable {
            if intent.expected_target.is_none()
                && state == GitIntegrationState::Unknown
                && self.first_publication_competitor(target, intent).await?
            {
                return Err(GitBackendError::StaleBase);
            }
            return Ok(state);
        }
        self.ensure_target_available(target, &intent.target_ref)
            .await?;
        canonical_directory(staging_directory)?;
        let repository = staging_directory.join("repository.git");
        self.validate_prepared(&repository, intent).await?;
        self.fetch_object(target, &repository, &intent.merge_commit)
            .await?;
        self.ensure_target_available(target, &intent.target_ref)
            .await?;
        let target_change = match &intent.expected_target {
            Some(target) => format!(
                "update {} {} {}",
                intent.target_ref.as_str(),
                intent.merge_commit.as_str(),
                target.as_str()
            ),
            None => format!(
                "create {} {}",
                intent.target_ref.as_str(),
                intent.merge_commit.as_str()
            ),
        };
        let changes = format!(
            "start\noption no-deref\n{}\noption no-deref\ncreate {} {}\nprepare\ncommit\n",
            target_change,
            intent.receipt_ref(),
            intent.merge_commit.as_str(),
        );
        let output = self
            .command(target, &["update-ref", "--stdin"], changes.as_bytes(), &[])
            .await?;
        let state = self.reconcile_integration(target, intent).await?;
        if intent.expected_target.is_none()
            && state == GitIntegrationState::Unknown
            && self.first_publication_competitor(target, intent).await?
        {
            return Err(GitBackendError::StaleBase);
        }
        if output.code != Some(0) && state == GitIntegrationState::Retryable {
            output.require_success("publish merge")?;
        }
        Ok(state)
    }

    async fn first_publication_competitor(
        &self,
        target: &Path,
        intent: &GitIntegrationIntent,
    ) -> Result<bool, GitBackendError> {
        match self
            .ensure_target_not_symbolic(target, &intent.target_ref)
            .await
        {
            Ok(()) => {}
            Err(GitBackendError::SymbolicTarget) => return Ok(false),
            Err(error) => return Err(error),
        }
        // A contradictory receipt or symbolic ref is uncertainty, not a clean
        // competing create. Only the latter can safely request Employee rework.
        Ok(self
            .read_ref(target, &intent.receipt_ref())
            .await?
            .is_none()
            && self
                .read_ref(target, intent.target_ref.as_str())
                .await?
                .is_some())
    }

    /// Reconciles actual target ancestry; a receipt alone never proves completion.
    ///
    /// An unchanged old target with no receipt is retryable. Contradictory refs,
    /// deleted targets, or unrelated history require a Core hold/configured outcome.
    pub async fn reconcile_integration(
        &self,
        target: &Path,
        intent: &GitIntegrationIntent,
    ) -> Result<GitIntegrationState, GitBackendError> {
        self.intent_repository(target, intent).await?;
        let symbolic = self
            .command(
                target,
                &["symbolic-ref", "--quiet", intent.target_ref.as_str()],
                &[],
                &[],
            )
            .await?;
        if symbolic.code == Some(0) {
            return Ok(GitIntegrationState::Unknown);
        }
        if symbolic.code != Some(1) {
            symbolic.require_success("reconcile target ref")?;
        }
        let current = self.read_ref(target, intent.target_ref.as_str()).await?;
        let receipt = self.read_ref(target, &intent.receipt_ref()).await?;
        if receipt
            .as_ref()
            .is_some_and(|receipt| *receipt != intent.merge_commit)
        {
            return Ok(GitIntegrationState::Unknown);
        }
        let Some(current) = current else {
            return Ok(if intent.expected_target.is_none() && receipt.is_none() {
                GitIntegrationState::Retryable
            } else {
                GitIntegrationState::Unknown
            });
        };
        if current == intent.merge_commit {
            self.validate_prepared(target, intent).await?;
            return Ok(GitIntegrationState::Applied);
        }
        if intent.expected_target.as_ref() == Some(&current) {
            return Ok(if receipt.is_none() {
                GitIntegrationState::Retryable
            } else {
                GitIntegrationState::Unknown
            });
        }
        let object = format!("{}^{{commit}}", intent.merge_commit.as_str());
        let exists = self
            .command(target, &["cat-file", "-e", &object], &[], &[])
            .await?;
        if exists.code == Some(0)
            && self
                .ancestor(target, &intent.merge_commit, &current)
                .await?
        {
            self.validate_prepared(target, intent).await?;
            return Ok(GitIntegrationState::Applied);
        }
        Ok(GitIntegrationState::Unknown)
    }

    async fn intent_repository(
        &self,
        target: &Path,
        intent: &GitIntegrationIntent,
    ) -> Result<PathBuf, GitBackendError> {
        intent.validate()?;
        let common = self.common_directory(target).await?;
        if common != intent.target_repository.as_path() {
            return Err(GitBackendError::InvalidIntent);
        }
        Ok(common)
    }

    pub(super) async fn read_ref(
        &self,
        target: &Path,
        reference: &str,
    ) -> Result<Option<GitObjectId>, GitBackendError> {
        let output = self
            .command(
                target,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    "--end-of-options",
                    reference,
                ],
                &[],
                &[],
            )
            .await?;
        if output.code == Some(1) {
            return Ok(None);
        }
        let bytes = output.require_success("read integration ref")?;
        let value = String::from_utf8(bytes).map_err(|_| GitBackendError::InvalidIntent)?;
        Ok(Some(GitObjectId::new(value.trim())?))
    }

    async fn validate_prepared(
        &self,
        repository: &Path,
        intent: &GitIntegrationIntent,
    ) -> Result<(), GitBackendError> {
        canonical_directory(repository)?;
        if intent.expected_target.is_none() {
            return if intent.merge_commit == intent.candidate.commit
                && self.tree(repository, &intent.candidate.commit).await? == intent.candidate.tree
            {
                Ok(())
            } else {
                Err(GitBackendError::InvalidIntent)
            };
        }
        let target = intent
            .expected_target
            .as_ref()
            .ok_or(GitBackendError::InvalidIntent)?;
        let parents = self
            .text(
                repository,
                &[
                    "rev-list",
                    "--parents",
                    "-n",
                    "1",
                    intent.merge_commit.as_str(),
                ],
                "prepared parents",
            )
            .await?;
        let expected = format!(
            "{} {} {}",
            intent.merge_commit.as_str(),
            target.as_str(),
            intent.candidate.commit.as_str()
        );
        if parents.trim() != expected
            || self.tree(repository, &intent.merge_commit).await? != intent.candidate.tree
            || self.tree(repository, &intent.candidate.commit).await? != intent.candidate.tree
            || !self
                .ancestor(repository, target, &intent.candidate.commit)
                .await?
        {
            return Err(GitBackendError::InvalidIntent);
        }
        Ok(())
    }
}

fn integration_lock(common: &Path, branch: &GitBranchRef) -> Result<File, GitBackendError> {
    let name = format!(
        "forge-integration-{:x}.lock",
        Sha256::digest(branch.as_str().as_bytes())
    );
    let path = common.join(name);
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path)?;
    let metadata = lock.metadata()?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != nix::unistd::Uid::effective().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(GitBackendError::UnsafePath);
    }
    lock.try_lock().map_err(|error| match error {
        fs::TryLockError::WouldBlock => GitBackendError::IntegrationBusy,
        fs::TryLockError::Error(error) => GitBackendError::Io(error),
    })?;
    Ok(lock)
}
