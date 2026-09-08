//! Exact Assignment/generation authority; never contextual Task execution.
use super::{RunGateway, ToolRequest, invalid};
use crate::{CoreError, resolution_queue};
use forge_application::engine::resolver_acceptance::{
    ResolutionAcceptanceContext, ResolutionSubmission, accept_resolution,
};
use forge_domain::{
    Actor, ActorId, CommandId, DomainEventKind, Timestamp,
    resolution::{EscalationState, ResolutionAnswer, ResolutionAssignmentState, ResolutionContext},
};
use forge_storage::{GatewaySubmissionRecord, GatewayWriteResult, RunProjection};
use serde::Deserialize;
use serde_json::{Value, json};

pub(super) fn context(run: &RunProjection) -> Result<ResolutionContext, CoreError> {
    let context: ResolutionContext = serde_json::from_value(run.context_manifest.clone())
        .map_err(|_| invalid("invalid Resolution context"))?;
    let data = context.data();
    if run.run_spec_version != 4
        || data.run_id != run.id
        || data.project_id != run.project_id
        || Some(data.employee_id) != run.employee_id
        || run.assignment.resolution().is_none_or(|owner| {
            owner.assignment_id != data.assignment.id
                || owner.escalation_id != data.escalation.id
                || owner.lease_generation != data.assignment.lease.generation
        })
    {
        return Err(invalid("Resolution context differs from owner"));
    }
    Ok(context)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArguments {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeclineArguments {
    reason: String,
}

impl RunGateway {
    pub(super) async fn execute_resolution(
        &self,
        request: ToolRequest,
        digest: [u8; 32],
    ) -> Result<Value, CoreError> {
        let mut tx = self.core.store.begin().await?;
        let run = tx
            .validate_gateway_scope(&self.scope)
            .await?
            .ok_or_else(|| invalid("Run scope revoked"))?;
        let context = context(&run)?;
        let hash: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        if let Some(receipt) = tx
            .gateway_receipt(self.scope, request.message_id, &hash)
            .await?
        {
            return Ok(receipt);
        }
        if !tx.resolution_run_authorized(&run).await? {
            return Err(invalid("Resolution assignment expired or changed"));
        }
        let mut project = tx
            .lock_project(run.project_id)
            .await?
            .ok_or_else(resolution_queue::invalid)?;
        let mut escalation = tx
            .load_escalation(context.data().escalation.id)
            .await?
            .ok_or_else(resolution_queue::invalid)?;
        if !resolution_queue::source_is_current(&mut tx, &escalation).await? {
            return Err(invalid("question source no longer current"));
        }
        if request.tool == "resolution.read" {
            let _: ReadArguments = serde_json::from_value(request.arguments)
                .map_err(|_| invalid("read accepts no authority fields"))?;
            return Ok(json!({"escalation":escalation,"assignment":context.data().assignment}));
        }
        let now = crate::canonical_clock::project_mutation_time(&project);
        let actor = Actor::employee(ActorId::from(run.require_employee_id()?.as_uuid()));
        let receipt = if request.tool == "resolution.submit" {
            let answer: ResolutionAnswer = serde_json::from_value(request.arguments)
                .map_err(|_| invalid("invalid Resolution answer"))?;
            let accepted = accept_resolution(
                &mut tx,
                &mut project,
                &ResolutionSubmission {
                    escalation_id: escalation.id,
                    expected_escalation_revision: escalation.revision,
                    assignment_id: context.data().assignment.id,
                    lease_generation: context.data().assignment.lease.generation,
                    answer,
                },
                ResolutionAcceptanceContext {
                    actor,
                    coordinator: self.core.actors.core,
                    command_id: CommandId::from(request.message_id),
                    now,
                    observed_wall: Timestamp::now_utc(),
                    run_id: Some(run.id),
                },
            )
            .await?;
            for event in accepted.events {
                tx.append_event_and_outbox(&event).await?;
            }
            json!({"status":"accepted","artifact_id":accepted.artifact_id,"escalation_id":accepted.escalation_id,"communication_resume_pending":accepted.communication_resume_pending,"task_resumed":accepted.task_resumed})
        } else if request.tool == "resolution.decline" {
            let args: DeclineArguments = serde_json::from_value(request.arguments)
                .map_err(|_| invalid("invalid decline arguments"))?;
            let mut assignment = tx
                .load_resolution_assignment(context.data().assignment.id)
                .await?
                .ok_or_else(resolution_queue::invalid)?;
            assignment.finish(ResolutionAssignmentState::Retired {
                reason: args.reason,
            })?;
            tx.update_resolution_assignment(&assignment).await?;
            let old = escalation.revision;
            escalation.advance(EscalationState::Queued)?;
            tx.save_escalation(&escalation, Some(old)).await?;
            resolution_queue::audit(
                &mut tx,
                &mut project,
                actor,
                DomainEventKind::EscalationRerouted,
                json!({"escalation_id":escalation.id,"assignment":assignment}),
            )
            .await?;
            json!({"status":"declined","escalation_id":escalation.id})
        } else {
            return Err(invalid("unsupported resolution action"));
        };
        match tx
            .record_gateway_submission(&GatewaySubmissionRecord {
                scope: self.scope,
                message_id: request.message_id,
                payload_hash: hash,
                receipt: receipt.clone(),
            })
            .await?
        {
            GatewayWriteResult::Applied => {}
            GatewayWriteResult::Replayed(value) => return Ok(value),
            GatewayWriteResult::Conflict => return Err(CoreError::IdempotencyConflict),
            GatewayWriteResult::Denied => return Err(invalid("Run scope revoked")),
        }
        // One-shot answer/decline is complete. Durable stop never releases the
        // physical reservation; supervisor observations alone establish quiescence.
        tx.request_run_stop(
            run.id,
            run.lease_fencing_token,
            run.environment_epoch,
            false,
        )
        .await?;
        tx.commit().await?;
        let _ = self
            .core
            .deliver_pending_stop_requests(run.project_id)
            .await;
        Ok(receipt)
    }
}
