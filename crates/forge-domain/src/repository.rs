//! Operator-selected local sources. Registration grants no direct worker access.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    Actor, DomainError, ProjectId, Timestamp,
    git::{GitBranchRef, GitInitialRevision, LocalGitPath},
};

/// Immutable Project source allowlist entry; changing a source requires a new entry.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRepository {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub name: String,
    pub source: LocalGitPath,
    pub target_ref: GitBranchRef,
    pub created_by: Actor,
    pub created_at: Timestamp,
}

impl ProjectRepository {
    /// Structural validation only: the execution boundary must verify the actual source.
    pub fn validate_snapshot(&self) -> Result<(), DomainError> {
        validate_id(self.id, "project_repository.id")?;
        self.project_id
            .validate_v7("project_repository.project_id")?;
        self.created_by.validate_snapshot()?;
        if self.name.trim().is_empty()
            || self.name.len() > 128
            || self.name.chars().any(char::is_control)
        {
            return Err(invalid(
                "project_repository.name",
                "must be nonblank, bounded and printable",
            ));
        }
        Ok(())
    }
}

/// Task-owned immutable source and surface identity, independent of Employee selection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskGitBinding {
    pub repository_id: Uuid,
    pub source: LocalGitPath,
    pub target_ref: GitBranchRef,
    /// Full operator-selected commit identity, not a claim of source verification.
    pub initial_base: GitInitialRevision,
    pub surface_id: Uuid,
}

impl TaskGitBinding {
    pub fn from_repository(
        repository: &ProjectRepository,
        initial_base: impl Into<GitInitialRevision>,
        surface_id: Uuid,
    ) -> Result<Self, DomainError> {
        repository.validate_snapshot()?;
        let binding = Self {
            repository_id: repository.id,
            source: repository.source.clone(),
            target_ref: repository.target_ref.clone(),
            initial_base: initial_base.into(),
            surface_id,
        };
        binding.validate_snapshot()?;
        Ok(binding)
    }

    pub fn validate_snapshot(&self) -> Result<(), DomainError> {
        validate_id(self.repository_id, "task.git.repository_id")?;
        validate_id(self.surface_id, "task.git.surface_id")
    }

    /// Rejects a stored Task whose source differs from its immutable allowlist entry.
    #[must_use]
    pub fn matches_repository(&self, repository: &ProjectRepository) -> bool {
        self.repository_id == repository.id
            && self.source == repository.source
            && self.target_ref == repository.target_ref
    }
}

fn validate_id(id: Uuid, field: &'static str) -> Result<(), DomainError> {
    if id.get_version_num() != 7 {
        return Err(invalid(field, "must be UUIDv7"));
    }
    Ok(())
}

fn invalid(field: &'static str, reason: &str) -> DomainError {
    DomainError::InvalidValue {
        field,
        reason: reason.into(),
    }
}
