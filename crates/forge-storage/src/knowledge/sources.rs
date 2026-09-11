use forge_domain::{
    ProjectId,
    knowledge::{KnowledgeSourceRef, MemoryScope, SourceCoverage},
};
use uuid::Uuid;

use crate::{StorageError, StorageTransaction, u64_to_i64};

impl StorageTransaction<'_> {
    /// Validates canonical existence and the source's visibility. Job input allowlists are checked by Core.
    pub async fn validate_knowledge_sources(
        &mut self,
        project_id: ProjectId,
        scope: MemoryScope,
        sources: &[KnowledgeSourceRef],
    ) -> Result<(), StorageError> {
        for source in sources {
            source
                .validate()
                .map_err(|source| StorageError::SnapshotInvariant {
                    aggregate: "knowledge_source",
                    source,
                })?;
            let exists: bool = match source {
                KnowledgeSourceRef::Artifact { artifact_id } => sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM artifacts WHERE project_id=$1 AND id=$2)")
                    .bind(project_id.as_uuid()).bind(artifact_id.as_uuid()).fetch_one(&mut *self.transaction).await?,
                KnowledgeSourceRef::Event { event_id } => {
                    let employee: Option<Uuid> = match scope { MemoryScope::Personal { employee_id } => Some(employee_id.as_uuid()), MemoryScope::Project => None };
                    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM event_log WHERE project_id=$1 AND id=$2 AND (COALESCE(payload->'scope'->>'visibility','project')='project' OR (payload->'scope'->>'visibility'='personal' AND payload->'scope'->>'employee_id'=$3::text)))")
                        .bind(project_id.as_uuid()).bind(event_id.as_uuid()).bind(employee).fetch_one(&mut *self.transaction).await?
                },
                KnowledgeSourceRef::TaskHandoff { handoff_id } => sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM task_handoffs WHERE project_id=$1 AND id=$2)")
                    .bind(project_id.as_uuid()).bind(handoff_id).fetch_one(&mut *self.transaction).await?,
                KnowledgeSourceRef::KnowledgePage { page_id, revision } => sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM knowledge_page_revisions r JOIN knowledge_pages p ON p.id=r.page_id WHERE r.project_id=$1 AND r.page_id=$2 AND r.revision=$3 AND p.status='published')")
                    .bind(project_id.as_uuid()).bind(page_id).bind(u64_to_i64(*revision,"knowledge.source_revision")?).fetch_one(&mut *self.transaction).await?,
            };
            if !exists {
                return Err(StorageError::NotFound {
                    aggregate: "knowledge_source",
                });
            }
        }
        Ok(())
    }

    /// Coverage must name actual canonical events, not an output-provided future sequence.
    pub async fn validate_memory_coverage(
        &mut self,
        project_id: ProjectId,
        coverage: SourceCoverage,
    ) -> Result<(), StorageError> {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM event_log WHERE project_id=$1 AND project_sequence=$2) AND EXISTS(SELECT 1 FROM event_log WHERE project_id=$1 AND project_sequence=$3)")
            .bind(project_id.as_uuid()).bind(u64_to_i64(coverage.first_event_sequence,"memory.coverage.first")?)
            .bind(u64_to_i64(coverage.last_event_sequence,"memory.coverage.last")?).fetch_one(&mut *self.transaction).await?;
        if !exists {
            return Err(StorageError::NotFound {
                aggregate: "memory_source_coverage",
            });
        }
        Ok(())
    }
}
