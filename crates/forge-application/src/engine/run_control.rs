//! Core-owned project breaker and fenced Run-stop orchestration.

use super::{ActiveRun, CommandTransaction};
use crate::CommandEnvelope;
use forge_domain::{
    AggregateRef, CommandId, DomainEvent, DomainEventKind, Project, TaskId, TaskWaitCondition,
    TaskWaitKind, Timestamp, WaitConditionId,
};
use forge_protocol::wire::CommandReceipt;
use serde_json::json;

use super::{
    CommandError, Engine,
    event::{event, event_payload},
    receipt::{finish_command, resource},
    task_support::{load_scoped_task, persist_task_and_project, retained_persistence, task_event},
};

const PROJECT_STOP_WAIT_DETAIL: &str = "project_execution_stopped";

impl Engine<'_> {
    #[allow(clippy::too_many_arguments)]
    pub async fn stop_project_execution(
        &self,
        transaction: &mut impl CommandTransaction,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
        reason: Option<String>,
    ) -> Result<CommandReceipt, CommandError> {
        project.stop_execution(now)?;
        let prior_project_revision =
            project
                .revision()
                .checked_sub(1)
                .ok_or(CommandError::InvalidTransport {
                    field: "project.revision",
                    reason: "cannot persist a zero revision".to_owned(),
                })?;
        transaction
            .update_project(&project, prior_project_revision)
            .await?;
        // Queued work is deliberately retained. The durable Project execution
        // gate prevents new claims while stopped; reopening it can reconcile
        // the same queue snapshots without inventing replacement Tasks.
        let active_runs = transaction
            .lock_active_runs_for_project(project.id())
            .await?;
        let stopped_runs = request_graceful_stops(transaction, &active_runs).await?;
        let mut events = pause_active_tasks(
            transaction,
            &mut project,
            &stopped_runs,
            self.actors.core,
            command_id,
            now,
        )
        .await?;
        for run in &stopped_runs {
            events.push(run_stop_event(
                &project,
                run,
                command_id,
                self.actors.core,
                "project_execution_stopped",
                now,
            )?);
        }
        let project_event = event(
            project.id(),
            AggregateRef::Project(project.id()),
            project.revision(),
            DomainEventKind::ProjectExecutionStopped,
            self.actors.human,
            command_id,
            reason,
            event_payload([]),
            now,
        )?;
        events.push(project_event);
        finish_command(
            transaction,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            events,
            Some(resource("project", project.id().as_uuid())),
        )
        .await
    }

    /// Requests stop for every active Run on an explicitly cancelled Task.
    /// The caller owns Task terminalization in the same transaction.
    pub async fn stop_active_task_runs(
        &self,
        transaction: &mut impl CommandTransaction,
        project_id: forge_domain::ProjectId,
        task_id: TaskId,
    ) -> Result<Vec<ActiveRun>, CommandError> {
        let active_runs = transaction
            .lock_active_runs_for_task(project_id, task_id)
            .await?;
        request_graceful_stops(transaction, &active_runs).await
    }
}

async fn request_graceful_stops(
    transaction: &mut impl CommandTransaction,
    runs: &[ActiveRun],
) -> Result<Vec<ActiveRun>, CommandError> {
    let mut stopped = Vec::new();
    for run in runs {
        if transaction
            .request_run_stop(
                run.id,
                run.lease_fencing_token,
                run.environment_epoch,
                false,
            )
            .await?
        {
            let _ = transaction
                .cancel_leased_queue_for_fenced_run(
                    run.id,
                    run.lease_fencing_token,
                    run.environment_epoch,
                )
                .await?;
            stopped.push(run.clone());
        }
    }
    Ok(stopped)
}

async fn pause_active_tasks(
    transaction: &mut impl CommandTransaction,
    project: &mut Project,
    runs: &[ActiveRun],
    actor: forge_domain::Actor,
    command_id: CommandId,
    now: Timestamp,
) -> Result<Vec<DomainEvent>, CommandError> {
    let mut events = Vec::new();
    for run in runs {
        let stored = load_scoped_task(transaction, project, run.task_id).await?;
        if stored.task.lifecycle().is_terminal()
            || stored.task.wait_conditions().any(|condition| {
                condition.kind() == &TaskWaitKind::ManualPause
                    && condition.detail() == Some(PROJECT_STOP_WAIT_DETAIL)
            })
        {
            continue;
        }
        let previous_task_revision = stored.task.revision().get();
        let mut task = stored.task;
        task.add_wait_condition(
            TaskWaitCondition::new(
                WaitConditionId::new(),
                TaskWaitKind::ManualPause,
                Some(PROJECT_STOP_WAIT_DETAIL.to_owned()),
                actor,
                now,
            )?,
            now,
        )?;
        let persistence = retained_persistence(project, &task, stored.persistence)?;
        persist_task_and_project(
            transaction,
            project,
            &task,
            persistence,
            previous_task_revision,
            now,
        )
        .await?;
        events.push(task_event(
            project,
            &task,
            DomainEventKind::TaskWaiting,
            actor,
            command_id,
            now,
            event_payload([("reason", json!(PROJECT_STOP_WAIT_DETAIL))]),
        )?);
    }
    Ok(events)
}

pub fn run_stop_event(
    project: &Project,
    run: &ActiveRun,
    command_id: CommandId,
    actor: forge_domain::Actor,
    reason_code: &str,
    now: Timestamp,
) -> Result<DomainEvent, CommandError> {
    event(
        project.id(),
        AggregateRef::Project(project.id()),
        project.revision(),
        DomainEventKind::RunStopRequested,
        actor,
        command_id,
        Some(reason_code.to_owned()),
        event_payload([
            ("run_id", json!(run.id)),
            ("lease_fencing_token", json!(run.lease_fencing_token)),
            ("environment_epoch", json!(run.environment_epoch)),
            ("reason_code", json!(reason_code)),
        ]),
        now,
    )
    .map_err(Into::into)
}
