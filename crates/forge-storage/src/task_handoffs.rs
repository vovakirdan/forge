//! Bounded Task-local handoff coordinates. Handoff bodies remain in Core storage.

use forge_domain::{ProjectId, TaskId};
use serde_json::Value;
use uuid::Uuid;

use crate::{PostgresStore, StorageError};

impl PostgresStore {
    pub async fn task_handoff_page(
        &self,
        project_id: ProjectId,
        task_id: TaskId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<Value>, StorageError> {
        if !(1..=21).contains(&limit) {
            return Err(StorageError::InvalidInput {
                reason: "invalid Task handoff page bound".into(),
            });
        }
        let rows: Vec<String> = sqlx::query_scalar(
            r#"SELECT jsonb_build_object(
                'id',id,'project_id',project_id,'task_id',task_id,
                'kind',kind,
                'source_kind',CASE WHEN run_id IS NOT NULL THEN 'run'
                    WHEN hook_invocation_id IS NOT NULL THEN 'hook'
                    WHEN integration_id IS NOT NULL THEN 'git_integration' ELSE 'command' END,
                'source_id',COALESCE(run_id,hook_invocation_id,integration_id,command_id),
                'created_at',created_at,'content_availability','unavailable'
            )::text FROM task_handoffs
            WHERE project_id=$1 AND task_id=$2 AND ($3::uuid IS NULL OR id>$3)
            ORDER BY id LIMIT $4"#,
        )
        .bind(project_id.as_uuid())
        .bind(task_id.as_uuid())
        .bind(after)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                serde_json::from_str(&row).map_err(|source| StorageError::Snapshot {
                    aggregate: "task_handoff_read",
                    source,
                })
            })
            .collect()
    }
}
