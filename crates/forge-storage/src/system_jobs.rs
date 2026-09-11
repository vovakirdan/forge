//! Durable semantic job admission shares Project locks and the physical Run ledger.

mod attempts;
mod management;
mod sources;
pub use attempts::SystemJobAttempt;

use crate::{PostgresStore, StorageError, StorageTransaction, u64_to_i64};
use forge_domain::{EmployeeId, ProjectId, TaskId, runtime::RuntimeBinding, system_job::*};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemJobSettings {
    pub revision: u64,
    pub policy: SystemJobPolicy,
    pub binding: RuntimeBinding,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemJobRecord {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub kind: SystemJobKind,
    pub source_task_id: Option<TaskId>,
    pub target_employee_id: Option<EmployeeId>,
    pub generation: u64,
    pub covered_sequence: u64,
    pub state: String,
    pub reason_code: Option<String>,
}

fn invalid() -> StorageError {
    StorageError::InvalidInput {
        reason: "SystemJob ownership, state or allowance is invalid".into(),
    }
}
fn decode<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, StorageError> {
    serde_json::from_value(v).map_err(|source| StorageError::Snapshot {
        aggregate: "system_job",
        source,
    })
}
fn job(row: sqlx::postgres::PgRow) -> Result<SystemJobRecord, StorageError> {
    Ok(SystemJobRecord {
        id: row.try_get("id")?,
        project_id: ProjectId::from(row.try_get::<Uuid, _>("project_id")?),
        kind: decode(Value::String(row.try_get("kind")?))?,
        source_task_id: row
            .try_get::<Option<Uuid>, _>("source_task_id")?
            .map(TaskId::from),
        target_employee_id: row
            .try_get::<Option<Uuid>, _>("target_employee_id")?
            .map(EmployeeId::from),
        generation: u64::try_from(row.try_get::<i64, _>("generation")?).map_err(|_| invalid())?,
        covered_sequence: u64::try_from(row.try_get::<i64, _>("covered_sequence")?)
            .map_err(|_| invalid())?,
        state: row.try_get("state")?,
        reason_code: row.try_get("reason_code")?,
    })
}

impl PostgresStore {
    pub async fn system_jobs(
        &self,
        project: ProjectId,
    ) -> Result<Vec<SystemJobRecord>, StorageError> {
        sqlx::query("SELECT * FROM system_jobs WHERE project_id=$1 ORDER BY id LIMIT 256")
            .bind(project.as_uuid())
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(job)
            .collect()
    }
    pub async fn system_job_reconciliation_ids(
        &self,
    ) -> Result<Vec<(ProjectId, Uuid)>, StorageError> {
        let rows:Vec<(Uuid,Uuid)>=sqlx::query_as("SELECT project_id,id FROM system_job_attempts WHERE state='running' ORDER BY created_at,id LIMIT 256")
            .fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|(p, id)| (ProjectId::from(p), id))
            .collect())
    }
    pub async fn system_job_status(&self, project: ProjectId) -> Result<Value, StorageError> {
        let mut tx = self.begin().await?;
        let settings = tx.system_job_settings(project).await?;
        let usage:Value=sqlx::query_scalar("SELECT jsonb_build_object('attempts_last_24h',count(*),'running',count(*) FILTER(WHERE state='running'),'usage_status','see_per_run_accounting_unknown_is_not_zero') FROM system_job_attempts WHERE project_id=$1 AND created_at>clock_timestamp()-interval '24 hours'")
            .bind(project.as_uuid()).fetch_one(&mut *tx.transaction).await?;
        let onboarding:Value=sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('employee_id',employee_id,'state',state,'revision',revision,'job_id',job_id,'receipt',receipt)),'[]'::jsonb) FROM employee_onboarding WHERE project_id=$1")
            .bind(project.as_uuid()).fetch_one(&mut *tx.transaction).await?;
        tx.commit().await?;
        Ok(
            serde_json::json!({"policy":settings.as_ref().map(|s|&s.policy),"settings_revision":settings.map(|s|s.revision),"jobs":self.system_jobs(project).await?,"usage":usage,"onboarding":onboarding}),
        )
    }
}

