use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    KnowledgeSourceRef, MemoryScope, invalid, validate_content, validate_id, validate_revision,
    validate_sources,
};
use crate::{DomainError, EmployeeId, ProjectId, TaskId, Timestamp};

/// A closed distinction between Task summary, personal memory and project knowledge.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DerivedMemoryKind {
    TaskSummary {
        task_id: TaskId,
    },
    EmployeeMemoryEntry {
        employee_id: EmployeeId,
        task_id: Option<TaskId>,
    },
    ProjectKnowledgeEntry {
        task_id: Option<TaskId>,
    },
}

impl DerivedMemoryKind {
    pub fn scope(&self) -> MemoryScope {
        match self {
            Self::EmployeeMemoryEntry { employee_id, .. } => MemoryScope::Personal {
                employee_id: *employee_id,
            },
            Self::TaskSummary { .. } | Self::ProjectKnowledgeEntry { .. } => MemoryScope::Project,
        }
    }

    pub fn task_id(&self) -> Option<TaskId> {
        match self {
            Self::TaskSummary { task_id } => Some(*task_id),
            Self::EmployeeMemoryEntry { task_id, .. } | Self::ProjectKnowledgeEntry { task_id } => {
                *task_id
            }
        }
    }
}

/// The exact canonical Event interval covered by one generation, never a freshness claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCoverage {
    pub first_event_sequence: u64,
    pub last_event_sequence: u64,
}

/// Immutable source-linked derived content. It has no accepted/authoritative flag.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DerivedMemoryEntry {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub revision: u64,
    pub subject: DerivedMemoryKind,
    pub markdown: String,
    pub source_refs: Vec<KnowledgeSourceRef>,
    pub content_hash: String,
    pub coverage: Option<SourceCoverage>,
    pub created_by_job_id: Uuid,
    pub created_at: Timestamp,
    pub withdrawn: bool,
}

impl DerivedMemoryEntry {
    pub fn validate(&self) -> Result<(), DomainError> {
        validate_id(self.id)?;
        validate_id(self.created_by_job_id)?;
        self.project_id.validate_v7("memory.project_id")?;
        validate_revision(self.revision)?;
        validate_content(&self.markdown, &self.content_hash)?;
        validate_sources(&self.source_refs)?;
        if self.source_refs.is_empty() {
            return Err(invalid("derived memory requires canonical evidence"));
        }
        if matches!(self.subject, DerivedMemoryKind::TaskSummary { .. }) && self.coverage.is_none()
        {
            return Err(invalid("Task summary requires canonical Event coverage"));
        }
        if let Some(task_id) = self.subject.task_id() {
            task_id.validate_v7("memory.task_id")?;
        }
        if let MemoryScope::Personal { employee_id } = self.subject.scope() {
            employee_id.validate_v7("memory.employee_id")?;
        }
        if self.coverage.is_some_and(|range| {
            range.first_event_sequence == 0
                || range.last_event_sequence < range.first_event_sequence
                || range.last_event_sequence > i64::MAX as u64
        }) {
            return Err(invalid("invalid canonical Event coverage"));
        }
        Ok(())
    }
}
