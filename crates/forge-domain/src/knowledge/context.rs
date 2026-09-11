//! Exact canonical knowledge supplied to one Employee attempt or explicit refresh.

use serde::{Deserialize, Serialize};

use crate::{DomainError, EmployeeId, ProjectId};

use super::{DerivedMemoryEntry, KnowledgePage, KnowledgePageStatus, invalid};

pub const REQUIRED_KNOWLEDGE_CONTEXT_BYTES: usize = 128 * 1024;
pub const OPTIONAL_KNOWLEDGE_CONTEXT_BYTES: usize = 32 * 1024;
pub const MAX_REQUIRED_KNOWLEDGE_PAGES: usize = 128;

/// Rendered into the provider instruction and stored unchanged in its context manifest.
/// Only `required_pages` carry managed Policy/Decision authority; derived memory never does.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeContextBundle {
    pub schema_version: u16,
    pub project_id: ProjectId,
    pub employee_id: EmployeeId,
    pub required_pages: Vec<KnowledgePage>,
    pub optional_pages: Vec<KnowledgePage>,
    pub derived_memory: Vec<DerivedMemoryEntry>,
    /// Optional candidates excluded by byte budget, stale sources or scope checks.
    pub omitted_optional_candidates: u32,
}

impl KnowledgeContextBundle {
    pub fn is_empty(&self) -> bool {
        self.required_pages.is_empty()
            && self.optional_pages.is_empty()
            && self.derived_memory.is_empty()
            && self.omitted_optional_candidates == 0
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        self.project_id
            .validate_v7("knowledge_context.project_id")?;
        self.employee_id
            .validate_v7("knowledge_context.employee_id")?;
        if self.schema_version != 1 || self.required_pages.len() > MAX_REQUIRED_KNOWLEDGE_PAGES {
            return Err(invalid("unsupported or oversized knowledge context"));
        }
        for (required, pages) in [(true, &self.required_pages), (false, &self.optional_pages)] {
            for page in pages {
                page.validate()?;
                if page.project_id != self.project_id
                    || page.status != KnowledgePageStatus::Published
                    || page.is_authoritative() != required
                {
                    return Err(invalid(
                        "knowledge context page scope or authority mismatch",
                    ));
                }
            }
        }
        for entry in &self.derived_memory {
            entry.validate()?;
            if entry.project_id != self.project_id
                || entry.withdrawn
                || !entry.subject.scope().visible_to(Some(self.employee_id))
            {
                return Err(invalid(
                    "knowledge context memory is not visible to this Employee",
                ));
            }
        }
        let required_size = serde_json::to_vec(&self.required_pages)
            .map_err(|_| invalid("cannot encode knowledge context"))?
            .len();
        let optional_size = serde_json::to_vec(&(&self.optional_pages, &self.derived_memory))
            .map_err(|_| invalid("cannot encode knowledge context"))?
            .len();
        if required_size > REQUIRED_KNOWLEDGE_CONTEXT_BYTES
            || optional_size > OPTIONAL_KNOWLEDGE_CONTEXT_BYTES
        {
            return Err(invalid(
                "knowledge context exceeds its explicit byte budget",
            ));
        }
        Ok(())
    }
}
