//! Mechanical queue, Lease, Run, and fenced Supervisor submission persistence.

use forge_domain::{EmployeeId, ProjectId, Timestamp};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use crate::{
    QueueEntry, QueueEntryInput, QueueState, RunDesiredState, RunObservedState, RunProjection,
    StorageError, StorageTransaction, StoredEmployee, database_timestamp, decode_snapshot,
    encode_object, i64_to_u64, queue_entry_from_row, run_projection_from_row, u32_to_i32,
    u64_to_i64,
};

/// Inputs Core supplies after atomically claiming an eligible queue entry.
#[derive(Clone, Debug)]
pub struct LeaseRunRequest {
    /// Claimed queue entry to turn into a Lease and Run.
    pub queue_entry: QueueEntry,
    /// Eligible Employee selected by Core policy.
    pub employee_id: EmployeeId,
    /// Fresh Lease identity.
    pub lease_id: Uuid,
    /// Fresh Run identity.
    pub run_id: Uuid,
    /// Absolute Lease expiry.
    pub lease_expires_at: Timestamp,
    /// Reserved future work-surface identity.
    pub task_work_surface_id: Option<Uuid>,
    /// Authorized runtime scope object.
    pub lease_scope: Value,
    /// Scheduler resource reservation object.
    pub resource_reservation: Value,
    /// Positive schema version of the immutable RunSpec object.
    pub run_spec_version: u16,
    /// Opaque immutable RunSpec object.
    pub run_spec: Value,
    /// Immutable context manifest object.
    pub context_manifest: Value,
}

/// Newly persisted Lease fence and Run projection sent to Supervisor.
#[derive(Clone, Debug)]
pub struct ProvisionedRun {
    /// Lease identity.
    pub lease_id: Uuid,
    /// Globally increasing fencing token assigned by PostgreSQL.
    pub lease_fencing_token: u64,
    /// Initial environment epoch.
    pub environment_epoch: u64,
    /// Provision-requested Run projection.
    pub run: RunProjection,
}

