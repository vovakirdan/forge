//! Progress is observational; the historical human tool now creates a canonical question.
use super::{escalation::EscalationArguments, invalid};
use crate::{
    CoreError, CoreService,
    event::event_payload,
    task_support::{load_scoped_task, task_event},
};
use forge_domain::{
    Actor, ActorId, CommandId, DomainEventKind, resolution::EscalationCategory, runtime::RunScope,
};
use forge_storage::{GatewaySubmissionRecord, GatewayWriteResult};
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
        if tool == "human.request" {
            return self
                .record_gateway_escalation(
                    scope,
                    message_id,
                    tool,
                    EscalationArguments {
                        question: detail.into(),
                        category: EscalationCategory::Clarification,
                        route_key: None,
                    },
                    payload_hash,
                )
                .await;
        }
        let mut tx = self.store.begin().await?;
        let run = tx
            .validate_gateway_scope(&scope)
            .await?
            .ok_or_else(|| invalid("Run scope is no longer active"))?;
        let project = tx
            .lock_project(run.project_id)
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "project",
            })?;
        let now = crate::canonical_clock::project_mutation_time(&project);
        let stored = load_scoped_task(&mut tx, &project, run.require_task_id()?).await?;
        let receipt = json!({"status":"accepted","message_id":message_id,"tool":tool,"wait_condition_id":null});
        match tx
            .record_gateway_submission(&GatewaySubmissionRecord {
                scope,
                message_id,
                payload_hash: payload_hash.into(),
                receipt: receipt.clone(),
            })
            .await?
        {
            GatewayWriteResult::Applied => {}
            GatewayWriteResult::Replayed(receipt) => return Ok(receipt),
            GatewayWriteResult::Conflict => return Err(CoreError::IdempotencyConflict),
            GatewayWriteResult::Denied => return Err(invalid("Run scope was revoked")),
        }
        let actor = Actor::employee(ActorId::from(run.require_employee_id()?.as_uuid()));
        let progress = task_event(
            &project,
            &stored.task,
            DomainEventKind::ToolGatewayCallAllowed,
            actor,
            CommandId::from(message_id),
            now,
            event_payload([
                ("tool", json!(tool)),
                ("category", json!("progress_reported")),
                ("run_id", json!(run.id)),
                ("note", json!(detail)),
            ]),
        )?;
        tx.append_event_and_outbox(&progress).await?;
        tx.commit().await?;
        Ok(receipt)
    }
}
