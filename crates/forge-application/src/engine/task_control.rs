//! Resume, priority, and dependency mutations outside draft/approval flow.

use super::CommandTransaction;
use crate::CommandEnvelope;
use forge_domain::{
    AggregateRef, CommandId, DomainEvent, DomainEventKind, LifecycleStatus, Project,
    TaskDependency, TaskDependencyGraph, TaskId, Timestamp, WaitConditionId,
};
use serde_json::json;

use super::{
    CommandError, Engine,
    event::{event, event_payload},
    receipt::{finish_command, resource},
    scheduler::enqueue_if_employee,
    task_support::{
        current_stage_executor, load_scoped_task, persist_task_and_project,
        pinned_pipeline_version, reject_unimplemented_system_stage, require_task_revision,
        retained_persistence, task_event,
    },
};

impl Engine<'_> {
    #[allow(clippy::too_many_arguments)]
    pub async fn resume_task(
        &self,
        transaction: &mut impl CommandTransaction,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
        task_id: TaskId,
        expected_task_revision: u64,
        wait_condition_id: WaitConditionId,
    ) -> Result<forge_protocol::wire::CommandReceipt, CommandError> {
        let stored = load_scoped_task(transaction, &project, task_id).await?;
        require_task_revision(&stored.task, expected_task_revision)?;
        let previous_revision = stored.task.revision().get();
        let mut task = stored.task;
        let resumed = task.resolve_wait_condition(wait_condition_id, now)?;
        let persistence = retained_persistence(&project, &task, stored.persistence)?;
        persist_task_and_project(
            transaction,
            &mut project,
            &task,
            persistence,
            previous_revision,
            now,
        )
        .await?;
        if resumed && task.lifecycle() == LifecycleStatus::InProgress {
            let version = pinned_pipeline_version(transaction, &project, &task).await?;
            let executor_kind = current_stage_executor(&version, &task)?;
            reject_unimplemented_system_stage(executor_kind)?;
            enqueue_if_employee(
                transaction,
                &project,
                &task,
                &persistence,
                executor_kind,
                self.clock.now(),
            )
            .await?;
        }
        let audit = task_event(
            &project,
            &task,
            if resumed {
                DomainEventKind::TaskResumed
            } else {
                DomainEventKind::TaskWaiting
            },
            self.actors.human,
            command_id,
            now,
            event_payload([("wait_condition_id", json!(wait_condition_id.as_uuid()))]),
        )?;
        finish_command(
            transaction,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            vec![audit],
            Some(resource("task", task.id().as_uuid())),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn set_task_priority(
        &self,
        transaction: &mut impl CommandTransaction,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
        task_id: TaskId,
        expected_task_revision: u64,
        priority_level_id: forge_domain::PriorityLevelId,
    ) -> Result<forge_protocol::wire::CommandReceipt, CommandError> {
        let stored = load_scoped_task(transaction, &project, task_id).await?;
        require_task_revision(&stored.task, expected_task_revision)?;
        // `execute_command` already holds the Project gate, so no dispatcher
        // or Supervisor reconciliation can create/release this Task's Lease
        // concurrently. A leased Run owns the sole active queue entry; keep
        // that snapshot until its fenced terminal observation. Its eventual
        // next stage/retry will inherit the newly persisted priority.
        let has_active_run = !transaction
            .lock_active_runs_for_task(project.id(), task_id)
            .await?
            .is_empty();
        let previous_revision = stored.task.revision().get();
        let mut task = stored.task;
        task.set_priority(project.priority_scheme(), priority_level_id.clone(), now)?;
        let persistence = retained_persistence(&project, &task, stored.persistence)?;
        persist_task_and_project(
            transaction,
            &mut project,
            &task,
            persistence,
            previous_revision,
            now,
        )
        .await?;
        if !has_active_run
            && matches!(
                task.lifecycle(),
                LifecycleStatus::Ready | LifecycleStatus::InProgress
            )
        {
            let version = pinned_pipeline_version(transaction, &project, &task).await?;
            enqueue_if_employee(
                transaction,
                &project,
                &task,
                &persistence,
                current_stage_executor(&version, &task)?,
                self.clock.now(),
            )
            .await?;
        }
        let audit = task_event(
            &project,
            &task,
            DomainEventKind::TaskPriorityChanged,
            self.actors.human,
            command_id,
            now,
            event_payload([("priority", json!(priority_level_id.as_str()))]),
        )?;
        finish_command(
            transaction,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            vec![audit],
            Some(resource("task", task.id().as_uuid())),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn change_dependency(
        &self,
        transaction: &mut impl CommandTransaction,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
        blocker_task_id: TaskId,
        blocked_task_id: TaskId,
        adding: bool,
    ) -> Result<forge_protocol::wire::CommandReceipt, CommandError> {
        let _ = load_scoped_task(transaction, &project, blocker_task_id).await?;
        let _ = load_scoped_task(transaction, &project, blocked_task_id).await?;
        let changed = if adding {
            let mut graph = TaskDependencyGraph::default();
            for dependency in transaction.list_dependencies(project.id()).await? {
                graph.add(dependency)?;
            }
            let dependency = TaskDependency::new(
                blocker_task_id,
                blocked_task_id,
                forge_domain::DependencyCondition::Done,
            )?;
            graph.add(dependency)?;
            transaction
                .insert_dependency(project.id(), dependency, self.actors.human)
                .await?
        } else {
            transaction
                .remove_dependency(project.id(), blocker_task_id, blocked_task_id)
                .await?
        };
        let mut dependent_events = Vec::<DomainEvent>::new();
        if changed {
            if adding {
                dependent_events.extend(
                    self.wait_for_unsatisfied_dependencies(
                        transaction,
                        &mut project,
                        blocked_task_id,
                        command_id,
                        now,
                    )
                    .await?,
                );
            } else {
                dependent_events.extend(
                    self.resolve_dependency_wait_for_blocker(
                        transaction,
                        &mut project,
                        blocked_task_id,
                        blocker_task_id,
                        command_id,
                        now,
                    )
                    .await?,
                );
            }
        }
        let mut events = Vec::<DomainEvent>::new();
        if changed {
            let previous_revision = project.revision();
            project.record_child_mutation(now)?;
            transaction
                .update_project(&project, previous_revision)
                .await?;
            events.push(event(
                project.id(),
                AggregateRef::Project(project.id()),
                project.revision(),
                if adding {
                    DomainEventKind::TaskDependencyCreated
                } else {
                    DomainEventKind::TaskDependencyRemoved
                },
                self.actors.human,
                command_id,
                None,
                event_payload([
                    ("blocker_task_id", json!(blocker_task_id.as_uuid())),
                    ("blocked_task_id", json!(blocked_task_id.as_uuid())),
                ]),
                now,
            )?);
        }
        events.extend(dependent_events);
        finish_command(
            transaction,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            events,
            Some(resource("task_dependency", blocked_task_id.as_uuid())),
        )
        .await
    }
}
