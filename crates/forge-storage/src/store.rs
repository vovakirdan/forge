//! PostgreSQL connection ownership, typed reads, and explicit row locks.

use std::time::Duration;

use forge_domain::{
    Artifact, ArtifactId, EmployeeId, Pipeline, PipelineId, PipelineVersion, PipelineVersionId,
    Project, ProjectId, StageId, Task, TaskId,
};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgPoolOptions};
use uuid::Uuid;

use crate::{
    ArtifactLocation, QueueEntry, QueueEntryInput, RunProjection, StorageError, StoredArtifact,
    StoredEmployee, StoredEvent, StoredTask, TaskPersistence, decode_snapshot, domain_timestamp,
    i32_to_u32, i64_to_u64,
};

/// The sole PostgreSQL access point used by Forge Core.
#[derive(Clone, Debug)]
pub struct PostgresStore {
    pub(crate) pool: PgPool,
}

impl PostgresStore {
    /// Opens a bounded PostgreSQL pool without changing schema state.
    pub async fn connect(database_url: &str) -> Result<Self, StorageError> {
        let pool = PgPoolOptions::new()
            .max_connections(16)
            .acquire_timeout(Duration::from_secs(10))
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    /// Wraps an already configured pool, primarily for controlled tests.
    #[must_use]
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Validates the explicit, already-applied Forge migration contract.
    pub async fn validate_schema(&self) -> Result<(), StorageError> {
        crate::validate_schema(&self.pool).await?;
        Ok(())
    }

    /// Starts the transaction that owns one complete Core command or scheduler decision.
    pub async fn begin(&self) -> Result<StorageTransaction<'_>, StorageError> {
        Ok(StorageTransaction {
            transaction: self.pool.begin().await?,
        })
    }

    /// Loads a Project's canonical snapshot without taking a write lock.
    pub async fn load_project(&self, id: ProjectId) -> Result<Option<Project>, StorageError> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT canonical_snapshot::text FROM projects WHERE id = $1")
                .bind(id.as_uuid())
                .fetch_optional(&self.pool)
                .await?;
        value
            .map(|value| decode_snapshot(&value, "project"))
            .transpose()
    }

    /// Loads a Task and its indexed scheduler fields without a write lock.
    pub async fn load_task(&self, id: TaskId) -> Result<Option<StoredTask>, StorageError> {
        let row = sqlx::query(
            "SELECT canonical_snapshot::text AS canonical_snapshot, priority_rank, attempt_count, task_work_surface_id FROM tasks WHERE id = $1",
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        row.map(stored_task_from_row).transpose()
    }

    /// Lists Project Tasks in durable local-key order.
    pub async fn list_tasks(&self, project_id: ProjectId) -> Result<Vec<StoredTask>, StorageError> {
        let rows = sqlx::query(
            "SELECT canonical_snapshot::text AS canonical_snapshot, priority_rank, attempt_count, task_work_surface_id FROM tasks WHERE project_id = $1 ORDER BY task_sequence ASC",
        )
        .bind(project_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(stored_task_from_row).collect()
    }

    /// Loads a named Pipeline catalog snapshot.
    pub async fn load_pipeline(&self, id: PipelineId) -> Result<Option<Pipeline>, StorageError> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT canonical_snapshot::text FROM pipelines WHERE id = $1")
                .bind(id.as_uuid())
                .fetch_optional(&self.pool)
                .await?;
        value
            .map(|value| decode_snapshot(&value, "pipeline"))
            .transpose()
    }

    /// Loads an immutable Pipeline version from its canonical `definition` snapshot.
    pub async fn load_pipeline_version(
        &self,
        id: PipelineVersionId,
    ) -> Result<Option<PipelineVersion>, StorageError> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT definition::text FROM pipeline_versions WHERE id = $1")
                .bind(id.as_uuid())
                .fetch_optional(&self.pool)
                .await?;
        value
            .map(|value| decode_snapshot(&value, "pipeline_version"))
            .transpose()
    }

    /// Loads an Employee's canonical snapshot.
    pub async fn load_employee(
        &self,
        id: EmployeeId,
    ) -> Result<Option<StoredEmployee>, StorageError> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT canonical_snapshot::text FROM employees WHERE id = $1")
                .bind(id.as_uuid())
                .fetch_optional(&self.pool)
                .await?;
        value
            .map(|value| {
                decode_snapshot(&value, "employee").map(|employee| StoredEmployee { employee })
            })
            .transpose()
    }

    /// Lists Employee snapshots in stable display-name order.
    pub async fn list_employees(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<StoredEmployee>, StorageError> {
        let values: Vec<String> = sqlx::query_scalar(
            "SELECT canonical_snapshot::text FROM employees WHERE project_id = $1 ORDER BY name ASC",
        )
        .bind(project_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        values
            .into_iter()
            .map(|value| {
                decode_snapshot(&value, "employee").map(|employee| StoredEmployee { employee })
            })
            .collect()
    }

    /// Loads immutable Artifact evidence and its attachment context.
    pub async fn load_artifact(
        &self,
        id: ArtifactId,
    ) -> Result<Option<StoredArtifact>, StorageError> {
        let row = sqlx::query(
            "SELECT canonical_snapshot::text AS canonical_snapshot, task_id, run_id, stage_id, producer_type, producer_id, producer::text AS producer FROM artifacts WHERE id = $1",
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        row.map(stored_artifact_from_row).transpose()
    }

    /// Reads Project event history strictly after an optional sequence cursor.
    pub async fn list_events(
        &self,
        project_id: ProjectId,
        after_sequence: Option<u64>,
        limit: u32,
    ) -> Result<Vec<StoredEvent>, StorageError> {
        let after = i64::try_from(after_sequence.unwrap_or_default()).map_err(|_| {
            StorageError::IntegerOutOfRange {
                field: "event.after_sequence",
            }
        })?;
        let limit = i64::from(limit.clamp(1, 1_000));
        let rows = sqlx::query(
            "SELECT id, project_id, project_sequence, aggregate_type, aggregate_id, aggregate_revision, event_type, event_schema_version, actor::text AS actor, command_id, reason, payload::text AS payload, occurred_at FROM event_log WHERE project_id = $1 AND project_sequence > $2 ORDER BY project_sequence ASC LIMIT $3",
        )
        .bind(project_id.as_uuid())
        .bind(after)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(stored_event_from_row).collect()
    }

    /// Reads current Run projections in newest-first order for one Task.
    pub async fn list_runs(&self, task_id: TaskId) -> Result<Vec<RunProjection>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, project_id, purpose, communication_assignment_id, resolution_assignment_id, hook_invocation_id, task_id, queue_entry_id, lease_id, employee_id, stage_id, attempt_number, lease_fencing_token, environment_epoch, last_sequence, desired_state, observed_state, run_spec_version, run_spec::text AS run_spec, context_manifest::text AS context_manifest, observed_details::text AS observed_details FROM runs WHERE task_id = $1 ORDER BY created_at DESC",
        )
        .bind(task_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(run_projection_from_row).collect()
    }
}

/// A transaction used only by Core to group snapshots, events, outbox, and scheduler state.
pub struct StorageTransaction<'store> {
    pub(crate) transaction: Transaction<'store, Postgres>,
}

