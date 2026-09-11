use forge_domain::knowledge::MemoryScope;
use forge_storage::KnowledgeProjectionOperation;
use serde::{Deserialize, Serialize};

use super::{
    MemorySearchDocument,
    agentmemory::{AgentMemoryError, ProjectionRecord, ProjectionScope},
};
use crate::{CoreError, CoreService};

/// Physical projection outcomes, never canonical Task or publication status.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionDrainReport {
    pub configured: bool,
    pub indexed: u32,
    pub retired: u32,
    pub deferred: u32,
}

impl CoreService {
    /// A bounded cyclic batch; every failure remains retryable with the same ID.
    /// The dedicated worker must not share a tick with Run supervision.
    pub async fn drain_memory_projections(&self) -> Result<ProjectionDrainReport, CoreError> {
        let Some(client) = &self.agentmemory else {
            return Ok(ProjectionDrainReport::default());
        };
        let mut report = ProjectionDrainReport {
            configured: true,
            ..Default::default()
        };
        let Ok(mut cursor) = self.memory_projection_cursor.try_lock() else {
            return Ok(report);
        };
        let operations = self
            .store
            .pending_knowledge_projections_global(*cursor, 16)
            .await?;
        for operation in operations {
            let mut tx = self.store.begin().await?;
            if !tx.try_lock_knowledge_projection_delivery().await? {
                break;
            }
            *cursor = Some(operation.id);
            let Some(operation) = self
                .store
                .load_knowledge_projection(operation.project_id, operation.id)
                .await?
            else {
                continue;
            };
            if operation.index_ready || operation.retired {
                continue;
            }
            // Only the advisory lock spans external I/O. Project commands stay free.
            let before = self.projection_document(&mut tx, &operation).await?;
            let result = match &before {
                Some(document) => client.upsert(&record(&operation, document)).await,
                None => client.forget(operation.id).await,
            };
            if let Err(error) = result {
                tx.fail_knowledge_projection(operation.id, error_code(&error))
                    .await?;
                tx.commit().await?;
                report.deferred += 1;
                continue;
            }
            // Fence late acknowledgements against concurrent publication/withdrawal.
            tx.lock_project(operation.project_id)
                .await?
                .ok_or(CoreError::NotFound {
                    aggregate: "project",
                })?;
            let current = self
                .store
                .load_knowledge_projection(operation.project_id, operation.id)
                .await?;
            let still_current = match current {
                Some(current) => self.projection_document(&mut tx, &current).await?.is_some(),
                None => false,
            };
            if before.is_none() && !still_current {
                tx.retire_knowledge_projection(operation.id).await?;
                report.retired += 1;
            } else if before.is_some()
                && still_current
                && tx
                    .mark_knowledge_projection_ready(operation.id, &operation.content_hash)
                    .await?
            {
                report.indexed += 1;
            } else {
                // A superseded upsert may have landed. Next delivery forgets it;
                // never convert a stale response into ready canonical evidence.
                tx.fail_knowledge_projection(operation.id, "superseded_during_delivery")
                    .await?;
                report.deferred += 1;
            }
            tx.commit().await?;
        }
        Ok(report)
    }
}

fn record(
    operation: &KnowledgeProjectionOperation,
    document: &MemorySearchDocument,
) -> ProjectionRecord {
    let (created_at, updated_at) = match document {
        MemorySearchDocument::KnowledgePage(page) => (page.created_at, page.revised_at),
        MemorySearchDocument::DerivedMemory(entry) => (entry.created_at, entry.created_at),
    };
    ProjectionRecord {
        id: operation.id,
        scope: ProjectionScope {
            project_id: operation.project_id,
            employee_id: match document.scope() {
                MemoryScope::Project => None,
                MemoryScope::Personal { employee_id } => Some(employee_id),
            },
        },
        revision: operation.revision,
        title: document.title().to_owned(),
        content: document.markdown().to_owned(),
        created_at: created_at.as_offset_date_time(),
        updated_at: updated_at.as_offset_date_time(),
    }
}

fn error_code(error: &AgentMemoryError) -> &'static str {
    match error {
        AgentMemoryError::InvalidConfiguration => "invalid_configuration",
        AgentMemoryError::InvalidInput => "invalid_input",
        AgentMemoryError::Timeout => "timeout",
        AgentMemoryError::Transport => "transport",
        AgentMemoryError::Rejected => "rejected",
        AgentMemoryError::ResponseTooLarge => "response_too_large",
        AgentMemoryError::InvalidResponse => "invalid_response",
        AgentMemoryError::IncompleteIndexing => "incomplete_indexing",
    }
}
