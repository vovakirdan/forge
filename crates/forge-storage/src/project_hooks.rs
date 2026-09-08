//! Immutable operator-controlled command registry; no execution inside a transaction.
use crate::{PostgresStore, StorageError, StorageTransaction, database_timestamp, encode_snapshot};
use forge_domain::{ProjectHookVersion, ProjectId};
use uuid::Uuid;

fn decode(value: &str) -> Result<ProjectHookVersion, StorageError> {
    let hook: ProjectHookVersion =
        serde_json::from_str(value).map_err(|source| StorageError::Snapshot {
            aggregate: "project_hook_version",
            source,
        })?;
    hook.validate_snapshot()
        .map_err(|_| StorageError::InvalidInput {
            reason: "invalid stored project hook version".into(),
        })?;
    Ok(hook)
}
impl StorageTransaction<'_> {
    pub async fn load_project_hook_version(
        &mut self,
        project_id: ProjectId,
        id: Uuid,
    ) -> Result<Option<ProjectHookVersion>, StorageError> {
        let value:Option<String>=sqlx::query_scalar("SELECT canonical_snapshot::text FROM project_hook_versions WHERE project_id=$1 AND id=$2")
            .bind(project_id.as_uuid()).bind(id).fetch_optional(&mut *self.transaction).await?;
        value.as_deref().map(decode).transpose()
    }
    pub async fn insert_project_hook_version(
        &mut self,
        hook: &ProjectHookVersion,
    ) -> Result<(), StorageError> {
        hook.validate_snapshot()
            .map_err(|_| StorageError::InvalidInput {
                reason: "invalid project hook version".into(),
            })?;
        let result=sqlx::query("INSERT INTO project_hook_versions(id,project_id,name,canonical_snapshot,created_at) VALUES($1,$2,$3,$4::jsonb,$5) ON CONFLICT DO NOTHING")
            .bind(hook.id).bind(hook.project_id.as_uuid()).bind(&hook.name).bind(encode_snapshot(hook,"project_hook_version")?).bind(database_timestamp(hook.created_at))
            .execute(&mut *self.transaction).await?;
        if result.rows_affected() != 1 {
            return Err(StorageError::InvalidInput {
                reason: "duplicate project hook version".into(),
            });
        }
        Ok(())
    }
}
impl PostgresStore {
    pub async fn project_hook_versions_page(
        &self,
        project_id: ProjectId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<ProjectHookVersion>, StorageError> {
        if !(1..=101).contains(&limit) {
            return Err(StorageError::InvalidInput {
                reason: "invalid hook page bound".into(),
            });
        }
        let values:Vec<String>=sqlx::query_scalar("SELECT canonical_snapshot::text FROM project_hook_versions WHERE project_id=$1 AND ($2::uuid IS NULL OR id>$2) ORDER BY id LIMIT $3")
            .bind(project_id.as_uuid()).bind(after).bind(i64::from(limit)).fetch_all(&self.pool).await?;
        values.iter().map(|value| decode(value)).collect()
    }
    pub async fn list_project_hook_versions(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<ProjectHookVersion>, StorageError> {
        let values:Vec<String>=sqlx::query_scalar("SELECT canonical_snapshot::text FROM project_hook_versions WHERE project_id=$1 ORDER BY created_at,id LIMIT 1024")
            .bind(project_id.as_uuid()).fetch_all(&self.pool).await?;
        values.iter().map(|value| decode(value)).collect()
    }
}
