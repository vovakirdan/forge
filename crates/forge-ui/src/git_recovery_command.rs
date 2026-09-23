//! Narrow browser commands for a held Git integration operation.

use forge_protocol::wire::CommandReceipt;
use serde::Deserialize;

use crate::{
    command::{CommandTarget, uuid_v7},
    http::ApiError,
};

const MAX_SAFE: u64 = 9_007_199_254_740_991;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    project_id: String,
    expected_revision: u64,
    payload: Payload,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    operation_id: String,
    expected_task_revision: u64,
    reason: String,
}

pub(crate) struct GitRecoveryCommand {
    expected_revision: u64,
    operation_id: String,
}

impl GitRecoveryCommand {
    pub(crate) fn parse(bytes: &[u8], target: CommandTarget) -> Result<Self, ApiError> {
        if !matches!(
            target,
            CommandTarget::RetryGitIntegration | CommandTarget::AcceptGitIntegrationResult
        ) {
            return Err(ApiError::BadRequest);
        }
        let value: Envelope = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !uuid_v7(&value.project_id)
            || !uuid_v7(&value.payload.operation_id)
            || !(1..MAX_SAFE).contains(&value.expected_revision)
            || !(1..MAX_SAFE).contains(&value.payload.expected_task_revision)
            || value.payload.reason.trim().is_empty()
            || value.payload.reason.len() > 4096
            || value.payload.reason.contains('\0')
        {
            return Err(ApiError::BadRequest);
        }
        Ok(Self {
            expected_revision: value.expected_revision,
            operation_id: value.payload.operation_id,
        })
    }

    pub(crate) fn validate_receipt(&self, bytes: &[u8]) -> Result<(), ApiError> {
        let receipt: CommandReceipt =
            serde_json::from_slice(bytes).map_err(|_| ApiError::BadGateway)?;
        if !uuid_v7(&receipt.command_id)
            || receipt.project_revision != self.expected_revision + 1
            || receipt.event_ids.is_empty()
            || !receipt.event_ids.iter().all(|id| uuid_v7(id))
            || !receipt.resource.as_ref().is_some_and(|resource| {
                resource.kind == "git_integration"
                    && resource.id.eq_ignore_ascii_case(&self.operation_id)
            })
        {
            return Err(ApiError::BadGateway);
        }
        Ok(())
    }
}