impl StorageTransaction<'_> {
    /// Commits every write made through this transaction.
    pub async fn commit(self) -> Result<(), StorageError> {
        self.transaction.commit().await?;
        Ok(())
    }

    /// Explicitly rolls back; dropping without commit also rolls back.
    pub async fn rollback(self) -> Result<(), StorageError> {
        self.transaction.rollback().await?;
        Ok(())
    }

    /// Serializes creation attempts for one caller-reserved Project identity.
    ///
    /// Project rows cannot be locked before their first insertion. A
    /// transaction-scoped advisory lock closes that idempotency race without
    /// retaining process-local coordination state. A hash collision merely
    /// serializes unrelated creation attempts; it never coalesces identities.
    pub async fn lock_project_creation(&mut self, id: ProjectId) -> Result<(), StorageError> {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(id.as_uuid())
            .execute(&mut *self.transaction)
            .await?;
        Ok(())
    }

    /// Loads and locks the Project row that serializes command revisions and event sequencing.
    pub async fn lock_project(&mut self, id: ProjectId) -> Result<Option<Project>, StorageError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT canonical_snapshot::text FROM projects WHERE id = $1 FOR UPDATE",
        )
        .bind(id.as_uuid())
        .fetch_optional(&mut *self.transaction)
        .await?;
        value
            .map(|value| decode_snapshot(&value, "project"))
            .transpose()
    }

    /// Loads and locks a Task row before Core applies a semantic transition.
    pub async fn lock_task(&mut self, id: TaskId) -> Result<Option<StoredTask>, StorageError> {
        let row = sqlx::query(
            "SELECT canonical_snapshot::text AS canonical_snapshot, priority_rank, attempt_count, task_work_surface_id FROM tasks WHERE id = $1 FOR UPDATE",
        )
        .bind(id.as_uuid())
        .fetch_optional(&mut *self.transaction)
        .await?;
        row.map(stored_task_from_row).transpose()
    }

    /// Loads and locks a Pipeline catalog row before Core changes its metadata.
    pub async fn lock_pipeline(
        &mut self,
        id: PipelineId,
    ) -> Result<Option<Pipeline>, StorageError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT canonical_snapshot::text FROM pipelines WHERE id = $1 FOR UPDATE",
        )
        .bind(id.as_uuid())
        .fetch_optional(&mut *self.transaction)
        .await?;
        value
            .map(|value| decode_snapshot(&value, "pipeline"))
            .transpose()
    }

    /// Locks an immutable Pipeline version while Core coordinates a dependent write.
    ///
    /// Pipeline versions are never updated after insertion; this lock only gives a
    /// command a consistent version boundary with its other aggregate writes.
    pub async fn lock_pipeline_version(
        &mut self,
        id: PipelineVersionId,
    ) -> Result<Option<PipelineVersion>, StorageError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT definition::text FROM pipeline_versions WHERE id = $1 FOR UPDATE",
        )
        .bind(id.as_uuid())
        .fetch_optional(&mut *self.transaction)
        .await?;
        value
            .map(|value| decode_snapshot(&value, "pipeline_version"))
            .transpose()
    }

    /// Loads and locks an Employee before Core changes its dispatch eligibility.
    pub async fn lock_employee(
        &mut self,
        id: EmployeeId,
    ) -> Result<Option<StoredEmployee>, StorageError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT canonical_snapshot::text FROM employees WHERE id = $1 FOR UPDATE",
        )
        .bind(id.as_uuid())
        .fetch_optional(&mut *self.transaction)
        .await?;
        value
            .map(|value| {
                decode_snapshot(&value, "employee").map(|employee| StoredEmployee { employee })
            })
            .transpose()
    }

    /// Locks immutable Artifact evidence while Core coordinates its Task link.
    ///
    /// Artifact contents remain append-only; this does not expose an update path.
    pub async fn lock_artifact(
        &mut self,
        id: ArtifactId,
    ) -> Result<Option<StoredArtifact>, StorageError> {
        let row = sqlx::query(
            "SELECT canonical_snapshot::text AS canonical_snapshot, task_id, run_id, stage_id, producer_type, producer_id, producer::text AS producer FROM artifacts WHERE id = $1 FOR UPDATE",
        )
        .bind(id.as_uuid())
        .fetch_optional(&mut *self.transaction)
        .await?;
        row.map(stored_artifact_from_row).transpose()
    }
}

