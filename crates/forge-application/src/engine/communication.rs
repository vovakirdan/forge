//! Inbox mutations share the Project lock, receipt and outbox boundary.
use forge_domain::{
    AggregateRef, CommandId, DomainEventKind, EmployeeState, LifecycleStatus, Project, Timestamp,
    communication::{EmployeeThread, EmployeeThreadInput, MessageTarget, SendMessageInput},
};
use forge_protocol::wire::CommandReceipt;
use serde_json::json;
use uuid::Uuid;

use super::{
    CommandError, CommandTransaction, Engine, RepositoryError,
    event::{event, event_payload},
    receipt::{finish_command, resource},
};
use crate::{CommandEnvelope, CommandPayload};

impl Engine<'_> {
    pub(super) async fn apply_communication_command(
        &self,
        tx: &mut impl CommandTransaction,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<CommandReceipt, CommandError> {
        let (kind, employee_id, thread_id, primary, sequence) = match &envelope.payload {
            CommandPayload::OpenEmployeeThread(input) => {
                let employee = tx
                    .lock_employee(input.employee_id)
                    .await?
                    .filter(|e| e.project_id() == project.id())
                    .ok_or(CommandError::NotFound {
                        aggregate: "employee",
                    })?;
                if employee.state() == EmployeeState::Retired {
                    return Err(invalid(
                        "employee",
                        "retired Employee cannot receive new conversations",
                    ));
                }
                if let Some(task_id) = input.task_id {
                    tx.lock_task(task_id)
                        .await?
                        .filter(|t| t.task.project_id() == project.id())
                        .ok_or(CommandError::NotFound { aggregate: "task" })?;
                }
                let thread_id = Uuid::now_v7();
                let thread = EmployeeThread::new(EmployeeThreadInput {
                    id: thread_id,
                    project_id: project.id(),
                    employee_id: input.employee_id,
                    task_id: input.task_id,
                    created_by: self.actors.human,
                    created_at: now,
                    updated_at: now,
                    revision: 1,
                    last_sequence: 0,
                })?;
                tx.insert_employee_thread(&thread).await?;
                (
                    DomainEventKind::EmployeeThreadOpened,
                    input.employee_id,
                    thread_id,
                    resource("employee_thread", thread_id),
                    0,
                )
            }
            CommandPayload::SendEmployeeMessage(input) => {
                let mut thread = tx
                    .lock_employee_thread(input.thread_id)
                    .await?
                    .filter(|t| t.data().project_id == project.id())
                    .ok_or(CommandError::NotFound {
                        aggregate: "employee thread",
                    })?;
                if thread.data().revision != input.expected_thread_revision {
                    return Err(RepositoryError::StaleRevision {
                        aggregate: "employee thread",
                    }
                    .into());
                }
                let employee_id = thread.data().employee_id;
                let employee = tx
                    .lock_employee(employee_id)
                    .await?
                    .filter(|e| e.project_id() == project.id())
                    .ok_or(CommandError::NotFound {
                        aggregate: "employee",
                    })?;
                if employee.state() == EmployeeState::Retired {
                    return Err(invalid("employee", "recipient is retired"));
                }
                input.target.validate()?;
                if let Some(context) = input.target.task_context() {
                    let stored = tx
                        .lock_task(context.task_id)
                        .await?
                        .filter(|t| t.task.project_id() == project.id())
                        .ok_or(CommandError::NotFound { aggregate: "task" })?;
                    let task = &stored.task;
                    let version = tx
                        .lock_pipeline_version(context.pipeline_version_id)
                        .await?
                        .ok_or(CommandError::NotFound {
                            aggregate: "pipeline version",
                        })?;
                    if version.stage(&context.stage_id).is_none_or(|stage| {
                        stage.executor_kind() != forge_domain::ExecutorKind::Employee
                    }) {
                        return Err(invalid(
                            "message.target",
                            "execution-directed messages require an Employee stage",
                        ));
                    }
                    if matches!(
                        task.lifecycle(),
                        LifecycleStatus::Done | LifecycleStatus::Cancelled
                    ) || task.pipeline().pipeline_version_id() != context.pipeline_version_id
                        || task.current_stage_id() != Some(&context.stage_id)
                        || task.current_stage_visit().map(|visit| visit.get())
                            != Some(context.stage_visit)
                    {
                        return Err(invalid(
                            "message.target",
                            "Task execution context is no longer current",
                        ));
                    }
                    if let MessageTarget::ExactRun {
                        run_id,
                        fencing_token,
                        environment_epoch,
                        ..
                    } = input.target
                    {
                        let runs = tx
                            .lock_active_runs_for_task(project.id(), context.task_id)
                            .await?;
                        if !runs.iter().any(|run| {
                            run.id == run_id
                                && run.employee_id == Some(employee_id)
                                && run.lease_fencing_token == fencing_token
                                && run.environment_epoch == environment_epoch
                        }) {
                            return Err(invalid(
                                "message.target",
                                "exact Run scope is no longer active",
                            ));
                        }
                    }
                }
                if let Some(reply_id) = input.reply_to {
                    tx.load_employee_message(reply_id)
                        .await?
                        .filter(|m| {
                            m.data().project_id == project.id()
                                && m.data().thread_id == input.thread_id
                        })
                        .ok_or(CommandError::NotFound {
                            aggregate: "reply message",
                        })?;
                }
                let message = thread.send(SendMessageInput {
                    id: Uuid::now_v7(),
                    expected_thread_revision: input.expected_thread_revision,
                    sender: self.actors.human,
                    target: input.target.clone(),
                    kind: input.kind,
                    requirement: input.requirement,
                    body: input.body.clone(),
                    reply_to: input.reply_to,
                    created_at: now,
                })?;
                tx.update_employee_thread(&thread, input.expected_thread_revision)
                    .await?;
                tx.insert_employee_message(&message).await?;
                (
                    DomainEventKind::EmployeeMessageSent,
                    employee_id,
                    input.thread_id,
                    resource("employee_message", message.data().id),
                    message.data().sequence,
                )
            }
            CommandPayload::WaiveMessageRequirement(input) => {
                let message = tx
                    .load_employee_message(input.message_id)
                    .await?
                    .filter(|m| m.data().project_id == project.id())
                    .ok_or(CommandError::NotFound {
                        aggregate: "employee message",
                    })?;
                let waiver = forge_domain::communication::MessageRequirementWaiver::new(
                    &message,
                    self.actors.human,
                    input.reason.clone(),
                    now,
                )?;
                tx.insert_message_requirement_waiver(&waiver).await?;
                (
                    DomainEventKind::EmployeeMessageRequirementWaived,
                    message.data().employee_id,
                    message.data().thread_id,
                    resource("message_requirement_waiver", message.data().id),
                    message.data().sequence,
                )
            }
            _ => return Err(CommandError::UnsupportedCommand),
        };
        let previous_revision = project.revision();
        project.record_child_mutation(now)?;
        tx.update_project(&project, previous_revision).await?;
        let audit = event(
            project.id(),
            AggregateRef::Project(project.id()),
            project.revision(),
            kind,
            self.actors.human,
            command_id,
            match &envelope.payload {
                CommandPayload::WaiveMessageRequirement(input) => Some(input.reason.clone()),
                _ => None,
            },
            event_payload([
                ("employee_id", json!(employee_id)),
                ("thread_id", json!(thread_id)),
                ("resource_id", json!(primary.id)),
                ("sequence", json!(sequence)),
            ]),
            now,
        )?;
        finish_command(
            tx,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            vec![audit],
            Some(primary),
        )
        .await
    }
}

fn invalid(field: &'static str, reason: &str) -> CommandError {
    CommandError::InvalidTransport {
        field,
        reason: reason.into(),
    }
}
