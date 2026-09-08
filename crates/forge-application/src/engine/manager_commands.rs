//! Explicit stop intent, never inferred physical completion or Task cancellation.
use forge_domain::{
    AggregateRef, CommandId, DomainEvent, DomainEventKind, EmployeeState, ExecutionStopMode,
    Project, Task, TaskWaitCondition, TaskWaitKind, Timestamp, WaitConditionId,
};
use forge_protocol::wire::CommandReceipt;
use serde_json::json;

use super::{
    ActiveRun, CommandError, CommandTransaction, Engine, RepositoryError, StoredTask,
    event::{event, event_payload},
    receipt::{finish_command, resource},
    task_support::{load_scoped_task, persist_task_and_project, require_task_revision, task_event},
};
use crate::{CommandEnvelope, CommandPayload};

impl Engine<'_> {
    pub(super) async fn manage_execution(
        &self,
        transaction: &mut impl CommandTransaction,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<CommandReceipt, CommandError> {
        let previous = project.revision();
        project.record_child_mutation(now)?;
        transaction.update_project(&project, previous).await?;
        let mut events = Vec::new();
        let target = match &envelope.payload {
            CommandPayload::StopEmployee {
                employee_id,
                expected_employee_revision,
                mode,
                reason,
            } => {
                let mut employee = transaction
                    .lock_employee(*employee_id)
                    .await?
                    .filter(|employee| employee.project_id() == project.id())
                    .ok_or(CommandError::NotFound {
                        aggregate: "employee",
                    })?;
                if employee.revision() != *expected_employee_revision {
                    return Err(RepositoryError::StaleRevision {
                        aggregate: "employee",
                    }
                    .into());
                }
                if employee.state() != EmployeeState::Retired {
                    employee.disable(now)?;
                    transaction
                        .update_employee(&employee, *expected_employee_revision)
                        .await?;
                }
                let runs = transaction
                    .lock_active_runs_for_employee(project.id(), *employee_id)
                    .await?;
                request_stops(transaction, &runs, *mode).await?;
                let detail = format!("employee_stopped={employee_id}");
                for run in &runs {
                    if let Some(owner) = run.assignment.task_stage() {
                        let stored = load_scoped_task(transaction, &project, owner.task_id).await?;
                        // A finishing prior visit cannot pause the later stage. Missing
                        // legacy visit evidence grants no inferred current ownership.
                        if owns_current_visit(run, &stored.task) {
                            events.push(
                                self.pause_snapshot(
                                    transaction,
                                    &mut project,
                                    stored,
                                    &detail,
                                    command_id,
                                    now,
                                )
                                .await?,
                            );
                        }
                    }
                    events.push(self.manager_stop_event(&project, run, *mode, command_id, now)?);
                }
                events.push(event(
                    project.id(),
                    AggregateRef::Employee(employee.id()),
                    employee.revision(),
                    DomainEventKind::EmployeeStopRequested,
                    self.actors.human,
                    command_id,
                    reason.clone(),
                    event_payload([
                        ("employee_id", json!(employee.id())),
                        ("mode", json!(mode)),
                        ("state", json!(employee.state())),
                        (
                            "run_ids",
                            json!(runs.iter().map(|run| run.id).collect::<Vec<_>>()),
                        ),
                    ]),
                    now,
                )?);
                resource("employee", employee.id().as_uuid())
            }
            CommandPayload::PauseTask {
                task_id,
                expected_task_revision,
                mode,
                reason,
            } => {
                let stored = load_scoped_task(transaction, &project, *task_id).await?;
                require_task_revision(&stored.task, *expected_task_revision)?;
                let runs = transaction
                    .lock_active_runs_for_task(project.id(), *task_id)
                    .await?;
                request_stops(transaction, &runs, *mode).await?;
                let audit = self
                    .pause_snapshot(
                        transaction,
                        &mut project,
                        stored,
                        "task_paused",
                        command_id,
                        now,
                    )
                    .await?;
                events.push(audit);
                for run in &runs {
                    events.push(self.manager_stop_event(&project, run, *mode, command_id, now)?);
                }
                events.push(event(
                    project.id(),
                    AggregateRef::Project(project.id()),
                    project.revision(),
                    DomainEventKind::TaskPauseRequested,
                    self.actors.human,
                    command_id,
                    reason.clone(),
                    event_payload([("task_id", json!(task_id)), ("mode", json!(mode))]),
                    now,
                )?);
                resource("task", task_id.as_uuid())
            }
            _ => return Err(CommandError::UnsupportedCommand),
        };
        finish_command(
            transaction,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            events,
            Some(target),
        )
        .await
    }

    async fn pause_snapshot(
        &self,
        transaction: &mut impl CommandTransaction,
        project: &mut Project,
        mut stored: StoredTask,
        detail: &str,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<DomainEvent, CommandError> {
        let existing = stored
            .task
            .wait_conditions()
            .find(|wait| wait.kind() == &TaskWaitKind::ManualPause && wait.detail() == Some(detail))
            .map(|wait| wait.id());
        let wait_id = if let Some(id) = existing {
            id
        } else {
            let previous = stored.task.revision().get();
            let id = WaitConditionId::new();
            stored.task.add_wait_condition(
                TaskWaitCondition::new(
                    id,
                    TaskWaitKind::ManualPause,
                    Some(detail.into()),
                    self.actors.human,
                    now,
                )?,
                now,
            )?;
            persist_task_and_project(
                transaction,
                project,
                &stored.task,
                stored.persistence,
                previous,
                now,
            )
            .await?;
            id
        };
        task_event(
            project,
            &stored.task,
            DomainEventKind::TaskWaiting,
            self.actors.human,
            command_id,
            now,
            event_payload([
                ("wait_condition_id", json!(wait_id)),
                ("reason_code", json!(detail)),
            ]),
        )
    }

    fn manager_stop_event(
        &self,
        project: &Project,
        run: &ActiveRun,
        mode: ExecutionStopMode,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<DomainEvent, CommandError> {
        Ok(event(
            project.id(),
            AggregateRef::Project(project.id()),
            project.revision(),
            DomainEventKind::RunStopRequested,
            self.actors.human,
            command_id,
            None,
            event_payload([
                ("run_id", json!(run.id)),
                ("lease_fencing_token", json!(run.lease_fencing_token)),
                ("environment_epoch", json!(run.environment_epoch)),
                ("mode", json!(mode)),
                ("reason_code", json!("manager_stop")),
            ]),
            now,
        )?)
    }
}

fn owns_current_visit(run: &ActiveRun, task: &Task) -> bool {
    !task.lifecycle().is_terminal()
        && run.assignment.task_stage().is_some_and(|owner| {
            owner.task_id == task.id() && Some(&owner.stage_id) == task.current_stage_id()
        })
        && run.stage_visit.is_some()
        && run.stage_visit == task.current_stage_visit()
}

async fn request_stops(
    transaction: &mut impl CommandTransaction,
    runs: &[ActiveRun],
    mode: ExecutionStopMode,
) -> Result<(), CommandError> {
    for run in runs {
        transaction
            .request_run_stop(
                run.id,
                run.lease_fencing_token,
                run.environment_epoch,
                mode.is_force(),
            )
            .await?;
        transaction
            .cancel_leased_queue_for_fenced_run(
                run.id,
                run.lease_fencing_token,
                run.environment_epoch,
            )
            .await?;
        if mode.is_force() {
            transaction
                .revoke_run_lease(run.id, run.lease_fencing_token, run.environment_epoch)
                .await?;
        }
    }
    Ok(())
}