pub(crate) fn stored_task_from_row(row: sqlx::postgres::PgRow) -> Result<StoredTask, StorageError> {
    let value: String = row.try_get("canonical_snapshot")?;
    let task: Task = decode_snapshot(&value, "task")?;
    Ok(StoredTask {
        task,
        persistence: TaskPersistence {
            priority_rank: row.try_get("priority_rank")?,
            attempt_count: i32_to_u32(row.try_get("attempt_count")?, "task.attempt_count")?,
            task_work_surface_id: row.try_get("task_work_surface_id")?,
        },
    })
}

pub(crate) fn stored_artifact_from_row(
    row: sqlx::postgres::PgRow,
) -> Result<StoredArtifact, StorageError> {
    let value: String = row.try_get("canonical_snapshot")?;
    let artifact: Artifact = decode_snapshot(&value, "artifact")?;
    let producer: String = row.try_get("producer_type")?;
    let producer = serde_json::from_value(Value::String(producer)).map_err(|source| {
        StorageError::Snapshot {
            aggregate: "artifact.producer_type",
            source,
        }
    })?;
    let stage_id = row
        .try_get::<Option<String>, _>("stage_id")?
        .map(|value| {
            StageId::new(value).map_err(|error| StorageError::InvalidInput {
                reason: format!("artifact.stage_id is invalid: {error}"),
            })
        })
        .transpose()?;
    let producer_data = json_from_row(&row, "producer", "artifact.producer")?;
    Ok(StoredArtifact {
        artifact,
        location: ArtifactLocation {
            task_id: row.try_get::<Option<Uuid>, _>("task_id")?.map(Into::into),
            run_id: row.try_get("run_id")?,
            stage_id,
            producer,
            producer_id: row.try_get("producer_id")?,
            producer_data,
        },
    })
}

