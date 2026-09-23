//! Durable wake scan and mechanically enforced immutable alarm identity.
use crate::{PostgresStore, StorageError, StorageTransaction, database_timestamp, encode_snapshot};
use forge_domain::{
    ProjectId, ResumeRejection, ScheduledResumeState, TaskResumeSchedule, Timestamp,
};
use uuid::Uuid;

impl PostgresStore {
    /// Returns a bounded Project-scoped retained alarm page in stable ID order.
    pub async fn task_resume_schedule_page(
        &self,
        project_id: ProjectId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<TaskResumeSchedule>, StorageError> {
        if !(1..=51).contains(&limit) {
            return Err(StorageError::InvalidInput {
                reason: "resume schedule page limit must be between 1 and 51".into(),
            });
        }
        let snapshots: Vec<String> = sqlx::query_scalar(
            "SELECT canonical_snapshot::text FROM task_resume_schedules \
             WHERE project_id=$1 AND ($2::uuid IS NULL OR id>$2) ORDER BY id LIMIT $3",
        )
        .bind(project_id.as_uuid())
        .bind(after)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        snapshots
            .into_iter()
            .map(|value| {
                let schedule: TaskResumeSchedule =
                    serde_json::from_str(&value).map_err(|source| StorageError::Snapshot {
                        aggregate: "resume_schedule",
                        source,
                    })?;
                validate(&schedule)?;
                if schedule.project_id != project_id {
                    return Err(invalid());
                }
                Ok(schedule)
            })
            .collect()
    }

    /// Bounded pending scan; callers acquire Project then alarm locks and recheck due time.
    pub async fn due_task_resume_schedules(
        &self,
        now: Timestamp,
    ) -> Result<Vec<(ProjectId, Uuid)>, StorageError> {
        let rows:Vec<(Uuid,Uuid)>=sqlx::query_as("SELECT project_id,id FROM task_resume_schedules WHERE schedule_state='pending' AND not_before<=$1 ORDER BY not_before,id LIMIT 64")
            .bind(database_timestamp(now)).fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|(project, id)| (ProjectId::from(project), id))
            .collect())
    }
}

impl StorageTransaction<'_> {
    pub async fn lock_task_resume_schedule(
        &mut self,
        id: Uuid,
    ) -> Result<Option<TaskResumeSchedule>, StorageError> {
        let value: Option<(String,time::OffsetDateTime)> = sqlx::query_as(
            "SELECT canonical_snapshot::text,not_before FROM task_resume_schedules WHERE id=$1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *self.transaction)
        .await?;
        value
            .map(|(value, not_before)| {
                let schedule: TaskResumeSchedule =
                    serde_json::from_str(&value).map_err(|source| StorageError::Snapshot {
                        aggregate: "resume_schedule",
                        source,
                    })?;
                validate(&schedule)?;
                // PostgreSQL stores microseconds; the immutable intent retains nanoseconds.
                if schedule
                    .not_before
                    .as_offset_date_time()
                    .unix_timestamp_nanos()
                    .div_euclid(1_000)
                    != not_before.unix_timestamp_nanos().div_euclid(1_000)
                {
                    return Err(invalid());
                }
                Ok(schedule)
            })
            .transpose()
    }

    pub async fn insert_task_resume_schedule(
        &mut self,
        schedule: &TaskResumeSchedule,
    ) -> Result<(), StorageError> {
        validate(schedule)?;
        if schedule.state != ScheduledResumeState::Pending {
            return Err(invalid());
        }
        sqlx::query("INSERT INTO task_resume_schedules(id,project_id,task_id,wait_condition_id,not_before,schedule_state,canonical_snapshot) VALUES($1,$2,$3,$4,$5,'pending',$6::jsonb)")
            .bind(schedule.id).bind(schedule.project_id.as_uuid()).bind(schedule.task_id.as_uuid()).bind(schedule.wait_condition_id.as_uuid())
            .bind(database_timestamp(schedule.not_before)).bind(encode_snapshot(schedule,"resume_schedule")?).execute(&mut *self.transaction).await?;
        Ok(())
    }

    pub async fn update_task_resume_schedule(
        &mut self,
        schedule: &TaskResumeSchedule,
    ) -> Result<(), StorageError> {
        validate(schedule)?;
        if schedule.state == ScheduledResumeState::Pending {
            return Err(invalid());
        }
        let changed=sqlx::query("UPDATE task_resume_schedules SET schedule_state=$2,canonical_snapshot=$3::jsonb WHERE id=$1 AND project_id=$4 AND schedule_state='pending'")
            .bind(schedule.id).bind(schedule.state.key()).bind(encode_snapshot(schedule,"resume_schedule")?).bind(schedule.project_id.as_uuid())
            .execute(&mut *self.transaction).await?;
        if changed.rows_affected() != 1 {
            return Err(StorageError::StaleRevision {
                aggregate: "resume schedule",
            });
        }
        Ok(())
    }

    /// The Project lock serializes these guards with stop, dependency and physical release.
    pub async fn scheduled_resume_gate(
        &mut self,
        schedule: &TaskResumeSchedule,
    ) -> Result<Option<ResumeRejection>, StorageError> {
        let (hold,dependency,retained):(bool,bool,bool)=sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM project_recovery_settings WHERE project_id=$1 AND hold), EXISTS(SELECT 1 FROM task_dependencies d JOIN tasks t ON t.id=d.blocker_task_id AND t.project_id=d.project_id WHERE d.project_id=$1 AND d.blocked_task_id=$2 AND t.lifecycle<>d.required_blocker_lifecycle), EXISTS(SELECT 1 FROM leases WHERE project_id=$1 AND task_id=$2 AND lease_state='active') OR EXISTS(SELECT 1 FROM run_environment_reservations WHERE project_id=$1 AND task_id=$2 AND released_at IS NULL) OR EXISTS(SELECT 1 FROM queue_entries WHERE project_id=$1 AND task_id=$2 AND queue_state='leased')")
            .bind(schedule.project_id.as_uuid()).bind(schedule.task_id.as_uuid()).fetch_one(&mut *self.transaction).await?;
        let integrating:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM git_integrations WHERE project_id=$1 AND task_id=$2 AND state NOT IN ('completed','retired'))")
            .bind(schedule.project_id.as_uuid()).bind(schedule.task_id.as_uuid()).fetch_one(&mut *self.transaction).await?;
        Ok(if hold {
            Some(ResumeRejection::RecoveryHold)
        } else if dependency {
            Some(ResumeRejection::DependencyUnsatisfied)
        } else if retained || integrating {
            Some(ResumeRejection::ExecutionRetained)
        } else {
            None
        })
    }
}

fn validate(schedule: &TaskResumeSchedule) -> Result<(), StorageError> {
    schedule.validate_snapshot().map_err(|_| invalid())
}
fn invalid() -> StorageError {
    StorageError::InvalidInput {
        reason: "invalid resume schedule".into(),
    }
}
