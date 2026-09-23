//! Bounded owner route configuration and Task escalation creation.
use crate::{
    command::{CommandTarget, uuid_v7},
    http::ApiError,
};
use forge_protocol::wire::CommandReceipt;
use serde::Deserialize;

const MAX_SAFE: u64 = 9_007_199_254_740_991;
fn positive(value: u64) -> bool {
    (1..MAX_SAFE).contains(&value)
}
fn stable(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
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
struct ConfigureRoute {
    route_key: String,
    employee_ids: Vec<String>,
    assignment_timeout_seconds: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raise {
    task_id: String,
    expected_task_revision: u64,
    route_key: Option<String>,
    category: Category,
    question: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Category {
    ActionApproval,
    Clarification,
    ScopeOrPolicyConflict,
    StaleOrInvalidTask,
    TechnicalDecision,
    Blocked,
}

pub(crate) struct ResolverCommand {
    expected_revision: u64,
    kind: &'static str,
    resource_id: Option<String>,
    may_advance_multiple: bool,
}
impl ResolverCommand {
    pub(crate) fn parse(bytes: &[u8], target: CommandTarget) -> Result<Self, ApiError> {
        let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !uuid_v7(&envelope.project_id) || !positive(envelope.expected_revision) {
            return Err(ApiError::BadRequest);
        }
        let (kind, resource_id, may_advance_multiple) = match target {
            CommandTarget::ConfigureResolverRoute => {
                let payload: ConfigureRoute =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                if !stable(&payload.route_key)
                    || payload.employee_ids.len() > 32
                    || !(30..=86_400).contains(&payload.assignment_timeout_seconds)
                    || !payload.employee_ids.iter().all(|id| uuid_v7(id))
                    || payload
                        .employee_ids
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != payload.employee_ids.len()
                {
                    return Err(ApiError::BadRequest);
                }
                ("project", Some(envelope.project_id), false)
            }
            CommandTarget::RaiseEscalation => {
                let payload: Raise =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                let _ = payload.category;
                if !uuid_v7(&payload.task_id)
                    || !positive(payload.expected_task_revision)
                    || payload.route_key.as_ref().is_some_and(|key| !stable(key))
                    || payload.question.trim().is_empty()
                    || payload.question.chars().count() > 20_000
                    || payload.question.contains('\0')
                {
                    return Err(ApiError::BadRequest);
                }
                ("escalation", None, true)
            }
            _ => return Err(ApiError::BadRequest),
        };
        Ok(Self {
            expected_revision: envelope.expected_revision,
            kind,
            resource_id,
            may_advance_multiple,
        })
    }
    pub(crate) fn validate_receipt(&self, bytes: &[u8]) -> Result<(), ApiError> {
        let receipt: CommandReceipt =
            serde_json::from_slice(bytes).map_err(|_| ApiError::BadGateway)?;
        let revision_ok = if self.may_advance_multiple {
            receipt.project_revision > self.expected_revision && receipt.project_revision < MAX_SAFE
        } else {
            receipt.project_revision == self.expected_revision + 1
        };
        if !uuid_v7(&receipt.command_id)
            || !revision_ok
            || receipt.event_ids.is_empty()
            || !receipt.event_ids.iter().all(|id| uuid_v7(id))
            || !receipt.resource.as_ref().is_some_and(|resource| {
                resource.kind == self.kind
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
