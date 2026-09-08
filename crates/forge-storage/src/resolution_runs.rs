//! Resolver purposes share the canonical capacity and fenced physical ledger.
use crate::{
    PostgresStore, RunProjection, StorageError, StorageTransaction, database_timestamp,
    encode_snapshot,
};
use forge_domain::{
    ProjectId, Timestamp,
    resolution::{
        Escalation, ResolutionAssignment, ResolutionAssignmentState, ResolutionContext, Resolver,
    },
    runtime::ResolutionRunSpec,
};
use sqlx::Row;
use uuid::Uuid;

impl PostgresStore {
    pub async fn resolution_queue_ids(
        &self,
        project: Option<ProjectId>,
        after: Option<Uuid>,
    ) -> Result<Vec<Uuid>, StorageError> {
        // Admission skips retained executions before bounding the page; otherwise
        // old unconfirmed stops can starve every later question indefinitely.
        // The global watchdog scan deliberately retains these rows for recovery.
        Ok(sqlx::query_scalar(r#"SELECT e.id FROM escalations e
            WHERE ($1::uuid IS NULL OR e.project_id=$1)
              AND (($1::uuid IS NOT NULL AND e.escalation_state='queued')
                OR ($1::uuid IS NULL AND e.escalation_state IN ('queued','assigned','needs_management_change')))
              AND ($1::uuid IS NULL OR NOT EXISTS(
                SELECT 1 FROM resolution_assignments a JOIN runs r ON r.resolution_assignment_id=a.id
                JOIN leases l ON forge_lease_owns_run(l,r)
                WHERE a.escalation_id=e.id AND (l.lease_state='active' OR EXISTS(
                    SELECT 1 FROM run_environment_reservations physical
                    WHERE physical.run_id=r.id AND physical.released_at IS NULL))))
            ORDER BY ($2::uuid IS NULL OR e.id>$2) DESC,e.id LIMIT 256"#)
            .bind(project.map(ProjectId::as_uuid)).bind(after).fetch_all(&self.pool).await?)
    }
}
impl StorageTransaction<'_> {
    /// Only accepted answers for this exact conversation owner, never unrelated
    /// Task memory. The operator still explicitly authorizes any new allowance.
    pub async fn communication_resolution_answers(
        &mut self,
        assignment: Uuid,
    ) -> Result<Vec<serde_json::Value>, StorageError> {
        Ok(sqlx::query_scalar("SELECT jsonb_build_object('artifact_id',a.id,'body',a.body_json,'source_run_id',e.source_run_id) FROM escalations e JOIN runs r ON r.id=e.source_run_id JOIN artifacts a ON a.id=(e.canonical_snapshot->'state'->>'artifact_id')::uuid AND a.project_id=e.project_id WHERE r.communication_assignment_id=$1 AND e.escalation_state='resolved' ORDER BY e.id DESC LIMIT 32").bind(assignment).fetch_all(&mut *self.transaction).await?)
    }
    pub async fn escalation_project(
        &mut self,
        id: Uuid,
    ) -> Result<Option<ProjectId>, StorageError> {
        Ok(
            sqlx::query_scalar::<_, Uuid>("SELECT project_id FROM escalations WHERE id=$1")
                .bind(id)
                .fetch_optional(&mut *self.transaction)
                .await?
                .map(ProjectId::from),
        )
    }
    pub async fn resolution_admission_open(
        &mut self,
        project: ProjectId,
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT execution_enabled AND NOT EXISTS(SELECT 1 FROM project_recovery_settings s WHERE s.project_id=projects.id AND s.hold) FROM projects WHERE id=$1").bind(project.as_uuid()).fetch_optional(&mut *self.transaction).await?.unwrap_or(false))
    }
    /// Even a retired assignment retains exclusive resolver execution until the
    /// common physical ledger proves quiescence. Logical completion is insufficient.
    pub async fn resolution_has_live_execution(
        &mut self,
        escalation: Uuid,
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM resolution_assignments a JOIN runs r ON r.resolution_assignment_id=a.id JOIN leases l ON forge_lease_owns_run(l,r) WHERE a.escalation_id=$1 AND (l.lease_state='active' OR EXISTS(SELECT 1 FROM run_environment_reservations e WHERE e.run_id=r.id AND e.released_at IS NULL)))")
            .bind(escalation).fetch_one(&mut *self.transaction).await?)
    }
    pub async fn resolution_run(
        &mut self,
        assignment: Uuid,
    ) -> Result<Option<RunProjection>, StorageError> {
        let id: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM runs WHERE resolution_assignment_id=$1")
                .bind(assignment)
                .fetch_optional(&mut *self.transaction)
                .await?;
        match id {
            Some(id) => self.load_run(id).await,
            None => Ok(None),
        }
    }
    pub async fn resolution_run_authorized(
        &mut self,
        run: &RunProjection,
    ) -> Result<bool, StorageError> {
        let Some(owner) = run.assignment.resolution() else {
            return Ok(false);
        };
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM resolution_assignments a JOIN escalations e ON e.id=a.escalation_id WHERE a.id=$1 AND a.escalation_id=$2 AND a.generation=$3 AND a.employee_id=$4 AND a.project_id=$5 AND a.assignment_state='active' AND a.expires_at>clock_timestamp() AND e.escalation_state='assigned' AND e.generation=a.generation AND e.canonical_snapshot->'state'->>'assignment_id'=a.id::text)")
            .bind(owner.assignment_id).bind(owner.escalation_id).bind(crate::u64_to_i64(owner.lease_generation,"resolution.generation")?).bind(run.require_employee_id()?.as_uuid()).bind(run.project_id.as_uuid()).fetch_one(&mut *self.transaction).await?)
    }
    /// Caller holds Project lock and has atomically published the Assignment.
    pub async fn create_resolution_run(
        &mut self,
        escalation: &Escalation,
        assignment: &ResolutionAssignment,
        spec: &ResolutionRunSpec,
        context: &ResolutionContext,
        lease_id: Uuid,
    ) -> Result<RunProjection, StorageError> {
        spec.validate().map_err(|_| invalid())?;
        assignment
            .validate_owner(escalation)
            .map_err(|_| invalid())?;
        let Resolver::Employee { employee_id } = assignment.resolver else {
            return Err(invalid());
        };
        let expires_at = assignment.lease.expires_at.ok_or_else(invalid)?;
        if assignment.state != ResolutionAssignmentState::Active
            || spec.project_id != escalation.project_id
            || spec.assignment.assignment_id != assignment.id
            || spec.assignment.escalation_id != escalation.id
            || spec.assignment.lease_generation != assignment.lease.generation
            || lease_id.get_version_num() != 7
            || context.data().assignment != *assignment
            || context.data().escalation != *escalation
            || context.data().run_id != spec.run_id
            || context.data().employee_id != employee_id
            || expires_at <= Timestamp::now_utc()
        {
            return Err(invalid());
        }
        let enabled:Option<bool>=sqlx::query_scalar("SELECT execution_enabled AND NOT EXISTS(SELECT 1 FROM project_recovery_settings s WHERE s.project_id=projects.id AND s.hold) FROM projects WHERE id=$1 FOR UPDATE").bind(escalation.project_id.as_uuid()).fetch_optional(&mut *self.transaction).await?;
        if enabled != Some(true)
            || !self
                .lock_employee_capacity(escalation.project_id, employee_id)
                .await?
            || self.resolution_has_live_execution(escalation.id).await?
        {
            return Err(invalid());
        }
        self.require_run_admission(
            escalation.project_id,
            4,
            &serde_json::to_value(spec).map_err(|_| invalid())?,
        )
        .await?;
        let current = self
            .load_resolution_assignment(assignment.id)
            .await?
            .ok_or_else(invalid)?;
        let question = self
            .load_escalation(escalation.id)
            .await?
            .ok_or_else(invalid)?;
        if current != *assignment || question != *escalation {
            return Err(invalid());
        }
        let lease=sqlx::query("INSERT INTO leases(id,project_id,employee_id,purpose,resolution_assignment_id,lease_scope,expires_at) VALUES($1,$2,$3,'resolution',$4,$5::jsonb,$6) RETURNING fencing_token,environment_epoch")
            .bind(lease_id).bind(spec.project_id.as_uuid()).bind(employee_id.as_uuid()).bind(assignment.id).bind(serde_json::json!({"assignment":spec.assignment})).bind(database_timestamp(expires_at)).fetch_one(&mut *self.transaction).await?;
        sqlx::query("INSERT INTO runs(id,project_id,employee_id,purpose,resolution_assignment_id,lease_id,attempt_number,lease_fencing_token,environment_epoch,run_spec_version,run_spec,context_manifest) VALUES($1,$2,$3,'resolution',$4,$5,1,$6,$7,4,$8::jsonb,$9::jsonb)")
            .bind(spec.run_id).bind(spec.project_id.as_uuid()).bind(employee_id.as_uuid()).bind(assignment.id).bind(lease_id).bind(lease.try_get::<i64,_>("fencing_token")?).bind(lease.try_get::<i64,_>("environment_epoch")?)
            .bind(encode_snapshot(spec,"resolution.spec")?).bind(encode_snapshot(context,"resolution.context")?).execute(&mut *self.transaction).await?;
        self.reserve_environment(spec.run_id, None).await?;
        self.load_run(spec.run_id).await?.ok_or_else(invalid)
    }
}
fn invalid() -> StorageError {
    StorageError::InvalidInput {
        reason: "Resolution admission is stale, unavailable, or violates ownership".into(),
    }
}
