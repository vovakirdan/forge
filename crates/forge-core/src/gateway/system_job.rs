use super::{RunGateway, ToolRequest, invalid};
use crate::{CoreError, system_jobs::result::SystemJobResult};
use forge_domain::{ExecutionAssignment, runtime::SystemJobRunSpec};
use forge_storage::{GatewaySubmissionRecord, GatewayWriteResult, RunProjection};
use serde_json::{Value, json};

pub(super) fn spec(run: &RunProjection) -> Result<SystemJobRunSpec, CoreError> {
    let spec: SystemJobRunSpec = serde_json::from_value(run.run_spec.clone())
        .map_err(|_| invalid("invalid SystemJob spec"))?;
    spec.validate()
        .map_err(|_| invalid("invalid SystemJob spec"))?;
    if run.run_spec_version != 7
        || run.project_id != spec.project_id
        || run.id != spec.run_id
        || run.employee_id.is_some()
        || run.assignment != ExecutionAssignment::SystemJob(spec.assignment.clone())
        || run.context_manifest.get("input")
            != Some(&serde_json::to_value(&spec.input).map_err(|_| invalid("invalid input"))?)
        || run
            .context_manifest
            .get("provider_instruction")
            .and_then(Value::as_str)
            != Some(spec.instruction.as_str())
    {
        return Err(invalid("SystemJob context differs from its pinned Run"));
    }
    Ok(spec)
}

impl RunGateway {
    pub(super) async fn execute_system_job(
        &self,
        request: ToolRequest,
        digest: [u8; 32],
    ) -> Result<Value, CoreError> {
        if self.assignment.system_job().is_none() {
            return Err(CoreError::Forbidden);
        }
        let mut tx = self.core.store.begin().await?;
        let run = tx
            .validate_gateway_scope(&self.scope)
            .await?
            .ok_or_else(|| invalid("SystemJob is no longer active"))?;
        let spec = spec(&run)?;
        if request.tool == "system_job.read" {
            if request.arguments != json!({}) {
                return Err(invalid("SystemJob read has no arguments"));
            }
            tx.commit().await?;
            return Ok(
                json!({"assignment":spec.assignment,"input":spec.input,"max_result_bytes":spec.max_result_bytes}),
            );
        }
        let result = SystemJobResult::decode(request.arguments.clone(), &spec)?;
        result.validate_sources(&mut tx, &spec).await?;
        let hash: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        let receipt = json!({"accepted":true,"job_id":spec.assignment.job_id,"attempt_id":spec.assignment.attempt_id,"run_id":run.id,"state":"result_received_awaiting_physical_quiescence","source_digest":spec.input.source_digest});
        match tx
            .record_gateway_submission(&GatewaySubmissionRecord {
                scope: self.scope,
                message_id: request.message_id,
                payload_hash: hash.clone(),
                receipt: receipt.clone(),
            })
            .await?
        {
            GatewayWriteResult::Applied => {}
            GatewayWriteResult::Replayed(value) => {
                tx.commit().await?;
                return Ok(value);
            }
            _ => return Err(invalid("result message conflicts with an earlier receipt")),
        }
        tx.save_system_job_result(
            spec.assignment.attempt_id,
            &request.arguments,
            request.message_id,
            &hash,
        )
        .await?;
        tx.request_run_stop(
            run.id,
            run.lease_fencing_token,
            run.environment_epoch,
            false,
        )
        .await?;
        tx.commit().await?;
        // Receipt is durable even if the connection drops during stop delivery.
        let _ = self
            .core
            .deliver_pending_stop_requests(self.project_id)
            .await;
        Ok(receipt)
    }
}
