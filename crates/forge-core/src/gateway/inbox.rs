//! Task-directed communication. General Inbox work uses a separate assignment.
use forge_domain::{
    Actor, ActorId, AggregateRef, CommandId, ContextSnapshot, DomainEventKind, LifecycleStatus,
    communication::{
        DeliveryReceipt, DeliveryReceiptKind, DeliveryRequirement, DeliveryScope, MessageKind,
        SendMessageInput, TaskMessageContext,
    },
};
use forge_storage::{
    GatewaySubmissionRecord, GatewayWriteResult, RunProjection, StorageTransaction,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{RunGateway, ToolRequest, invalid};
use crate::{
    CoreError,
    event::{event, event_payload},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArguments {
    after: Option<Uuid>,
    limit: Option<u32>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AckArguments {
    target_message_id: Uuid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyArguments {
    target_message_id: Uuid,
    body: String,
}

impl RunGateway {
    pub(super) async fn execute_inbox(
        &self,
        request: ToolRequest,
        digest: [u8; 32],
    ) -> Result<Value, CoreError> {
        if self.assignment.communication().is_some() && request.tool == "inbox.list" {
            return self.execute_communication(request, digest).await;
        }
        let mut tx = self.core.store.begin().await?;
        let run = tx
            .validate_gateway_scope(&self.scope)
            .await?
            .ok_or_else(|| invalid("Run scope is no longer active"))?;
        let mut scope = if run.assignment.communication().is_some() {
            super::communication_run::delivery_scope(&run)?
        } else {
            task_scope(&mut tx, &run).await?
        };
        if request.tool == "inbox.list" {
            let args: ListArguments = serde_json::from_value(request.arguments)
                .map_err(|_| invalid("invalid Inbox list arguments"))?;
            let limit = args.limit.unwrap_or(25);
            if !(1..=100).contains(&limit) || args.after.is_some_and(|id| id.get_version_num() != 7)
            {
                return Err(invalid("invalid Inbox page"));
            }
            let mut items = tx
                .messages_for_task_delivery(&scope, args.after, limit + 1)
                .await?;
            let more = items.len() > limit as usize;
            items.truncate(limit as usize);
            for message in &items {
                scope.thread_id = message.data().thread_id;
                scope.validate_message(message)?;
            }
            let next_after = more.then(|| items.last().map(|m| m.data().id)).flatten();
            tx.commit().await?;
            return Ok(json!({"messages":items,"next_after":next_after,
                "delivery":"read_only","acknowledgement_recorded":false}));
        }
        let (target_message_id, body) = if request.tool == "inbox.acknowledge" {
            let args: AckArguments = serde_json::from_value(request.arguments)
                .map_err(|_| invalid("invalid acknowledgement arguments"))?;
            (args.target_message_id, None)
        } else {
            let args: ReplyArguments = serde_json::from_value(request.arguments)
                .map_err(|_| invalid("invalid reply arguments"))?;
            (args.target_message_id, Some(args.body))
        };
        if target_message_id.get_version_num() != 7 {
            return Err(invalid("message must be UUIDv7"));
        }
        if run
            .assignment
            .communication()
            .is_some_and(|owner| owner.source_message_id != target_message_id)
        {
            return Err(invalid(
                "message is not this Communication assignment source",
            ));
        }
        let message = tx
            .load_employee_message(target_message_id)
            .await?
            .filter(|message| message.data().project_id == run.project_id)
            .ok_or(CoreError::NotFound {
                aggregate: "employee message",
            })?;
        scope.thread_id = message.data().thread_id;
        scope.validate_message(&message)?;
        let hash: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        if let Some(receipt) = tx
            .gateway_receipt(self.scope, request.message_id, &hash)
            .await?
        {
            return Ok(receipt);
        }
        if run.assignment.communication().is_some()
            && tx.communication_run_is_complete(run.id).await?
        {
            return Err(invalid("Communication assignment is already complete"));
        }
        let mut project = tx
            .lock_project(run.project_id)
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "project",
            })?;
        let now = crate::canonical_clock::project_mutation_time(&project);
        let actor = Actor::employee(ActorId::from(run.require_employee_id()?.as_uuid()));
        let reply = if let Some(body) = body {
            let mut thread = tx
                .lock_employee_thread(message.data().thread_id)
                .await?
                .ok_or(CoreError::NotFound {
                    aggregate: "employee thread",
                })?;
            let revision = thread.data().revision;
            let reply = thread.send(SendMessageInput {
                id: Uuid::now_v7(),
                expected_thread_revision: revision,
                sender: actor,
                target: message.data().target.clone(),
                kind: MessageKind::Reply,
                requirement: DeliveryRequirement::Informational,
                body,
                reply_to: Some(target_message_id),
                created_at: now,
            })?;
            tx.update_employee_thread(&thread, revision).await?;
            tx.insert_employee_message(&reply).await?;
            Some(reply)
        } else {
            None
        };
        let kind = if reply.is_some() {
            DeliveryReceiptKind::Answered
        } else {
            DeliveryReceiptKind::Acknowledged
        };
        let receipt = DeliveryReceipt::new(
            &message,
            scope,
            kind,
            reply.as_ref().map(|m| m.data().id),
            now,
        )?;
        let inserted = tx.insert_delivery_receipt(&receipt).await?;
        if !inserted {
            return Err(invalid(
                "receipt already recorded; retry the original message_id",
            ));
        }
        let result = json!({"status":"accepted","message_id":request.message_id,
            "target_message_id":target_message_id,"receipt":receipt,"reply":reply});
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
        if inserted {
            let revision = project.revision();
            project.record_child_mutation(now)?;
            tx.update_project(&project, revision).await?;
            let audit = event(
                project.id(),
                AggregateRef::Project(project.id()),
                project.revision(),
                if reply.is_some() {
                    DomainEventKind::EmployeeMessageAnswered
                } else {
                    DomainEventKind::EmployeeMessageAcknowledged
                },
                actor,
                CommandId::from(request.message_id),
                None,
                event_payload([
                    ("message_id", json!(target_message_id)),
                    ("run_id", json!(run.id)),
                    ("reply_id", json!(receipt.reply_id())),
                    ("thread_id", json!(message.data().thread_id)),
                ]),
                now,
            )?;
            tx.append_event_and_outbox(&audit).await?;
        }
        tx.commit().await?;
        Ok(result)
    }
}

pub(crate) async fn task_scope(
    tx: &mut StorageTransaction<'_>,
    run: &RunProjection,
) -> Result<DeliveryScope, CoreError> {
    let context: ContextSnapshot = serde_json::from_value(run.context_manifest.clone())
        .map_err(|_| invalid("invalid pinned Run context"))?;
    let input = context.data();
    let task = tx
        .lock_task(run.require_task_id()?)
        .await?
        .ok_or(CoreError::NotFound { aggregate: "task" })?
        .task;
    let visit = input
        .stage_visit
        .ok_or_else(|| invalid("historical Run has no Inbox assignment"))?;
    if input.run_id != run.id
        || Some(input.employee_id) != run.employee_id
        || input.project_id != run.project_id
        || input.task_id != run.require_task_id()?
        || task.project_id() != run.project_id
        || input.pipeline_version_id != task.pipeline().pipeline_version_id()
        || input.stage_id.as_str() != run.require_task_stage()?.stage_id.as_str()
        || task.current_stage_id() != Some(&input.stage_id)
        || task.current_stage_visit().map(|v| v.get()) != Some(visit)
        || task.lifecycle() != LifecycleStatus::InProgress
    {
        return Err(invalid("Run no longer owns this Task visit"));
    }
    Ok(DeliveryScope {
        project_id: run.project_id,
        employee_id: run.require_employee_id()?,
        // The bounded query ignores thread_id; each retrieved message supplies it
        // before domain validation, and only validated receipts are persisted.
        thread_id: run.id,
        run_id: run.id,
        fencing_token: run.lease_fencing_token,
        environment_epoch: run.environment_epoch,
        task_context: Some(TaskMessageContext {
            task_id: run.require_task_id()?,
            pipeline_version_id: input.pipeline_version_id,
            stage_id: input.stage_id.clone(),
            stage_visit: visit,
        }),
    })
}
