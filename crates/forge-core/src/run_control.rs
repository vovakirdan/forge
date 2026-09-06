//! Core-owned project breaker and fenced Run-stop orchestration.

use forge_application::CommandEnvelope;
use forge_domain::{
    AggregateRef, CommandId, DomainEvent, DomainEventKind, Project, TaskId, TaskWaitCondition,
    TaskWaitKind, Timestamp, WaitConditionId,
};
use forge_protocol::supervisor::v1::{CoreToSupervisor, StopMode, StopRun, core_to_supervisor};
use forge_protocol::wire::CommandReceipt;
use forge_storage::{FencedWrite, RunDesiredState, RunProjection, StorageTransaction};
use serde_json::json;
use uuid::Uuid;

use crate::{
    CoreError, CoreService,
    command::{finish_command, resource},
    event::{event, event_payload},
    task_support::{load_scoped_task, persist_task_and_project, retained_persistence, task_event},
};

const PROJECT_STOP_WAIT_DETAIL: &str = "project_execution_stopped";

impl CoreService {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn stop_project_execution(
        &self,
        transaction: &mut StorageTransaction<'_>,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
        reason: Option<String>,
    ) -> Result<CommandReceipt, CoreError> {
        project.stop_execution(now)?;
        let prior_project_revision =
            project
                .revision()
                .checked_sub(1)
                .ok_or(CoreError::InvalidTransport {
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
    pub(crate) async fn stop_active_task_runs(
        &self,
        transaction: &mut StorageTransaction<'_>,
        project_id: forge_domain::ProjectId,
        task_id: TaskId,
    ) -> Result<Vec<RunProjection>, CoreError> {
        let active_runs = transaction
            .lock_active_runs_for_task(project_id, task_id)
            .await?;
        request_graceful_stops(transaction, &active_runs).await
    }

    /// Replays committed desired stops after the transaction boundary. Sending
    /// is idempotent for a Run fence and never writes canonical state.
    pub(crate) async fn deliver_pending_stop_requests(
        &self,
        project_id: forge_domain::ProjectId,
    ) -> Result<usize, CoreError> {
        let mut transaction = self.store.begin().await?;
        let runs = transaction.lock_active_runs_for_project(project_id).await?;
        transaction.commit().await?;
        let mut delivered = 0_usize;
        for run in runs {
            let Some(stop) = stop_message(&run) else {
                continue;
            };
            self.supervisor.send(stop).await?;
            delivered = delivered.saturating_add(1);
        }
        Ok(delivered)
    }
}

async fn request_graceful_stops(
    transaction: &mut StorageTransaction<'_>,
    runs: &[RunProjection],
) -> Result<Vec<RunProjection>, CoreError> {
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
            == FencedWrite::Applied
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
    transaction: &mut StorageTransaction<'_>,
    project: &mut Project,
    runs: &[RunProjection],
    actor: forge_domain::Actor,
    command_id: CommandId,
    now: Timestamp,
) -> Result<Vec<DomainEvent>, CoreError> {
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

pub(crate) fn run_stop_event(
    project: &Project,
    run: &RunProjection,
    command_id: CommandId,
    actor: forge_domain::Actor,
    reason_code: &str,
    now: Timestamp,
) -> Result<DomainEvent, CoreError> {
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

fn stop_message(run: &RunProjection) -> Option<CoreToSupervisor> {
    let mode = match run.desired_state {
        RunDesiredState::StopRequested => StopMode::Graceful,
        RunDesiredState::ForceStopRequested => StopMode::Force,
        _ => return None,
    };
    Some(CoreToSupervisor {
        message: Some(core_to_supervisor::Message::StopRun(StopRun {
            command_id: Uuid::now_v7().to_string(),
            run_id: run.id.to_string(),
            lease_fencing_token: run.lease_fencing_token,
            environment_epoch: run.environment_epoch,
            mode: mode as i32,
            reason_code: "core_stop_requested".to_owned(),
            grace_period_ms: run
                .run_spec
                .pointer("/binding/limits/stop_grace_seconds")
                .and_then(serde_json::Value::as_u64)
                .map(|seconds| seconds.saturating_mul(1_000))
                .unwrap_or(2_000),
        })),
    })
}

#[cfg(test)]
mod tests {
    use forge_storage::{RunDesiredState, RunObservedState, RunProjection};
    use serde_json::json;

    use super::stop_message;

    #[test]
    fn pending_graceful_stop_becomes_a_fenced_transport_request() {
        let run = RunProjection {
            id: uuid::Uuid::now_v7(),
            project_id: forge_domain::ProjectId::new(),
            task_id: forge_domain::TaskId::new(),
            queue_entry_id: uuid::Uuid::now_v7(),
            lease_id: uuid::Uuid::now_v7(),
            employee_id: forge_domain::EmployeeId::new(),
            stage_id: "work".to_owned(),
            attempt_number: 1,
            lease_fencing_token: 7,
            environment_epoch: 2,
            last_sequence: 1,
            desired_state: RunDesiredState::StopRequested,
            observed_state: RunObservedState::Running,
            run_spec_version: 1,
            run_spec: json!({}),
            context_manifest: json!({}),
            observed_details: json!({}),
        };

        let message = stop_message(&run);

        assert!(message.is_some());
    }
}
