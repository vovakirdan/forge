//! Durable one-shot alarms reuse the normal resume command under the same transaction.
use crate::{
    CoreError, CoreService,
    event::{event, event_payload},
};
use forge_application::{CommandContext, CommandEnvelope, SystemClock, execute_in_transaction};
use forge_domain::{
    Actor, AggregateRef, CommandId, DomainEventKind, LifecycleStatus, ResumeRejection,
    ScheduledResumeState, TaskResumeSchedule, Timestamp,
};
use forge_protocol::wire::{CommandName, CommandRequest};
use serde_json::json;
use uuid::Uuid;

impl CoreService {
    /// Finalizes at most 64 due alarms. Storage failures roll back and remain retryable;
    /// changed policy/domain preconditions become a durable rejection, never an auto-retry.
    pub async fn resume_due_tasks(&self, observed_wall: Timestamp) -> Result<usize, CoreError> {
        // Until inventory is reconciled, the persisted boot-policy gate may still
        // describe the preceding host boot. Defer instead of guessing a rejection.
        if !self.fake_runtime_enabled && !self.supervisor.is_reconciled().await {
            return Ok(0);
        }
        let mut finalized = 0;
        for (project_id, id) in self.store.due_task_resume_schedules(observed_wall).await? {
            let mut tx = self.store.begin().await?;
            let Some(mut project) = tx.lock_project(project_id).await? else {
                continue;
            };
            let Some(mut schedule) = tx.lock_task_resume_schedule(id).await? else {
                continue;
            };
            if schedule.project_id != project_id
                || schedule.state != ScheduledResumeState::Pending
                || schedule.not_before > observed_wall
            {
                continue;
            }
            let now = crate::canonical_clock::project_mutation_time(&project);
            let stored = tx.lock_task(schedule.task_id).await?;
            let rejection = if let Some(stored) = &stored {
                let task = &stored.task;
                if task.project_id() != project_id
                    || task.revision().get() != schedule.expected_task_revision
                    || task.pipeline().pipeline_version_id() != schedule.pipeline_version_id
                    || task.current_stage_id() != Some(&schedule.stage_id)
                    || task.current_stage_visit() != Some(schedule.stage_visit)
                {
                    Some(ResumeRejection::TaskChanged)
                } else if task.lifecycle() != LifecycleStatus::Waiting
                    || task
                        .clone()
                        .resolve_wait_condition(schedule.wait_condition_id, now)
                        .is_err()
                {
                    Some(ResumeRejection::WaitAbsent)
                } else if project.execution_gate() != forge_domain::ProjectExecutionGate::Open {
                    Some(ResumeRejection::ProjectStopped)
                } else if tx
                    .load_escalation_for_wait(project_id, task.id(), schedule.wait_condition_id)
                    .await?
                    .is_some()
                {
                    Some(ResumeRejection::ResolutionRequired)
                } else {
                    tx.scheduled_resume_gate(&schedule).await?
                }
            } else {
                Some(ResumeRejection::TaskChanged)
            };
            let actor = Actor::system_manager(self.actors.core.id());
            let command_id = if let Some(reason) = rejection {
                schedule.resolve(ScheduledResumeState::Rejected { reason, at: now })?;
                CommandId::new()
            } else {
                let envelope = resume_envelope(&schedule, project.revision())?;
                let context = CommandContext {
                    project_id,
                    actor,
                    core_actor: self.actors.core,
                    capabilities: vec![CommandName::ResumeTask],
                };
                let receipt =
                    execute_in_transaction(&mut tx, &context, &envelope, &SystemClock).await?;
                let command_id =
                    CommandId::from(Uuid::parse_str(&receipt.command_id).map_err(|_| {
                        CoreError::InvalidTransport {
                            field: "schedule.command_id",
                            reason: "invalid internal receipt".into(),
                        }
                    })?);
                // The ordinary command advanced Project time/revision in this same transaction.
                project = tx
                    .lock_project(project_id)
                    .await?
                    .ok_or(CoreError::NotFound {
                        aggregate: "project",
                    })?;
                schedule.resolve(ScheduledResumeState::Applied {
                    command_id,
                    at: project.updated_at(),
                })?;
                command_id
            };
            tx.update_task_resume_schedule(&schedule).await?;
            let previous = project.revision();
            let audit_time = crate::canonical_clock::project_mutation_time(&project);
            project.record_child_mutation(audit_time)?;
            tx.update_project(&project, previous).await?;
            tx.append_event_and_outbox(&event(
                project_id,
                AggregateRef::Project(project_id),
                project.revision(),
                DomainEventKind::TaskResumeScheduleChanged,
                actor,
                command_id,
                Some(schedule.reason.clone()),
                event_payload([("schedule", json!(schedule))]),
                audit_time,
            )?)
            .await?;
            tx.commit().await?;
            finalized += 1;
            // Delivery is postcommit. Offline Supervisor leaves an ordinary queued intent.
            self.dispatch_after_command(project_id).await;
        }
        Ok(finalized)
    }
}

fn resume_envelope(
    schedule: &TaskResumeSchedule,
    revision: u64,
) -> Result<CommandEnvelope, CoreError> {
    Ok(CommandEnvelope::parse(
        CommandName::ResumeTask,
        CommandRequest {
            project_id: schedule.project_id.to_string(),
            expected_revision: revision,
            payload: serde_json::Map::from_iter([
                ("task_id".into(), json!(schedule.task_id)),
                (
                    "expected_task_revision".into(),
                    json!(schedule.expected_task_revision),
                ),
                (
                    "wait_condition_id".into(),
                    json!(schedule.wait_condition_id),
                ),
            ]),
        },
        format!("scheduled-resume/{}", schedule.id),
    )?)
}
