//! Narrow browser contract for the two canonical dependency commands.
use serde::Deserialize;

use crate::{
    command::{CommandTarget, uuid_v7},
    http::ApiError,
};
use forge_protocol::wire::CommandReceipt;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DependencyCommand {
    project_id: String,
    expected_revision: u64,
    payload: DependencyPayload,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DependencyPayload {
    blocker_task_id: String,
    blocked_task_id: String,
    #[serde(default)]
    required_condition: Option<String>,
}

impl DependencyCommand {
    pub(crate) fn parse(bytes: &[u8], target: CommandTarget) -> Result<Self, ApiError> {
        let shape: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !shape.is_object()
            || !shape
                .get("payload")
                .is_some_and(serde_json::Value::is_object)
        {
            return Err(ApiError::BadRequest);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        let condition_ok = match target {
            CommandTarget::CreateDependency => {
                value.payload.required_condition.as_deref() == Some("task_done")
            }
            CommandTarget::RemoveDependency => {
                shape.pointer("/payload/required_condition").is_none()
            }
            _ => false,
        };
        if !uuid_v7(&value.project_id)
            || !uuid_v7(&value.payload.blocker_task_id)
            || !uuid_v7(&value.payload.blocked_task_id)
            || value
                .payload
                .blocker_task_id
                .eq_ignore_ascii_case(&value.payload.blocked_task_id)
            || !(1..9_007_199_254_740_991_u64).contains(&value.expected_revision)
            || !condition_ok
        {
            return Err(ApiError::BadRequest);
        }
        Ok(value)
    }

    pub(crate) fn validate_receipt(&self, bytes: &[u8]) -> Result<(), ApiError> {
        let receipt: CommandReceipt =
            serde_json::from_slice(bytes).map_err(|_| ApiError::BadGateway)?;
        let changed = !receipt.event_ids.is_empty();
        if !uuid_v7(&receipt.command_id)
            || !receipt.event_ids.iter().all(|id| uuid_v7(id))
            || (changed && receipt.project_revision <= self.expected_revision)
            || (!changed && receipt.project_revision != self.expected_revision)
            || receipt.project_revision > 9_007_199_254_740_991_u64
            || !receipt.resource.as_ref().is_some_and(|resource| {
                resource.kind == "task_dependency"
                    && resource
                        .id
                        .eq_ignore_ascii_case(&self.payload.blocked_task_id)
            })
        {
            return Err(ApiError::BadGateway);
        }
        Ok(())
    }
}
