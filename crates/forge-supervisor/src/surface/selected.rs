//! New Task surfaces start from an immutable export, never from a second mutable source read.
use super::path_str;
use crate::{SupervisorError, git::GitBackend, podman::PreparedSource};
use forge_domain::git::GitInitialRevision;
use std::{fs, path::Path};
#[cfg(test)]
mod tests;

pub(super) async fn prepare_git(
    path: &Path,
    source: &PreparedSource,
) -> Result<Option<String>, SupervisorError> {
    let repository = path.join("repository.git");
    let worktree = path.join("worktree");
    let backend = GitBackend::default();
    backend
        .text(
            path,
            &[
                "init",
                "--bare",
                "--template=",
                &format!(
                    "--object-format={}",
                    crate::git::format_name(source.descriptor.object_format)
                ),
                path_str(&repository)?,
            ],
            "selected surface init",
        )
        .await
        .map_err(|_| SupervisorError::SurfacePreparationFailed)?;
    let base = match &source.descriptor.selected_revision {
        GitInitialRevision::Commit(commit) => {
            backend
                .text(
                    path,
                    &[
                        "--git-dir",
                        path_str(&repository)?,
                        "fetch",
                        "--no-tags",
                        "--no-write-fetch-head",
                        "--no-recurse-submodules",
                        "--",
                        path_str(&source.directory.join("source.bundle"))?,
                        commit.as_str(),
                    ],
                    "selected surface fetch",
                )
                .await
                .map_err(|_| SupervisorError::SurfacePreparationFailed)?;
            backend
                .text(
                    path,
                    &[
                        "--git-dir",
                        path_str(&repository)?,
                        "worktree",
                        "add",
                        "--detach",
                        path_str(&worktree)?,
                        commit.as_str(),
                    ],
                    "selected surface worktree",
                )
                .await
                .map_err(|_| SupervisorError::SurfacePreparationFailed)?;
            Some(commit.as_str().to_owned())
        }
        GitInitialRevision::Unborn { .. } => {
            backend
                .text(
                    path,
                    &[
                        "--git-dir",
                        path_str(&repository)?,
                        "worktree",
                        "add",
                        "--orphan",
                        "-b",
                        "forge-task",
                        path_str(&worktree)?,
                    ],
                    "unborn surface worktree",
                )
                .await
                .map_err(|_| SupervisorError::SurfacePreparationFailed)?;
            None
        }
    };
    fs::write(
        worktree.join(".git"),
        b"gitdir: ../repository.git/worktrees/worktree\n",
    )?;
    fs::write(
        repository.join("worktrees/worktree/gitdir"),
        b"../../../worktree/.git\n",
    )?;
    Ok(base)
}
