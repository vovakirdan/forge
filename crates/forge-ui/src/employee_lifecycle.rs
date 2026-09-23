//! Narrow browser contract for the three Core-owned Employee lifecycle commands.

use forge_protocol::wire::{CommandReceipt, MAX_COMMAND_AUDIT_DETAIL_CHARACTERS};
use serde::Deserialize;

use crate::{command::uuid_v7, http::ApiError};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmployeeLifecycle {
    project_id: String,
    expected_revision: u64,
    payload: Payload,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    employee_id: String,
    expected_employee_revision: u64,
    reason: Option<String>,
}

impl EmployeeLifecycle {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, ApiError> {
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !uuid_v7(&value.project_id)
            || !uuid_v7(&value.payload.employee_id)
            || !(1..MAX_SAFE_INTEGER).contains(&value.expected_revision)
            || !(1..MAX_SAFE_INTEGER).contains(&value.payload.expected_employee_revision)
            || value.payload.reason.as_ref().is_some_and(|reason| {
                reason.trim().is_empty()
                    || reason.contains('\0')
                    || reason.chars().count() > MAX_COMMAND_AUDIT_DETAIL_CHARACTERS
            })
        {
            return Err(ApiError::BadRequest);
        }
        Ok(value)
    }

    pub(crate) fn validate_receipt(&self, bytes: &[u8]) -> Result<(), ApiError> {
        let receipt: CommandReceipt =
            serde_json::from_slice(bytes).map_err(|_| ApiError::BadGateway)?;
        if !uuid_v7(&receipt.command_id)
            || receipt.project_revision != self.expected_revision + 1
            || receipt.event_ids.is_empty()
            || !receipt.event_ids.iter().all(|id| uuid_v7(id))
            || !receipt.resource.as_ref().is_some_and(|resource| {
                resource.kind == "employee"
                    && resource.id.eq_ignore_ascii_case(&self.payload.employee_id)
                    && uuid_v7(&resource.id)
            })
        {
            return Err(ApiError::BadGateway);
        }
        Ok(())
    }
}
