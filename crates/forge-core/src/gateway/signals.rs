//! Named Run signals share the separate Gateway receipt transaction.
//!
//! Human questions use EscalationPending, not the Pipeline-owned DecisionRequired
//! wait: they can occur on any employee stage and require explicit human resume.

use super::invalid;
use crate::{
    CoreError, CoreService,
    event::event_payload,
    run_control::run_stop_event,
    task_support::{load_scoped_task, persist_task_and_project, retained_persistence, task_event},
};
use forge_domain::{
    Actor, ActorId, CommandId, DomainEventKind, LifecycleStatus, TaskWaitCondition, TaskWaitKind,
    WaitConditionId, runtime::RunScope,
};
use forge_storage::{FencedWrite, GatewaySubmissionRecord, GatewayWriteResult};
use serde_json::{Value, json};
use uuid::Uuid;

impl CoreService {
    pub(crate) async fn record_gateway_signal(
        &self,
        scope: RunScope,
        message_id: Uuid,
        tool: &str,
        detail: &str,
        payload_hash: &str,
    ) -> Result<Value, CoreError> {
        if !matches!(tool, "progress.report" | "human.request")
            || detail.trim().is_empty()
            || detail.chars().count() > 10_000
        {
            return Err(invalid(
                "signal must be a named tool with 1..10000 detail characters",
            ));
        }
        let mut transaction = self.store.begin().await?;
        let run = transaction
            .validate_gateway_scope(&scope)
            .await?
            .ok_or_else(|| invalid("Run scope is no longer active"))?;
        let mut project =
            transaction
                .lock_project(run.project_id)
                .await?
                .ok_or(CoreError::NotFound {
                    aggregate: "project",
                })?;
        let now = crate::canonical_clock::project_mutation_time(&project);
        let stored = load_scoped_task(&mut transaction, &project, run.task_id).await?;
        let wait_id = (tool == "human.request").then(WaitConditionId::new);
        let receipt = json!({"status":"accepted","message_id":message_id,"tool":tool,"wait_condition_id":wait_id});
        match transaction
            .record_gateway_submission(&GatewaySubmissionRecord {
                scope,
                message_id,
                payload_hash: payload_hash.to_owned(),
                receipt: receipt.clone(),
            })
            .await?
        {
            GatewayWriteResult::Applied => {}
            GatewayWriteResult::Replayed(receipt) => return Ok(receipt),
            GatewayWriteResult::Conflict => return Err(CoreError::IdempotencyConflict),
            GatewayWriteResult::Denied => return Err(invalid("Run scope was revoked")),
        }
        let actor = Actor::employee(ActorId::from(run.employee_id.as_uuid()));
        let command_id = CommandId::from(message_id);
        let mut task = stored.task;
        if let Some(wait_id) = wait_id {
            if task.lifecycle() != LifecycleStatus::InProgress
                || task.current_stage_id().map(|stage| stage.as_str())
                    != Some(run.stage_id.as_str())
            {
                return Err(invalid(
                    "human request must belong to the active employee stage",
                ));
            }
            let revision = task.revision().get();
            task.add_wait_condition(
                TaskWaitCondition::new(
                    wait_id,
                    TaskWaitKind::EscalationPending,
                    Some(detail.to_owned()),
                    actor,
                    now,
                )?,
                now,
            )?;
            let persistence = retained_persistence(&project, &task, stored.persistence)?;
            persist_task_and_project(
                &mut transaction,
                &mut project,
                &task,
                persistence,
                revision,
                now,
            )
            .await?;
            if transaction
                .request_run_stop(
                    scope.run_id,
                    scope.fencing_token,
                    scope.environment_epoch,
                    false,
                )
                .await?
                != FencedWrite::Applied
                || transaction
                    .cancel_leased_queue_for_fenced_run(
                        scope.run_id,
                        scope.fencing_token,
                        scope.environment_epoch,
                    )
                    .await?
                    != FencedWrite::Applied
            {
                return Err(invalid("Run stop could not retain its current fence"));
            }
            let waiting = task_event(
                &project,
                &task,
                DomainEventKind::TaskWaiting,
                actor,
                command_id,
                now,
                event_payload([
                    ("wait_kind", json!("escalation_pending")),
                    ("wait_condition_id", json!(wait_id)),
                    ("run_id", json!(run.id)),
                    ("question", json!(detail)),
                ]),
            )?;
            let stopping = run_stop_event(
                &project,
                &run,
                command_id,
                actor,
                "human_decision_requested",
                now,
            )?;
            transaction.append_event_and_outbox(&waiting).await?;
            transaction.append_event_and_outbox(&stopping).await?;
        } else {
            let progress = task_event(
                &project,
                &task,
                DomainEventKind::ToolGatewayCallAllowed,
                actor,
                command_id,
                now,
                event_payload([
                    ("tool", json!(tool)),
                    ("category", json!("progress_reported")),
                    ("run_id", json!(run.id)),
                    ("note", json!(detail)),
                ]),
            )?;
            transaction.append_event_and_outbox(&progress).await?;
        }
        transaction.commit().await?;
        if wait_id.is_some() {
            // Delivery is a postcommit optimization. The durable desired stop
            // survives a disconnected Supervisor and is replayed on reconnect.
            if self
                .deliver_pending_stop_requests(project.id())
                .await
                .is_err()
            {
                tracing::warn!(run_id=%run.id,"human-request stop committed; delivery awaits Supervisor reconnect");
            }
        }
        Ok(receipt)
    }
}
