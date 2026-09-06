//! Core reconciliation between one canonical dependency edge and Task waits.

use super::{CommandTransaction, StoredTask};
use forge_domain::{
    CommandId, DomainEvent, DomainEventKind, LifecycleStatus, Project, Task, TaskId,
    TaskWaitCondition, TaskWaitKind, Timestamp, WaitConditionId,
};
use serde_json::json;

use super::{
    CommandError, Engine,
    event::event_payload,
    run_control::run_stop_event,
    scheduler::enqueue_if_employee,
    task_support::{
        current_stage_executor, load_scoped_task, persist_task_and_project,
        pinned_pipeline_version, retained_persistence, task_event,
    },
};

const DEPENDENCY_WAIT_PREFIX: &str = "blocker_task_id=";

/// Adds one typed wait for every unsatisfied `task_done` dependency.
///
/// The caller owns persistence because it may be composing this with another
/// Task transition, such as approval or a stage outcome.
pub async fn add_unsatisfied_dependency_waits(
    transaction: &mut impl CommandTransaction,
    project: &Project,
    task: &mut Task,
    actor: forge_domain::Actor,
    now: Timestamp,
) -> Result<bool, CommandError> {
    if matches!(
        task.lifecycle(),
        LifecycleStatus::Draft | LifecycleStatus::Done | LifecycleStatus::Cancelled
    ) {
        return Ok(false);
    }

    let dependencies = transaction.list_dependencies(project.id()).await?;
    let blockers = dependencies
        .into_iter()
        .filter(|dependency| dependency.blocked_task_id() == task.id())
        .map(|dependency| dependency.blocker_task_id())
        .collect::<Vec<_>>();
    let mut added = false;
    for blocker_task_id in blockers {
        let blocker = transaction
            .lock_task(blocker_task_id)
            .await?
            .ok_or(CommandError::NotFound { aggregate: "task" })?;
        if blocker.task.project_id() != project.id() {
            return Err(CommandError::InvalidTransport {
                field: "task_dependency.blocker_task_id",
                reason: "does not belong to the command project".to_owned(),
            });
        }
        if blocker.task.lifecycle() == LifecycleStatus::Done
            || has_dependency_wait(task, blocker_task_id)
        {
            continue;
        }
        task.add_wait_condition(
            TaskWaitCondition::new(
                WaitConditionId::new(),
                TaskWaitKind::Dependency,
                Some(dependency_wait_detail(blocker_task_id)),
                actor,
                now,
            )?,
            now,
        )?;
        added = true;
    }
    Ok(added)
}