pub(crate) fn stored_event_from_row(
    row: sqlx::postgres::PgRow,
) -> Result<StoredEvent, StorageError> {
    let aggregate_type = enum_from_row(&row, "aggregate_type", "event.aggregate_type")?;
    Ok(StoredEvent {
        id: row.try_get::<Uuid, _>("id")?.into(),
        project_id: row.try_get::<Uuid, _>("project_id")?.into(),
        project_sequence: i64_to_u64(row.try_get("project_sequence")?, "event.project_sequence")?,
        aggregate_type,
        aggregate_id: row.try_get("aggregate_id")?,
        aggregate_revision: i64_to_u64(
            row.try_get("aggregate_revision")?,
            "event.aggregate_revision",
        )?,
        event_type: row.try_get("event_type")?,
        schema_version: u16::try_from(row.try_get::<i16, _>("event_schema_version")?).map_err(
            |_| StorageError::IntegerOutOfRange {
                field: "event.schema_version",
            },
        )?,
        actor: json_from_row(&row, "actor", "event.actor")?,
        command_id: row
            .try_get::<Option<Uuid>, _>("command_id")?
            .map(Into::into),
        reason: row.try_get("reason")?,
        payload: json_from_row(&row, "payload", "event.payload")?,
        occurred_at: domain_timestamp(row.try_get("occurred_at")?),
    })
}

pub(crate) fn run_projection_from_row(
    row: sqlx::postgres::PgRow,
) -> Result<RunProjection, StorageError> {
    let context_manifest: serde_json::Value =
        json_from_row(&row, "context_manifest", "run.context_manifest")?;
    let employee_id = row
        .try_get::<Option<Uuid>, _>("employee_id")?
        .map(EmployeeId::from);
    let purpose = row.try_get::<&str, _>("purpose")?;
    if (purpose == "hook") != employee_id.is_none() {
        return Err(StorageError::InvalidInput {
            reason: "execution Employee owner differs from purpose".into(),
        });
    }
    let assignment = match purpose {
        "task_stage" => {
            forge_domain::ExecutionAssignment::TaskStage(forge_domain::TaskStageAssignment {
                task_id: row.try_get::<Uuid, _>("task_id")?.into(),
                queue_entry_id: row.try_get("queue_entry_id")?,
                stage_id: StageId::new(row.try_get::<String, _>("stage_id")?).map_err(|_| {
                    StorageError::InvalidInput {
                        reason: "invalid TaskStage owner".into(),
                    }
                })?,
            })
        }
        "communication" => {
            let context: forge_domain::communication::CommunicationContext =
                serde_json::from_value(context_manifest.clone()).map_err(|source| {
                    StorageError::Snapshot {
                        aggregate: "communication.context",
                        source,
                    }
                })?;
            if context.data().assignment.assignment_id
                != row.try_get::<Uuid, _>("communication_assignment_id")?
                || context.data().project_id.as_uuid() != row.try_get::<Uuid, _>("project_id")?
                || context.data().employee_id.as_uuid() != row.try_get::<Uuid, _>("employee_id")?
                || context.data().run_id != row.try_get::<Uuid, _>("id")?
            {
                return Err(StorageError::InvalidInput {
                    reason: "Communication context differs from Run owner".into(),
                });
            }
            forge_domain::ExecutionAssignment::Communication(context.data().assignment.clone())
        }
        "resolution" => {
            let context: forge_domain::resolution::ResolutionContext =
                serde_json::from_value(context_manifest.clone()).map_err(|source| {
                    StorageError::Snapshot {
                        aggregate: "resolution.context",
                        source,
                    }
                })?;
            let data = context.data();
            if data.assignment.id != row.try_get::<Uuid, _>("resolution_assignment_id")?
                || data.project_id.as_uuid() != row.try_get::<Uuid, _>("project_id")?
                || data.employee_id.as_uuid() != row.try_get::<Uuid, _>("employee_id")?
                || data.run_id != row.try_get::<Uuid, _>("id")?
                || row.try_get::<i16, _>("run_spec_version")? != 4
            {
                return Err(StorageError::InvalidInput {
                    reason: "Resolution context differs from Run owner".into(),
                });
            }
            forge_domain::ExecutionAssignment::Resolution(forge_domain::ResolutionAssignmentRef {
                assignment_id: data.assignment.id,
                escalation_id: data.assignment.escalation_id,
                lease_generation: data.assignment.lease.generation,
            })
        }
        "hook" => {
            let spec: forge_domain::runtime::HookRunSpec =
                json_from_row(&row, "run_spec", "hook.run_spec").and_then(|value| {
                    serde_json::from_value(value).map_err(|source| StorageError::Snapshot {
                        aggregate: "hook.run_spec",
                        source,
                    })
                })?;
            spec.validate().map_err(|_| StorageError::InvalidInput {
                reason: "invalid Hook RunSpec".into(),
            })?;
            if spec.assignment.invocation_id != row.try_get::<Uuid, _>("hook_invocation_id")?
                || spec.run_id != row.try_get::<Uuid, _>("id")?
                || spec.project_id.as_uuid() != row.try_get::<Uuid, _>("project_id")?
                || row.try_get::<i16, _>("run_spec_version")? != 5
                || row.try_get::<Option<Uuid>, _>("task_id")?.is_some()
                || row.try_get::<Option<Uuid>, _>("queue_entry_id")?.is_some()
                || row.try_get::<Option<String>, _>("stage_id")?.is_some()
            {
                return Err(StorageError::InvalidInput {
                    reason: "Hook snapshot differs from ownerless Run".into(),
                });
            }
            forge_domain::ExecutionAssignment::Hook(spec.assignment)
        }
        _ => {
            return Err(StorageError::InvalidInput {
                reason: "unsupported execution assignment".into(),
            });
        }
    };
    assignment
        .validate()
        .map_err(|_| StorageError::InvalidInput {
            reason: "invalid execution owner".into(),
        })?;
    Ok(RunProjection {
        id: row.try_get("id")?,
        project_id: row.try_get::<Uuid, _>("project_id")?.into(),
        assignment,
        lease_id: row.try_get("lease_id")?,
        employee_id,
        attempt_number: i32_to_u32(row.try_get("attempt_number")?, "run.attempt_number")?,
        lease_fencing_token: i64_to_u64(
            row.try_get("lease_fencing_token")?,
            "run.lease_fencing_token",
        )?,
        environment_epoch: i64_to_u64(row.try_get("environment_epoch")?, "run.environment_epoch")?,
        last_sequence: i64_to_u64(row.try_get("last_sequence")?, "run.last_sequence")?,
        desired_state: enum_from_row(&row, "desired_state", "run.desired_state")?,
        observed_state: enum_from_row(&row, "observed_state", "run.observed_state")?,
        run_spec_version: u16::try_from(row.try_get::<i16, _>("run_spec_version")?).map_err(
            |_| StorageError::IntegerOutOfRange {
                field: "run.run_spec_version",
            },
        )?,
        run_spec: json_from_row(&row, "run_spec", "run.run_spec")?,
        context_manifest,
        observed_details: json_from_row(&row, "observed_details", "run.observed_details")?,
    })
}

