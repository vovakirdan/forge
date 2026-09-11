//! Task-private workspaces. Preparation never writes to the source repository.

use forge_domain::runtime::{RuntimeLaunchSpec, SurfaceAccess, SurfaceSpec};
use forge_protocol::supervisor::v1::ProvisionRun;
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
use tokio::process::Command;

use crate::SupervisorError;
mod pinned;
mod selected;
#[cfg(test)]
pub(crate) use pinned::tests::fixture as pinned_fixture;

#[derive(Clone, Debug)]
pub(crate) struct PreparedSurface {
    pub mount: Option<PathBuf>,
    pub workdir: &'static str,
    pub read_only: bool,
}

#[derive(Deserialize, Serialize)]
struct SurfaceManifest {
    task_id: String,
    source: SurfaceSpec,
    base_commit: Option<String>,
}

/// Resolves only a host-owned Task surface; capture is separately fenced and quiescent.
pub(crate) fn capture_worktree(
    root: &Path,
    task_id: &str,
    surface_id: uuid::Uuid,
) -> Result<PathBuf, SupervisorError> {
    let surfaces = root.join("surfaces");
    let path = surfaces.join(surface_id.to_string());
    let manifests = root.join("surface-manifests");
    for directory in [root, &surfaces, &path, &manifests] {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::symlink_metadata(directory)?;
        if !metadata.is_dir()
            || metadata.uid() != nix::unistd::Uid::effective().as_raw()
            || metadata.mode() & 0o077 != 0
            || fs::canonicalize(directory)? != directory
        {
            return Err(SupervisorError::UnsafeSurface);
        }
    }
    let manifest = read_manifest(&manifests.join(format!("{surface_id}.json")))?;
    if manifest.task_id != task_id
        || !matches!(
            manifest.source,
            SurfaceSpec::FilesystemSandbox
                | SurfaceSpec::GitWorktree { .. }
                | SurfaceSpec::GitUnborn { .. }
        )
    {
        return Err(SupervisorError::UnsafeSurface);
    }
    Ok(path.join("worktree"))
}

/// Resolves a retained host-owned manifest without creating or repairing any path.
pub(crate) fn inspection_worktree(
    root: &Path,
    task_id: &str,
    source: &crate::journal::GitSourceScope,
) -> Result<PathBuf, SupervisorError> {
    use std::os::unix::fs::MetadataExt;
    let surfaces = root.join("surfaces");
    let path = surfaces.join(source.surface_id.to_string());
    let manifests = root.join("surface-manifests");
    for directory in [root, &surfaces, &path, &manifests] {
        let metadata = fs::symlink_metadata(directory)?;
        if !metadata.is_dir()
            || metadata.uid() != nix::unistd::Uid::effective().as_raw()
            || metadata.mode() & 0o077 != 0
            || fs::canonicalize(directory)? != directory
        {
            return Err(SupervisorError::UnsafeSurface);
        }
    }
    let manifest = read_manifest(&manifests.join(format!("{}.json", source.surface_id)))?;
    if manifest.task_id != task_id
        || manifest.source != source.source
        || match (&manifest.source, manifest.base_commit.as_deref()) {
            (SurfaceSpec::GitUnborn { .. }, None) => false,
            (_, Some(commit)) => forge_domain::git::GitObjectId::new(commit).is_err(),
            _ => true,
        }
    {
        return Err(SupervisorError::UnsafeSurface);
    }
    Ok(path.join("worktree"))
}

#[cfg(test)]
pub(crate) async fn prepare(
    root: &Path,
    provision: &ProvisionRun,
    spec: &RuntimeLaunchSpec,
) -> Result<PreparedSurface, SupervisorError> {
    prepare_selected(root, provision, spec, None).await
}

