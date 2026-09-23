//! Aggregate, audit, and idempotency writes executed inside a Core transaction.

use forge_domain::{
    Actor, Employee, EmployeeState, Pipeline, PipelineVersion, Project, ProjectExecutionGate,
    ProjectId, Task, TaskDependency, TaskId,
};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use crate::task_write::{insert_task_row, update_task_row};
use crate::{
    IdempotencyRecord, StorageError, StorageTransaction, StoredEvent, TaskPersistence,
    database_timestamp, encode_object, encode_snapshot, encode_value, enum_text, i64_to_u64,
    u64_to_i64, value_object,
};

impl StorageTransaction<'_> {
    /// Must be called under the Project lock, which also serializes Task creation.
    pub async fn project_has_tasks(&mut self, project_id: ProjectId) -> Result<bool, StorageError> {
        Ok(
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tasks WHERE project_id=$1)")
                .bind(project_id.as_uuid())
                .fetch_one(&mut *self.transaction)
                .await?,
        )
    }

    /// Inserts a new Project snapshot and its scheduler-facing columns.
    pub async fn insert_project(&mut self, project: &Project) -> Result<(), StorageError> {
        let next = project.task_sequence().checked_add(1).ok_or_else(|| {
            StorageError::IntegerOutOfRange {
                field: "project.next_task_sequence",
            }
        })?;
        sqlx::query(
            "INSERT INTO projects (id, name, revision, next_task_sequence, execution_enabled, priority_scheme, cancellation_reasons, properties, canonical_snapshot, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7::jsonb, $8::jsonb, $9::jsonb, $10, $11)",
        )
        .bind(project.id().as_uuid())
        .bind(project.name())
        .bind(u64_to_i64(project.revision(), "project.revision")?)
        .bind(u64_to_i64(next, "project.next_task_sequence")?)
        .bind(project.execution_gate() == ProjectExecutionGate::Open)
        .bind(encode_object(project.priority_scheme(), "project.priority_scheme")?)
        .bind(cancellation_reasons_json(project)?)
        .bind(encode_object(project.property_schema(), "project.property_schema")?)
        .bind(encode_snapshot(project, "project")?)
        .bind(database_timestamp(project.created_at()))
        .bind(database_timestamp(project.updated_at()))
        .execute(&mut *self.transaction)
        .await?;
        Ok(())
    }

    /// Persists a Project against the revision Core observed before its mutations.
    pub async fn update_project(
        &mut self,
        project: &Project,
        expected_revision: u64,
    ) -> Result<(), StorageError> {
        if project.revision() <= expected_revision {
            return Err(StorageError::InvalidInput {
                reason: "updated project revision must advance beyond its loaded revision"
                    .to_owned(),
            });
        }
        let next = project.task_sequence().checked_add(1).ok_or_else(|| {
            StorageError::IntegerOutOfRange {
                field: "project.next_task_sequence",
            }
        })?;
        let result = sqlx::query(
            "UPDATE projects SET name = $2, revision = $3, next_task_sequence = $4, execution_enabled = $5, priority_scheme = $6::jsonb, cancellation_reasons = $7::jsonb, properties = $8::jsonb, canonical_snapshot = $9::jsonb WHERE id = $1 AND revision = $10",
        )
        .bind(project.id().as_uuid())
        .bind(project.name())
        .bind(u64_to_i64(project.revision(), "project.revision")?)
        .bind(u64_to_i64(next, "project.next_task_sequence")?)
        .bind(project.execution_gate() == ProjectExecutionGate::Open)
        .bind(encode_object(project.priority_scheme(), "project.priority_scheme")?)
        .bind(cancellation_reasons_json(project)?)
        .bind(encode_object(project.property_schema(), "project.property_schema")?)
        .bind(encode_snapshot(project, "project")?)
        .bind(u64_to_i64(expected_revision, "project.expected_revision")?)
        .execute(&mut *self.transaction)
        .await?;
        require_one(result.rows_affected(), "project")
    }

    /// Inserts an immutable Pipeline catalog version graph.
    pub async fn insert_pipeline_version(
        &mut self,
        version: &PipelineVersion,
    ) -> Result<(), StorageError> {
        let (definition, created_by, created_at) = pipeline_version_columns(version)?;
        sqlx::query(
            "INSERT INTO pipeline_versions (id, pipeline_id, version, definition, created_by, created_at, published_at) VALUES ($1, $2, $3, $4::jsonb, $5::jsonb, $6, $6)",
        )
        .bind(version.id().as_uuid())
        .bind(version.pipeline_id().as_uuid())
        .bind(i64::from(version.version()))
        .bind(definition)
        .bind(created_by)
        .bind(database_timestamp(created_at))
        .execute(&mut *self.transaction)
        .await?;
        Ok(())
    }

    /// Inserts a named Pipeline catalog row; the deferred default-version FK allows one transaction.
    pub async fn insert_pipeline(&mut self, pipeline: &Pipeline) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO pipelines (id, project_id, name, default_version_id, revision, deleted_at, canonical_snapshot, latest_version) VALUES ($1, $2, $3, $4, $7, $5, $6::jsonb, $8)",
        )
        .bind(pipeline.id().as_uuid())
        .bind(pipeline.project_id().as_uuid())
        .bind(pipeline.name())
        .bind(pipeline.default_version_id().as_uuid())
        .bind(pipeline.deleted_at().map(database_timestamp))
        .bind(encode_snapshot(pipeline, "pipeline")?)
        .bind(u64_to_i64(pipeline.revision(), "pipeline.revision")?)
        .bind(i64::from(pipeline.latest_version()))
        .execute(&mut *self.transaction)
        .await?;
        Ok(())
    }

    /// Persists mutable Pipeline catalog metadata without changing historical versions.
    pub async fn update_pipeline(
        &mut self,
        pipeline: &Pipeline,
        expected_revision: u64,
    ) -> Result<(), StorageError> {
        pipeline
            .validate_snapshot()
            .map_err(|error| StorageError::InvalidInput {
                reason: error.to_string(),
            })?;
        if expected_revision.checked_add(1) != Some(pipeline.revision()) {
            return Err(StorageError::InvalidInput {
                reason: "pipeline revision must advance exactly once".into(),
            });
        }
        let result = sqlx::query(
            "UPDATE pipelines SET name = $2, default_version_id = $3, deleted_at = $4, canonical_snapshot = $5::jsonb, revision = $6, latest_version=$7 WHERE id = $1 AND project_id=$8 AND revision=$9",
        )
        .bind(pipeline.id().as_uuid())
        .bind(pipeline.name())
        .bind(pipeline.default_version_id().as_uuid())
        .bind(pipeline.deleted_at().map(database_timestamp))
        .bind(encode_snapshot(pipeline, "pipeline")?)
        .bind(u64_to_i64(pipeline.revision(), "pipeline.revision")?)
        .bind(i64::from(pipeline.latest_version()))
        .bind(pipeline.project_id().as_uuid())
        .bind(u64_to_i64(expected_revision,"pipeline.expected_revision")?)
        .execute(&mut *self.transaction)
        .await?;
        if result.rows_affected() != 1 {
            return Err(StorageError::StaleRevision {
                aggregate: "pipeline",
            });
        }
        Ok(())
    }

    /// Inserts a canonical Employee snapshot and its simple dispatch indexes.
    pub async fn insert_employee(&mut self, employee: &Employee) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO employees (id, project_id, name, employee_state, roles, skills, stage_eligibility, provider_preferences, runtime_policy, properties, revision, canonical_snapshot, created_at, updated_at, max_concurrent_runs) VALUES ($1, $2, $3, $4, $5::jsonb, '[]'::jsonb, $6::jsonb, '{}'::jsonb, '{}'::jsonb, '{}'::jsonb, $10, $7::jsonb, $8, $9, $11)",
        )
        .bind(employee.id().as_uuid())
        .bind(employee.project_id().as_uuid())
        .bind(employee.name())
        .bind(employee_state_text(employee.state()))
        .bind(encode_value(&json!([employee.role().as_str()]), "employee.roles")?)
        .bind(encode_object(employee.stage_eligibility(), "employee.stage_eligibility")?)
        .bind(encode_snapshot(employee, "employee")?)
        .bind(database_timestamp(employee.created_at()))
        .bind(database_timestamp(employee.updated_at()))
        .bind(u64_to_i64(employee.revision(), "employee.revision")?)
        .bind(i32::from(employee.max_concurrent_runs()))
        .execute(&mut *self.transaction)
        .await?;
        Ok(())
    }

    /// Persists an exact next revision; a stale writer cannot overwrite newer configuration.
    pub async fn update_employee(
        &mut self,
        employee: &Employee,
        expected_revision: u64,
    ) -> Result<(), StorageError> {
        employee
            .validate_snapshot()
            .map_err(|error| StorageError::InvalidInput {
                reason: error.to_string(),
            })?;
        if expected_revision.checked_add(1) != Some(employee.revision()) {
            return Err(StorageError::InvalidInput {
                reason: "employee revision must advance exactly once".to_owned(),
            });
        }
        let result = sqlx::query(
            "UPDATE employees SET name = $2, employee_state = $3, roles = $4::jsonb, stage_eligibility = $5::jsonb, canonical_snapshot = $6::jsonb, revision = $7, updated_at=$8, max_concurrent_runs=$9 WHERE id = $1 AND project_id=$10 AND revision=$11",
        )
        .bind(employee.id().as_uuid())
        .bind(employee.name())
        .bind(employee_state_text(employee.state()))
        .bind(encode_value(&json!([employee.role().as_str()]), "employee.roles")?)
        .bind(encode_object(employee.stage_eligibility(), "employee.stage_eligibility")?)
        .bind(encode_snapshot(employee, "employee")?)
        .bind(u64_to_i64(employee.revision(), "employee.revision")?)
        .bind(database_timestamp(employee.updated_at()))
        .bind(i32::from(employee.max_concurrent_runs()))
        .bind(employee.project_id().as_uuid())
        .bind(u64_to_i64(expected_revision, "employee.expected_revision")?)
        .execute(&mut *self.transaction)
        .await?;
        if result.rows_affected() != 1 {
            return Err(StorageError::StaleRevision {
                aggregate: "employee",
            });
        }
        Ok(())
    }

    /// Inserts one typed Task snapshot and copies indexed fields required by scheduling.
    pub async fn insert_task(
        &mut self,
        task: &Task,
        persistence: TaskPersistence,
    ) -> Result<(), StorageError> {
        insert_task_row(&mut self.transaction, task, persistence).await
    }

    /// Updates a Task snapshot only after Core has applied a valid semantic transition.
    pub async fn update_task(
        &mut self,
        task: &Task,
        persistence: TaskPersistence,
        expected_revision: u64,
    ) -> Result<(), StorageError> {
        update_task_row(&mut self.transaction, task, persistence, expected_revision).await
    }

    /// Lists dependency edges in both directions through one canonical relation table.
    pub async fn list_dependencies(
        &mut self,
        project_id: ProjectId,
    ) -> Result<Vec<TaskDependency>, StorageError> {
        let rows = sqlx::query(
            "SELECT blocker_task_id, blocked_task_id, required_blocker_lifecycle FROM task_dependencies WHERE project_id = $1 ORDER BY blocker_task_id, blocked_task_id",
        )
        .bind(project_id.as_uuid())
        .fetch_all(&mut *self.transaction)
        .await?;
        rows.into_iter()
            .map(|row| {
                let lifecycle: String = row.try_get("required_blocker_lifecycle")?;
                if lifecycle != "done" {
                    return Err(StorageError::InvalidInput {
                        reason: "M0 task dependency has a non-done condition".to_owned(),
                    });
                }
                TaskDependency::new(
                    row.try_get::<Uuid, _>("blocker_task_id")?.into(),
                    row.try_get::<Uuid, _>("blocked_task_id")?.into(),
                    forge_domain::DependencyCondition::Done,
                )
                .map_err(|error| StorageError::InvalidInput {
                    reason: format!("stored task dependency is invalid: {error}"),
                })
            })
            .collect()
    }

    /// Inserts a graph edge after Core has proved it is acyclic.
    pub async fn insert_dependency(
        &mut self,
        project_id: ProjectId,
        dependency: TaskDependency,
        created_by: Actor,
    ) -> Result<bool, StorageError> {
        let result = sqlx::query(
            "INSERT INTO task_dependencies (project_id, blocker_task_id, blocked_task_id, required_blocker_lifecycle, created_by) VALUES ($1, $2, $3, 'done', $4::jsonb) ON CONFLICT (blocker_task_id, blocked_task_id) DO NOTHING",
        )
        .bind(project_id.as_uuid())
        .bind(dependency.blocker_task_id().as_uuid())
        .bind(dependency.blocked_task_id().as_uuid())
        .bind(encode_object(&created_by, "dependency.created_by")?)
        .execute(&mut *self.transaction)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Removes one directed dependency and reports whether it existed.
    pub async fn remove_dependency(
        &mut self,
        project_id: ProjectId,
        blocker_task_id: TaskId,
        blocked_task_id: TaskId,
    ) -> Result<bool, StorageError> {
        let result = sqlx::query(
            "DELETE FROM task_dependencies WHERE project_id = $1 AND blocker_task_id = $2 AND blocked_task_id = $3",
        )
        .bind(project_id.as_uuid())
        .bind(blocker_task_id.as_uuid())
        .bind(blocked_task_id.as_uuid())
        .execute(&mut *self.transaction)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Returns a stored receipt for caller retry processing.
    pub async fn lookup_idempotency(
        &mut self,
        project_id: ProjectId,
        key: &str,
    ) -> Result<Option<IdempotencyRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT project_id, idempotency_key, command_id, command_name, expected_revision, request_hash, actor::text AS actor, receipt::text AS receipt, event_id, response_revision FROM idempotency_keys WHERE project_id = $1 AND idempotency_key = $2",
        )
        .bind(project_id.as_uuid())
        .bind(key)
        .fetch_optional(&mut *self.transaction)
        .await?;
        row.map(idempotency_from_row).transpose()
    }

    /// Inserts the final receipt in the same transaction as all command effects.
    pub async fn insert_idempotency(
        &mut self,
        record: &IdempotencyRecord,
    ) -> Result<(), StorageError> {
        require_non_blank(&record.key, "idempotency key")?;
        require_non_blank(&record.command_name, "command name")?;
        require_non_blank(&record.request_hash, "request hash")?;
        let result = sqlx::query(
            "INSERT INTO idempotency_keys (project_id, idempotency_key, command_id, command_name, expected_revision, request_hash, actor, receipt, event_id, response_revision) VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb, $8::jsonb, $9, $10)",
        )
        .bind(record.project_id.as_uuid())
        .bind(&record.key)
        .bind(record.command_id.as_uuid())
        .bind(&record.command_name)
        .bind(record.expected_revision.map(|value| u64_to_i64(value, "idempotency.expected_revision")).transpose()?)
        .bind(&record.request_hash)
        .bind(encode_object(&record.actor, "idempotency.actor")?)
        .bind(encode_object(&record.receipt, "idempotency.receipt")?)
        .bind(record.event_id.map(|id| id.as_uuid()))
        .bind(record.response_revision.map(|value| u64_to_i64(value, "idempotency.response_revision")).transpose()?)
        .execute(&mut *self.transaction)
        .await?;
        require_one(result.rows_affected(), "idempotency key")
    }

    /// Appends an immutable Event and its pending outbox envelope under the Project row lock.
    pub async fn append_event_and_outbox(
        &mut self,
        event: &forge_domain::DomainEvent,
    ) -> Result<StoredEvent, StorageError> {
        let locked: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM projects WHERE id = $1 FOR UPDATE")
                .bind(event.project_id().as_uuid())
                .fetch_optional(&mut *self.transaction)
                .await?;
        if locked.is_none() {
            return Err(StorageError::NotFound {
                aggregate: "project",
            });
        }
        let last: Option<i64> = sqlx::query_scalar(
            "SELECT project_sequence FROM event_log WHERE project_id = $1 ORDER BY project_sequence DESC LIMIT 1",
        )
        .bind(event.project_id().as_uuid())
        .fetch_optional(&mut *self.transaction)
        .await?;
        let sequence = i64_to_u64(last.unwrap_or_default(), "event.project_sequence")?
            .checked_add(1)
            .ok_or_else(|| StorageError::IntegerOutOfRange {
                field: "event.project_sequence",
            })?;
        let stored = StoredEvent::from_domain(event, sequence)?;
        let result = sqlx::query(
            "INSERT INTO event_log (id, project_id, project_sequence, aggregate_type, aggregate_id, aggregate_revision, event_type, event_schema_version, actor, command_id, reason, payload, occurred_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::jsonb, $10, $11, $12::jsonb, $13)",
        )
        .bind(stored.id.as_uuid())
        .bind(stored.project_id.as_uuid())
        .bind(u64_to_i64(stored.project_sequence, "event.project_sequence")?)
        .bind(enum_text(&stored.aggregate_type, "event.aggregate_type")?)
        .bind(stored.aggregate_id)
        .bind(u64_to_i64(stored.aggregate_revision, "event.aggregate_revision")?)
        .bind(&stored.event_type)
        .bind(i16::try_from(stored.schema_version).map_err(|_| StorageError::IntegerOutOfRange { field: "event.schema_version" })?)
        .bind(encode_object(&stored.actor, "event.actor")?)
        .bind(stored.command_id.map(|id| id.as_uuid()))
        .bind(&stored.reason)
        .bind(encode_object(&stored.payload, "event.payload")?)
        .bind(database_timestamp(stored.occurred_at))
        .execute(&mut *self.transaction)
        .await?;
        require_one(result.rows_affected(), "event")?;
        let envelope = stored.envelope();
        sqlx::query(
            "INSERT INTO outbox (id, project_id, event_id, subject, envelope) VALUES ($1, $2, $3, $4, $5::jsonb)",
        )
        .bind(Uuid::now_v7())
        .bind(stored.project_id.as_uuid())
        .bind(stored.id.as_uuid())
        .bind(stored.subject())
        .bind(encode_object(&envelope, "outbox.envelope")?)
        .execute(&mut *self.transaction)
        .await?;
        Ok(stored)
    }
}