pub(crate) fn queue_entry_from_row(row: sqlx::postgres::PgRow) -> Result<QueueEntry, StorageError> {
    let project_id = row.try_get::<Uuid, _>("project_id")?.into();
    let task_id = row.try_get::<Uuid, _>("task_id")?.into();
    let pipeline_id = row.try_get::<Uuid, _>("pipeline_id")?.into();
    let pipeline_version_id = row.try_get::<Uuid, _>("pipeline_version_id")?.into();
    let stage_id = StageId::new(row.try_get::<String, _>("stage_id")?).map_err(|error| {
        StorageError::InvalidInput {
            reason: format!("queue.stage_id is invalid: {error}"),
        }
    })?;
    Ok(QueueEntry {
        id: row.try_get("id")?,
        input: QueueEntryInput {
            project_id,
            task_id,
            task_sequence: i64_to_u64(row.try_get("task_sequence")?, "queue.task_sequence")?,
            pipeline_id,
            pipeline_version_id,
            stage_id,
            task_revision: i64_to_u64(row.try_get("task_revision")?, "queue.task_revision")?,
            attempt_number: i32_to_u32(row.try_get("attempt_number")?, "queue.attempt_number")?,
            priority_level_id: row.try_get("priority_level_id")?,
            priority_rank: row.try_get("priority_rank")?,
            resource_profile: json_from_row(&row, "resource_profile", "queue.resource_profile")?,
            eligible_at: domain_timestamp(row.try_get("eligible_at")?),
        },
        state: enum_from_row(&row, "queue_state", "queue.state")?,
    })
}

pub(crate) fn json_from_row(
    row: &sqlx::postgres::PgRow,
    column: &str,
    field: &'static str,
) -> Result<Value, StorageError> {
    let value: String = row.try_get(column)?;
    serde_json::from_str(&value).map_err(|source| StorageError::Snapshot {
        aggregate: field,
        source,
    })
}

fn enum_from_row<T: serde::de::DeserializeOwned>(
    row: &sqlx::postgres::PgRow,
    column: &str,
    field: &'static str,
) -> Result<T, StorageError> {
    let value: String = row.try_get(column)?;
    serde_json::from_value(Value::String(value)).map_err(|source| StorageError::Snapshot {
        aggregate: field,
        source,
    })
}
