//! Filesystem boundaries and ref checks shared by candidate and integration work.

use std::{
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
};

use forge_domain::git::{GitBranchRef, GitCandidate, GitObjectId, LocalGitPath};

use super::{GitBackend, GitBackendError};

pub(super) fn path_text(path: &Path) -> Result<&str, GitBackendError> {
    path.to_str().ok_or(GitBackendError::UnsafePath)
}

pub(super) fn canonical_directory(path: &Path) -> Result<PathBuf, GitBackendError> {
    LocalGitPath::new(path_text(path)?)?;
    let canonical = fs::canonicalize(path)?;
    if canonical != path || !fs::symlink_metadata(path)?.is_dir() {
        return Err(GitBackendError::UnsafePath);
    }
    Ok(canonical)
}

pub(super) fn fresh_private_directory(path: &Path) -> Result<(), GitBackendError> {
    LocalGitPath::new(path_text(path)?)?;
    let parent = path.parent().ok_or(GitBackendError::UnsafePath)?;
    canonical_directory(parent)?;
    let metadata = fs::metadata(parent)?;
    if metadata.uid() != nix::unistd::Uid::effective().as_raw() || metadata.mode() & 0o077 != 0 {
        return Err(GitBackendError::UnsafePath);
    }
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(GitBackendError::DestinationExists)
        }
        Err(error) => Err(error.into()),
    }
}

impl GitBackend {
    pub(super) async fn ensure_supported_candidate(
        &self,
        repository: &Path,
        candidate: &GitCandidate,
    ) -> Result<(), GitBackendError> {
        if self.tree(repository, &candidate.commit).await? != candidate.tree {
            return Err(GitBackendError::CandidateMismatch);
        }
        let entries = self
            .command(
                repository,
                &["ls-tree", "-r", "-z", candidate.commit.as_str()],
                &[],
                &[],
            )
            .await?
            .require_success("candidate tree scope")?;
        if entries
            .split(|byte| *byte == 0)
            .any(|entry| entry.starts_with(b"160000 "))
        {
            return Err(GitBackendError::SubmodulesUnsupported);
        }
        Ok(())
    }

    pub(super) async fn common_directory(&self, path: &Path) -> Result<PathBuf, GitBackendError> {
        canonical_directory(path)?;
        let common = self
            .text(
                path,
                &["rev-parse", "--path-format=absolute", "--git-common-dir"],
                "repository identity",
            )
            .await?;
        canonical_directory(Path::new(common.trim_end_matches('\n')))
    }

