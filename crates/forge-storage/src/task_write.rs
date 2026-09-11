//! Normalized Task projection writes derived from a Core-owned domain snapshot.

use forge_domain::{Task, TerminalData};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{
    StorageError, TaskPersistence, database_timestamp, encode_object, encode_snapshot,
    encode_value, enum_text, u32_to_i32, u64_to_i64,
};

pub(crate) async fn insert_task_row(
    transaction: &mut Transaction<'_, Postgres>,
    task: &Task,
    persistence: TaskPersistence,
) -> Result<(), StorageError> {
    let columns = TaskColumns::from_task(task, persistence)?;
    sqlx::query(
        "INSERT INTO tasks (id, project_id, task_sequence, task_key, title, description, definition_of_done, task_kind, properties, priority_level_id, priority_rank, lifecycle, pipeline_id, pipeline_version_id, current_stage_id, wait_conditions, resume_to_lifecycle, cancellation_reason_id, cancellation_note, attempt_count, task_work_surface_id, created_by, revision, canonical_snapshot, created_at, updated_at, project_repository_id, base_sha) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::jsonb, $10, $11, $12, $13, $14, $15, $16::jsonb, $17, $18, $19, $20, $21, $22::jsonb, $23, $24::jsonb, $25, $26, $27, $28)",
    )
    .bind(task.id().as_uuid())
    .bind(task.project_id().as_uuid())
    .bind(columns.sequence)
    .bind(columns.key)
    .bind(columns.title)
    .bind(columns.description)
    .bind(columns.definition_of_done)
    .bind(columns.kind)
    .bind(columns.properties)
    .bind(columns.priority_level_id)
    .bind(columns.priority_rank)
    .bind(columns.lifecycle)
    .bind(columns.pipeline_id)
    .bind(columns.pipeline_version_id)
    .bind(columns.current_stage_id)
    .bind(columns.wait_conditions)
    .bind(columns.resume_to_lifecycle)
    .bind(columns.cancellation_reason_id)
    .bind(columns.cancellation_note)
    .bind(columns.attempt_count)
    .bind(columns.task_work_surface_id)
    .bind(encode_object(&task.created_by(), "task.created_by")?)
    .bind(columns.revision)
    .bind(columns.snapshot)
    .bind(database_timestamp(task.created_at()))
    .bind(database_timestamp(task.updated_at()))
    .bind(columns.project_repository_id)
    .bind(columns.base_sha)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

pub(crate) async fn update_task_row(
    transaction: &mut Transaction<'_, Postgres>,
    task: &Task,
    persistence: TaskPersistence,
    expected_revision: u64,
) -> Result<(), StorageError> {
    if task.revision().get() <= expected_revision {
        return Err(StorageError::InvalidInput {
            reason: "updated task revision must advance beyond its loaded revision".to_owned(),
        });
    }
    let columns = TaskColumns::from_task(task, persistence)?;
    let result = sqlx::query(
        "UPDATE tasks SET title = $2, description = $3, definition_of_done = $4, task_kind = $5, properties = $6::jsonb, priority_level_id = $7, priority_rank = $8, lifecycle = $9, pipeline_id = $10, pipeline_version_id = $11, current_stage_id = $12, wait_conditions = $13::jsonb, resume_to_lifecycle = $14, cancellation_reason_id = $15, cancellation_note = $16, attempt_count = $17, task_work_surface_id = $18, revision = $19, canonical_snapshot = $20::jsonb, project_repository_id = $22, base_sha = $23 WHERE id = $1 AND revision = $21",
    )
    .bind(task.id().as_uuid())
    .bind(columns.title)
    .bind(columns.description)
    .bind(columns.definition_of_done)
    .bind(columns.kind)
    .bind(columns.properties)
    .bind(columns.priority_level_id)
    .bind(columns.priority_rank)
    .bind(columns.lifecycle)
    .bind(columns.pipeline_id)
    .bind(columns.pipeline_version_id)
    .bind(columns.current_stage_id)
    .bind(columns.wait_conditions)
    .bind(columns.resume_to_lifecycle)
    .bind(columns.cancellation_reason_id)
    .bind(columns.cancellation_note)
    .bind(columns.attempt_count)
    .bind(columns.task_work_surface_id)
    .bind(columns.revision)
    .bind(columns.snapshot)
    .bind(u64_to_i64(expected_revision, "task.expected_revision")?)
    .bind(columns.project_repository_id)
    .bind(columns.base_sha)
    .execute(&mut **transaction)
    .await?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(StorageError::StaleRevision { aggregate: "task" })
    }
}

struct TaskColumns {
    sequence: i64,
    key: String,
    title: String,
    description: String,
    definition_of_done: Option<String>,
    kind: String,
    properties: String,
    priority_level_id: String,
    priority_rank: i32,
    lifecycle: String,
    pipeline_id: Uuid,
    pipeline_version_id: Uuid,
    current_stage_id: Option<String>,
    wait_conditions: String,
    resume_to_lifecycle: Option<String>,
    cancellation_reason_id: Option<String>,
    cancellation_note: Option<String>,
    attempt_count: i32,
    task_work_surface_id: Option<Uuid>,
    project_repository_id: Option<Uuid>,
    base_sha: Option<String>,
    revision: i64,
    snapshot: String,
}

