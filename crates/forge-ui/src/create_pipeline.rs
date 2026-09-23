//! Exact browser boundary for a complete first Pipeline graph.

use forge_protocol::wire::CommandReceipt;
use serde::Deserialize;

use crate::{
    command::uuid_v7,
    http::ApiError,
    pipeline_management_command::{Definition, valid_definition},
};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    project_id: String,
    expected_revision: u64,
    payload: serde_json::Value,
}

pub(crate) struct CreatePipeline {
    expected_revision: u64,
}
impl CreatePipeline {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, ApiError> {
        let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !uuid_v7(&envelope.project_id)
            || !(1..MAX_SAFE_INTEGER).contains(&envelope.expected_revision)
        {
            return Err(ApiError::BadRequest);
        }
        let mut payload = envelope
            .payload
            .as_object()
            .cloned()
            .ok_or(ApiError::BadRequest)?;
        let name = payload
            .remove("name")
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or(ApiError::BadRequest)?;
        if name.trim().is_empty() || name.chars().count() > 128 {
            return Err(ApiError::BadRequest);
        }
        let definition: Definition = serde_json::from_value(serde_json::Value::Object(payload))
            .map_err(|_| ApiError::BadRequest)?;
        if !valid_definition(&definition) {
            return Err(ApiError::BadRequest);
        }
        Ok(Self {
            expected_revision: envelope.expected_revision,
        })
    }

    pub(crate) fn validate_receipt(&self, bytes: &[u8]) -> Result<(), ApiError> {
        let receipt: CommandReceipt =
            serde_json::from_slice(bytes).map_err(|_| ApiError::BadGateway)?;
        if !uuid_v7(&receipt.command_id)
            || receipt.project_revision != self.expected_revision + 1
            || receipt.event_ids.len() < 2
            || !receipt.event_ids.iter().all(|id| uuid_v7(id))
            || !receipt
                .resource
                .as_ref()
                .is_some_and(|resource| resource.kind == "pipeline" && uuid_v7(&resource.id))
        {
            return Err(ApiError::BadGateway);
        }
        Ok(())
    }
}
