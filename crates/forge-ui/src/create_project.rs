//! Exact browser creation boundary for a caller-reserved Project identity.

use forge_protocol::wire::CommandReceipt;
use serde::Deserialize;

use crate::{command::uuid_v7, http::ApiError};

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
    name: String,
}

pub(crate) struct CreateProject {
    id: String,
}
impl CreateProject {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, ApiError> {
        let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !uuid_v7(&envelope.project_id)
            || envelope.expected_revision != 0
            || envelope.payload.name.trim().is_empty()
            || envelope.payload.name.chars().count() > 240
        {
            return Err(ApiError::BadRequest);
        }
        Ok(Self {
            id: envelope.project_id,
        })
    }

    pub(crate) fn validate_receipt(&self, bytes: &[u8]) -> Result<(), ApiError> {
        let receipt: CommandReceipt =
            serde_json::from_slice(bytes).map_err(|_| ApiError::BadGateway)?;
        if !uuid_v7(&receipt.command_id)
            || receipt.project_revision != 1
            || receipt.event_ids.is_empty()
            || !receipt.event_ids.iter().all(|id| uuid_v7(id))
            || !receipt.resource.as_ref().is_some_and(|resource| {
                resource.kind == "project" && resource.id.eq_ignore_ascii_case(&self.id)
            })
        {
            return Err(ApiError::BadGateway);
        }
        Ok(())
    }
}
