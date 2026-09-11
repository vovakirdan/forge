//! Source transport reads the registered repository, never a previous worker clone.
use std::{fs, io::Read, path::Path};

pub(crate) use forge_domain::git::GitSourceDescriptor;
use forge_domain::git::{
    GitInitialRevision, GitObjectFormat, GitSourceRequest, TaskGitSourcePolicy,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    GitBackend, GitBackendError,
    repository::{canonical_directory, fresh_private_directory, path_text},
};

/// Frozen before bundle export; replay must not re-resolve a moving branch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct GitSourceSelection {
    pub request: GitSourceRequest,
    pub selected_revision: GitInitialRevision,
    pub object_format: GitObjectFormat,
    pub target_key: String,
    pub target_exists: bool,
}

pub(crate) fn format_name(format: GitObjectFormat) -> &'static str {
    match format {
        GitObjectFormat::Sha1 => "sha1",
        GitObjectFormat::Sha256 => "sha256",
    }
}

impl GitBackend {
    pub(crate) async fn source_key(
        &self,
        request: &GitSourceRequest,
    ) -> Result<String, GitBackendError> {
        self.target_key(request.source.as_path(), &request.target_ref)
            .await
    }

    pub(crate) async fn target_key(
        &self,
        source: &Path,
        target_ref: &forge_domain::git::GitBranchRef,
    ) -> Result<String, GitBackendError> {
        let common = self.common_directory(source).await?;
        Ok(format!("{}\n{}", path_text(&common)?, target_ref.as_str()))
    }

    pub(crate) async fn target_exists(
        &self,
        source: &Path,
        target_ref: &forge_domain::git::GitBranchRef,
    ) -> Result<bool, GitBackendError> {
        self.ensure_target_not_symbolic(source, target_ref).await?;
        Ok(self.read_ref(source, target_ref.as_str()).await?.is_some())
    }

    pub(crate) async fn select_source(
        &self,
        request: &GitSourceRequest,
        target_seen: bool,
    ) -> Result<GitSourceSelection, GitBackendError> {
        request.validate()?;
        let source = request.source.as_path();
        let target_key = self.source_key(request).await?;
        self.ensure_target_not_symbolic(source, &request.target_ref)
            .await?;
        let format = self
            .text(
                source,
                &["rev-parse", "--show-object-format"],
                "source object format",
            )
            .await?;
        let object_format = match format.trim() {
            "sha1" => GitObjectFormat::Sha1,
            "sha256" => GitObjectFormat::Sha256,
            _ => return Err(GitBackendError::InvalidIntent),
        };
        if object_format != request.initial_revision.object_format() {
            return Err(GitBackendError::InvalidIntent);
        }
        let target = self.read_ref(source, request.target_ref.as_str()).await?;
        if target.is_none()
            && (target_seen || matches!(request.initial_revision, GitInitialRevision::Commit(_)))
        {
            return Err(GitBackendError::TargetMissing);
        }
        let selected_revision = match &request.policy {
            TaskGitSourcePolicy::PinnedCommit { commit } => {
                GitInitialRevision::Commit(self.resolve_commit(source, commit.as_str()).await?)
            }
            TaskGitSourcePolicy::LatestTarget => match &target {
                Some(commit) => {
                    GitInitialRevision::Commit(self.resolve_commit(source, commit.as_str()).await?)
                }
                None => {
                    if !matches!(request.initial_revision, GitInitialRevision::Unborn { object_format: expected } if expected == object_format)
                    {
                        return Err(GitBackendError::InvalidIntent);
                    }
                    GitInitialRevision::Unborn { object_format }
                }
            },
        };
        Ok(GitSourceSelection {
            request: request.clone(),
            selected_revision,
            object_format,
            target_key,
            target_exists: target.is_some(),
        })
    }