impl TaskColumns {
    fn from_task(task: &Task, persistence: TaskPersistence) -> Result<Self, StorageError> {
        let (project_repository_id, base_sha) = match task.work_surface() {
            forge_domain::TaskWorkSurface::None => (None, None),
            forge_domain::TaskWorkSurface::Git(binding) => {
                if persistence.task_work_surface_id != Some(binding.surface_id) {
                    return Err(StorageError::InvalidInput {
                        reason: "Task Git surface and scheduler projection disagree".into(),
                    });
                }
                (
                    Some(binding.repository_id),
                    binding
                        .initial_base
                        .commit()
                        .map(|commit| commit.as_str().to_owned()),
                )
            }
        };
        let waits =
            serde_json::to_value(task.wait_conditions().collect::<Vec<_>>()).map_err(|source| {
                StorageError::Snapshot {
                    aggregate: "task.wait_conditions",
                    source,
                }
            })?;
        let (cancellation_reason_id, cancellation_note) = match task.terminal_data() {
            Some(TerminalData::Cancelled(data)) => (
                Some(data.reason_id().as_str().to_owned()),
                data.note().map(ToOwned::to_owned),
            ),
            Some(TerminalData::Completed(_)) | None => (None, None),
        };
        Ok(Self {
            sequence: u64_to_i64(task.key().sequence().get(), "task.sequence")?,
            key: task.key().to_string(),
            title: task.spec().title().to_owned(),
            description: task.spec().description().to_owned(),
            definition_of_done: task.spec().definition_of_done().map(ToOwned::to_owned),
            kind: enum_text(&task.kind(), "task.kind")?,
            properties: encode_object(task.spec().properties(), "task.properties")?,
            priority_level_id: task.priority_level_id().as_str().to_owned(),
            priority_rank: persistence.priority_rank,
            lifecycle: task.lifecycle().to_string(),
            pipeline_id: task.pipeline().pipeline_id().as_uuid(),
            pipeline_version_id: task.pipeline().pipeline_version_id().as_uuid(),
            current_stage_id: task.current_stage_id().map(|id| id.as_str().to_owned()),
            wait_conditions: encode_value(&waits, "task.wait_conditions")?,
            resume_to_lifecycle: task
                .resume_to_lifecycle()
                .map(|value| enum_text(&value, "task.resume_to_lifecycle"))
                .transpose()?,
            cancellation_reason_id,
            cancellation_note,
            attempt_count: u32_to_i32(persistence.attempt_count, "task.attempt_count")?,
            task_work_surface_id: persistence.task_work_surface_id,
            project_repository_id,
            base_sha,
            revision: u64_to_i64(task.revision().get(), "task.revision")?,
            snapshot: encode_snapshot(task, "task")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{
        Actor, ActorId, CancellationReason, CancellationReasonCatalog, CancellationReasonId,
        CancellationRequest, NewTask, PipelineId, PipelineVersionId, PriorityScheme, ProjectId,
        ProjectLocalSequence, StageId, Task, TaskKey, TaskKind, TaskPipelineBinding,
        TaskProperties, TaskScope, TaskSource, TaskSpec, TaskSpecInput, Timestamp,
    };

    use super::{TaskColumns, TaskPersistence};

    #[test]
    fn cancelled_task_projects_its_reason_and_note() {
        let priorities = PriorityScheme::default_three_levels().expect("priority scheme");
        let now = Timestamp::now_utc();
        let mut task = Task::new_draft(
            NewTask {
                id: forge_domain::TaskId::new(),
                project_id: ProjectId::new(),
                key: TaskKey::new(ProjectLocalSequence::new(1).expect("sequence")),
                kind: TaskKind::Delivery,
                spec: TaskSpec::new(TaskSpecInput {
                    title: "M0 task".to_owned(),
                    description: String::new(),
                    rationale: None,
                    definition_of_done: None,
                    scope: TaskScope::default(),
                    properties: TaskProperties::empty(),
                    labels: Default::default(),
                })
                .expect("task spec"),
                priority_level_id: priorities.default_level_id().clone(),
                pipeline: TaskPipelineBinding::new(
                    PipelineId::new(),
                    PipelineVersionId::new(),
                    StageId::new("work").expect("stage"),
                ),
                source: TaskSource::Human,
                created_by: Actor::human(ActorId::new()),
                created_at: now,
            },
            &priorities,
        )
        .expect("draft task");
        let catalog = CancellationReasonCatalog::new([
            CancellationReason::unspecified().expect("fallback cancellation reason")
        ])
        .expect("cancellation catalog");
        task.cancel(
            &catalog,
            CancellationRequest {
                reason_id: Some(
                    CancellationReasonId::new(CancellationReasonId::UNSPECIFIED)
                        .expect("reason id"),
                ),
                note: Some("No longer needed".to_owned()),
                cancelled_by: Actor::human(ActorId::new()),
                cancelled_at: now,
            },
        )
        .expect("cancel task");

        let columns =
            TaskColumns::from_task(&task, TaskPersistence::default()).expect("task projection");
        assert_eq!(
            columns.cancellation_reason_id.as_deref(),
            Some("unspecified")
        );
        assert_eq!(
            columns.cancellation_note.as_deref(),
            Some("No longer needed")
        );
    }
}
