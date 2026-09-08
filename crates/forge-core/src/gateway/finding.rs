//! A fenced Employee can report observations, never promote or triage backlog.
use super::{RunGateway, ToolRequest, invalid};
use crate::CoreError;
use forge_application::{CommandContext, ReportFindingCommand, SystemClock, engine::Engine};
use forge_domain::{Actor, ActorId, CommandId};
use forge_protocol::wire::CommandName;
use forge_storage::{GatewaySubmissionRecord, GatewayWriteResult};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportArguments {
    description: String,
    severity: String,
    #[serde(default)]
    evidence: BTreeSet<forge_domain::ArtifactId>,
}
impl RunGateway {
    pub(super) async fn report_finding(
        &self,
        request: ToolRequest,
        digest: [u8; 32],
    ) -> Result<Value, CoreError> {
        let args: ReportArguments = serde_json::from_value(request.arguments)
            .map_err(|_| invalid("invalid Finding report"))?;
        let hash: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        let mut tx = self.core.store.begin().await?;
        let run = tx
            .validate_gateway_scope(&self.scope)
            .await?
            .ok_or_else(|| invalid("Run scope revoked"))?;
        let delivery = super::task_scope(&mut tx, &run).await?;
        if let Some(value) = tx
            .gateway_receipt(self.scope, request.message_id, &hash)
            .await?
        {
            return Ok(value);
        }
        let task = run.require_task_id()?;
        if delivery
            .task_context
            .as_ref()
            .is_none_or(|context| context.task_id != task)
        {
            return Err(invalid("Finding source Task differs"));
        }
        let mut project = tx
            .lock_project(self.project_id)
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "project",
            })?;
        let now = crate::canonical_clock::project_mutation_time(&project);
        let context = CommandContext {
            project_id: self.project_id,
            actor: Actor::employee(ActorId::from(run.require_employee_id()?.as_uuid())),
            core_actor: self.core.actors.core,
            capabilities: vec![CommandName::ReportFinding],
        };
        let input = ReportFindingCommand {
            source_task_id: task,
            description: args.description,
            severity: args.severity,
            evidence: args.evidence,
        };
        let (finding, audit) = Engine::new(&context, &SystemClock)
            .record_finding(
                &mut tx,
                &mut project,
                &input,
                Some(self.scope),
                CommandId::from(request.message_id),
                now,
            )
            .await?;
        let result = json!({"status":"reported","finding_id":finding.id,"revision":finding.revision,"task_created":false});
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
        tx.append_event_and_outbox(&audit).await?;
        tx.commit().await?;
        Ok(result)
    }
}
