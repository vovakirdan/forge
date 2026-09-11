use forge_domain::{Actor, ActorId, AggregateRef, CommandId, DomainEventKind, runtime::RunScope};
use forge_storage::{GatewaySubmissionRecord, GatewayWriteResult, KnowledgeContextRefreshRecord};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    CoreError, CoreService,
    event::{event, event_payload},
};

impl CoreService {
    /// Explicit fenced tool refresh. Never edits the initial manifest or claims initial prompt delivery.
    pub async fn refresh_run_knowledge(
        &self,
        scope: RunScope,
        message_id: Uuid,
        payload_hash: &str,
    ) -> Result<Value, CoreError> {
        let mut tx = self.store.begin().await?;
        let run = tx
            .validate_gateway_scope(&scope)
            .await?
            .ok_or(CoreError::Forbidden)?;
        if run.assignment.hook().is_some() || run.assignment.system_job().is_some() {
            return Err(CoreError::Forbidden);
        }
        let employee_id = run.require_employee_id()?;
        if !run.context_manifest["capability_grants"]
            .as_array()
            .is_some_and(|grants| {
                grants
                    .iter()
                    .any(|grant| grant.as_str() == Some("memory.refresh"))
            })
        {
            return Err(CoreError::Forbidden);
        }
        if let Some(receipt) = tx.gateway_receipt(scope, message_id, payload_hash).await? {
            return Ok(receipt);
        }
        let mut project = tx
            .lock_project(run.project_id)
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "project",
            })?;
        let initial_id = run.context_manifest["context_snapshot_id"]
            .as_str()
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or_else(super::invalid_encoding)?;
        let bundle = super::compile_knowledge_context(&mut tx, run.project_id, employee_id).await?;
        let now = crate::canonical_clock::project_mutation_time(&project);
        let snapshot_id = Uuid::now_v7();
        let mut snapshot = run.context_manifest.clone();
        snapshot["context_snapshot_id"] = json!(snapshot_id);
        snapshot["created_at"] = json!(now);
        snapshot["knowledge_context"] =
            serde_json::to_value(&bundle).map_err(|_| super::invalid_encoding())?;
        // Revalidate the complete historical-purpose schema after replacing only M3 inputs.
        if run.assignment.task_stage().is_some() {
            let _: forge_domain::ContextSnapshot =
                serde_json::from_value(snapshot.clone()).map_err(|_| super::invalid_encoding())?;
        } else if run.assignment.communication().is_some() {
            let _: forge_domain::communication::CommunicationContext =
                serde_json::from_value(snapshot.clone()).map_err(|_| super::invalid_encoding())?;
        } else {
            let _: forge_domain::resolution::ResolutionContext =
                serde_json::from_value(snapshot.clone()).map_err(|_| super::invalid_encoding())?;
        }
        let content_hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).map_err(|_| super::invalid_encoding())?)
        );
        let receipt = json!({"status":"accepted","message_id":message_id,"tool":"memory.refresh","delivery":"tool_response","initial_context_snapshot_id":initial_id,"context_snapshot_id":snapshot_id,"content_hash":content_hash,"context":snapshot});
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
            GatewayWriteResult::Replayed(value) => return Ok(value),
            GatewayWriteResult::Conflict => return Err(CoreError::IdempotencyConflict),
            GatewayWriteResult::Denied => return Err(CoreError::Forbidden),
        }
        tx.insert_knowledge_context_refresh(&KnowledgeContextRefreshRecord {
            id: snapshot_id,
            project_id: run.project_id,
            run_id: run.id,
            employee_id,
            initial_context_snapshot_id: initial_id,
            message_id,
            content_hash: content_hash.clone(),
            context_snapshot: snapshot,
            created_at: now,
        })
        .await?;
        let prior_revision = project.revision();
        project.record_child_mutation(now)?;
        tx.update_project(&project, prior_revision).await?;
        tx.append_event_and_outbox(&event(
            run.project_id,
            AggregateRef::Project(run.project_id),
            project.revision(),
            DomainEventKind::KnowledgeContextRefreshed,
            Actor::employee(ActorId::from(employee_id.as_uuid())),
            CommandId::from(message_id),
            None,
            event_payload([
                ("run_id", json!(run.id)),
                ("context_snapshot_id", json!(snapshot_id)),
                ("initial_context_snapshot_id", json!(initial_id)),
                ("content_hash", json!(content_hash)),
                ("delivery", json!("tool_response")),
                (
                    "scope",
                    json!({"visibility":"personal","employee_id":employee_id}),
                ),
            ]),
            now,
        )?)
        .await?;
        tx.commit().await?;
        Ok(receipt)
    }
}
