//! Immutable selected-file inputs, independent of Git candidates and stage verdicts.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{ArtifactId, DomainError, ProjectId, TaskId};

mod operation;
pub use operation::{
    FileCaptureRequest, FileSnapshotOperation, FileSnapshotSource, FileSnapshotState,
};

/// Maximum number of explicitly selected files in one snapshot.
pub const MAX_SNAPSHOT_FILES: usize = 64;
/// Bound for one snapshot, including empty and binary files.
pub const MAX_SNAPSHOT_BYTES: u64 = 16 * 1024 * 1024;
/// Bound for the number of snapshots attached to future Runs of a Task.
pub const MAX_TASK_FILE_INPUTS: usize = 8;
/// Reserved structural Artifact kind; it never implies stage acceptance.
pub const FILE_SNAPSHOT_KIND: &str = "file_snapshot";

/// One immutable file body stored separately from the canonical manifest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotFile {
    pub path: String,
    pub size_bytes: u64,
    pub executable: bool,
    pub sha256: String,
    pub object_key: String,
}

/// Canonical, path-relative description of a selected-file Artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileSnapshotManifest {
    pub schema_version: u16,
    pub project_id: ProjectId,
    pub source_task_id: TaskId,
    pub files: Vec<SnapshotFile>,
}

/// Frozen input to a future Run. Materialization never overlays its WorkSurface.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskFileInput {
    pub artifact_id: ArtifactId,
    pub manifest: FileSnapshotManifest,
}

impl TaskFileInput {
    pub fn validate(&self, project_id: ProjectId) -> Result<(), DomainError> {
        self.artifact_id.validate_v7("file_input.artifact_id")?;
        if self.manifest.project_id != project_id {
            return Err(invalid("snapshot belongs to another Project"));
        }
        self.manifest.validate(self.artifact_id)
    }
}

impl FileSnapshotManifest {
    pub fn validate(&self, artifact_id: ArtifactId) -> Result<(), DomainError> {
        self.project_id.validate_v7("file_snapshot.project_id")?;
        self.source_task_id
            .validate_v7("file_snapshot.source_task_id")?;
        if self.schema_version != 1 {
            return Err(invalid("unsupported manifest version"));
        }
        validate_selected_paths(
            &self
                .files
                .iter()
                .map(|file| file.path.clone())
                .collect::<Vec<_>>(),
        )?;
        let mut total = 0_u64;
        for file in &self.files {
            if file.sha256.len() != 64
                || !file
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                || file.object_key
                    != snapshot_object_key(self.project_id, artifact_id, &file.sha256)
            {
                return Err(invalid("invalid digest or scoped object key"));
            }
            total = total
                .checked_add(file.size_bytes)
                .ok_or_else(|| invalid("snapshot too large"))?;
        }
        if total > MAX_SNAPSHOT_BYTES {
            return Err(invalid("snapshot exceeds byte limit"));
        }
        Ok(())
    }
}

/// Stable immutable object namespace; a caller cannot select another Project's key.
#[must_use]
pub fn snapshot_object_key(project: ProjectId, artifact: ArtifactId, digest: &str) -> String {
    format!("file-snapshots/{project}/{artifact}/{digest}")
}

/// Rejects ambiguous paths, directories, credential names and service metadata.
pub fn validate_selected_paths(paths: &[String]) -> Result<(), DomainError> {
    if paths.is_empty() || paths.len() > MAX_SNAPSHOT_FILES {
        return Err(invalid("select between 1 and 64 files"));
    }
    let mut unique = BTreeSet::new();
    for path in paths {
        validate_snapshot_path(path)?;
        if !unique.insert(path) {
            return Err(invalid("duplicate selected path"));
        }
    }
    for path in paths {
        if unique.iter().any(|other| {
            other.len() > path.len()
                && other.starts_with(path.as_str())
                && other.as_bytes()[path.len()] == b'/'
        }) {
            return Err(invalid("file paths overlap a directory prefix"));
        }
    }
    Ok(())
}

/// Validates a relative file name without accessing a filesystem.
pub fn validate_snapshot_path(path: &str) -> Result<(), DomainError> {
    if path.is_empty()
        || path.len() > 1024
        || path
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '\\' | '*' | '?' | '[' | ']'))
        || path.split('/').any(|component| {
            component.is_empty()
                || component == "."
                || component == ".."
                || forbidden_component(component)
        })
    {
        return Err(invalid("unsafe or reserved file path"));
    }
    Ok(())
}

/// Service/credential directories are denied even when selected as the import root.
#[must_use]
pub fn forbidden_component(component: &str) -> bool {
    let name = component.to_ascii_lowercase();
    matches!(
        name.as_str(),
        ".git"
            | ".forge"
            | ".forge-m2"
            | ".codex"
            | ".claude"
            | ".ssh"
            | ".aws"
            | ".gnupg"
            | ".config"
            | "auth.json"
            | "credentials.json"
            | "credentials"
            | "secrets"
            | "secrets.txt"
            | "tokens.json"
            | "id_rsa"
            | "id_ed25519"
            | "master-key"
    ) || name.starts_with(".forge-")
        || name == ".env"
        || name.starts_with(".env.")
        || name.ends_with(".pem")
        || name.ends_with(".p12")
        || name.ends_with(".key")
}

fn invalid(reason: &str) -> DomainError {
    DomainError::InvalidValue {
        field: "file_snapshot",
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_reject_traversal_credentials_globs_and_overlap() {
        for path in [
            "",
            "/etc/passwd",
            "a/../b",
            "a//b",
            "./x",
            ".git/config",
            "a/.env",
            "key.pem",
            "a*",
            "a\\b",
            "x\ny",
        ] {
            assert!(validate_snapshot_path(path).is_err(), "{path:?}");
        }
        assert!(validate_selected_paths(&["a".into(), "a/b".into()]).is_err());
        assert!(validate_selected_paths(&["a".into(), "a".into()]).is_err());
    }

    #[test]
    fn manifest_accepts_empty_file_but_rejects_cross_project_and_object_key() {
        let artifact_id = ArtifactId::new();
        let project_id = ProjectId::new();
        let sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let mut input = TaskFileInput {
            artifact_id,
            manifest: FileSnapshotManifest {
                schema_version: 1,
                project_id,
                source_task_id: TaskId::new(),
                files: vec![SnapshotFile {
                    path: "docs/empty.txt".into(),
                    size_bytes: 0,
                    executable: false,
                    sha256: sha256.into(),
                    object_key: snapshot_object_key(project_id, artifact_id, sha256),
                }],
            },
        };
        assert!(input.validate(project_id).is_ok());
        assert!(input.validate(ProjectId::new()).is_err());
        input.manifest.files[0].object_key = "other-project/object".into();
        assert!(input.validate(project_id).is_err());
    }
}
