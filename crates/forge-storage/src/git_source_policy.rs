//! Source settings are append-only and do not mutate Task or previously issued RunSpec.

use forge_domain::{ProjectId, TaskId, git::TaskGitSourceSetting};
use sqlx::Row;

use crate::{
    PostgresStore, StorageError, StorageTransaction, encode_value, i64_to_u64, u64_to_i64,
};

fn decode(row: Option<sqlx::postgres::PgRow>) -> Result<TaskGitSourceSetting, StorageError> {
    let Some(row) = row else {
        return Ok(TaskGitSourceSetting::default());
    };
    let policy: serde_json::Value = row.try_get("policy")?;
    let setting = TaskGitSourceSetting {
        revision: i64_to_u64(row.try_get("revision")?, "source_policy.revision")?,
        policy: serde_json::from_value(policy).map_err(|source| StorageError::Snapshot {
            aggregate: "source_policy",
            source,
        })?,
    };
    setting.validate().map_err(|_| StorageError::InvalidInput {
        reason: "invalid source policy snapshot".into(),
    })?;
    Ok(setting)
}

impl StorageTransaction<'_> {
    pub async fn record_git_source_selection(
        &mut self,
        run_id: uuid::Uuid,
        descriptor: &forge_domain::git::GitSourceDescriptor,
    ) -> Result<(), StorageError> {
        let data = serde_json::to_value(descriptor).map_err(|source| StorageError::Snapshot {
            aggregate: "source_selection",
            source,
        })?;
        sqlx::query("INSERT INTO run_git_source_selections(run_id,descriptor) VALUES($1,$2::jsonb) ON CONFLICT DO NOTHING")
            .bind(run_id).bind(encode_value(&data,"source_selection")?).execute(&mut *self.transaction).await?;
        let current: serde_json::Value =
            sqlx::query_scalar("SELECT descriptor FROM run_git_source_selections WHERE run_id=$1")
                .bind(run_id)
                .fetch_one(&mut *self.transaction)
                .await?;
        if current != data {
            return Err(StorageError::InvalidInput {
                reason: "Run source selection cannot change".into(),
            });
        }
        Ok(())
    }
    pub async fn task_git_source_setting(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<TaskGitSourceSetting, StorageError> {
        decode(sqlx::query("SELECT revision,policy FROM task_git_source_policies WHERE project_id=$1 AND task_id=$2 ORDER BY revision DESC LIMIT 1")
            .bind(project.as_uuid()).bind(task.as_uuid()).fetch_optional(&mut *self.transaction).await?)
    }

    pub async fn insert_task_git_source_setting(
        &mut self,
        project: ProjectId,
        task: TaskId,
        setting: &TaskGitSourceSetting,
    ) -> Result<(), StorageError> {
        setting.validate().map_err(|_| StorageError::InvalidInput {
            reason: "invalid source policy revision".into(),
        })?;
        let policy =
            serde_json::to_value(&setting.policy).map_err(|source| StorageError::Snapshot {
                aggregate: "source_policy",
                source,
            })?;
        let changed = sqlx::query("INSERT INTO task_git_source_policies(project_id,task_id,revision,policy) SELECT $1,$2,$3,$4::jsonb WHERE EXISTS(SELECT 1 FROM tasks WHERE project_id=$1 AND id=$2 AND project_repository_id IS NOT NULL) AND $3=COALESCE((SELECT max(revision) FROM task_git_source_policies WHERE project_id=$1 AND task_id=$2),1)+1")
            .bind(project.as_uuid()).bind(task.as_uuid()).bind(u64_to_i64(setting.revision, "source_policy.revision")?)
            .bind(encode_value(&policy, "source_policy")?).execute(&mut *self.transaction).await?;
        if changed.rows_affected() != 1 {
            return Err(StorageError::InvalidInput {
                reason: "stale or out-of-scope Task Git source policy".into(),
            });
        }
        Ok(())
    }
}

impl PostgresStore {
    pub async fn task_git_source_setting(
        &self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<TaskGitSourceSetting, StorageError> {
        decode(sqlx::query("SELECT revision,policy FROM task_git_source_policies WHERE project_id=$1 AND task_id=$2 ORDER BY revision DESC LIMIT 1")
            .bind(project.as_uuid()).bind(task.as_uuid()).fetch_optional(&self.pool).await?)
    }
}
