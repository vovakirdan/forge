//! Retained reports and revisioned triage; no Task creation decisions in SQL.
use crate::{PostgresStore, StorageError, StorageTransaction, encode_snapshot};
use forge_domain::{ProjectId, TaskId, finding::Finding};
use uuid::Uuid;

impl PostgresStore {
    pub async fn finding_page(
        &self,
        project: ProjectId,
        task: Option<TaskId>,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<Finding>, StorageError> {
        if !(1..=101).contains(&limit) {
            return Err(invalid());
        }
        let rows:Vec<String>=sqlx::query_scalar("SELECT canonical_snapshot::text FROM findings WHERE project_id=$1 AND ($2::uuid IS NULL OR source_task_id=$2) AND ($3::uuid IS NULL OR id>$3) ORDER BY id LIMIT $4")
            .bind(project.as_uuid()).bind(task.map(|id|id.as_uuid())).bind(after).bind(i64::from(limit)).fetch_all(&self.pool).await?;
        rows.into_iter().map(decode).collect()
    }
}
impl StorageTransaction<'_> {
    pub async fn lock_finding(&mut self, id: Uuid) -> Result<Option<Finding>, StorageError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT canonical_snapshot::text FROM findings WHERE id=$1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *self.transaction)
        .await?;
        value.map(decode).transpose()
    }
    pub async fn insert_finding(&mut self, finding: &Finding) -> Result<(), StorageError> {
        validate(finding)?;
        if finding.state.key() != "open" || finding.revision != 1 {
            return Err(invalid());
        }
        if let Some(scope) = finding.source_run {
            let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs WHERE id=$1 AND project_id=$2 AND task_id=$3 AND lease_fencing_token=$4 AND environment_epoch=$5 AND employee_id=$6)")
                .bind(scope.run_id).bind(finding.project_id.as_uuid()).bind(finding.source_task_id.as_uuid()).bind(i64::try_from(scope.fencing_token).map_err(|_|invalid())?).bind(i64::try_from(scope.environment_epoch).map_err(|_|invalid())?).bind(finding.reported_by.id().as_uuid()).fetch_one(&mut *self.transaction).await?;
            if !valid || finding.reported_by.kind() != forge_domain::ActorKind::Employee {
                return Err(invalid());
            }
        }
        sqlx::query("INSERT INTO findings(id,project_id,source_task_id,source_run_id,revision,state,canonical_snapshot) VALUES($1,$2,$3,$4,1,'open',$5::jsonb)")
            .bind(finding.id).bind(finding.project_id.as_uuid()).bind(finding.source_task_id.as_uuid()).bind(finding.source_run.map(|scope|scope.run_id)).bind(encode_snapshot(finding,"finding")?).execute(&mut *self.transaction).await?;
        Ok(())
    }
    pub async fn update_finding(
        &mut self,
        finding: &Finding,
        expected: u64,
    ) -> Result<(), StorageError> {
        validate(finding)?;
        let changed=sqlx::query("UPDATE findings SET revision=$2,state=$3,target_task_id=$4,canonical_snapshot=$5::jsonb WHERE id=$1 AND revision=$6")
            .bind(finding.id).bind(i64::try_from(finding.revision).map_err(|_|invalid())?).bind(finding.state.key()).bind(finding.state.target_task().map(|id|id.as_uuid())).bind(encode_snapshot(finding,"finding")?).bind(i64::try_from(expected).map_err(|_|invalid())?).execute(&mut *self.transaction).await?;
        if changed.rows_affected() != 1 {
            return Err(StorageError::StaleRevision {
                aggregate: "finding",
            });
        }
        Ok(())
    }
}
fn validate(value: &Finding) -> Result<(), StorageError> {
    value.validate_snapshot().map_err(|_| invalid())
}
fn decode(value: String) -> Result<Finding, StorageError> {
    let value: Finding = serde_json::from_str(&value).map_err(|source| StorageError::Snapshot {
        aggregate: "finding",
        source,
    })?;
    validate(&value)?;
    Ok(value)
}
fn invalid() -> StorageError {
    StorageError::InvalidInput {
        reason: "invalid Finding record or scope".into(),
    }
}