impl StorageTransaction<'_> {
    pub async fn system_job_settings(
        &mut self,
        project: ProjectId,
    ) -> Result<Option<SystemJobSettings>, StorageError> {
        let value:Option<Value>=sqlx::query_scalar("SELECT jsonb_build_object('revision',revision,'policy',policy,'binding',binding) FROM project_system_job_settings WHERE project_id=$1")
            .bind(project.as_uuid()).fetch_optional(&mut *self.transaction).await?;
        let settings: Option<SystemJobSettings> = value.map(decode).transpose()?;
        if let Some(s) = &settings {
            s.policy.validate().map_err(|_| invalid())?;
            s.binding.validate().map_err(|_| invalid())?;
            if !matches!(s.binding.surface, forge_domain::runtime::SurfaceSpec::None)
                || s.binding.execution_profile.project_id() != project
            {
                return Err(invalid());
            }
        }
        Ok(settings)
    }
    /// The caller owns the Project command lock and writes the matching receipt/event.
    pub async fn save_system_job_settings(
        &mut self,
        project: ProjectId,
        settings: &SystemJobSettings,
    ) -> Result<(), StorageError> {
        settings.policy.validate().map_err(|_| invalid())?;
        settings.binding.validate().map_err(|_| invalid())?;
        if settings.revision == 0
            || settings.binding.execution_profile.project_id() != project
            || !matches!(
                settings.binding.surface,
                forge_domain::runtime::SurfaceSpec::None
            )
        {
            return Err(invalid());
        }
        let n=sqlx::query("INSERT INTO project_system_job_settings(project_id,revision,policy,binding) VALUES($1,$2,$3,$4) ON CONFLICT(project_id) DO UPDATE SET revision=EXCLUDED.revision,policy=EXCLUDED.policy,binding=EXCLUDED.binding,updated_at=clock_timestamp() WHERE project_system_job_settings.revision=EXCLUDED.revision-1")
            .bind(project.as_uuid()).bind(u64_to_i64(settings.revision,"system_job.settings_revision")?).bind(serde_json::to_value(&settings.policy).map_err(|_|invalid())?)
            .bind(serde_json::to_value(&settings.binding).map_err(|_|invalid())?).execute(&mut *self.transaction).await?.rows_affected();
        if n != 1 {
            return Err(invalid());
        }
        Ok(())
    }
    pub async fn lock_system_job(
        &mut self,
        project: ProjectId,
        id: Uuid,
    ) -> Result<Option<SystemJobRecord>, StorageError> {
        sqlx::query("SELECT * FROM system_jobs WHERE project_id=$1 AND id=$2 FOR UPDATE")
            .bind(project.as_uuid())
            .bind(id)
            .fetch_optional(&mut *self.transaction)
            .await?
            .map(job)
            .transpose()
    }
    pub async fn pending_system_job(
        &mut self,
        project: ProjectId,
    ) -> Result<Option<SystemJobRecord>, StorageError> {
        sqlx::query("SELECT j.* FROM system_jobs j WHERE j.project_id=$1 AND j.state='pending' AND j.eligible_at<=clock_timestamp() AND NOT EXISTS(SELECT 1 FROM system_job_attempts a JOIN runs r ON r.id=a.run_id JOIN leases l ON forge_lease_owns_run(l,r) WHERE a.job_id=j.id AND (a.state='running' OR l.lease_state='active' OR EXISTS(SELECT 1 FROM run_environment_reservations e WHERE e.run_id=r.id AND e.released_at IS NULL))) ORDER BY j.eligible_at,j.id LIMIT 1 FOR UPDATE OF j")
            .bind(project.as_uuid()).fetch_optional(&mut *self.transaction).await?.map(job).transpose()
    }
    /// Coalesces only larger canonical source watermarks. Active attempt snapshots stay immutable.
    pub async fn enqueue_task_summary(
        &mut self,
        project: ProjectId,
        task: TaskId,
        sequence: u64,
        delay: u32,
    ) -> Result<Uuid, StorageError> {
        if sequence == 0 || delay > 300 {
            return Err(invalid());
        }
        let id:Uuid=sqlx::query_scalar("INSERT INTO system_jobs(id,project_id,kind,source_task_id,state,input,covered_sequence,eligible_at) VALUES($1,$2,'summarization',$3,'pending','{}',$4,clock_timestamp()+make_interval(secs=>$5)) ON CONFLICT(project_id,source_task_id) WHERE kind='summarization' DO UPDATE SET covered_sequence=greatest(system_jobs.covered_sequence,EXCLUDED.covered_sequence), state=CASE WHEN EXCLUDED.covered_sequence<=system_jobs.covered_sequence OR system_jobs.state='running' THEN system_jobs.state ELSE 'pending' END, generation=CASE WHEN EXCLUDED.covered_sequence>system_jobs.covered_sequence AND system_jobs.state IN ('completed','held','cancelled') THEN system_jobs.generation+1 ELSE system_jobs.generation END, reason_code=CASE WHEN EXCLUDED.covered_sequence>system_jobs.covered_sequence THEN NULL ELSE system_jobs.reason_code END,eligible_at=CASE WHEN EXCLUDED.covered_sequence>system_jobs.covered_sequence THEN EXCLUDED.eligible_at ELSE system_jobs.eligible_at END,updated_at=clock_timestamp() RETURNING id")
            .bind(Uuid::now_v7()).bind(project.as_uuid()).bind(task.as_uuid()).bind(u64_to_i64(sequence,"system_job.coverage")?).bind(f64::from(delay))
            .fetch_one(&mut *self.transaction).await?;
        Ok(id)
    }
    pub async fn set_system_job_state(
        &mut self,
        id: Uuid,
        state: &str,
        reason: Option<&str>,
        next_generation: bool,
    ) -> Result<(), StorageError> {
        if !matches!(
            state,
            "pending" | "running" | "completed" | "held" | "cancelled"
        ) {
            return Err(invalid());
        }
        sqlx::query("UPDATE system_jobs SET state=$2,reason_code=$3,generation=generation+CASE WHEN $4 THEN 1 ELSE 0 END,updated_at=clock_timestamp(),eligible_at=clock_timestamp()+interval '5 seconds' WHERE id=$1")
            .bind(id).bind(state).bind(reason).bind(next_generation).execute(&mut *self.transaction).await?;
        Ok(())
    }
    pub async fn onboarding_allowed(
        &mut self,
        project: ProjectId,
        employee: EmployeeId,
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM employee_onboarding WHERE project_id=$1 AND employee_id=$2 AND state IN ('completed','skipped','legacy_bypass'))")
            .bind(project.as_uuid()).bind(employee.as_uuid()).fetch_one(&mut *self.transaction).await?)
    }
}
