//! Mechanical persistence for immutable file inputs and quiescent capture requests.

use forge_domain::{
    ArtifactId, ProjectId, TaskId,
    file_snapshot::{FileSnapshotOperation, FileSnapshotSource, FileSnapshotState, TaskFileInput},
};
use uuid::Uuid;

use crate::{PostgresStore, StorageError, StorageTransaction, database_timestamp, encode_snapshot};

fn decode(value: &str) -> Result<FileSnapshotOperation, StorageError> {
    serde_json::from_str(value).map_err(|source| StorageError::Snapshot {
        aggregate: "file_snapshot",
        source,
    })
}

impl PostgresStore {
    pub async fn pending_file_snapshots(&self) -> Result<Vec<FileSnapshotOperation>, StorageError> {
        self.pending_file_snapshots_after(None).await
    }

    /// One bounded circular page, so incomplete older captures cannot starve imports.
    pub async fn pending_file_snapshots_after(
        &self,
        after: Option<Uuid>,
    ) -> Result<Vec<FileSnapshotOperation>, StorageError> {
        let values: Vec<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM task_file_snapshots WHERE state='pending' ORDER BY CASE WHEN $1::uuid IS NULL OR id>$1 THEN 0 ELSE 1 END,id LIMIT 32")
            .bind(after)
            .fetch_all(&self.pool).await?;
        values.iter().map(|value| decode(value)).collect()
    }

    pub async fn list_file_snapshots(
        &self,
        project: ProjectId,
        task: TaskId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<FileSnapshotOperation>, StorageError> {
        let values: Vec<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM task_file_snapshots WHERE project_id=$1 AND task_id=$2 AND ($3::uuid IS NULL OR id>$3) ORDER BY id LIMIT $4")
            .bind(project.as_uuid()).bind(task.as_uuid()).bind(after).bind(i64::from(limit.clamp(1,101))).fetch_all(&self.pool).await?;
        values.iter().map(|value| decode(value)).collect()
    }
}

impl StorageTransaction<'_> {
    pub async fn latest_file_capture_writer(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<Option<crate::RunProjection>, StorageError> {
        let id: Option<Uuid> = sqlx::query_scalar("SELECT r.id FROM runs r WHERE r.project_id=$1 AND r.task_id=$2 AND r.run_spec->'binding'->>'access'='read_write' ORDER BY r.lease_fencing_token DESC LIMIT 1")
            .bind(project.as_uuid()).bind(task.as_uuid()).fetch_optional(&mut *self.transaction).await?;
        match id {
            Some(id) => self.load_run(id).await,
            None => Ok(None),
        }
    }
    pub async fn insert_file_snapshot(
        &mut self,
        op: &FileSnapshotOperation,
    ) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO task_file_snapshots(id,project_id,task_id,artifact_id,state,capture_surface,canonical_snapshot,created_at) VALUES($1,$2,$3,$4,'pending',$5,$6::jsonb,$7)")
            .bind(op.id).bind(op.project_id.as_uuid()).bind(op.task_id.as_uuid()).bind(op.artifact_id.as_uuid())
            .bind(matches!(op.source, FileSnapshotSource::TaskSurface { .. }))
            .bind(encode_snapshot(op, "file_snapshot")?).bind(database_timestamp(op.created_at))
            .execute(&mut *self.transaction).await?;
        Ok(())
    }

    pub async fn lock_file_snapshot(
        &mut self,
        id: Uuid,
    ) -> Result<Option<FileSnapshotOperation>, StorageError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT canonical_snapshot::text FROM task_file_snapshots WHERE id=$1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *self.transaction)
        .await?;
        value.as_deref().map(decode).transpose()
    }

    pub async fn finish_file_snapshot(
        &mut self,
        op: &FileSnapshotOperation,
    ) -> Result<(), StorageError> {
        let state = match op.state {
            FileSnapshotState::Sealed => "sealed",
            FileSnapshotState::Failed => "failed",
            FileSnapshotState::Pending => {
                return Err(StorageError::InvalidInput {
                    reason: "snapshot is not finished".into(),
                });
            }
        };
        let result = sqlx::query("UPDATE task_file_snapshots SET state=$2,canonical_snapshot=$3::jsonb WHERE id=$1 AND state='pending'")
            .bind(op.id).bind(state).bind(encode_snapshot(op, "file_snapshot")?).execute(&mut *self.transaction).await?;
        if result.rows_affected() != 1 {
            return Err(StorageError::StaleRevision {
                aggregate: "file_snapshot",
            });
        }
        Ok(())
    }

    /// Called while holding Project and Task locks, shared with dispatch admission.
    pub async fn file_capture_is_busy(&mut self, task: TaskId) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM run_environment_reservations WHERE task_id=$1 AND released_at IS NULL) OR EXISTS(SELECT 1 FROM leases WHERE task_id=$1 AND lease_state='active') OR EXISTS(SELECT 1 FROM task_file_snapshots WHERE task_id=$1 AND state='pending' AND capture_surface) OR EXISTS(SELECT 1 FROM git_integrations WHERE task_id=$1 AND state NOT IN ('completed','retired'))")
            .bind(task.as_uuid()).fetch_one(&mut *self.transaction).await?)
    }

    pub async fn file_input_from_artifact(
        &mut self,
        project: ProjectId,
        artifact: ArtifactId,
    ) -> Result<Option<TaskFileInput>, StorageError> {
        let value: Option<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM task_file_snapshots WHERE project_id=$1 AND artifact_id=$2 AND state='sealed'")
            .bind(project.as_uuid()).bind(artifact.as_uuid()).fetch_optional(&mut *self.transaction).await?;
        value
            .map(|value| {
                let op = decode(&value)?;
                let manifest = op.manifest.ok_or(StorageError::InvalidInput {
                    reason: "sealed snapshot has no manifest".into(),
                })?;
                Ok(TaskFileInput {
                    artifact_id: artifact,
                    manifest,
                })
            })
            .transpose()
    }

    pub async fn attach_file_input(
        &mut self,
        project: ProjectId,
        task: TaskId,
        input: &TaskFileInput,
    ) -> Result<(), StorageError> {
        input
            .validate(project)
            .map_err(|_| StorageError::InvalidInput {
                reason: "invalid snapshot input".into(),
            })?;
        sqlx::query("INSERT INTO task_file_inputs(project_id,task_id,artifact_id,canonical_snapshot) VALUES($1,$2,$3,$4::jsonb) ON CONFLICT(task_id,artifact_id) DO NOTHING")
            .bind(project.as_uuid()).bind(task.as_uuid()).bind(input.artifact_id.as_uuid()).bind(encode_snapshot(input, "file_input")?)
            .execute(&mut *self.transaction).await?;
        Ok(())
    }

    pub async fn task_file_inputs(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<Vec<TaskFileInput>, StorageError> {
        let values: Vec<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM task_file_inputs WHERE project_id=$1 AND task_id=$2 ORDER BY artifact_id")
            .bind(project.as_uuid()).bind(task.as_uuid()).fetch_all(&mut *self.transaction).await?;
        values
            .iter()
            .map(|value| {
                let input: TaskFileInput =
                    serde_json::from_str(value).map_err(|source| StorageError::Snapshot {
                        aggregate: "file_input",
                        source,
                    })?;
                input
                    .validate(project)
                    .map_err(|_| StorageError::InvalidInput {
                        reason: "invalid stored input".into(),
                    })?;
                Ok(input)
            })
            .collect()
    }
}
