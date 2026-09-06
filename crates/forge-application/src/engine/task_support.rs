//! Shared Core helpers for safely mutating a Task under the Project lock.

use super::{CommandTransaction, StoredTask, TaskPersistence};
use forge_domain::{
    AggregateRef, CommandId, DomainEvent, DomainEventKind, ExecutorKind, PipelineVersion, Project,
    Task, TaskId, TaskWaitCondition, Timestamp,
};
use serde_json::json;

use super::{
    CommandError,
    event::{event, event_payload},
    pipeline_access::ensure_pinned_pipeline_valid,
    scheduler::task_persistence,
};

pub async fn load_scoped_task(
    transaction: &mut impl CommandTransaction,
    project: &Project,
    task_id: TaskId,
) -> Result<StoredTask, CommandError> {
    let stored = transaction
        .lock_task(task_id)
        .await?
        .ok_or(CommandError::NotFound { aggregate: "task" })?;
    if stored.task.project_id() != project.id() {
        return Err(CommandError::InvalidTransport {
            field: "task_id",
            reason: "does not belong to the command project".to_owned(),
        });
    }
    Ok(stored)
}

pub fn require_task_revision(task: &Task, expected: u64) -> Result<(), CommandError> {
    if task.revision().get() == expected {
        Ok(())
    } else {
        Err(CommandError::InvalidTransport {
            field: "expected_task_revision",
            reason: "does not match the current task revision".to_owned(),
        })
    }
}

pub async fn pinned_pipeline_version(
    transaction: &mut impl CommandTransaction,
    project: &Project,
    task: &Task,
) -> Result<PipelineVersion, CommandError> {
    let version = transaction
        .lock_pipeline_version(task.pipeline().pipeline_version_id())
        .await?
        .ok_or(CommandError::NotFound {
            aggregate: "pinned pipeline version",
        })?;
    let pipeline = transaction
        .lock_pipeline(task.pipeline().pipeline_id())
        .await?
        .ok_or(CommandError::NotFound {
            aggregate: "pinned pipeline",
        })?;
    ensure_pinned_pipeline_valid(project, &pipeline, &version)?;
    Ok(version)
}

pub fn current_stage_executor(
    version: &PipelineVersion,
    task: &Task,
) -> Result<ExecutorKind, CommandError> {
    let stage_id = task
        .current_stage_id()
        .ok_or(CommandError::InvalidTransport {
            field: "task.current_stage_id",
            reason: "is absent for an active task".to_owned(),
        })?;
    version
        .stage(stage_id)
        .map(|stage| stage.executor_kind())
        .ok_or(CommandError::InvalidTransport {
            field: "task.current_stage_id",
            reason: "is absent from the pinned pipeline version".to_owned(),
        })
}

pub fn stage_wait_for_current_task(
    task: &Task,
    executor_kind: ExecutorKind,
    actor: forge_domain::Actor,
    now: Timestamp,
) -> Result<Option<TaskWaitCondition>, CommandError> {
    if !executor_kind.requires_wait() {
        return Ok(None);
    }
    let stage_id = task
        .current_stage_id()
        .cloned()
        .ok_or(CommandError::InvalidTransport {
            field: "task.current_stage_id",
            reason: "is absent after task approval".to_owned(),
        })?;
    let stage_visit = task
        .current_stage_visit()
        .ok_or(CommandError::InvalidTransport {
            field: "task.current_stage_visit",
            reason: "is absent after task approval".to_owned(),
        })?;
    Ok(Some(TaskWaitCondition::for_stage(
        forge_domain::WaitConditionId::new(),
        stage_id,
        stage_visit,
        None,
        actor,
        now,
    )?))
}

pub fn retained_persistence(
    project: &Project,
    task: &Task,
    existing: TaskPersistence,
) -> Result<TaskPersistence, CommandError> {
    let mut persistence = task_persistence(project, task, existing.attempt_count)?;
    persistence.task_work_surface_id = existing.task_work_surface_id;
    Ok(persistence)
}

pub async fn persist_task_and_project(
    transaction: &mut impl CommandTransaction,
    project: &mut Project,
    task: &Task,
    persistence: TaskPersistence,
    expected_task_revision: u64,
    now: Timestamp,
) -> Result<(), CommandError> {
    let expected_project_revision = project.revision();
    project.record_child_mutation(now)?;
    // A Task revision makes every queued snapshot of its former state stale.
    // Leasing is intentionally excluded: an active Run must be stopped and
    // terminally observed through the fenced runtime path instead.
    transaction
        .invalidate_queued_entries_for_task(project.id(), task.id())
        .await?;
    transaction
        .update_task(task, persistence, expected_task_revision)
        .await?;
    transaction
        .update_project(project, expected_project_revision)
        .await?;
    Ok(())
}

pub fn task_event(
    project: &Project,
    task: &Task,
    kind: DomainEventKind,
    actor: forge_domain::Actor,
    command_id: CommandId,
    now: Timestamp,
    payload: serde_json::Map<String, serde_json::Value>,
) -> Result<DomainEvent, CommandError> {
    Ok(event(
        project.id(),
        AggregateRef::Task(task.id()),
        task.revision().get(),
        kind,
        actor,
        command_id,
        None,
        payload,
        now,
    )?)
}

pub fn stage_payload(task: &Task) -> serde_json::Map<String, serde_json::Value> {
    event_payload([
        (
            "stage_id",
            json!(task.current_stage_id().map(|stage| stage.as_str())),
        ),
        (
            "stage_visit",
            json!(task.current_stage_visit().map(|visit| visit.get())),
        ),
    ])
}

pub fn reject_unimplemented_system_stage(executor_kind: ExecutorKind) -> Result<(), CommandError> {
    if executor_kind == ExecutorKind::System {
        return Err(CommandError::InvalidTransport {
            field: "pipeline.stage.executor_kind",
            reason: "system stages are not implemented in M0".to_owned(),
        });
    }
    Ok(())
}
