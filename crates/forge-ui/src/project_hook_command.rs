//! Exact owner boundary for immutable Project hook configuration.

use forge_domain::{ProjectHookVersion, ProjectId, Timestamp};
use forge_protocol::wire::CommandReceipt;
use serde::Deserialize;

use crate::{command::uuid_v7, http::ApiError};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    project_id: String,
    expected_revision: u64,
    payload: serde_json::Value,
}

pub(crate) struct ConfigureProjectHook {
    expected_revision: u64,
}

impl ConfigureProjectHook {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, ApiError> {
        let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !uuid_v7(&envelope.project_id)
            || !(1..MAX_SAFE_INTEGER).contains(&envelope.expected_revision)
        {
            return Err(ApiError::BadRequest);
        }
        let project = ProjectId::from(
            uuid::Uuid::parse_str(&envelope.project_id).map_err(|_| ApiError::BadRequest)?,
        );
        let mut payload = envelope
            .payload
            .as_object()
            .cloned()
            .ok_or(ApiError::BadRequest)?;
        if payload.contains_key("id")
            || payload.contains_key("project_id")
            || payload.contains_key("created_at")
        {
            return Err(ApiError::BadRequest);
        }
        payload.insert("id".into(), serde_json::json!(uuid::Uuid::now_v7()));
        payload.insert("project_id".into(), serde_json::json!(project));
        payload.insert("created_at".into(), serde_json::json!(Timestamp::now_utc()));
        let version: ProjectHookVersion =
            serde_json::from_value(serde_json::Value::Object(payload))
                .map_err(|_| ApiError::BadRequest)?;
        version
            .validate_snapshot()
            .map_err(|_| ApiError::BadRequest)?;
        Ok(Self {
            expected_revision: envelope.expected_revision,
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
                resource.kind == "project_hook_version" && uuid_v7(&resource.id)
            })
        {
            return Err(ApiError::BadGateway);
        }
        Ok(())
    }
}