    /// Export only the frozen selected commit. The temporary repository contains no host config.
    pub(crate) async fn export_source(
        &self,
        selection: &GitSourceSelection,
        destination: &Path,
    ) -> Result<GitSourceDescriptor, GitBackendError> {
        match fs::symlink_metadata(destination) {
            Ok(_) => return read_export(destination, selection),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let parent = destination.parent().ok_or(GitBackendError::UnsafePath)?;
        canonical_directory(parent)?;
        let temporary = parent.join(format!("export-{}", uuid::Uuid::now_v7()));
        fresh_private_directory(&temporary)?;
        let private = temporary.join("repository.git");
        self.text(
            &temporary,
            &[
                "init",
                "--bare",
                "--template=",
                &format!("--object-format={}", format_name(selection.object_format)),
                path_text(&private)?,
            ],
            "source export repository",
        )
        .await?;
        let public = temporary.join("public");
        fresh_private_directory(&public)?;
        let (bundle_file, bundle_sha256, bundle_bytes) = match &selection.selected_revision {
            GitInitialRevision::Commit(commit) => {
                self.fetch_object(&private, selection.request.source.as_path(), commit)
                    .await?;
                self.text(
                    &private,
                    &["update-ref", "refs/forge/source", commit.as_str()],
                    "source export ref",
                )
                .await?;
                let bundle = public.join("source.bundle");
                self.text(
                    &private,
                    &["bundle", "create", path_text(&bundle)?, "refs/forge/source"],
                    "source bundle",
                )
                .await?;
                let (digest, bytes) = bundle_digest(&bundle)?;
                (Some("source.bundle".into()), Some(digest), Some(bytes))
            }
            GitInitialRevision::Unborn { .. } => (None, None, None),
        };
        let descriptor = GitSourceDescriptor {
            schema_version: 1,
            repository_id: selection.request.repository_id,
            target_ref: selection.request.target_ref.clone(),
            policy_revision: selection.request.policy_revision,
            policy: selection.request.policy.clone(),
            selected_revision: selection.selected_revision.clone(),
            object_format: selection.object_format,
            bundle_file,
            bundle_sha256,
            bundle_bytes,
        };
        descriptor.validate_request(&selection.request)?;
        crate::surface::private_write(
            &public.join("descriptor.json"),
            &serde_json::to_vec(&descriptor).map_err(|_| GitBackendError::InvalidIntent)?,
        )
        .map_err(|_| GitBackendError::UnsafePath)?;
        fs::File::open(&public)?.sync_all()?;
        fs::rename(&public, destination)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(descriptor)
    }

    pub(super) async fn ensure_target_not_symbolic(
        &self,
        source: &Path,
        branch: &forge_domain::git::GitBranchRef,
    ) -> Result<(), GitBackendError> {
        let output = self
            .command(
                source,
                &["symbolic-ref", "--quiet", branch.as_str()],
                &[],
                &[],
            )
            .await?;
        match output.code {
            Some(1) => Ok(()),
            Some(0) => Err(GitBackendError::SymbolicTarget),
            code => Err(GitBackendError::Command {
                operation: "source ref",
                exit_code: code,
            }),
        }
    }
}

fn read_export(
    destination: &Path,
    selection: &GitSourceSelection,
) -> Result<GitSourceDescriptor, GitBackendError> {
    canonical_directory(destination)?;
    let descriptor: GitSourceDescriptor =
        crate::podman::read_private_json(&destination.join("descriptor.json"))
            .map_err(|_| GitBackendError::UnsafePath)?;
    descriptor.validate_request(&selection.request)?;
    if descriptor.schema_version != 1
        || descriptor.repository_id != selection.request.repository_id
        || descriptor.target_ref != selection.request.target_ref
        || descriptor.policy_revision != selection.request.policy_revision
        || descriptor.policy != selection.request.policy
        || descriptor.selected_revision != selection.selected_revision
        || descriptor.object_format != selection.object_format
    {
        return Err(GitBackendError::InvalidIntent);
    }
    match &descriptor.selected_revision {
        GitInitialRevision::Commit(_) => {
            let (digest, bytes) = bundle_digest(&destination.join("source.bundle"))?;
            if descriptor.bundle_file.as_deref() != Some("source.bundle")
                || descriptor.bundle_sha256.as_ref() != Some(&digest)
                || descriptor.bundle_bytes != Some(bytes)
            {
                return Err(GitBackendError::InvalidIntent);
            }
        }
        GitInitialRevision::Unborn { .. }
            if descriptor.bundle_file.is_none()
                && descriptor.bundle_sha256.is_none()
                && descriptor.bundle_bytes.is_none()
                && !destination.join("source.bundle").try_exists()? => {}
        _ => return Err(GitBackendError::InvalidIntent),
    }
    Ok(descriptor)
}

fn bundle_digest(path: &Path) -> Result<(String, u64), GitBackendError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != nix::unistd::Uid::effective().as_raw()
        || metadata.len() > 256 * 1024 * 1024
    {
        return Err(GitBackendError::UnsafePath);
    }
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > 256 * 1024 * 1024 {
            return Err(GitBackendError::OutputLimit);
        }
        hash.update(&buffer[..count]);
    }
    file.sync_all()?;
    Ok((format!("{:x}", hash.finalize()), bytes))
}
