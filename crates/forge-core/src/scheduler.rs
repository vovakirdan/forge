//! Deterministic queue inputs and M0 dispatch primitives.

use forge_domain::{ExecutorKind, LifecycleStatus, Project, Task, Timestamp};
use forge_storage::{QueueEntryInput, StorageTransaction, TaskPersistence};
use serde_json::json;

use crate::CoreError;

/// Derives the normalized scheduler projection from canonical Task and Project state.
pub(crate) fn task_persistence(
    project: &Project,
    task: &Task,
    attempt_count: u32,
) -> Result<TaskPersistence, CoreError> {
    let level = project
        .priority_scheme()
        .get(task.priority_level_id())
        .ok_or_else(|| CoreError::InvalidTransport {
            field: "task.priority_level_id",
            reason: "is absent from the owning project priority scheme".to_owned(),
        })?;
    Ok(TaskPersistence {
        priority_rank: level.rank(),
        attempt_count,
        task_work_surface_id: None,
    })
}

/// Builds a deduplicated queue entry for a current Employee-owned stage.
pub(crate) fn queue_input(
    project: &Project,
    task: &Task,
    persistence: &TaskPersistence,
    eligible_at: Timestamp,
) -> Result<QueueEntryInput, CoreError> {
    let stage_id = task
        .current_stage_id()
        .cloned()
        .ok_or_else(|| CoreError::InvalidTransport {
            field: "task.current_stage_id",
            reason: "is required before queueing an employee stage".to_owned(),
        })?;
    let attempt_number =
        persistence
            .attempt_count
            .checked_add(1)
            .ok_or_else(|| CoreError::InvalidTransport {
                field: "task.attempt_count",
                reason: "cannot exceed u32::MAX".to_owned(),
            })?;
    Ok(QueueEntryInput {
        project_id: project.id(),
        task_id: task.id(),
        task_sequence: task.key().sequence().get(),
        pipeline_id: task.pipeline().pipeline_id(),
        pipeline_version_id: task.pipeline().pipeline_version_id(),
        stage_id,
        task_revision: task.revision().get(),
        attempt_number,
        priority_level_id: task.priority_level_id().as_str().to_owned(),
        priority_rank: persistence.priority_rank,
        resource_profile: json!({}),
        eligible_at,
    })
}

/// Inserts the current Task stage into the queue only when Pipeline assigns it
/// to an Employee. Human/external stages already carry a domain wait instead.
pub(crate) async fn enqueue_if_employee(
    transaction: &mut StorageTransaction<'_>,
    project: &Project,
    task: &Task,
    persistence: &TaskPersistence,
    executor_kind: ExecutorKind,
    eligible_at: Timestamp,
) -> Result<(), CoreError> {
    if executor_kind == ExecutorKind::Employee
        && matches!(
            task.lifecycle(),
            LifecycleStatus::Ready | LifecycleStatus::InProgress
        )
    {
        let input = queue_input(project, task, persistence, eligible_at)?;
        let _ = transaction.enqueue(&input).await?;
    }
    Ok(())
}
