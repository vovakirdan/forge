//! Immutable operator source registry. No filesystem or Git work runs inside a transaction.
use forge_domain::{ProjectId, ProjectRepository};
use uuid::Uuid;

use crate::{PostgresStore, StorageError, StorageTransaction, database_timestamp, encode_snapshot};

fn decode(value: &str) -> Result<ProjectRepository, StorageError> {
    let repository: ProjectRepository =
        serde_json::from_str(value).map_err(|source| StorageError::Snapshot {
            aggregate: "project_repository",
            source,
        })?;
    repository
        .validate_snapshot()
        .map_err(|_| StorageError::InvalidInput {
            reason: "invalid stored project repository".into(),
        })?;
    Ok(repository)
}

impl StorageTransaction<'_> {
    pub async fn load_project_repository(
        &mut self,
        id: Uuid,
    ) -> Result<Option<ProjectRepository>, StorageError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT canonical_snapshot::text FROM project_repositories WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *self.transaction)
        .await?;
        value.map(|value| decode(&value)).transpose()
    }

    pub async fn insert_project_repository(
        &mut self,
        repository: &ProjectRepository,
    ) -> Result<(), StorageError> {
        repository
            .validate_snapshot()
            .map_err(|_| StorageError::InvalidInput {
                reason: "invalid project repository".into(),
            })?;
        let result = sqlx::query("INSERT INTO project_repositories (id, project_id, name, canonical_snapshot, created_at) VALUES ($1,$2,$3,$4::jsonb,$5) ON CONFLICT DO NOTHING")
            .bind(repository.id).bind(repository.project_id.as_uuid()).bind(&repository.name)
            .bind(encode_snapshot(repository, "project_repository")?).bind(database_timestamp(repository.created_at))
            .execute(&mut *self.transaction).await?;
        if result.rows_affected() != 1 {
            return Err(StorageError::InvalidInput {
                reason: "duplicate project repository".into(),
            });
        }
        Ok(())
    }
}

impl PostgresStore {
    /// Lists registered sources for management; paths are not granted to worker shell.
    pub async fn list_project_repositories(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<ProjectRepository>, StorageError> {
        let values: Vec<String> = sqlx::query_scalar("SELECT canonical_snapshot::text FROM project_repositories WHERE project_id=$1 ORDER BY name,id")
            .bind(project_id.as_uuid()).fetch_all(&self.pool).await?;
        values.iter().map(|value| decode(value)).collect()
    }
}
