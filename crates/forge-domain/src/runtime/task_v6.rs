//! V6 requires source delivery and explicit file inputs; V2 remains a legacy contract.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{RuntimeBinding, RuntimeSpecError, SurfaceAccess, SurfaceSpec};
use crate::{
    ProjectId,
    git::{GitInitialRevision, GitSourceRequest},
};

pub const TASK_RUN_SPEC_VERSION: u16 = 6;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRunSpecV6 {
    pub schema_version: u16,
    pub project_id: ProjectId,
    pub surface_id: Uuid,
    pub binding: RuntimeBinding,
    pub instruction: String,
    pub source_request: Option<GitSourceRequest>,
    #[serde(default)]
    pub file_inputs: Vec<crate::file_snapshot::TaskFileInput>,
}

impl TaskRunSpecV6 {
    pub fn validate(&self) -> Result<(), RuntimeSpecError> {
        if self.schema_version != TASK_RUN_SPEC_VERSION
            || self.project_id.validate_v7("run.project_id").is_err()
            || self.surface_id.get_version_num() != 7
            || self.binding.execution_profile.project_id() != self.project_id
            || self.instruction.trim().is_empty()
            || self.instruction.len() > 512 * 1024
            || self.file_inputs.len() > crate::file_snapshot::MAX_TASK_FILE_INPUTS
        {
            return Err(RuntimeSpecError::InvalidProfile);
        }
        self.binding.validate()?;
        let mut input_ids = std::collections::BTreeSet::new();
        for input in &self.file_inputs {
            input
                .validate(self.project_id)
                .map_err(|_| RuntimeSpecError::InvalidProfile)?;
            if !input_ids.insert(input.artifact_id) {
                return Err(RuntimeSpecError::InvalidProfile);
            }
        }
        let is_writer = self.binding.access == SurfaceAccess::ReadWrite
            && matches!(
                self.binding.surface,
                SurfaceSpec::GitWorktree { .. } | SurfaceSpec::GitUnborn { .. }
            );
        if is_writer != self.source_request.is_some() {
            return Err(RuntimeSpecError::InvalidSurface);
        }
        if let Some(request) = &self.source_request {
            request
                .validate()
                .map_err(|_| RuntimeSpecError::InvalidSurface)?;
            let matches = match (&self.binding.surface, &request.initial_revision) {
                (
                    SurfaceSpec::GitWorktree {
                        repository,
                        base_ref,
                    },
                    GitInitialRevision::Commit(commit),
                ) => {
                    repository == request.source.as_path().to_string_lossy().as_ref()
                        && base_ref == commit.as_str()
                }
                (
                    SurfaceSpec::GitUnborn {
                        repository,
                        object_format,
                    },
                    GitInitialRevision::Unborn {
                        object_format: initial,
                    },
                ) => {
                    repository == request.source.as_path().to_string_lossy().as_ref()
                        && object_format == initial
                }
                _ => false,
            };
            if !matches {
                return Err(RuntimeSpecError::InvalidSurface);
            }
        }
        Ok(())
    }
}
