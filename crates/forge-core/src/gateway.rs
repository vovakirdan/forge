//! One immutable Run capability surface, shared by HTTP and maintained MCP SDK.

mod audit;
mod calls;
mod catalog;
mod mcp;
mod signals;
mod socket;

use crate::{CoreError, CoreService};
use forge_domain::{ContextSnapshot, ProjectId, TaskId, runtime::RunScope};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use uuid::Uuid;

pub use socket::GatewayHandle;
pub(crate) const MAX_REQUEST_BYTES: usize = 256 * 1024;

#[derive(Clone)]
pub(super) struct RunGateway {
    core: CoreService,
    scope: RunScope,
    project_id: ProjectId,
    task_id: TaskId,
    grants: Arc<BTreeSet<String>>,
    closed: Arc<AtomicBool>,
    requests: Arc<tokio::sync::Semaphore>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ToolRequest {
    message_id: Uuid,
    tool: String,
    arguments: Value,
}

impl CoreService {
    /// Binds only this Run's owner-only socket, never the administrative Core API.
    pub async fn serve_run_gateway(
        &self,
        run_id: Uuid,
        root: &Path,
    ) -> Result<GatewayHandle, CoreError> {
        let run = self
            .store
            .load_run(run_id)
            .await?
            .ok_or(CoreError::NotFound { aggregate: "run" })?;
        let context: ContextSnapshot = serde_json::from_value(run.context_manifest.clone())
            .map_err(|_| invalid("stored context is invalid"))?;
        if context.data().run_id != run_id
            || context.data().project_id != run.project_id
            || context.data().task_id != run.task_id
            || context.data().employee_id != run.employee_id
            || context.data().stage_id.as_str() != run.stage_id
        {
            return Err(invalid("stored context scope differs from Run"));
        }
        let gateway = RunGateway {
            core: self.clone(),
            scope: RunScope {
                run_id,
                fencing_token: run.lease_fencing_token,
                environment_epoch: run.environment_epoch,
            },
            project_id: run.project_id,
            task_id: run.task_id,
            grants: Arc::new(
                context
                    .data()
                    .capability_grants
                    .iter()
                    .filter(|grant| catalog::logical_tool(grant).is_some())
                    .cloned()
                    .collect(),
            ),
            closed: Arc::new(AtomicBool::new(false)),
            requests: Arc::new(tokio::sync::Semaphore::new(32)),
        };
        gateway.authorize().await?;
        socket::serve(gateway, root).await
    }
}

impl RunGateway {
    async fn authorize(&self) -> Result<(), CoreError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(invalid("Run Gateway is closed"));
        }
        let mut transaction = self.core.store.begin().await?;
        let run = transaction
            .validate_gateway_scope(&self.scope)
            .await?
            .ok_or_else(|| invalid("Run scope is no longer active"))?;
        if run.project_id != self.project_id || run.task_id != self.task_id {
            return Err(invalid("Run scope changed"));
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn invoke(&self, request: ToolRequest) -> Result<Value, CoreError> {
        self.authorize().await?;
        self.core
            .reject_run_secrets(self.scope, &request.arguments)
            .await?;
        if request.message_id.get_version_num() != 7 || !self.grants.contains(&request.tool) {
            self.audit_decision(
                &request.tool,
                Some(request.message_id),
                false,
                "scope_or_grant_denied",
            )
            .await?;
            return Err(invalid("message must be UUIDv7 and tool must be granted"));
        }
        let bytes = serde_json::to_vec(&json!({"tool":request.tool,"arguments":request.arguments}))
            .map_err(|_| invalid("invalid arguments"))?;
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(invalid("arguments exceed request limit"));
        }
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        self.audit_decision(
            &request.tool,
            Some(request.message_id),
            true,
            "capability_granted",
        )
        .await?;
        let tool = request.tool.clone();
        let message_id = request.message_id;
        let result = self.execute(request, digest).await;
        if result.is_err() {
            // Failure to audit a rejection must not transform an already
            // rejected request into success, nor expose the underlying input.
            let _ = self
                .audit_decision(&tool, Some(message_id), false, "call_rejected")
                .await;
        }
        result
    }
}

fn invalid(reason: &str) -> CoreError {
    CoreError::InvalidTransport {
        field: "gateway",
        reason: reason.to_owned(),
    }
}

fn safe_error(error: &CoreError) -> Value {
    let (code, message) = match error {
        CoreError::IdempotencyConflict => (
            "idempotency_conflict",
            "Reuse the original message and payload, or create a new message_id.",
        ),
        CoreError::Storage(_) => (
            "temporarily_unavailable",
            "Forge storage is unavailable; retry this same message_id.",
        ),
        CoreError::NotFound { .. } => ("not_found", "No readable task exists in this Project."),
        _ => (
            "request_rejected",
            "Check the granted tool schema and active Run scope; Forge made no unsupported transition.",
        ),
    };
    json!({"status":"error","code":code,"message":message})
}
