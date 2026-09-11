use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    KnowledgePageKind, KnowledgePageStatus, KnowledgeSourceRef, invalid, require_authority,
    validate_content, validate_id, validate_revision, validate_sources,
};
use crate::{Actor, CommandId, DomainError, ProjectId, Timestamp};

/// Editorial content accepted by a named management command.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgePageContent {
    pub title: String,
    pub markdown: String,
    #[serde(default)]
    pub source_refs: Vec<KnowledgeSourceRef>,
}

/// Every state change creates a new immutable revision; Supersede replaces published content.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum KnowledgePageMutation {
    Author {
        page_id: Uuid,
        expected_page_revision: u64,
        kind: KnowledgePageKind,
        #[serde(flatten)]
        content: KnowledgePageContent,
    },
    Publish {
        page_id: Uuid,
        expected_page_revision: u64,
    },
    Supersede {
        page_id: Uuid,
        expected_page_revision: u64,
        #[serde(flatten)]
        content: KnowledgePageContent,
    },
    Withdraw {
        page_id: Uuid,
        expected_page_revision: u64,
    },
}

impl KnowledgePageMutation {
    /// Validates transport shape independently from current publication state.
    pub fn validate(&self) -> Result<(), DomainError> {
        validate_id(self.page_id())?;
        if self.expected_revision() > i64::MAX as u64
            || (!matches!(self, Self::Author { .. }) && self.expected_revision() == 0)
        {
            return Err(invalid("invalid expected page revision"));
        }
        if let Some(content) = self.content() {
            if content.title.trim().is_empty() || content.title.len() > 200 {
                return Err(invalid("title must be nonblank and at most 200 bytes"));
            }
            validate_content(&content.markdown, &"0".repeat(64))?;
            validate_sources(&content.source_refs)?;
        }
        Ok(())
    }

    pub fn page_id(&self) -> Uuid {
        match self {
            Self::Author { page_id, .. }
            | Self::Publish { page_id, .. }
            | Self::Supersede { page_id, .. }
            | Self::Withdraw { page_id, .. } => *page_id,
        }
    }

    pub fn expected_revision(&self) -> u64 {
        match self {
            Self::Author {
                expected_page_revision,
                ..
            }
            | Self::Publish {
                expected_page_revision,
                ..
            }
            | Self::Supersede {
                expected_page_revision,
                ..
            }
            | Self::Withdraw {
                expected_page_revision,
                ..
            } => *expected_page_revision,
        }
    }

    pub fn content(&self) -> Option<&KnowledgePageContent> {
        match self {
            Self::Author { content, .. } | Self::Supersede { content, .. } => Some(content),
            Self::Publish { .. } | Self::Withdraw { .. } => None,
        }
    }
}

/// Canonical current or historical page revision. Human command attribution is mandatory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgePage {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub revision: u64,
    pub kind: KnowledgePageKind,
    pub status: KnowledgePageStatus,
    pub content: KnowledgePageContent,
    pub content_hash: String,
    pub actor: Actor,
    pub command_id: CommandId,
    pub created_at: Timestamp,
    pub revised_at: Timestamp,
    pub supersedes_revision: Option<u64>,
}

impl KnowledgePage {
    /// Applies the pure revision/publication rule. Core supplies the computed content digest.
    pub fn apply(
        current: Option<&Self>,
        project_id: ProjectId,
        mutation: &KnowledgePageMutation,
        actor: Actor,
        command_id: CommandId,
        now: Timestamp,
        content_hash: String,
    ) -> Result<Self, DomainError> {
        mutation.validate()?;
        require_authority(actor)?;
        let actual = current.map_or(0, |page| page.revision);
        if actual != mutation.expected_revision() {
            return Err(invalid(format!(
                "stale page revision: expected {}, current {actual}",
                mutation.expected_revision()
            )));
        }
        if current
            .is_some_and(|page| page.project_id != project_id || page.id != mutation.page_id())
        {
            return Err(invalid("page does not belong to this Project and identity"));
        }
        if current.is_some_and(|page| now < page.revised_at) {
            return Err(DomainError::NonMonotonicTimestamp);
        }
        let (kind, status, content, supersedes_revision) = match (mutation, current) {
            (KnowledgePageMutation::Author { kind, content, .. }, None) => {
                (*kind, KnowledgePageStatus::Draft, content.clone(), None)
            }
            (KnowledgePageMutation::Author { kind, content, .. }, Some(page))
                if page.status == KnowledgePageStatus::Draft && page.kind == *kind =>
            {
                (*kind, KnowledgePageStatus::Draft, content.clone(), None)
            }
            (KnowledgePageMutation::Publish { .. }, Some(page))
                if page.status == KnowledgePageStatus::Draft =>
            {
                (
                    page.kind,
                    KnowledgePageStatus::Published,
                    page.content.clone(),
                    None,
                )
            }
            (KnowledgePageMutation::Supersede { content, .. }, Some(page))
                if page.status == KnowledgePageStatus::Published =>
            {
                (
                    page.kind,
                    KnowledgePageStatus::Published,
                    content.clone(),
                    Some(page.revision),
                )
            }
            (KnowledgePageMutation::Withdraw { .. }, Some(page))
                if page.status != KnowledgePageStatus::Withdrawn =>
            {
                (
                    page.kind,
                    KnowledgePageStatus::Withdrawn,
                    page.content.clone(),
                    page.supersedes_revision,
                )
            }
            _ => {
                return Err(invalid(
                    "operation is not allowed in the current publication state",
                ));
            }
        };
        let page = Self {
            id: mutation.page_id(),
            project_id,
            revision: actual.checked_add(1).ok_or(DomainError::RevisionOverflow)?,
            kind,
            status,
            content,
            content_hash,
            actor,
            command_id,
            created_at: current.map_or(now, |page| page.created_at),
            revised_at: now,
            supersedes_revision,
        };
        page.validate()?;
        Ok(page)
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        validate_id(self.id)?;
        self.project_id.validate_v7("knowledge.project_id")?;
        self.command_id.validate_v7("knowledge.command_id")?;
        require_authority(self.actor)?;
        validate_revision(self.revision)?;
        if self.content.title.trim().is_empty() || self.content.title.len() > 200 {
            return Err(invalid("title must be nonblank and at most 200 bytes"));
        }
        validate_content(&self.content.markdown, &self.content_hash)?;
        validate_sources(&self.content.source_refs)?;
        if self.revised_at < self.created_at
            || self
                .supersedes_revision
                .is_some_and(|revision| revision == 0 || revision >= self.revision)
        {
            return Err(invalid("invalid revision history"));
        }
        Ok(())
    }

    pub fn is_authoritative(&self) -> bool {
        self.status == KnowledgePageStatus::Published
            && matches!(
                self.kind,
                KnowledgePageKind::Policy | KnowledgePageKind::Decision
            )
    }
}