impl Engine<'_> {
    /// Makes a live blocked Task visibly wait after a dependency was added or
    /// its blocker was cancelled. Any active Run receives a fenced stop in the
    /// same canonical transaction.
    pub async fn wait_for_unsatisfied_dependencies(
        &self,
        transaction: &mut impl CommandTransaction,
        project: &mut Project,
        task_id: TaskId,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<Vec<DomainEvent>, CommandError> {
        let stored = load_scoped_task(transaction, project, task_id).await?;
        let was_in_progress = stored.task.lifecycle() == LifecycleStatus::InProgress;
        let previous_revision = stored.task.revision().get();
        let mut task = stored.task;
        if !add_unsatisfied_dependency_waits(transaction, project, &mut task, self.actors.core, now)
            .await?
        {
            return Ok(Vec::new());
        }
        let stopped_runs = if was_in_progress {
            self.stop_active_task_runs(transaction, project.id(), task.id())
                .await?
        } else {
            Vec::new()
        };
        let persistence = retained_persistence(project, &task, stored.persistence)?;
        persist_task_and_project(
            transaction,
            project,
            &task,
            persistence,
            previous_revision,
            now,
        )
        .await?;
        let mut events = vec![task_event(
            project,
            &task,
            DomainEventKind::TaskWaiting,
            self.actors.core,
            command_id,
            now,
            event_payload([("reason", json!("dependency_unsatisfied"))]),
        )?];
        for run in &stopped_runs {
            events.push(run_stop_event(
                project,
                run,
                command_id,
                self.actors.core,
                "dependency_unsatisfied",
                now,
            )?);
        }
        Ok(events)
    }

    /// Resolves waits held by one dependency once its blocker is `done`.
    pub async fn resolve_dependency_waits_after_completion(
        &self,
        transaction: &mut impl CommandTransaction,
        project: &mut Project,
        blocker_task_id: TaskId,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<Vec<DomainEvent>, CommandError> {
        let blocked_task_ids = transaction
            .list_dependencies(project.id())
            .await?
            .into_iter()
            .filter(|dependency| dependency.blocker_task_id() == blocker_task_id)
            .map(|dependency| dependency.blocked_task_id())
            .collect::<Vec<_>>();
        let mut events = Vec::new();
        for blocked_task_id in blocked_task_ids {
            events.extend(
                self.resolve_dependency_wait_for_blocker(
                    transaction,
                    project,
                    blocked_task_id,
                    blocker_task_id,
                    command_id,
                    now,
                )
                .await?,
            );
        }
        Ok(events)
    }

    /// Materializes visible dependency waits for every mutable dependent when a
    /// blocker cannot satisfy `task_done` (currently, when it is cancelled).
    pub async fn wait_dependents_for_unsatisfied_blocker(
        &self,
        transaction: &mut impl CommandTransaction,
        project: &mut Project,
        blocker_task_id: TaskId,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<Vec<DomainEvent>, CommandError> {
        let blocked_task_ids = transaction
            .list_dependencies(project.id())
            .await?
            .into_iter()
            .filter(|dependency| dependency.blocker_task_id() == blocker_task_id)
            .map(|dependency| dependency.blocked_task_id())
            .collect::<Vec<_>>();
        let mut events = Vec::new();
        for blocked_task_id in blocked_task_ids {
            events.extend(
                self.wait_for_unsatisfied_dependencies(
                    transaction,
                    project,
                    blocked_task_id,
                    command_id,
                    now,
                )
                .await?,
            );
        }
        Ok(events)
    }

    pub async fn resolve_dependency_wait_for_blocker(
        &self,
        transaction: &mut impl CommandTransaction,
        project: &mut Project,
        blocked_task_id: TaskId,
        blocker_task_id: TaskId,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<Vec<DomainEvent>, CommandError> {
        let stored = load_scoped_task(transaction, project, blocked_task_id).await?;
        let wait_ids = dependency_wait_ids(&stored, blocker_task_id);
        if wait_ids.is_empty() {
            return Ok(Vec::new());
        }
        let previous_revision = stored.task.revision().get();
        let mut task = stored.task;
        let mut resumed = false;
        for wait_id in wait_ids {
            resumed |= task.resolve_wait_condition(wait_id, now)?;
        }
        let persistence = retained_persistence(project, &task, stored.persistence)?;
        persist_task_and_project(
            transaction,
            project,
            &task,
            persistence,
            previous_revision,
            now,
        )
        .await?;
        if resumed {
            let version = pinned_pipeline_version(transaction, project, &task).await?;
            enqueue_if_employee(
                transaction,
                project,
                &task,
                &persistence,
                current_stage_executor(&version, &task)?,
                self.clock.now(),
            )
            .await?;
        }
        Ok(vec![task_event(
            project,
            &task,
            if resumed {
                DomainEventKind::TaskResumed
            } else {
                DomainEventKind::TaskWaiting
            },
            self.actors.core,
            command_id,
            now,
            event_payload([("blocker_task_id", json!(blocker_task_id.as_uuid()))]),
        )?])
    }
}

fn dependency_wait_ids(stored: &StoredTask, blocker_task_id: TaskId) -> Vec<WaitConditionId> {
    stored
        .task
        .wait_conditions()
        .filter(|condition| is_dependency_wait(condition, blocker_task_id))
        .map(TaskWaitCondition::id)
        .collect()
}

fn has_dependency_wait(task: &Task, blocker_task_id: TaskId) -> bool {
    task.wait_conditions()
        .any(|condition| is_dependency_wait(condition, blocker_task_id))
}

fn is_dependency_wait(condition: &TaskWaitCondition, blocker_task_id: TaskId) -> bool {
    condition.kind() == &TaskWaitKind::Dependency
        && condition.detail() == Some(dependency_wait_detail(blocker_task_id).as_str())
}

fn dependency_wait_detail(blocker_task_id: TaskId) -> String {
    format!("{DEPENDENCY_WAIT_PREFIX}{blocker_task_id}")
}

#[cfg(test)]
mod tests {
    use super::{DEPENDENCY_WAIT_PREFIX, dependency_wait_detail};

    #[test]
    fn dependency_wait_detail_is_stable_and_human_readable() {
        let id = forge_domain::TaskId::new();

        let detail = dependency_wait_detail(id);

        assert!(detail.starts_with(DEPENDENCY_WAIT_PREFIX));
        assert!(detail.ends_with(&id.to_string()));
    }
}
