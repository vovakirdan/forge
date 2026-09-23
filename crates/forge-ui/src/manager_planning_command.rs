//! Exact Project management intent commands without scheduler inference.

use forge_protocol::wire::CommandReceipt;
use serde::Deserialize;

use crate::{
    command::{CommandTarget, uuid_v7},
    http::ApiError,
};

const MAX_SAFE: u64 = 9_007_199_254_740_991;
fn positive(value: u64) -> bool {
    (1..MAX_SAFE).contains(&value)
}
fn reason(value: Option<&str>, required: bool) -> bool {
    match value {
        Some(value) => !value.trim().is_empty() && value.len() <= 10_000 && !value.contains('\0'),
        None => !required,
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    project_id: String,
    expected_revision: u64,
    payload: serde_json::Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetNext {
    task_id: String,
    expected_task_revision: u64,
    employee_id: String,
    reason: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClearNext {
    task_id: String,
    expected_task_revision: u64,
    reason: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Schedule {
    task_id: String,
    expected_task_revision: u64,
    wait_condition_id: String,
    not_before: String,
    reason: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cancel {
    schedule_id: String,
    reason: Option<String>,
}

pub(crate) struct ManagerPlanningCommand {
    expected_revision: u64,
    resource_kind: &'static str,
    resource_id: Option<String>,
}
impl ManagerPlanningCommand {
    pub(crate) fn parse(bytes: &[u8], target: CommandTarget) -> Result<Self, ApiError> {
        let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !uuid_v7(&envelope.project_id) || !positive(envelope.expected_revision) {
            return Err(ApiError::BadRequest);
        }
        let (resource_kind, resource_id) = match target {
            CommandTarget::SetNextRunEmployee => {
                let value: SetNext =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                if !uuid_v7(&value.task_id)
                    || !uuid_v7(&value.employee_id)
                    || !positive(value.expected_task_revision)
                    || !reason(value.reason.as_deref(), false)
                {
                    return Err(ApiError::BadRequest);
                }
                ("task", Some(value.task_id))
            }
            CommandTarget::ClearNextRunEmployee => {
                let value: ClearNext =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                if !uuid_v7(&value.task_id)
                    || !positive(value.expected_task_revision)
                    || !reason(value.reason.as_deref(), false)
                {
                    return Err(ApiError::BadRequest);
                }
                ("task", Some(value.task_id))
            }
            CommandTarget::ScheduleTaskResume => {
                let value: Schedule =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                if !uuid_v7(&value.task_id)
                    || !uuid_v7(&value.wait_condition_id)
                    || !positive(value.expected_task_revision)
                    || !reason(Some(&value.reason), true)
                    || time::OffsetDateTime::parse(
                        &value.not_before,
                        &time::format_description::well_known::Rfc3339,
                    )
                    .is_err()
                {
                    return Err(ApiError::BadRequest);
                }
                ("task_resume_schedule", None)
            }
            CommandTarget::CancelTaskResume => {
                let value: Cancel =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                if !uuid_v7(&value.schedule_id) || !reason(value.reason.as_deref(), false) {
                    return Err(ApiError::BadRequest);
                }
                ("task_resume_schedule", Some(value.schedule_id))
            }
            _ => return Err(ApiError::BadRequest),
        };
        Ok(Self {
            expected_revision: envelope.expected_revision,
            resource_kind,
            resource_id,
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
                resource.kind == self.resource_kind
                    && uuid_v7(&resource.id)
                    && self
                        .resource_id
                        .as_ref()
                        .is_none_or(|id| id.eq_ignore_ascii_case(&resource.id))
            })
        {
            return Err(ApiError::BadGateway);
        }
        Ok(())
    }
}
