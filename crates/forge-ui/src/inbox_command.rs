//! Closed browser command shapes for durable Inbox mutations.
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
    payload: serde_json::Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenPayload {
    employee_id: String,
    task_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendPayload {
    thread_id: String,
    expected_thread_revision: u64,
    target: Target,
    kind: Kind,
    requirement: Requirement,
    body: String,
    reply_to: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WaivePayload {
    message_id: String,
    reason: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetryPayload {
    run_id: String,
    reason: String,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Target {
    Inbox,
    TaskExecution {
        context: Context,
    },
    ExactRun {
        context: Context,
        run_id: String,
        fencing_token: u64,
        environment_epoch: u64,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Context {
    task_id: String,
    pipeline_version_id: String,
    stage_id: String,
    stage_visit: u64,
}
#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Instruction,
    Question,
    Notification,
    Reply,
}
#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Requirement {
    Informational,
    Acknowledged,
    Answered,
}
fn revision(value: u64) -> bool {
    (1..MAX_SAFE).contains(&value)
}
fn stage(value: &str) -> bool {
    value.len() <= 64
        && value.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
fn context_valid(value: &Context) -> bool {
    uuid_v7(&value.task_id)
        && uuid_v7(&value.pipeline_version_id)
        && stage(&value.stage_id)
        && revision(value.stage_visit)
}

pub(crate) struct InboxCommand {
    expected_revision: u64,
    resource_kind: &'static str,
    resource_id: Option<String>,
}
impl InboxCommand {
    pub(crate) fn parse(bytes: &[u8], target: CommandTarget) -> Result<Self, ApiError> {
        let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !uuid_v7(&envelope.project_id) || !revision(envelope.expected_revision) {
            return Err(ApiError::BadRequest);
        }
        let (resource_kind, resource_id) = match target {
            CommandTarget::OpenEmployeeThread => {
                let value: OpenPayload =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                if !uuid_v7(&value.employee_id)
                    || value.task_id.as_deref().is_some_and(|id| !uuid_v7(id))
                {
                    return Err(ApiError::BadRequest);
                }
                ("employee_thread", None)
            }
            CommandTarget::SendEmployeeMessage => {
                let value: SendPayload =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                let target_valid = match &value.target {
                    Target::Inbox => true,
                    Target::TaskExecution { context } => context_valid(context),
                    Target::ExactRun {
                        context,
                        run_id,
                        fencing_token,
                        environment_epoch,
                    } => {
                        context_valid(context)
                            && uuid_v7(run_id)
                            && revision(*fencing_token)
                            && revision(*environment_epoch)
                    }
                };
                if !uuid_v7(&value.thread_id)
                    || !revision(value.expected_thread_revision)
                    || !target_valid
                    || value.body.trim().is_empty()
                    || value.body.len() > 32 * 1024
                    || value.reply_to.as_deref().is_some_and(|id| !uuid_v7(id))
                    || (value.kind == Kind::Reply) != value.reply_to.is_some()
                    || (matches!(value.kind, Kind::Reply | Kind::Notification)
                        && value.requirement != Requirement::Informational)
                {
                    return Err(ApiError::BadRequest);
                }
                ("employee_message", None)
            }
            CommandTarget::WaiveMessageRequirement => {
                let value: WaivePayload =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                if !uuid_v7(&value.message_id)
                    || value.reason.trim().is_empty()
                    || value.reason.len() > 4096
                {
                    return Err(ApiError::BadRequest);
                }
                ("message_requirement_waiver", Some(value.message_id))
            }
            CommandTarget::RetryCommunication => {
                let value: RetryPayload =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                if !uuid_v7(&value.run_id)
                    || value.reason.trim().is_empty()
                    || value.reason.len() > 4096
                {
                    return Err(ApiError::BadRequest);
                }
                ("run", Some(value.run_id))
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
                        .is_none_or(|id| resource.id.eq_ignore_ascii_case(id))
            })
        {
            return Err(ApiError::BadGateway);
        }
        Ok(())
    }
}
