//! A question holds only its exact execution source; context never grants Task authority.
use super::{RunGateway, ToolRequest, invalid};
use crate::{CoreError, CoreService, run_control::run_stop_event};
use forge_application::{CommandContext, RaiseEscalationInput, SystemClock, engine::Engine};
use forge_domain::{
    Actor, ActorId, CommandId, LifecycleStatus, WaitConditionId, resolution::*, runtime::RunScope,
};
use forge_storage::{FencedWrite, GatewaySubmissionRecord, GatewayWriteResult};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EscalationArguments {
    pub question: String,
    pub category: EscalationCategory,
    #[serde(default)]
    pub route_key: Option<String>,
}

impl RunGateway {
    pub(super) async fn raise_escalation(
        &self,
        request: ToolRequest,
        digest: [u8; 32],
    ) -> Result<Value, CoreError> {
        let args: EscalationArguments = serde_json::from_value(request.arguments)
            .map_err(|_| invalid("invalid escalation arguments"))?;
        let hash: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        self.core
            .record_gateway_escalation(
                self.scope,
                request.message_id,
                "escalation.raise",
                args,
                &hash,
            )
            .await
    }
}

impl CoreService {
    pub(super) async fn record_gateway_escalation(
        &self,
        scope: RunScope,
        message_id: Uuid,
        tool: &str,
        args: EscalationArguments,
        hash: &str,
    ) -> Result<Value, CoreError> {
        if !matches!(tool, "human.request" | "escalation.raise")
            || args.question.trim().is_empty()
            || args.question.contains('\0')
            || args.question.chars().count() > 20_000
        {
            return Err(invalid("question must be bounded and nonblank"));
        }
        let mut tx = self.store.begin().await?;
        let run = tx
            .validate_gateway_scope(&scope)
            .await?
            .ok_or_else(|| invalid("Run scope revoked"))?;
        let mut project = tx
            .lock_project(run.project_id)
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "project",
            })?;
        if let Some(prior) = tx.gateway_receipt(scope, message_id, hash).await? {
            return Ok(prior);
        }
        let actor = Actor::employee(ActorId::from(run.require_employee_id()?.as_uuid()));
        let now = crate::canonical_clock::project_mutation_time(&project);
        let context = CommandContext {
            project_id: run.project_id,
            actor,
            core_actor: self.actors.core,
            capabilities: Vec::new(),
        };
        let engine = Engine::new(&context, &SystemClock);
        let command = CommandId::from(message_id);
        let escalation_id = Uuid::now_v7();
        let wait_id = run.assignment.task_stage().map(|_| WaitConditionId::new());
        if wait_id.is_none()
            && (tool == "human.request" || run.assignment.communication().is_none())
        {
            return Err(invalid("this purpose cannot raise a Task question"));
        }
        // Reserve the immutable reply before withdrawing the source's authority.
        // Any later validation error rolls back this receipt with the question.
        let receipt = json!({"status":"accepted","message_id":message_id,"tool":tool,"wait_condition_id":wait_id,"escalation_id":escalation_id});
        match tx
            .record_gateway_submission(&GatewaySubmissionRecord {
                scope,
                message_id,
                payload_hash: hash.to_owned(),
                receipt: receipt.clone(),
            })
            .await?
        {
            GatewayWriteResult::Applied => {}
            GatewayWriteResult::Replayed(value) => return Ok(value),
            GatewayWriteResult::Conflict => return Err(CoreError::IdempotencyConflict),
            GatewayWriteResult::Denied => return Err(invalid("Run scope revoked")),
        }
        let events = if let Some(wait) = wait_id {
            let task_id = run.require_task_id()?;
            let task = tx
                .lock_task(task_id)
                .await?
                .ok_or(CoreError::NotFound { aggregate: "task" })?
                .task;
            if task.lifecycle() != LifecycleStatus::InProgress
                || task.current_stage_id() != Some(&run.require_task_stage()?.stage_id)
            {
                return Err(invalid("question does not own the active Task stage"));
            }
            let pinned: forge_domain::ContextSnapshot =
                serde_json::from_value(run.context_manifest.clone())
                    .map_err(|_| invalid("invalid pinned context"))?;
            // Retained M1 contexts predate Inbox visits; their active Run fence
            // still authorizes the existing human.request tool, not new Inbox tools.
            if tool == "escalation.raise" || pinned.data().stage_visit.is_some() {
                super::task_scope(&mut tx, &run).await?;
            }
            let input = RaiseEscalationInput {
                task_id,
                expected_task_revision: task.revision().get(),
                route_key: args.route_key,
                category: args.category,
                question: args.question,
            };
            engine
                .raise_task_escalation(
                    &mut tx,
                    &mut project,
                    &input,
                    escalation_id,
                    wait,
                    actor,
                    command,
                    now,
                )
                .await?
                .1
        } else {
            let pinned = super::communication_run::context(&run)?;
            let route = match args.route_key {
                Some(key) => Some(
                    tx.load_resolver_route(project.id(), &key)
                        .await?
                        .ok_or_else(|| invalid("unknown resolver route"))?,
                ),
                None => None,
            };
            let mut escalation = Escalation {
                id: escalation_id,
                project_id: project.id(),
                source: EscalationSource::Communication(CommunicationEscalationSource {
                    assignment: pinned.data().assignment.clone(),
                    run_id: run.id,
                    fencing_token: scope.fencing_token,
                    environment_epoch: scope.environment_epoch,
                }),
                category: args.category,
                question: args.question,
                route,
                allowed_outcomes: Vec::new(),
                created_by: actor,
                created_at: now,
                revision: 1,
                generation: 0,
                next_candidate: 0,
                state: EscalationState::Queued,
            };
            escalation.validate_snapshot()?;
            tx.hold_communication_run(run.id).await?;
            if tx
                .request_run_stop(run.id, scope.fencing_token, scope.environment_epoch, false)
                .await?
                != FencedWrite::Applied
            {
                return Err(invalid("Communication stop lost its exact fence"));
            }
            let previous = project.revision();
            project.record_child_mutation(now)?;
            tx.update_project(&project, previous).await?;
            let mut events = vec![run_stop_event(
                &project,
                &run,
                command,
                actor,
                "escalation_raised",
                now,
            )?];
            engine
                .record_escalation(
                    &mut tx,
                    &project,
                    &mut escalation,
                    actor,
                    command,
                    now,
                    &mut events,
                )
                .await?;
            events
        };
        for event in events {
            tx.append_event_and_outbox(&event).await?;
        }
        tx.commit().await?;
        if self
            .deliver_pending_stop_requests(project.id())
            .await
            .is_err()
        {
            tracing::warn!(run_id=%run.id,"escalation stop committed; delivery awaits reconnect");
        }
        Ok(receipt)
    }
}
