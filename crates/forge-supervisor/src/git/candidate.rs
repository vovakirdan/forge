//! Candidates and snapshots are committed revisions, never an implicit git add.

use std::{
    fs,
    path::{Path, PathBuf},
};

use forge_domain::git::{GitCandidate, GitObjectId};

use super::{GitBackend, GitBackendError, repository::path_text};

impl GitBackend {
    /// Verifies a clean committed HEAD after Core positively released its writer.
    ///
    /// The boolean is an explicit caller precondition, not process observation by
    /// this adapter. Core must retain its Task/surface reservation during this call.
    pub async fn verify_candidate(
        &self,
        worktree: &Path,
        expected_head: &GitObjectId,
        writer_quiescent: bool,
    ) -> Result<GitCandidate, GitBackendError> {
        if !writer_quiescent {
            return Err(GitBackendError::WriterActive);
        }
        self.private_source(worktree).await?;
        let actual_worktree = self
            .text(
                worktree,
                &["rev-parse", "--show-toplevel"],
                "worktree scope",
            )
            .await?;
        if Path::new(actual_worktree.trim_end_matches('\n')) != worktree {
            return Err(GitBackendError::UnsafePath);
        }
        if self.resolve_commit(worktree, "HEAD").await? != *expected_head {
            return Err(GitBackendError::HeadChanged);
        }
        let files = self
            .command(worktree, &["ls-files", "-v", "-z"], &[], &[])
            .await?
            .require_success("index flags")?;
        // Sparse and assume-unchanged entries can hide workspace edits from status.
        if files.split(|byte| *byte == 0).any(|file| {
            file.first()
                .is_some_and(|flag| flag.is_ascii_lowercase() || *flag == b'S')
        }) {
            return Err(GitBackendError::DirtyCandidate);
        }
        let index = self
            .command(worktree, &["ls-files", "--stage", "-z"], &[], &[])
            .await?
            .require_success("index scope")?;
        // Nested status can load a submodule's separate executable configuration.
        if index
            .split(|byte| *byte == 0)
            .any(|entry| entry.starts_with(b"160000 "))
        {
            return Err(GitBackendError::SubmodulesUnsupported);
        }
        let status = self
            .command(
                worktree,
                &[
                    "status",
                    "--porcelain=v1",
                    "-z",
                    "--untracked-files=all",
                    "--ignore-submodules=none",
                ],
                &[],
                &[],
            )
            .await?
            .require_success("candidate status")?;
        if !status.is_empty() {
            return Err(GitBackendError::DirtyCandidate);
        }
        let tree = self.tree(worktree, expected_head).await?;
        if self.resolve_commit(worktree, "HEAD").await? != *expected_head {
            return Err(GitBackendError::HeadChanged);
        }
        Ok(GitCandidate {
            commit: expected_head.clone(),
            tree,
        })
    }

    /// Creates a new private checkout of exactly the pinned candidate commit.
    ///
    /// Ignored/untracked workspace files are not copied. The caller grants a
    /// read-only mount for review or an independent writable testing copy for QA.
    /// The source remains unchanged and an existing destination is never reused.
    pub async fn snapshot_candidate(
        &self,
        source: &Path,
        candidate: &GitCandidate,
        destination: &Path,
    ) -> Result<PathBuf, GitBackendError> {
        self.private_source(source).await?;
        let repository = self.clone_private(source, destination).await?;
        self.fetch_object(&repository, source, &candidate.commit)
            .await?;
        self.ensure_supported_candidate(&repository, candidate)
            .await?;
        let worktree = destination.join("worktree");
        self.text(
            &repository,
            &[
                "worktree",
                "add",
                "--detach",
                path_text(&worktree)?,
                candidate.commit.as_str(),
            ],
            "candidate checkout",
        )
        .await?;
        // The existing sandbox mount places the complete private root at /workspace.
        fs::write(
            worktree.join(".git"),
            b"gitdir: ../repository.git/worktrees/worktree\n",
        )?;
        fs::write(
            repository.join("worktrees/worktree/gitdir"),
            b"../../../worktree/.git\n",
        )?;
        Ok(worktree)
    }
}
