use forge_domain::{EmployeeId, ProjectId, Timestamp};
use serde_json::Value;
use uuid::Uuid;

use crate::{StorageError, StorageTransaction};

/// Scoped immutable refresh, recorded with its Gateway receipt and audit Event.
pub struct KnowledgeContextRefreshRecord {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub run_id: Uuid,
    pub employee_id: EmployeeId,
    pub initial_context_snapshot_id: Uuid,
    pub message_id: Uuid,
    pub content_hash: String,
    pub context_snapshot: Value,
    pub created_at: Timestamp,
}

impl StorageTransaction<'_> {
    pub async fn insert_knowledge_context_refresh(
        &mut self,
        record: &KnowledgeContextRefreshRecord,
    ) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO run_knowledge_context_refreshes(id,project_id,run_id,employee_id,initial_context_snapshot_id,message_id,content_hash,context_snapshot,created_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)")
            .bind(record.id).bind(record.project_id.as_uuid()).bind(record.run_id).bind(record.employee_id.as_uuid())
            .bind(record.initial_context_snapshot_id).bind(record.message_id).bind(&record.content_hash).bind(&record.context_snapshot)
            .bind(record.created_at.as_offset_date_time()).execute(&mut *self.transaction).await?;
        Ok(())
    }
}