pub(crate) async fn prepare_selected(
    root: &Path,
    provision: &ProvisionRun,
    spec: &RuntimeLaunchSpec,
    source: Option<&crate::podman::PreparedSource>,
) -> Result<PreparedSurface, SupervisorError> {
    if matches!(
        spec.binding.surface,
        SurfaceSpec::GitCandidateSnapshot { .. } | SurfaceSpec::GitUnbornCandidateSnapshot { .. }
    ) {
        return pinned::prepare(root, provision, spec).await;
    }
    if spec.binding.surface == SurfaceSpec::None {
        return Ok(PreparedSurface {
            mount: None,
            workdir: "/workspace",
            read_only: false,
        });
    }
    let surfaces = root.join("surfaces");
    private_directory(&surfaces)?;
    let path = surfaces.join(spec.surface_id.to_string());
    // Metadata is host-owned. The entire workspace tree is deliberately writable
    // by its employee, so authoritative manifests cannot live inside that mount.
    let manifests = root.join("surface-manifests");
    private_directory(&manifests)?;
    let manifest_path = manifests.join(format!("{}.json", spec.surface_id));
    if path.exists() {
        private_directory(&path)?;
        let manifest = read_manifest(&manifest_path)?;
        if manifest.task_id != provision.task_id || manifest.source != spec.binding.surface {
            return Err(SupervisorError::UnsafeSurface);
        }
    } else {
        private_directory(&path)?;
        let base_commit = if let Some(source) = source {
            selected::prepare_git(&path, source).await?
        } else {
            match &spec.binding.surface {
                SurfaceSpec::GitWorktree {
                    repository,
                    base_ref,
                } => Some(prepare_git(&path, repository, base_ref).await?),
                SurfaceSpec::FilesystemSandbox => {
                    private_directory(&path.join("worktree"))?;
                    None
                }
                SurfaceSpec::None => None,
                SurfaceSpec::GitCandidateSnapshot { .. }
                | SurfaceSpec::GitUnbornCandidateSnapshot { .. }
                | SurfaceSpec::GitUnborn { .. } => return Err(SupervisorError::UnsafeSurface),
            }
        };
        let manifest = SurfaceManifest {
            task_id: provision.task_id.clone(),
            source: spec.binding.surface.clone(),
            base_commit,
        };
        private_write(&manifest_path, &serde_json::to_vec(&manifest)?)?;
    }
    let read_only = spec.binding.access == SurfaceAccess::ReadOnly;
    let mount = if read_only {
        let snapshots = root.join("snapshots");
        private_directory(&snapshots)?;
        let snapshot = snapshots.join(format!(
            "{}-{}",
            provision.run_id, provision.environment_epoch
        ));
        if snapshot.exists() {
            return Err(SupervisorError::UnsafeSurface);
        }
        copy_tree(&path, &snapshot)?;
        snapshot
    } else {
        path
    };
    Ok(PreparedSurface {
        mount: Some(mount),
        workdir: "/workspace/worktree",
        read_only,
    })
}

async fn prepare_git(path: &Path, source: &str, base_ref: &str) -> Result<String, SupervisorError> {
    let repository = path.join("repository.git");
    git(&[
        "clone",
        "--bare",
        "--no-local",
        "--no-hardlinks",
        "--",
        source,
        path_str(&repository)?,
    ])
    .await?;
    let revision = format!("{base_ref}^{{commit}}");
    let commit = git(&[
        "--git-dir",
        path_str(&repository)?,
        "rev-parse",
        "--verify",
        "--end-of-options",
        &revision,
    ])
    .await?;
    let commit = commit.trim().to_owned();
    if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(SupervisorError::UnsafeSurface);
    }
    let worktree = path.join("worktree");
    git(&[
        "--git-dir",
        path_str(&repository)?,
        "worktree",
        "add",
        "--detach",
        path_str(&worktree)?,
        &commit,
    ])
    .await?;
    // Git 2.43 writes absolute linked-worktree paths. These two metadata links
    // are made relative so the entire private repository survives the mount at
    // /workspace without sharing or rewriting the source repository's .git.
    fs::write(
        worktree.join(".git"),
        b"gitdir: ../repository.git/worktrees/worktree\n",
    )?;
    fs::write(
        repository.join("worktrees/worktree/gitdir"),
        b"../../../worktree/.git\n",
    )?;
    Ok(commit)
}

async fn git(args: &[&str]) -> Result<String, SupervisorError> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(300),
        Command::new("git")
            .args(["-c", "core.hooksPath=/dev/null"])
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_COUNT", "0")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| SupervisorError::SurfacePreparationFailed)??;
    if !output.status.success() {
        return Err(SupervisorError::SurfacePreparationFailed);
    }
    String::from_utf8(output.stdout).map_err(|_| SupervisorError::SurfacePreparationFailed)
}

