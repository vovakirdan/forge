//! Core-facing cancellation and active-Run locking primitives.

use forge_domain::{ProjectId, TaskId};

use crate::{
    FencedWrite, RunProjection, StorageError, StorageTransaction, run_lifecycle::fenced_result,
    run_projection_from_row,
};

impl StorageTransaction<'_> {
    /// Cancels every not-yet-claimed queue entry in a Project.
    ///
    /// Leased work deliberately remains leased: Core must request a stop for
    /// its active Run and retain the Employee reservation until terminal Run
    /// observation confirms that the executor has stopped.
    pub async fn cancel_queued_entries_for_project(
        &mut self,
        project_id: ProjectId,
    ) -> Result<u64, StorageError> {
        cancel_queued_entries(&mut self.transaction, project_id, None).await
    }

    /// Cancels every not-yet-claimed queue entry for one Task.
    ///
    /// Core calls this before a Task revision mutation or replacement enqueue,
    /// preventing a stale queued revision from blocking the active-entry index.
    pub async fn invalidate_queued_entries_for_task(
        &mut self,
        project_id: ProjectId,
        task_id: TaskId,
    ) -> Result<u64, StorageError> {
        cancel_queued_entries(&mut self.transaction, project_id, Some(task_id)).await
    }

    /// Locks active Lease-backed Runs in a Project and returns their exact
    /// fencing scope to Core for subsequent `StopRun` requests.
    pub async fn lock_active_runs_for_project(
        &mut self,
        project_id: ProjectId,
    ) -> Result<Vec<RunProjection>, StorageError> {
        lock_active_runs(&mut self.transaction, project_id, None, None).await
    }

    /// Locks TaskStage writers and Hooks for this Task, including retired
    /// logical Leases with unresolved physical reservations. Hook context grants
    /// stop authority only; it never becomes a Task writer identity.
    pub async fn lock_active_runs_for_task(
        &mut self,
        project_id: ProjectId,
        task_id: TaskId,
    ) -> Result<Vec<RunProjection>, StorageError> {
        lock_active_runs(&mut self.transaction, project_id, Some(task_id), None).await
    }

    /// Includes logically revoked executions until physical quiescence is proven.
    pub async fn lock_active_runs_for_employee(
        &mut self,
        project_id: ProjectId,
        employee_id: forge_domain::EmployeeId,
    ) -> Result<Vec<RunProjection>, StorageError> {
        lock_active_runs(&mut self.transaction, project_id, None, Some(employee_id)).await
    }

    /// Marks the queue entry that belongs to this fenced Run as cancelled.
    ///
    /// This is only for an explicitly cancelled active Task. It does not
    /// release the Lease; Core must wait for terminal observation and call
    /// `release_lease_after_terminal_observation` separately.
    pub async fn cancel_leased_queue_for_fenced_run(
        &mut self,
        run_id: uuid::Uuid,
        lease_fencing_token: u64,
        environment_epoch: u64,
    ) -> Result<FencedWrite, StorageError> {
        let result = sqlx::query(
            "UPDATE queue_entries AS q SET queue_state = 'cancelled', cancelled_at = clock_timestamp() FROM runs AS r JOIN leases AS l ON l.id = r.lease_id AND l.project_id = r.project_id AND l.task_id = r.task_id AND l.employee_id = r.employee_id AND l.fencing_token = r.lease_fencing_token AND l.environment_epoch = r.environment_epoch WHERE r.id = $1 AND r.lease_fencing_token = $2 AND r.environment_epoch = $3 AND q.id = r.queue_entry_id AND q.queue_state = 'leased' AND l.lease_state = 'active'",
        )
        .bind(run_id)
        .bind(crate::u64_to_i64(
            lease_fencing_token,
            "run.lease_fencing_token",
        )?)
        .bind(crate::u64_to_i64(
            environment_epoch,
            "run.environment_epoch",
        )?)
        .execute(&mut *self.transaction)
        .await?;
        let communication = sqlx::query("UPDATE communication_assignments c SET state='held',updated_at=clock_timestamp() FROM runs r JOIN leases l ON forge_lease_owns_run(l,r) WHERE r.id=$1 AND r.lease_fencing_token=$2 AND r.environment_epoch=$3 AND r.purpose='communication' AND c.id=r.communication_assignment_id AND c.attempt_number=r.attempt_number AND c.state='leased'")
            .bind(run_id).bind(crate::u64_to_i64(lease_fencing_token,"run.fence")?).bind(crate::u64_to_i64(environment_epoch,"run.epoch")?)
            .execute(&mut *self.transaction).await?;
        fenced_result(
            &mut self.transaction,
            run_id,
            result.rows_affected() + communication.rows_affected(),
        )
        .await
    }
}

async fn cancel_queued_entries(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    project_id: ProjectId,
    task_id: Option<TaskId>,
) -> Result<u64, StorageError> {
    let result = if let Some(task_id) = task_id {
        sqlx::query(
            "UPDATE queue_entries SET queue_state = 'cancelled', cancelled_at = clock_timestamp() WHERE project_id = $1 AND task_id = $2 AND queue_state = 'queued'",
        )
        .bind(project_id.as_uuid())
        .bind(task_id.as_uuid())
        .execute(&mut **transaction)
        .await?
    } else {
        sqlx::query(
            "UPDATE queue_entries SET queue_state = 'cancelled', cancelled_at = clock_timestamp() WHERE project_id = $1 AND queue_state = 'queued'",
        )
        .bind(project_id.as_uuid())
        .execute(&mut **transaction)
        .await?
    };
    Ok(result.rows_affected())
}

async fn lock_active_runs(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    project_id: ProjectId,
    task_id: Option<TaskId>,
    employee_id: Option<forge_domain::EmployeeId>,
) -> Result<Vec<RunProjection>, StorageError> {
    let rows = sqlx::query(
        "SELECT r.id, r.project_id, r.purpose, r.communication_assignment_id, r.resolution_assignment_id, r.hook_invocation_id, r.task_id, r.queue_entry_id, r.lease_id, r.employee_id, r.stage_id, r.attempt_number, r.lease_fencing_token, r.environment_epoch, r.last_sequence, r.desired_state, r.observed_state, r.run_spec_version, r.run_spec::text AS run_spec, r.context_manifest::text AS context_manifest, r.observed_details::text AS observed_details FROM runs r JOIN leases l ON forge_lease_owns_run(l,r) WHERE r.project_id=$1 AND ($2::uuid IS NULL OR (r.purpose='task_stage' AND r.task_id=$2) OR (r.purpose='hook' AND EXISTS(SELECT 1 FROM hook_invocations h WHERE h.id=r.hook_invocation_id AND h.project_id=r.project_id AND h.run_id=r.id AND h.task_id=$2))) AND ($3::uuid IS NULL OR r.employee_id=$3) AND (l.lease_state='active' OR EXISTS(SELECT 1 FROM run_environment_reservations e WHERE e.run_id=r.id AND e.released_at IS NULL)) ORDER BY r.created_at ASC,r.id ASC FOR UPDATE OF r,l"
    ).bind(project_id.as_uuid()).bind(task_id.map(|id| id.as_uuid()))
        .bind(employee_id.map(|id| id.as_uuid())).fetch_all(&mut **transaction).await?;
    rows.into_iter().map(run_projection_from_row).collect()
}