    pub(super) async fn private_source(&self, path: &Path) -> Result<PathBuf, GitBackendError> {
        if fs::symlink_metadata(path.join(".git")).is_ok_and(|metadata| metadata.is_symlink()) {
            return Err(GitBackendError::UnsafePath);
        }
        let common = self.common_directory(path).await?;
        let root = path.parent().ok_or(GitBackendError::UnsafePath)?;
        if !common.starts_with(root) {
            return Err(GitBackendError::UnsafePath);
        }
        // Workers may write their Git metadata, but host Git must not follow an
        // alternate object database or metadata symlink outside that sandbox.
        if common.join("objects/info/alternates").try_exists()?
            || common.join("info/grafts").try_exists()?
        {
            return Err(GitBackendError::UnsafePath);
        }
        let mut pending = vec![common.clone()];
        let mut inspected = 0_usize;
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                let metadata = fs::symlink_metadata(entry.path())?;
                inspected += 1;
                if inspected > 100_000
                    || metadata.is_symlink()
                    || (!metadata.is_file() && !metadata.is_dir())
                    || (metadata.is_file() && metadata.nlink() != 1)
                {
                    return Err(GitBackendError::UnsafePath);
                }
                if metadata.is_dir() {
                    pending.push(entry.path());
                }
            }
        }
        // Git status can invoke clean/process filters while refreshing an index.
        // Query effective names (including worktree config and includes) before
        // any such traversal, without printing or rewriting their secret values.
        let filters = self
            .command(
                path,
                &[
                    "config",
                    "--includes",
                    "--name-only",
                    "--get-regexp",
                    "^filter\\.",
                ],
                &[],
                &[],
            )
            .await?;
        match filters.code {
            Some(0) => return Err(GitBackendError::ExternalFilter),
            Some(1) => {}
            code => {
                return Err(GitBackendError::Command {
                    operation: "private config validation",
                    exit_code: code,
                });
            }
        }
        Ok(common)
    }

    pub(super) async fn resolve_commit(
        &self,
        path: &Path,
        expression: &str,
    ) -> Result<GitObjectId, GitBackendError> {
        let revision = format!("{expression}^{{commit}}");
        let object = self
            .text(
                path,
                &["rev-parse", "--verify", "--end-of-options", &revision],
                "resolve commit",
            )
            .await?;
        Ok(GitObjectId::new(object.trim())?)
    }

    pub(super) async fn tree(
        &self,
        path: &Path,
        commit: &GitObjectId,
    ) -> Result<GitObjectId, GitBackendError> {
        let revision = format!("{}^{{tree}}", commit.as_str());
        let object = self
            .text(
                path,
                &["rev-parse", "--verify", "--end-of-options", &revision],
                "resolve tree",
            )
            .await?;
        Ok(GitObjectId::new(object.trim())?)
    }

    pub(super) async fn ancestor(
        &self,
        path: &Path,
        ancestor: &GitObjectId,
        descendant: &GitObjectId,
    ) -> Result<bool, GitBackendError> {
        let output = self
            .command(
                path,
                &[
                    "merge-base",
                    "--is-ancestor",
                    ancestor.as_str(),
                    descendant.as_str(),
                ],
                &[],
                &[],
            )
            .await?;
        match output.code {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            code => Err(GitBackendError::Command {
                operation: "ancestry",
                exit_code: code,
            }),
        }
    }

    pub(super) async fn ensure_target_available(
        &self,
        target: &Path,
        branch: &GitBranchRef,
    ) -> Result<(), GitBackendError> {
        let symbolic = self
            .command(
                target,
                &["symbolic-ref", "--quiet", branch.as_str()],
                &[],
                &[],
            )
            .await?;
        match symbolic.code {
            Some(0) => return Err(GitBackendError::SymbolicTarget),
            Some(1) => {}
            code => {
                return Err(GitBackendError::Command {
                    operation: "target ref",
                    exit_code: code,
                });
            }
        }
        let output = self
            .command(target, &["worktree", "list", "--porcelain", "-z"], &[], &[])
            .await?
            .require_success("worktree inventory")?;
        let needle = format!("branch {}", branch.as_str());
        if output
            .split(|byte| *byte == 0)
            .any(|field| field == needle.as_bytes())
        {
            return Err(GitBackendError::TargetCheckedOut);
        }
        Ok(())
    }

    pub(super) async fn clone_private(
        &self,
        source: &Path,
        destination: &Path,
    ) -> Result<PathBuf, GitBackendError> {
        canonical_directory(source)?;
        fresh_private_directory(destination)?;
        let repository = destination.join("repository.git");
        self.text(
            destination,
            &[
                "clone",
                "--bare",
                "--no-local",
                "--no-hardlinks",
                "--no-tags",
                "--",
                path_text(source)?,
                path_text(&repository)?,
            ],
            "private clone",
        )
        .await?;
        Ok(repository)
    }

    pub(super) async fn fetch_object(
        &self,
        destination: &Path,
        source: &Path,
        object: &GitObjectId,
    ) -> Result<(), GitBackendError> {
        self.text(
            destination,
            &[
                "fetch",
                "--no-tags",
                "--no-write-fetch-head",
                "--no-recurse-submodules",
                "--",
                path_text(source)?,
                object.as_str(),
            ],
            "import object",
        )
        .await?;
        self.resolve_commit(destination, object.as_str()).await?;
        Ok(())
    }
}
