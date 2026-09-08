//! A scheduled resume captures permission for one exact wait, not a general retry policy.
use super::{
    CommandError, CommandTransaction, Engine,
    event::{event, event_payload},
    receipt::{finish_command, resource},
    task_support::{
        current_stage_executor, load_scoped_task, pinned_pipeline_version, require_task_revision,
    },
};
use crate::{CommandEnvelope, CommandPayload};
use forge_domain::{
    AggregateRef, CommandId, DomainEventKind, ExecutorKind, LifecycleStatus, Project,
    ScheduledResumeState, TaskResumeSchedule, Timestamp,
};
use forge_protocol::wire::CommandReceipt;
use serde_json::json;
use uuid::Uuid;

impl Engine<'_> {
    pub(super) async fn manage_scheduled_resume(
        &self,
        transaction: &mut impl CommandTransaction,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<CommandReceipt, CommandError> {
        let (schedule, reason) = match &envelope.payload {
            CommandPayload::ScheduleTaskResume {
                task_id,
                expected_task_revision,
                wait_condition_id,
                not_before,
                reason,
            } => {
                let stored = load_scoped_task(transaction, &project, *task_id).await?;
                require_task_revision(&stored.task, *expected_task_revision)?;
                let version = pinned_pipeline_version(transaction, &project, &stored.task).await?;
                if *not_before <= self.clock.now()
                    || stored.task.lifecycle() != LifecycleStatus::Waiting
                    || current_stage_executor(&version, &stored.task)? != ExecutorKind::Employee
                {
                    return Err(refusal(
                        "requires a future wall-clock deadline and a waiting Employee stage",
                    ));
                }
                // Prove the exact existing command contract without changing the Task.
                super::task_support::ensure_wait_resumable(
                    transaction,
                    &stored.task,
                    *wait_condition_id,
                )
                .await?;
                stored
                    .task
                    .clone()
                    .resolve_wait_condition(*wait_condition_id, now)?;
                let schedule = TaskResumeSchedule {
                    id: Uuid::now_v7(),
                    project_id: project.id(),
                    task_id: *task_id,
                    expected_task_revision: *expected_task_revision,
                    pipeline_version_id: version.id(),
                    stage_id: stored
                        .task
                        .current_stage_id()
                        .cloned()
                        .ok_or_else(|| refusal("Task has no stage"))?,
                    stage_visit: stored
                        .task
                        .current_stage_visit()
                        .ok_or_else(|| refusal("Task has no stage visit"))?,
                    wait_condition_id: *wait_condition_id,
                    not_before: *not_before,
                    reason: reason.clone(),
                    created_by: self.actors.human,
                    created_at: now,
                    state: ScheduledResumeState::Pending,
                };
                schedule.validate_snapshot()?;
                transaction.insert_task_resume_schedule(&schedule).await?;
                (schedule, Some(reason.clone()))
            }
            CommandPayload::CancelTaskResume {
                schedule_id,
                reason,
            } => {
                let mut schedule = transaction
                    .lock_task_resume_schedule(*schedule_id)
                    .await?
                    .filter(|schedule| schedule.project_id == project.id())
                    .ok_or(CommandError::NotFound {
                        aggregate: "resume schedule",
                    })?;
                schedule.resolve(ScheduledResumeState::Cancelled { at: now })?;
                transaction.update_task_resume_schedule(&schedule).await?;
                (schedule, reason.clone())
            }
            _ => return Err(CommandError::UnsupportedCommand),
        };
        let previous = project.revision();
        project.record_child_mutation(now)?;
        transaction.update_project(&project, previous).await?;
        let audit = event(
            project.id(),
            AggregateRef::Project(project.id()),
            project.revision(),
            DomainEventKind::TaskResumeScheduleChanged,
            self.actors.human,
            command_id,
            reason,
            event_payload([("schedule", json!(schedule))]),
            now,
        )?;
        finish_command(
            transaction,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            vec![audit],
            Some(resource("task_resume_schedule", schedule.id)),
        )
        .await
    }
}

fn refusal(reason: &str) -> CommandError {
    CommandError::InvalidTransport {
        field: "task.resume_schedule",
        reason: reason.into(),
    }
}