fn idempotency_from_row(row: sqlx::postgres::PgRow) -> Result<IdempotencyRecord, StorageError> {
    let expected_revision = row
        .try_get::<Option<i64>, _>("expected_revision")?
        .map(|value| i64_to_u64(value, "idempotency.expected_revision"))
        .transpose()?;
    let response_revision = row
        .try_get::<Option<i64>, _>("response_revision")?
        .map(|value| i64_to_u64(value, "idempotency.response_revision"))
        .transpose()?;
    Ok(IdempotencyRecord {
        project_id: row.try_get::<Uuid, _>("project_id")?.into(),
        key: row.try_get("idempotency_key")?,
        command_id: row.try_get::<Uuid, _>("command_id")?.into(),
        command_name: row.try_get("command_name")?,
        expected_revision,
        request_hash: row.try_get("request_hash")?,
        actor: json_from_text(row.try_get("actor")?, "idempotency.actor")?,
        receipt: json_from_text(row.try_get("receipt")?, "idempotency.receipt")?,
        event_id: row.try_get::<Option<Uuid>, _>("event_id")?.map(Into::into),
        response_revision,
    })
}

fn cancellation_reasons_json(project: &Project) -> Result<String, StorageError> {
    let catalog = value_object(
        project.cancellation_reasons(),
        "project.cancellation_reasons",
    )?;
    let values = catalog
        .get("reasons")
        .and_then(Value::as_object)
        .ok_or_else(|| StorageError::InvalidInput {
            reason: "project cancellation-reason snapshot has no reasons map".to_owned(),
        })?
        .values()
        .cloned()
        .collect::<Vec<_>>();
    encode_value(&Value::Array(values), "project.cancellation_reasons")
}

