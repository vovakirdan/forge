//! Canonical, transaction-local context reads. No projection network calls hold locks.

use forge_domain::{
    EmployeeId, ProjectId,
    knowledge::{DerivedMemoryEntry, KnowledgePage, KnowledgeSourceRef},
};

use crate::{StorageError, StorageTransaction};

impl StorageTransaction<'_> {
    /// Caller holds the Project lock, shared by canonical knowledge mutations.
    /// One overflow sentinel ensures required authority is never silently truncated.
    pub async fn required_knowledge_context_pages(
        &mut self,
        project_id: ProjectId,
    ) -> Result<Vec<KnowledgePage>, StorageError> {
        let values: Vec<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM knowledge_pages WHERE project_id=$1 AND status='published' AND kind IN ('policy','decision') ORDER BY kind,id LIMIT 129")
            .bind(project_id.as_uuid()).fetch_all(&mut *self.transaction).await?;
        values
            .iter()
            .map(|value| super::decode_page(value))
            .collect()
    }

    pub async fn optional_knowledge_context_pages(
        &mut self,
        project_id: ProjectId,
    ) -> Result<Vec<KnowledgePage>, StorageError> {
        let values: Vec<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM knowledge_pages WHERE project_id=$1 AND status='published' AND kind NOT IN ('policy','decision') ORDER BY kind,id LIMIT 16")
            .bind(project_id.as_uuid()).fetch_all(&mut *self.transaction).await?;
        values
            .iter()
            .map(|value| super::decode_page(value))
            .collect()
    }

    pub async fn visible_knowledge_context_memory(
        &mut self,
        project_id: ProjectId,
        employee_id: EmployeeId,
    ) -> Result<Vec<DerivedMemoryEntry>, StorageError> {
        let values: Vec<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM derived_memory_entries WHERE project_id=$1 AND NOT withdrawn AND (employee_id IS NULL OR employee_id=$2) ORDER BY (employee_id IS NOT NULL) DESC,id DESC LIMIT 32")
            .bind(project_id.as_uuid()).bind(employee_id.as_uuid()).fetch_all(&mut *self.transaction).await?;
        values
            .iter()
            .map(|value| super::derived::decode_entry(value))
            .collect()
    }

    /// A historical source remains auditable but must not revive stale advice in a fresh prompt.
    pub async fn knowledge_context_sources_current(
        &mut self,
        project_id: ProjectId,
        entry: &DerivedMemoryEntry,
    ) -> Result<bool, StorageError> {
        match self
            .validate_knowledge_sources(project_id, entry.subject.scope(), &entry.source_refs)
            .await
        {
            Ok(()) => {}
            Err(StorageError::NotFound { .. }) => return Ok(false),
            Err(error) => return Err(error),
        }
        for source in &entry.source_refs {
            if let KnowledgeSourceRef::KnowledgePage { page_id, revision } = source {
                let current: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM knowledge_pages WHERE project_id=$1 AND id=$2 AND status='published' AND revision=$3)")
                    .bind(project_id.as_uuid()).bind(page_id).bind(crate::u64_to_i64(*revision,"context.source_revision")?).fetch_one(&mut *self.transaction).await?;
                if !current {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}
