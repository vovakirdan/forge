use serde::{Deserialize, Serialize};

use crate::{DomainError, PipelineId, PipelineVersionId, ProjectId, Timestamp};

/// Named Pipeline catalog entry. The graph itself lives only in immutable versions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Pipeline {
    id: PipelineId,
    project_id: ProjectId,
    name: String,
    default_version_id: PipelineVersionId,
    deleted_at: Option<Timestamp>,
}

impl Pipeline {
    /// Creates an active Pipeline catalog entry with its selected default version.
    pub fn new(
        id: PipelineId,
        project_id: ProjectId,
        name: impl Into<String>,
        default_version_id: PipelineVersionId,
    ) -> Result<Self, DomainError> {
        let name = name.into();
        if name.trim().is_empty() || name.chars().count() > 128 {
            return Err(DomainError::InvalidPipeline {
                reason: "pipeline name must be non-blank and at most 128 characters".to_owned(),
            });
        }
        Ok(Self {
            id,
            project_id,
            name,
            default_version_id,
            deleted_at: None,
        })
    }

    /// Returns the catalog identity.
    #[must_use]
    pub const fn id(&self) -> PipelineId {
        self.id
    }

    /// Returns Project ownership.
    #[must_use]
    pub const fn project_id(&self) -> ProjectId {
        self.project_id
    }

    /// Returns the human-visible name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the version selected for new Tasks.
    #[must_use]
    pub const fn default_version_id(&self) -> PipelineVersionId {
        self.default_version_id
    }

    /// Returns whether new Tasks may select this Pipeline.
    #[must_use]
    pub const fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }

    /// Returns when the catalog entry was soft-deleted, if ever.
    #[must_use]
    pub const fn deleted_at(&self) -> Option<Timestamp> {
        self.deleted_at
    }

    /// Switches the default only; it never mutates a historical version.
    pub fn set_default_version(&mut self, version_id: PipelineVersionId) {
        self.default_version_id = version_id;
    }

    /// Soft-deletes the catalog entry while preserving existing Task bindings.
    pub fn soft_delete(&mut self, deleted_at: Timestamp) {
        self.deleted_at = Some(deleted_at);
    }

    /// Revalidates a deserialized catalog snapshot before storage exposes it.
    ///
    /// # Errors
    ///
    /// Returns a domain error if the persisted catalog entry cannot be
    /// reconstructed through its canonical constructor.
    pub fn validate_snapshot(&self) -> Result<(), DomainError> {
        self.id.validate_v7("pipeline.id")?;
        self.project_id.validate_v7("pipeline.project_id")?;
        self.default_version_id
            .validate_v7("pipeline.default_version_id")?;
        let mut restored = Self::new(
            self.id,
            self.project_id,
            self.name.clone(),
            self.default_version_id,
        )?;
        if let Some(deleted_at) = self.deleted_at {
            restored.soft_delete(deleted_at);
        }
        if restored == *self {
            Ok(())
        } else {
            Err(DomainError::InvalidPipeline {
                reason: "pipeline snapshot is not canonical".to_owned(),
            })
        }
    }
}
