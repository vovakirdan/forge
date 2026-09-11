//! Canonical human-managed knowledge and evidence-backed, non-authoritative memory.

mod context;
mod derived;
mod page;
#[cfg(test)]
mod tests;

pub use context::{
    KnowledgeContextBundle, MAX_REQUIRED_KNOWLEDGE_PAGES, OPTIONAL_KNOWLEDGE_CONTEXT_BYTES,
    REQUIRED_KNOWLEDGE_CONTEXT_BYTES,
};
pub use derived::{DerivedMemoryEntry, DerivedMemoryKind, SourceCoverage};
pub use page::{KnowledgePage, KnowledgePageContent, KnowledgePageMutation};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Actor, ActorKind, ArtifactId, DomainError, EmployeeId, EventId};

/// Bounded canonical Markdown; large source bodies remain Artifacts.
pub const MAX_KNOWLEDGE_MARKDOWN_BYTES: usize = 65_536;
/// Maximum evidence references in one revision or generated entry.
pub const MAX_KNOWLEDGE_SOURCES: usize = 128;

/// Meaning of a human-managed page. Only published Policy/Decision is authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgePageKind {
    Introduction,
    Architecture,
    Guide,
    Policy,
    Decision,
}

/// Publication state of the current canonical revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgePageStatus {
    Draft,
    Published,
    Withdrawn,
}

/// Scope is authoritative in Forge; a search projection cannot expand it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "visibility", rename_all = "snake_case")]
pub enum MemoryScope {
    Project,
    Personal { employee_id: EmployeeId },
}

impl MemoryScope {
    /// Checks only the viewer portion of scope; the caller also checks Project.
    pub fn visible_to(self, employee_id: Option<EmployeeId>) -> bool {
        match self {
            Self::Project => true,
            Self::Personal { employee_id: owner } => employee_id == Some(owner),
        }
    }
}

/// Immutable canonical evidence references; raw logs and free-form URLs are not sources.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KnowledgeSourceRef {
    Artifact { artifact_id: ArtifactId },
    Event { event_id: EventId },
    TaskHandoff { handoff_id: Uuid },
    KnowledgePage { page_id: Uuid, revision: u64 },
}

impl KnowledgeSourceRef {
    pub fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::Artifact { artifact_id } => artifact_id.validate_v7("knowledge.artifact_id"),
            Self::Event { event_id } => event_id.validate_v7("knowledge.event_id"),
            Self::TaskHandoff { handoff_id } => validate_id(*handoff_id),
            Self::KnowledgePage { page_id, revision } => {
                validate_id(*page_id)?;
                validate_revision(*revision)
            }
        }
    }
}

pub(super) fn invalid(reason: impl Into<String>) -> DomainError {
    DomainError::InvalidValue {
        field: "knowledge",
        reason: reason.into(),
    }
}

pub(super) fn validate_id(id: Uuid) -> Result<(), DomainError> {
    if id.get_version_num() != 7 {
        return Err(invalid("identity must be UUIDv7"));
    }
    Ok(())
}

pub(super) fn validate_revision(revision: u64) -> Result<(), DomainError> {
    if revision == 0 || revision > i64::MAX as u64 {
        return Err(invalid("revision must be a positive database integer"));
    }
    Ok(())
}

pub(super) fn validate_content(markdown: &str, hash: &str) -> Result<(), DomainError> {
    if markdown.trim().is_empty() || markdown.len() > MAX_KNOWLEDGE_MARKDOWN_BYTES {
        return Err(invalid("Markdown must be nonblank and at most 65536 bytes"));
    }
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid("content hash must be lowercase SHA-256"));
    }
    Ok(())
}

pub(super) fn validate_sources(sources: &[KnowledgeSourceRef]) -> Result<(), DomainError> {
    if sources.len() > MAX_KNOWLEDGE_SOURCES {
        return Err(invalid("at most 128 source references are allowed"));
    }
    for (index, source) in sources.iter().enumerate() {
        source.validate()?;
        if sources[..index].contains(source) {
            return Err(invalid("duplicate source reference"));
        }
    }
    Ok(())
}

pub(super) fn require_authority(actor: Actor) -> Result<(), DomainError> {
    actor.validate_snapshot()?;
    if !matches!(actor.kind(), ActorKind::Human | ActorKind::SystemManager) {
        return Err(invalid(
            "only a human or authorized Manager may author or publish knowledge",
        ));
    }
    Ok(())
}