pub(crate) fn private_directory(path: &Path) -> Result<(), SupervisorError> {
    use std::os::unix::fs::MetadataExt;
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != nix::unistd::Uid::effective().as_raw()
        || metadata.mode() & 0o077 != 0
        || fs::canonicalize(path)? != path
    {
        return Err(SupervisorError::UnsafeSurface);
    }
    Ok(())
}

fn read_manifest(path: &Path) -> Result<SurfaceManifest, SupervisorError> {
    use std::{io::Read, os::unix::fs::MetadataExt};
    const MAX_MANIFEST_BYTES: u64 = 16 * 1024;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != nix::unistd::Uid::effective().as_raw()
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
        || metadata.len() > MAX_MANIFEST_BYTES
    {
        return Err(SupervisorError::UnsafeSurface);
    }
    let mut bytes = Vec::new();
    file.take(MAX_MANIFEST_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(SupervisorError::UnsafeSurface);
    }
    Ok(serde_json::from_slice(&bytes)?)
}

pub(crate) fn private_write(path: &Path, bytes: &[u8]) -> Result<(), SupervisorError> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), SupervisorError> {
    private_directory(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = target.join(entry.file_name());
        if kind.is_symlink() {
            std::os::unix::fs::symlink(fs::read_link(entry.path())?, target)?;
        } else if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            return Err(SupervisorError::UnsafeSurface);
        }
    }
    Ok(())
}

fn path_str(path: &Path) -> Result<&str, SupervisorError> {
    path.to_str().ok_or(SupervisorError::UnsafeSurface)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("forge-surface-{}", crate::new_id()));
            private_directory(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn git_worktree_uses_only_task_private_metadata_after_relocation() {
        let fixture = Fixture::new();
        let source = fixture.0.join("source");
        private_directory(&source).unwrap();
        let status = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&source)
            .status()
            .await
            .unwrap();
        assert!(status.success());
        fs::write(source.join("example.txt"), b"source version\n").unwrap();
        assert!(
            Command::new("git")
                .args(["add", "example.txt"])
                .current_dir(&source)
                .status()
                .await
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .args([
                    "-c",
                    "user.name=Forge Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "-c",
                    "core.hooksPath=/dev/null",
                    "commit",
                    "--quiet",
                    "-m",
                    "fixture"
                ])
                .current_dir(&source)
                .status()
                .await
                .unwrap()
                .success()
        );
        let task = fixture.0.join("task");
        private_directory(&task).unwrap();
        prepare_git(&task, source.to_str().unwrap(), "HEAD")
            .await
            .unwrap();
        let relocated = fixture.0.join("relocated");
        fs::rename(&task, &relocated).unwrap();
        let status = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(relocated.join("worktree"))
            .output()
            .await
            .unwrap();
        assert!(
            status.status.success(),
            "{}",
            String::from_utf8_lossy(&status.stderr)
        );
        assert!(status.stdout.is_empty());
        fs::write(relocated.join("worktree/example.txt"), b"task change\n").unwrap();
        assert_eq!(
            fs::read(source.join("example.txt")).unwrap(),
            b"source version\n"
        );
        assert!(!source.join(".git/worktrees").exists());
    }

    #[test]
    fn read_only_snapshot_copies_bytes_without_dereferencing_external_symlinks() {
        let fixture = Fixture::new();
        let source = fixture.0.join("source");
        private_directory(&source).unwrap();
        fs::write(source.join("file"), b"before").unwrap();
        fs::write(fixture.0.join("private"), b"must not be copied").unwrap();
        std::os::unix::fs::symlink("../private", source.join("link")).unwrap();
        let snapshot = fixture.0.join("snapshot");
        copy_tree(&source, &snapshot).unwrap();
        fs::write(source.join("file"), b"after").unwrap();
        assert_eq!(fs::read(snapshot.join("file")).unwrap(), b"before");
        assert!(
            fs::symlink_metadata(snapshot.join("link"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn host_manifest_read_rejects_oversized_or_linked_input() {
        let fixture = Fixture::new();
        let oversized = fixture.0.join("oversized.json");
        private_write(&oversized, &[b' '; 16 * 1024 + 1]).unwrap();
        assert!(matches!(
            read_manifest(&oversized),
            Err(SupervisorError::UnsafeSurface)
        ));
        let link = fixture.0.join("linked.json");
        std::os::unix::fs::symlink(&oversized, &link).unwrap();
        assert!(read_manifest(&link).is_err());
    }
}
