//! Bounded taskless Inbox authority. Optional context is never an execution grant.

use forge_domain::{
    Actor, ActorId, AggregateRef, CommandId, DomainEventKind,
    communication::{CommunicationContext, DeliveryScope},
};
use forge_storage::{GatewaySubmissionRecord, GatewayWriteResult, RunProjection};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{RunGateway, ToolRequest, invalid};
use crate::{
    CoreError,
    event::{event, event_payload},
};

pub(super) fn context(run: &RunProjection) -> Result<CommunicationContext, CoreError> {
    let value: CommunicationContext = serde_json::from_value(run.context_manifest.clone())
        .map_err(|_| invalid("invalid Communication context"))?;
    let data = value.data();
    if run.run_spec_version != 3
        || run.assignment.communication() != Some(&data.assignment)
        || data.run_id != run.id
        || data.project_id != run.project_id
        || Some(data.employee_id) != run.employee_id
    {
        return Err(invalid("Communication context differs from Run owner"));
    }
    Ok(value)
}

pub(super) fn delivery_scope(run: &RunProjection) -> Result<DeliveryScope, CoreError> {
    let context = context(run)?;
    let scope = DeliveryScope {
        project_id: run.project_id,
        employee_id: run.require_employee_id()?,
        thread_id: context.data().assignment.thread_id,
        run_id: run.id,
        fencing_token: run.lease_fencing_token,
        environment_epoch: run.environment_epoch,
        task_context: None,
    };
    scope.validate_message(&context.data().source_message)?;
    Ok(scope)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArguments {
    after: Option<Uuid>,
    limit: Option<u32>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompleteArguments {}

impl RunGateway {
    pub(super) async fn execute_communication(
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
        let owner = &context.data().assignment;
        let scope = delivery_scope(&run)?;
        if request.tool == "inbox.list" {
            let args: ListArguments = serde_json::from_value(request.arguments)
                .map_err(|_| invalid("invalid Inbox list arguments"))?;
            let limit = args.limit.unwrap_or(25);
            if !(1..=100).contains(&limit) || args.after.is_some_and(|id| id.get_version_num() != 7)
            {
                return Err(invalid("invalid Inbox page"));
            }
            let mut items = tx
                .messages_for_communication(&scope, owner.source_message_id, args.after, limit + 1)
                .await?;
            let more = items.len() > limit as usize;
            items.truncate(limit as usize);
            for message in &items {
                scope.validate_message(message)?;
            }
            let next_after = more.then(|| items.last().map(|m| m.data().id)).flatten();
            tx.commit().await?;
            return Ok(
                json!({"messages":items,"next_after":next_after,"source_message_id":owner.source_message_id,
                "delivery":"read_only","acknowledgement_recorded":false}),
            );
        }
        if request.tool != "communication.complete" {
            return Err(invalid("unsupported Communication action"));
        }
        let _: CompleteArguments = serde_json::from_value(request.arguments)
            .map_err(|_| invalid("completion accepts no authority fields"))?;
        let hash: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        if let Some(receipt) = tx
            .gateway_receipt(self.scope, request.message_id, &hash)
            .await?
        {
            return Ok(receipt);
        }
        if !tx
            .communication_requirement_satisfied(self.scope, owner.source_message_id)
            .await?
        {
            return Err(invalid(
                "assigned source requires an Employee acknowledgement or answer",
            ));
        }
        if !tx.complete_communication_run(run.id).await? {
            return Err(invalid("Communication is no longer active"));
        }
        let result = json!({"status":"accepted","message_id":request.message_id,"assignment_id":owner.assignment_id,
            "run_id":run.id,"physical_quiescence":false});
        match tx
            .record_gateway_submission(&GatewaySubmissionRecord {
                scope: self.scope,
                message_id: request.message_id,
                payload_hash: hash,
                receipt: result.clone(),
            })
            .await?
        {
            GatewayWriteResult::Applied => {}
            GatewayWriteResult::Replayed(value) => return Ok(value),
            GatewayWriteResult::Conflict => return Err(CoreError::IdempotencyConflict),
            GatewayWriteResult::Denied => return Err(invalid("Run scope revoked")),
        }
        let mut project = tx
            .lock_project(run.project_id)
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "project",
            })?;
        let now = crate::canonical_clock::project_mutation_time(&project);
        let previous = project.revision();
        project.record_child_mutation(now)?;
        tx.update_project(&project, previous).await?;
        tx.append_event_and_outbox(&event(
            project.id(),
            AggregateRef::Project(project.id()),
            project.revision(),
            DomainEventKind::CommunicationCompleted,
            Actor::employee(ActorId::from(run.require_employee_id()?.as_uuid())),
            CommandId::from(request.message_id),
            None,
            event_payload([
                ("run_id", json!(run.id)),
                ("assignment", json!(run.assignment)),
            ]),
            now,
        )?)
        .await?;
        tx.commit().await?;
        if self
            .core
            .reconcile_runtime_inputs(Some(run.project_id))
            .await
            .is_err()
        {
            tracing::warn!(run_id=%run.id,"deferred runtime input close delivery failed");
        }
        Ok(result)
    }
}
