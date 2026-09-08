//! Read-only projections used by Core-owned local transports.

use forge_domain::{PipelineVersion, ProjectId, TaskId};
use uuid::Uuid;

use crate::store::stored_artifact_from_row;
use crate::{
    PostgresStore, RunProjection, StorageError, StoredArtifact, decode_snapshot,
    run_projection_from_row,
};

const RUN_COLUMNS: &str = "id, project_id, purpose, communication_assignment_id, resolution_assignment_id, hook_invocation_id, task_id, queue_entry_id, lease_id, employee_id, stage_id, attempt_number, lease_fencing_token, environment_epoch, last_sequence, desired_state, observed_state, run_spec_version, run_spec::text AS run_spec, context_manifest::text AS context_manifest, observed_details::text AS observed_details";

impl PostgresStore {
    /// Lists all durable Project identities for recovery and stop reconciliation.
    pub async fn list_project_ids(&self) -> Result<Vec<ProjectId>, StorageError> {
        let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM projects ORDER BY id ASC")
            .fetch_all(&self.pool)
            .await?;
        Ok(ids.into_iter().map(ProjectId::from).collect())
    }

    /// Lists Projects whose durable execution gate allows scheduler reconciliation.
    pub async fn list_execution_open_project_ids(&self) -> Result<Vec<ProjectId>, StorageError> {
        let ids: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM projects WHERE execution_enabled ORDER BY id ASC")
                .fetch_all(&self.pool)
                .await?;
        Ok(ids.into_iter().map(ProjectId::from).collect())
    }

    /// Lists immutable Pipeline versions owned by one Project in stable order.
    pub async fn list_pipeline_versions(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<PipelineVersion>, StorageError> {
        let values: Vec<String> = sqlx::query_scalar(
            "SELECT pipeline_versions.definition::text FROM pipeline_versions JOIN pipelines ON pipelines.id = pipeline_versions.pipeline_id WHERE pipelines.project_id = $1 ORDER BY pipelines.name ASC, pipeline_versions.version ASC",
        )
        .bind(project_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        values
            .into_iter()
            .map(|value| decode_snapshot(&value, "pipeline_version"))
            .collect()
    }

    /// Loads one current Run projection without granting any mutation authority.
    pub async fn load_run(&self, id: Uuid) -> Result<Option<RunProjection>, StorageError> {
        let query = format!("SELECT {RUN_COLUMNS} FROM runs WHERE id = $1");
        let row = sqlx::query(&query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(run_projection_from_row).transpose()
    }

    /// Lists current Run projections for one Project in deterministic creation order.
    pub async fn list_runs_for_project(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<RunProjection>, StorageError> {
        let query = format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE project_id = $1 ORDER BY created_at DESC, id DESC"
        );
        let rows = sqlx::query(&query)
            .bind(project_id.as_uuid())
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(run_projection_from_row).collect()
    }

    /// Lists immutable Artifact evidence directly attached to one Task.
    pub async fn list_artifacts_for_task(
        &self,
        task_id: TaskId,
    ) -> Result<Vec<StoredArtifact>, StorageError> {
        let rows = sqlx::query(
            "SELECT canonical_snapshot::text AS canonical_snapshot, task_id, run_id, stage_id, producer_type, producer_id, producer::text AS producer FROM artifacts WHERE task_id = $1 ORDER BY created_at ASC, id ASC",
        )
        .bind(task_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(stored_artifact_from_row).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::RUN_COLUMNS;

    #[test]
    fn run_projection_query_keeps_the_fencing_and_state_columns() {
        assert!(RUN_COLUMNS.contains("lease_fencing_token"));
        assert!(RUN_COLUMNS.contains("desired_state"));
    }
}
