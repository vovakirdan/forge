//! Secret-free named runtime configuration for an existing Employee.

use forge_domain::{ProjectId, runtime::RuntimeBinding};
use forge_protocol::wire::CommandReceipt;
use serde::Deserialize;

use crate::{command::uuid_v7, http::ApiError};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

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
    employee_id: String,
    binding: RuntimeBinding,
}

pub(crate) struct ConfigureEmployeeRuntime {
    expected_revision: u64,
    employee_id: String,
}

impl ConfigureEmployeeRuntime {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, ApiError> {
        let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !uuid_v7(&envelope.project_id)
            || !uuid_v7(&envelope.payload.employee_id)
            || !(1..MAX_SAFE_INTEGER).contains(&envelope.expected_revision)
        {
            return Err(ApiError::BadRequest);
        }
        let project = ProjectId::from(
            uuid::Uuid::parse_str(&envelope.project_id).map_err(|_| ApiError::BadRequest)?,
        );
        envelope
            .payload
            .binding
            .validate()
            .map_err(|_| ApiError::BadRequest)?;
        if envelope.payload.binding.execution_profile.project_id() != project {
            return Err(ApiError::BadRequest);
        }
        Ok(Self {
            expected_revision: envelope.expected_revision,
            employee_id: envelope.payload.employee_id,
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
                resource.kind == "employee" && resource.id.eq_ignore_ascii_case(&self.employee_id)
            })
        {
            return Err(ApiError::BadGateway);
        }
        Ok(())
    }
}
