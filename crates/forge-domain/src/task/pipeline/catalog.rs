use serde::{Deserialize, Serialize};

use crate::{DomainError, PipelineId, PipelineVersion, PipelineVersionId, ProjectId, Timestamp};

/// Named Pipeline catalog entry. The graph itself lives only in immutable versions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Pipeline {
    id: PipelineId,
    project_id: ProjectId,
    name: String,
    default_version_id: PipelineVersionId,
    deleted_at: Option<Timestamp>,
    #[serde(default = "initial_revision")]
    revision: u64,
    #[serde(default = "initial_version")]
    latest_version: u32,
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
            revision: 1,
            latest_version: 1,
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

    /// Monotonic catalog revision, separate from immutable graph version numbers.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Highest published version, even when an older graph is the default.
    #[must_use]
    pub const fn latest_version(&self) -> u32 {
        self.latest_version
    }

    /// Reserves the next graph number without mutating this catalog.
    pub fn next_version(&self) -> Result<u32, DomainError> {
        self.ensure_active()?;
        self.latest_version
            .checked_add(1)
            .ok_or_else(|| invalid("pipeline version exhausted"))
    }

    /// Records an already validated new immutable graph and optionally selects it.
    pub fn publish_version(
        &mut self,
        version: &PipelineVersion,
        make_default: bool,
    ) -> Result<(), DomainError> {
        self.ensure_version_scope(version)?;
        if version.version() != self.next_version()? {
            return Err(invalid("publication must use the next pipeline version"));
        }
        self.advance()?;
        self.latest_version = version.version();
        if make_default {
            self.default_version_id = version.id();
        }
        Ok(())
    }

    /// Switches the default only; it never mutates a historical version.
    pub fn set_default_version(&mut self, version: &PipelineVersion) -> Result<(), DomainError> {
        self.ensure_active()?;
        self.ensure_version_scope(version)?;
        if version.version() > self.latest_version {
            return Err(invalid("default version has not been published"));
        }
        self.advance()?;
        self.default_version_id = version.id();
        Ok(())
    }

    /// Soft-deletes the catalog entry while preserving existing Task bindings.
    pub fn soft_delete(&mut self, deleted_at: Timestamp) -> Result<(), DomainError> {
        self.ensure_active()?;
        self.advance()?;
        self.deleted_at = Some(deleted_at);
        Ok(())
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
        if self.revision == 0 || self.latest_version == 0 {
            return Err(invalid(
                "pipeline revision and latest version must be positive",
            ));
        }
        restored.deleted_at = self.deleted_at;
        restored.revision = self.revision;
        restored.latest_version = self.latest_version;
        if restored == *self {
            Ok(())
        } else {
            Err(DomainError::InvalidPipeline {
                reason: "pipeline snapshot is not canonical".to_owned(),
            })
        }
    }

    fn ensure_active(&self) -> Result<(), DomainError> {
        if self.is_deleted() {
            Err(invalid("deleted pipeline cannot be changed"))
        } else {
            Ok(())
        }
    }

    fn ensure_version_scope(&self, version: &PipelineVersion) -> Result<(), DomainError> {
        if version.pipeline_id() != self.id || version.project_id() != self.project_id {
            Err(invalid("version does not belong to the pipeline"))
        } else {
            Ok(())
        }
    }

    fn advance(&mut self) -> Result<(), DomainError> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("pipeline revision exhausted"))?;
        Ok(())
    }
}

const fn initial_revision() -> u64 {
    1
}
const fn initial_version() -> u32 {
    1
}
fn invalid(reason: &str) -> DomainError {
    DomainError::InvalidPipeline {
        reason: reason.into(),
    }
}
