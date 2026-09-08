//! Changing future admission does not transfer active work or resolve a Task wait.
use forge_domain::{
    AggregateRef, CommandId, ConstraintEndReason, DomainEventKind, ExecutorKind,
    NextRunConstraintState, NextRunEmployeeConstraint, Project, Timestamp,
};
use forge_protocol::wire::CommandReceipt;
use serde_json::json;
use uuid::Uuid;

use super::{
    CommandError, CommandTransaction, Engine,
    event::{event, event_payload},
    receipt::{finish_command, resource},
    task_support::{
        current_stage_executor, load_scoped_task, pinned_pipeline_version, require_task_revision,
    },
};
use crate::{CommandEnvelope, CommandPayload};

impl Engine<'_> {
    pub(super) async fn manage_dispatch_constraint(
        &self,
        transaction: &mut impl CommandTransaction,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<CommandReceipt, CommandError> {
        let (task_id, expected_revision, reason) = match &envelope.payload {
            CommandPayload::SetNextRunEmployee {
                task_id,
                expected_task_revision,
                reason,
                ..
            }
            | CommandPayload::ClearNextRunEmployee {
                task_id,
                expected_task_revision,
                reason,
            } => (*task_id, *expected_task_revision, reason),
            _ => return Err(CommandError::UnsupportedCommand),
        };
        let stored = load_scoped_task(transaction, &project, task_id).await?;
        require_task_revision(&stored.task, expected_revision)?;
        let old = transaction
            .load_task_dispatch_constraint(project.id(), task_id)
            .await?;
        let replacement =
            if let CommandPayload::SetNextRunEmployee { employee_id, .. } = envelope.payload {
                let task = &stored.task;
                let version = pinned_pipeline_version(transaction, &project, task).await?;
                if task.lifecycle().is_terminal()
                    || current_stage_executor(&version, task)? != ExecutorKind::Employee
                {
                    return Err(refusal("requires a current nonterminal Employee stage"));
                }
                let stage_id = task
                    .current_stage_id()
                    .cloned()
                    .ok_or_else(|| refusal("Task has no active stage"))?;
                let stage_visit = task
                    .current_stage_visit()
                    .ok_or_else(|| refusal("Task has no active visit"))?;
                let employee = transaction
                    .lock_employee(employee_id)
                    .await?
                    .filter(|employee| employee.project_id() == project.id())
                    .ok_or(CommandError::NotFound {
                        aggregate: "employee",
                    })?;
                if !employee.is_eligible_for(version.id(), &stage_id) {
                    return Err(refusal("Employee is disabled or ineligible for this stage"));
                }
                Some(NextRunEmployeeConstraint {
                    id: Uuid::now_v7(),
                    project_id: project.id(),
                    task_id,
                    pipeline_version_id: version.id(),
                    stage_id,
                    stage_visit,
                    employee_id,
                    created_by: self.actors.human,
                    created_at: now,
                    state: NextRunConstraintState::Pending,
                })
            } else {
                None
            };
        if old.is_none() && replacement.is_none() {
            return Err(CommandError::NotFound {
                aggregate: "dispatch constraint",
            });
        }
        let previous = project.revision();
        project.record_child_mutation(now)?;
        transaction.update_project(&project, previous).await?;
        let mut states = Vec::new();
        if let Some(mut prior) = old {
            let expected = prior.state.clone();
            prior.transition(NextRunConstraintState::Cancelled {
                reason: if replacement.is_some() {
                    ConstraintEndReason::Superseded
                } else {
                    ConstraintEndReason::Cleared
                },
            })?;
            transaction
                .update_task_dispatch_constraint(&prior, &expected)
                .await?;
            states.push(prior);
        }
        if let Some(next) = replacement {
            transaction.insert_task_dispatch_constraint(&next).await?;
            states.push(next);
        }
        let mut events = Vec::with_capacity(states.len());
        for state in &states {
            events.push(event(
                project.id(),
                AggregateRef::Task(task_id),
                stored.task.revision().get(),
                DomainEventKind::TaskDispatchConstraintChanged,
                self.actors.human,
                command_id,
                reason.clone(),
                event_payload([("constraint", json!(state))]),
                now,
            )?);
        }
        finish_command(
            transaction,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            events,
            Some(resource("task", task_id.as_uuid())),
        )
        .await
    }
}

fn refusal(reason: &str) -> CommandError {
    CommandError::InvalidTransport {
        field: "task.next_run_employee",
        reason: reason.into(),
    }
}
