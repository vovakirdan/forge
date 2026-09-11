use super::*;
use crate::{RunProjection, database_timestamp};
use forge_domain::{Timestamp, runtime::SystemJobRunSpec};

#[derive(Clone, Debug)]
pub struct SystemJobAttempt {
    pub id: Uuid,
    pub core_instance: Uuid,
    pub state: String,
    pub spec: SystemJobRunSpec,
    pub result: Option<Value>,
    pub physical_quiescent: bool,
    pub deadline_expired: bool,
}

impl StorageTransaction<'_> {
    pub async fn next_system_job_memory_revision(
        &mut self,
        job: Uuid,
        subject_key: &str,
    ) -> Result<(Uuid, u64), StorageError> {
        let (id,revision):(Uuid,i64)=sqlx::query_as("INSERT INTO system_job_memory_heads(job_id,subject_key,entry_id,revision) VALUES($1,$2,$3,1) ON CONFLICT(job_id,subject_key) DO UPDATE SET revision=system_job_memory_heads.revision+1 RETURNING entry_id,revision")
            .bind(job).bind(subject_key).bind(Uuid::now_v7()).fetch_one(&mut *self.transaction).await?;
        Ok((id, u64::try_from(revision).map_err(|_| invalid())?))
    }
    pub async fn system_job_attempt(
        &mut self,
        project: ProjectId,
        id: Uuid,
    ) -> Result<Option<SystemJobAttempt>, StorageError> {
        let row=sqlx::query("SELECT a.*, NOT EXISTS(SELECT 1 FROM run_environment_reservations e WHERE e.run_id=a.run_id AND e.released_at IS NULL) AND NOT EXISTS(SELECT 1 FROM runs r JOIN leases l ON forge_lease_owns_run(l,r) WHERE r.id=a.run_id AND l.lease_state='active') AS physical_quiescent, a.created_at+make_interval(secs=>(a.spec->'binding'->'limits'->>'wall_seconds')::double precision)<clock_timestamp() AS deadline_expired FROM system_job_attempts a WHERE a.project_id=$1 AND a.id=$2 FOR UPDATE OF a")
            .bind(project.as_uuid()).bind(id).fetch_optional(&mut *self.transaction).await?;
        row.map(|row| {
            Ok(SystemJobAttempt {
                id: row.try_get("id")?,
                core_instance: row.try_get("core_instance")?,
                state: row.try_get("state")?,
                spec: decode(row.try_get("spec")?)?,
                result: row.try_get("result")?,
                physical_quiescent: row.try_get("physical_quiescent")?,
                deadline_expired: row.try_get("deadline_expired")?,
            })
        })
        .transpose()
    }
    pub async fn system_job_run_authorized(
        &mut self,
        run: &RunProjection,
    ) -> Result<bool, StorageError> {
        let Some(owner) = run.assignment.system_job() else {
            return Ok(false);
        };
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM system_job_attempts a JOIN system_jobs j ON j.id=a.job_id JOIN project_system_job_settings s ON s.project_id=j.project_id WHERE a.id=$1 AND a.project_id=$2 AND a.run_id=$3 AND a.job_id=$4 AND a.generation=$5 AND j.generation=a.generation AND a.state='running' AND a.result IS NULL AND j.state='running' AND s.policy->>'enabled'='true' AND NOT EXISTS(SELECT 1 FROM project_recovery_settings rs WHERE rs.project_id=j.project_id AND rs.hold))")
            .bind(owner.attempt_id).bind(run.project_id.as_uuid()).bind(run.id).bind(owner.job_id).bind(u64_to_i64(owner.generation,"system_job.generation")?)
            .fetch_one(&mut *self.transaction).await?)
    }
    /// Checks both this semantic pool and the shared host/account physical pool.
    pub async fn system_job_allowance(
        &mut self,
        job: &SystemJobRecord,
        settings: &SystemJobSettings,
    ) -> Result<Option<&'static str>, StorageError> {
        if !settings.policy.enabled || !self.resolution_admission_open(job.project_id).await? {
            return Ok(Some("disabled"));
        }
        let (active,attempts,daily):(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM system_job_attempts a WHERE a.project_id=$1 AND (a.state='running' OR EXISTS(SELECT 1 FROM run_environment_reservations e WHERE e.run_id=a.run_id AND e.released_at IS NULL))), (SELECT count(*) FROM system_job_attempts WHERE job_id=$2 AND generation=$3), (SELECT count(*) FROM system_job_attempts WHERE project_id=$1 AND created_at>clock_timestamp()-interval '24 hours')")
            .bind(job.project_id.as_uuid()).bind(job.id).bind(u64_to_i64(job.generation,"system_job.generation")?).fetch_one(&mut *self.transaction).await?;
        if attempts >= i64::from(settings.policy.max_attempts_per_job) {
            return Ok(Some("attempt_limit"));
        }
        if daily >= i64::from(settings.policy.max_attempts_per_day) {
            return Ok(Some("daily_limit"));
        }
        if active >= i64::from(settings.policy.max_concurrent) {
            return Ok(Some("semantic_capacity"));
        }
        if !self
            .lock_run_admission(job.project_id, Some(&settings.binding.execution_profile))
            .await?
        {
            return Ok(Some("shared_capacity"));
        }
        Ok(None)
    }
    pub async fn create_system_job_run(
        &mut self,
        job: &SystemJobRecord,
        spec: &SystemJobRunSpec,
        instance: Uuid,
        lease_id: Uuid,
    ) -> Result<RunProjection, StorageError> {
        spec.validate().map_err(|_| invalid())?;
        if instance.get_version_num() != 7
            || lease_id.get_version_num() != 7
            || spec.assignment.job_id != job.id
            || spec.project_id != job.project_id
            || spec.assignment.kind != job.kind
            || spec.assignment.generation != job.generation
            || spec.input.covered_sequence != job.covered_sequence
            || spec.input.source_task_id != job.source_task_id
            || spec.input.target_employee_id != job.target_employee_id
            || job.state != "pending"
        {
            return Err(invalid());
        }
        let current = self
            .lock_system_job(job.project_id, job.id)
            .await?
            .ok_or_else(invalid)?;
        if current.state != "pending"
            || current.generation != job.generation
            || current.covered_sequence != job.covered_sequence
        {
            return Err(invalid());
        }
        let settings = self
            .system_job_settings(job.project_id)
            .await?
            .ok_or_else(invalid)?;
        let mut expected = settings.binding.clone();
        expected.limits.wall_seconds = expected
            .limits
            .wall_seconds
            .min(settings.policy.wall_seconds);
        expected.limits.stop_grace_seconds = expected
            .limits
            .stop_grace_seconds
            .min(expected.limits.wall_seconds);
        if spec.binding != expected || spec.max_result_bytes != settings.policy.max_result_bytes {
            return Err(invalid());
        }
        if spec.binding.system_prompt.len()
            + spec.binding.employee_prompt.len()
            + spec.instruction.len()
            + 4
            > settings.policy.max_input_bytes as usize
        {
            return Err(invalid());
        }
        spec.input
            .validate(job.kind, settings.policy.max_input_bytes)
            .map_err(|_| invalid())?;
        use sha2::Digest;
        let hash: String =
            sha2::Sha256::digest(serde_json::to_vec(&spec.input.context).map_err(|_| invalid())?)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
        if spec.input.source_digest != hash {
            return Err(invalid());
        }
        if self.system_job_allowance(job, &settings).await?.is_some() {
            return Err(invalid());
        }
        let value = serde_json::to_value(spec).map_err(|_| invalid())?;
        self.require_run_admission(job.project_id, 7, &value)
            .await?;
        sqlx::query("INSERT INTO system_job_attempts(id,project_id,job_id,generation,run_id,core_instance,spec) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(spec.assignment.attempt_id).bind(spec.project_id.as_uuid()).bind(job.id).bind(u64_to_i64(job.generation,"system_job.generation")?).bind(spec.run_id).bind(instance).bind(&value)
            .execute(&mut *self.transaction).await?;
        let expiry = Timestamp::from_offset_date_time(
            Timestamp::now_utc().as_offset_date_time()
                + time::Duration::seconds(i64::from(spec.binding.limits.wall_seconds) + 60),
        );
        let lease=sqlx::query("INSERT INTO leases(id,project_id,purpose,system_job_attempt_id,lease_scope,expires_at) VALUES($1,$2,'system_job',$3,$4,$5) RETURNING fencing_token,environment_epoch")
            .bind(lease_id).bind(spec.project_id.as_uuid()).bind(spec.assignment.attempt_id).bind(serde_json::json!({"assignment":spec.assignment})).bind(database_timestamp(expiry))
            .fetch_one(&mut *self.transaction).await?;
        sqlx::query("INSERT INTO runs(id,project_id,purpose,system_job_attempt_id,lease_id,attempt_number,lease_fencing_token,environment_epoch,run_spec_version,run_spec,context_manifest) VALUES($1,$2,'system_job',$3,$4,1,$5,$6,7,$7,$8)")
            .bind(spec.run_id).bind(spec.project_id.as_uuid()).bind(spec.assignment.attempt_id).bind(lease_id).bind(lease.try_get::<i64,_>("fencing_token")?).bind(lease.try_get::<i64,_>("environment_epoch")?)
            .bind(value).bind(serde_json::json!({"schema_version":7,"run_id":spec.run_id,"project_id":spec.project_id,"assignment":spec.assignment,"input":spec.input,"capability_grants":["system_job.read","system_job.submit_result"],"provider_instruction":spec.instruction}))
            .execute(&mut *self.transaction).await?;
        self.reserve_environment(spec.run_id, None).await?;
        self.set_system_job_state(job.id, "running", None, false)
            .await?;
        self.load_run(spec.run_id).await?.ok_or_else(invalid)
    }
    /// Result acceptance is one-shot. The caller validates the frozen source allowlist.
    pub async fn save_system_job_result(
        &mut self,
        attempt: Uuid,
        result: &Value,
        message: Uuid,
        hash: &str,
    ) -> Result<(), StorageError> {
        if message.get_version_num() != 7
            || hash.len() != 64
            || !hash.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err(invalid());
        }
        let n=sqlx::query("UPDATE system_job_attempts SET result=$2,result_message_id=$3,result_hash=$4 WHERE id=$1 AND state='running' AND result IS NULL")
            .bind(attempt).bind(result).bind(message).bind(hash).execute(&mut *self.transaction).await?.rows_affected();
        if n != 1 {
            return Err(invalid());
        }
        Ok(())
    }
    pub async fn retire_system_job_attempt(
        &mut self,
        id: Uuid,
        state: &str,
    ) -> Result<(), StorageError> {
        if !matches!(state, "completed" | "stopped" | "failed" | "superseded") {
            return Err(invalid());
        }
        let n=sqlx::query("UPDATE system_job_attempts a SET state=$2,completed_at=clock_timestamp() WHERE a.id=$1 AND a.state='running' AND NOT EXISTS(SELECT 1 FROM run_environment_reservations e WHERE e.run_id=a.run_id AND e.released_at IS NULL) AND NOT EXISTS(SELECT 1 FROM runs r JOIN leases l ON forge_lease_owns_run(l,r) WHERE r.id=a.run_id AND l.lease_state='active')")
            .bind(id).bind(state).execute(&mut *self.transaction).await?.rows_affected();
        if n != 1 {
            return Err(invalid());
        }
        Ok(())
    }
}