fn employee_state_text(state: EmployeeState) -> &'static str {
    match state {
        EmployeeState::Enabled => "active",
        EmployeeState::Disabled => "disabled",
        EmployeeState::Retired => "retired",
    }
}

fn pipeline_version_columns(
    version: &PipelineVersion,
) -> Result<(String, String, forge_domain::Timestamp), StorageError> {
    let definition = encode_snapshot(version, "pipeline_version")?;
    let value: Value =
        serde_json::from_str(&definition).map_err(|source| StorageError::Snapshot {
            aggregate: "pipeline_version",
            source,
        })?;
    let created_by = value
        .get("created_by")
        .ok_or_else(|| StorageError::InvalidInput {
            reason: "pipeline version snapshot has no created_by field".to_owned(),
        })?;
    let created_at =
        value
            .get("created_at")
            .cloned()
            .ok_or_else(|| StorageError::InvalidInput {
                reason: "pipeline version snapshot has no created_at field".to_owned(),
            })?;
    let created_at =
        serde_json::from_value(created_at).map_err(|source| StorageError::Snapshot {
            aggregate: "pipeline_version.created_at",
            source,
        })?;
    Ok((
        definition,
        encode_object(created_by, "pipeline_version.created_by")?,
        created_at,
    ))
}

fn json_from_text(value: String, field: &'static str) -> Result<Value, StorageError> {
    serde_json::from_str(&value).map_err(|source| StorageError::Snapshot {
        aggregate: field,
        source,
    })
}

fn require_one(rows: u64, aggregate: &'static str) -> Result<(), StorageError> {
    if rows == 1 {
        Ok(())
    } else {
        Err(StorageError::StaleRevision { aggregate })
    }
}

fn require_non_blank(value: &str, field: &str) -> Result<(), StorageError> {
    if value.trim().is_empty() {
        Err(StorageError::InvalidInput {
            reason: format!("{field} must not be blank"),
        })
    } else {
        Ok(())
    }
}