impl StorageTransaction<'_> {
    /// Adds one deduplicated queue entry for a Core-selected Employee stage.
    pub async fn enqueue(
        &mut self,
        input: &QueueEntryInput,
    ) -> Result<Option<QueueEntry>, StorageError> {
        let row = sqlx::query(
            "INSERT INTO queue_entries (id, project_id, task_id, task_sequence, pipeline_id, pipeline_version_id, stage_id, task_revision, attempt_number, priority_level_id, priority_rank, resource_profile, eligible_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12::jsonb, $13) ON CONFLICT (project_id, task_id, stage_id, task_revision) DO NOTHING RETURNING id, project_id, task_id, task_sequence, pipeline_id, pipeline_version_id, stage_id, task_revision, attempt_number, priority_level_id, priority_rank, resource_profile::text AS resource_profile, eligible_at, queue_state",
        )
        .bind(Uuid::now_v7())
        .bind(input.project_id.as_uuid())
        .bind(input.task_id.as_uuid())
        .bind(u64_to_i64(input.task_sequence, "queue.task_sequence")?)
        .bind(input.pipeline_id.as_uuid())
        .bind(input.pipeline_version_id.as_uuid())
        .bind(input.stage_id.as_str())
        .bind(u64_to_i64(input.task_revision, "queue.task_revision")?)
        .bind(u32_to_i32(input.attempt_number, "queue.attempt_number")?)
        .bind(&input.priority_level_id)
        .bind(input.priority_rank)
        .bind(encode_object(&input.resource_profile, "queue.resource_profile")?)
        .bind(database_timestamp(input.eligible_at))
        .fetch_optional(&mut *self.transaction)
        .await?;
        row.map(queue_entry_from_row).transpose()
    }

    /// Claims the next eligible entry using PostgreSQL row locks and canonical ordering.
    ///
    /// The Project gate and current Task stage are locked with the queue row so
    /// a committed stop or stage change cannot race a newly issued Lease.
    pub async fn claim_next(
        &mut self,
        project_id: ProjectId,
    ) -> Result<Option<QueueEntry>, StorageError> {
        let row = sqlx::query(
            "WITH candidate AS (SELECT q.id FROM queue_entries q JOIN projects p ON p.id = q.project_id JOIN tasks t ON t.id = q.task_id AND t.project_id = q.project_id WHERE q.project_id = $1 AND q.queue_state = 'queued' AND q.eligible_at <= clock_timestamp() AND p.execution_enabled AND (NOT EXISTS (SELECT 1 FROM project_recovery_settings prs WHERE prs.project_id=p.id AND prs.hold) OR EXISTS (SELECT 1 FROM run_recovery_decisions rd WHERE rd.recovery_queue_entry_id=q.id AND rd.assessment='not_started_confirmed')) AND t.lifecycle IN ('ready', 'in_progress') AND t.revision = q.task_revision AND t.current_stage_id = q.stage_id AND NOT EXISTS (SELECT 1 FROM run_environment_reservations r WHERE r.task_id=t.id AND r.released_at IS NULL) AND NOT EXISTS (SELECT 1 FROM task_dependencies d JOIN tasks blocker ON blocker.id = d.blocker_task_id AND blocker.project_id = d.project_id WHERE d.project_id = q.project_id AND d.blocked_task_id = q.task_id AND blocker.lifecycle <> d.required_blocker_lifecycle) ORDER BY q.priority_rank DESC, q.eligible_at ASC, q.task_sequence ASC, q.id ASC FOR UPDATE OF p, t, q SKIP LOCKED LIMIT 1) UPDATE queue_entries q SET queue_state = 'leased', claimed_at = clock_timestamp() FROM candidate WHERE q.id = candidate.id RETURNING q.id, q.project_id, q.task_id, q.task_sequence, q.pipeline_id, q.pipeline_version_id, q.stage_id, q.task_revision, q.attempt_number, q.priority_level_id, q.priority_rank, q.resource_profile::text AS resource_profile, q.eligible_at, q.queue_state",
        )
        .bind(project_id.as_uuid())
        .fetch_optional(&mut *self.transaction)
        .await?;
        row.map(queue_entry_from_row).transpose()
    }

    /// Restores a claimed entry when Core cannot form a Lease in this transaction.
    pub async fn release_queue_claim(
        &mut self,
        queue_entry_id: Uuid,
    ) -> Result<bool, StorageError> {
        let result = sqlx::query(
            "UPDATE queue_entries SET queue_state = 'queued', claimed_at = NULL WHERE id = $1 AND queue_state = 'leased'",
        )
        .bind(queue_entry_id)
        .execute(&mut *self.transaction)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Finds currently unleased active Employees under row locks; Core applies stage policy.
    pub async fn lock_available_employees(
        &mut self,
        project_id: ProjectId,
    ) -> Result<Vec<StoredEmployee>, StorageError> {
        let values: Vec<String> = sqlx::query_scalar(
            "SELECT e.canonical_snapshot::text FROM employees e WHERE e.project_id = $1 AND e.employee_state = 'active' AND NOT EXISTS (SELECT 1 FROM leases l WHERE l.employee_id = e.id AND l.lease_state = 'active') AND NOT EXISTS (SELECT 1 FROM run_environment_reservations r WHERE r.employee_id=e.id AND r.released_at IS NULL) ORDER BY e.name ASC FOR UPDATE OF e SKIP LOCKED",
        )
        .bind(project_id.as_uuid())
        .fetch_all(&mut *self.transaction)
        .await?;
        values
            .into_iter()
            .map(|value| {
                decode_snapshot(&value, "employee").map(|employee| StoredEmployee { employee })
            })
            .collect()
    }

    /// Converts a leased queue entry into one active Lease and provision-requested Run.
    /// Rechecks every dispatch gate under locks before it starts work.
    pub async fn create_lease_and_run(
        &mut self,
        request: &LeaseRunRequest,
    ) -> Result<ProvisionedRun, StorageError> {
        if request.queue_entry.state != QueueState::Leased {
            return Err(StorageError::InvalidInput {
                reason: "a run can only be created from a leased queue entry".to_owned(),
            });
        }
        let run_spec_version = i16::try_from(request.run_spec_version).map_err(|_| {
            StorageError::IntegerOutOfRange {
                field: "run.run_spec_version",
            }
        })?;
        if run_spec_version == 0 {
            return Err(StorageError::InvalidInput {
                reason: "run.run_spec_version must be greater than zero".to_owned(),
            });
        }
        let execution_enabled: Option<bool> =
            sqlx::query_scalar("SELECT execution_enabled FROM projects WHERE id = $1 FOR UPDATE")
                .bind(request.queue_entry.input.project_id.as_uuid())
                .fetch_optional(&mut *self.transaction)
                .await?;
        let Some(execution_enabled) = execution_enabled else {
            return Err(StorageError::NotFound {
                aggregate: "project",
            });
        };
        if !execution_enabled {
            return Err(StorageError::InvalidInput {
                reason: "project execution is stopped".to_owned(),
            });
        }
        let task_current: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM tasks WHERE id = $1 AND project_id = $2 AND lifecycle IN ('ready', 'in_progress') AND revision = $3 AND current_stage_id = $4 FOR UPDATE",
        )
        .bind(request.queue_entry.input.task_id.as_uuid())
        .bind(request.queue_entry.input.project_id.as_uuid())
        .bind(u64_to_i64(
            request.queue_entry.input.task_revision,
            "queue.task_revision",
        )?)
        .bind(request.queue_entry.input.stage_id.as_str())
        .fetch_optional(&mut *self.transaction)
        .await?;
        if task_current.is_none() {
            return Err(StorageError::StaleRevision {
                aggregate: "task dispatch state",
            });
        }
        let blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM task_dependencies d JOIN tasks blocker ON blocker.id = d.blocker_task_id AND blocker.project_id = d.project_id WHERE d.project_id = $1 AND d.blocked_task_id = $2 AND blocker.lifecycle <> d.required_blocker_lifecycle)",
        )
        .bind(request.queue_entry.input.project_id.as_uuid())
        .bind(request.queue_entry.input.task_id.as_uuid())
        .fetch_one(&mut *self.transaction)
        .await?;
        if blocked {
            return Err(StorageError::StaleRevision {
                aggregate: "task dependency gate",
            });
        }
        let active: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM queue_entries WHERE id = $1 AND queue_state = 'leased' AND project_id = $2 AND task_id = $3 AND pipeline_id = $4 AND pipeline_version_id = $5 AND stage_id = $6 AND task_revision = $7 AND attempt_number = $8 FOR UPDATE",
        )
        .bind(request.queue_entry.id)
        .bind(request.queue_entry.input.project_id.as_uuid())
        .bind(request.queue_entry.input.task_id.as_uuid())
        .bind(request.queue_entry.input.pipeline_id.as_uuid())
        .bind(request.queue_entry.input.pipeline_version_id.as_uuid())
        .bind(request.queue_entry.input.stage_id.as_str())
        .bind(u64_to_i64(
            request.queue_entry.input.task_revision,
            "queue.task_revision",
        )?)
        .bind(u32_to_i32(
            request.queue_entry.input.attempt_number,
            "queue.attempt_number",
        )?)
        .fetch_optional(&mut *self.transaction)
        .await?;
        if active.is_none() {
            return Err(StorageError::StaleRevision {
                aggregate: "queue entry",
            });
        }
        let employee_active: Option<Uuid> = sqlx::query_scalar(
            "SELECT e.id FROM employees e WHERE e.id = $1 AND e.project_id = $2 AND e.employee_state = 'active' AND NOT EXISTS (SELECT 1 FROM leases l WHERE l.employee_id = e.id AND l.lease_state = 'active') FOR UPDATE OF e",
        )
        .bind(request.employee_id.as_uuid())
        .bind(request.queue_entry.input.project_id.as_uuid())
        .fetch_optional(&mut *self.transaction)
        .await?;
        if employee_active.is_none() {
            return Err(StorageError::StaleRevision {
                aggregate: "employee",
            });
        }
        let row = sqlx::query(
            "INSERT INTO leases (id, project_id, task_id, queue_entry_id, employee_id, task_work_surface_id, lease_scope, resource_reservation, expires_at) VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb, $8::jsonb, $9) RETURNING fencing_token, environment_epoch",
        )
        .bind(request.lease_id)
        .bind(request.queue_entry.input.project_id.as_uuid())
        .bind(request.queue_entry.input.task_id.as_uuid())
        .bind(request.queue_entry.id)
        .bind(request.employee_id.as_uuid())
        .bind(request.task_work_surface_id)
        .bind(encode_object(&request.lease_scope, "lease.scope")?)
        .bind(encode_object(&request.resource_reservation, "lease.resource_reservation")?)
        .bind(database_timestamp(request.lease_expires_at))
        .fetch_one(&mut *self.transaction)
        .await?;
        let fencing_token = i64_to_u64(row.try_get("fencing_token")?, "lease.fencing_token")?;
        let environment_epoch =
            i64_to_u64(row.try_get("environment_epoch")?, "lease.environment_epoch")?;
        sqlx::query(
            "INSERT INTO runs (id, project_id, task_id, queue_entry_id, lease_id, employee_id, stage_id, attempt_number, lease_fencing_token, environment_epoch, run_spec_version, run_spec, context_manifest) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12::jsonb, $13::jsonb)",
        )
        .bind(request.run_id)
        .bind(request.queue_entry.input.project_id.as_uuid())
        .bind(request.queue_entry.input.task_id.as_uuid())
        .bind(request.queue_entry.id)
        .bind(request.lease_id)
        .bind(request.employee_id.as_uuid())
        .bind(request.queue_entry.input.stage_id.as_str())
        .bind(u32_to_i32(request.queue_entry.input.attempt_number, "run.attempt_number")?)
        .bind(u64_to_i64(fencing_token, "run.lease_fencing_token")?)
        .bind(u64_to_i64(environment_epoch, "run.environment_epoch")?)
        .bind(run_spec_version)
        .bind(encode_object(&request.run_spec, "run.spec")?)
        .bind(encode_object(&request.context_manifest, "run.context_manifest")?)
        .execute(&mut *self.transaction)
        .await?;
        Ok(ProvisionedRun {
            lease_id: request.lease_id,
            lease_fencing_token: fencing_token,
            environment_epoch,
            run: RunProjection {
                id: request.run_id,
                project_id: request.queue_entry.input.project_id,
                task_id: request.queue_entry.input.task_id,
                queue_entry_id: request.queue_entry.id,
                lease_id: request.lease_id,
                employee_id: request.employee_id,
                stage_id: request.queue_entry.input.stage_id.as_str().to_owned(),
                attempt_number: request.queue_entry.input.attempt_number,
                lease_fencing_token: fencing_token,
                environment_epoch,
                last_sequence: 0,
                desired_state: RunDesiredState::ProvisionRequested,
                observed_state: RunObservedState::Unknown,
                run_spec_version: request.run_spec_version,
                run_spec: request.run_spec.clone(),
                context_manifest: request.context_manifest.clone(),
                observed_details: json!({}),
            },
        })
    }

    /// Reads a current Run projection under the scheduler transaction.
    pub async fn load_run(&mut self, run_id: Uuid) -> Result<Option<RunProjection>, StorageError> {
        let row = sqlx::query(
            "SELECT id, project_id, task_id, queue_entry_id, lease_id, employee_id, stage_id, attempt_number, lease_fencing_token, environment_epoch, last_sequence, desired_state, observed_state, run_spec_version, run_spec::text AS run_spec, context_manifest::text AS context_manifest, observed_details::text AS observed_details FROM runs WHERE id = $1",
        )
        .bind(run_id)
        .fetch_optional(&mut *self.transaction)
        .await?;
        row.map(run_projection_from_row).transpose()
    }
}
