//! Narrow management commands with explicit scope and receipt validation.
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
fn reason(value: &Option<String>) -> bool {
    value.as_ref().is_none_or(|value| {
        !value.trim().is_empty() && value.len() <= 10_000 && !value.contains('\0')
    })
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
struct ProjectPayload {
    reason: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskStopPayload {
    task_id: String,
    expected_task_revision: u64,
    mode: StopMode,
    reason: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmployeeStopPayload {
    employee_id: String,
    expected_employee_revision: u64,
    mode: StopMode,
    reason: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskResumePayload {
    task_id: String,
    expected_task_revision: u64,
    wait_condition_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolutionAnswer {
    disposition: Disposition,
    summary: String,
    recommended_outcome_key: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Disposition {
    ContinueStage,
    NeedsManagementChange,
    ForwardToHuman,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmitResolutionPayload {
    escalation_id: String,
    expected_escalation_revision: u64,
    assignment_id: String,
    lease_generation: u64,
    answer: ResolutionAnswer,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReroutePayload {
    escalation_id: String,
    expected_escalation_revision: u64,
    reason: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryAssessmentPayload {
    run_id: String,
    assessment: RecoveryAssessment,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum RecoveryAssessment {
    NotStartedConfirmed,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum StopMode {
    Graceful,
    Force,
}
pub(crate) struct ManagementCommand {
    expected_revision: u64,
    resource_kind: &'static str,
    resource_id: String,
    may_advance_multiple: bool,
}
impl ManagementCommand {
    pub(crate) fn parse(bytes: &[u8], target: CommandTarget) -> Result<Self, ApiError> {
        let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !uuid_v7(&envelope.project_id) || !positive(envelope.expected_revision) {
            return Err(ApiError::BadRequest);
        }
        let (resource_kind, resource_id, may_advance_multiple) = match target {
            CommandTarget::StartProjectExecution | CommandTarget::StopProjectExecution => {
                let payload: ProjectPayload =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                if !reason(&payload.reason) {
                    return Err(ApiError::BadRequest);
                }
                (
                    "project",
                    envelope.project_id,
                    matches!(target, CommandTarget::StopProjectExecution),
                )
            }
            CommandTarget::PauseTask => {
                let payload: TaskStopPayload =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                let _ = payload.mode;
                if !uuid_v7(&payload.task_id)
                    || !positive(payload.expected_task_revision)
                    || !reason(&payload.reason)
                {
                    return Err(ApiError::BadRequest);
                }
                ("task", payload.task_id, true)
            }
            CommandTarget::ResumeTask => {
                let payload: TaskResumePayload =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                if !uuid_v7(&payload.task_id)
                    || !uuid_v7(&payload.wait_condition_id)
                    || !positive(payload.expected_task_revision)
                {
                    return Err(ApiError::BadRequest);
                }
                ("task", payload.task_id, false)
            }
            CommandTarget::StopEmployee => {
                let payload: EmployeeStopPayload =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                let _ = payload.mode;
                if !uuid_v7(&payload.employee_id)
                    || !positive(payload.expected_employee_revision)
                    || !reason(&payload.reason)
                {
                    return Err(ApiError::BadRequest);
                }
                ("employee", payload.employee_id, true)
            }
            CommandTarget::SubmitHumanResolution => {
                let payload: SubmitResolutionPayload =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                let _ = payload.answer.disposition;
                if !uuid_v7(&payload.escalation_id)
                    || !uuid_v7(&payload.assignment_id)
                    || !positive(payload.expected_escalation_revision)
                    || !positive(payload.lease_generation)
                    || payload.answer.summary.trim().is_empty()
                    || payload.answer.summary.len() > 20_000
                    || payload.answer.summary.contains('\0')
                    || payload
                        .answer
                        .recommended_outcome_key
                        .as_ref()
                        .is_some_and(|key| key.is_empty() || key.len() > 64)
                {
                    return Err(ApiError::BadRequest);
                }
                ("escalation", payload.escalation_id, true)
            }
            CommandTarget::RerouteEscalation => {
                let payload: ReroutePayload =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                if !uuid_v7(&payload.escalation_id)
                    || !positive(payload.expected_escalation_revision)
                    || payload.reason.trim().is_empty()
                    || payload.reason.len() > 10_000
                    || payload.reason.contains('\0')
                {
                    return Err(ApiError::BadRequest);
                }
                ("escalation", payload.escalation_id, true)
            }
            CommandTarget::AcceptRunRecoveryAssessment => {
                let payload: RecoveryAssessmentPayload =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                let _ = payload.assessment;
                if !uuid_v7(&payload.run_id) {
                    return Err(ApiError::BadRequest);
                }
                ("run", payload.run_id, true)
            }
            _ => return Err(ApiError::BadRequest),
        };
        Ok(Self {
            expected_revision: envelope.expected_revision,
            resource_kind,
            resource_id,
            may_advance_multiple,
        })
    }
    pub(crate) fn validate_receipt(&self, bytes: &[u8]) -> Result<(), ApiError> {
        let receipt: CommandReceipt =
            serde_json::from_slice(bytes).map_err(|_| ApiError::BadGateway)?;
        let valid_revision = if self.may_advance_multiple {
            receipt.project_revision > self.expected_revision
                && receipt.project_revision <= MAX_SAFE
        } else {
            receipt.project_revision == self.expected_revision + 1
        };
        if !uuid_v7(&receipt.command_id)
            || !valid_revision
            || receipt.event_ids.is_empty()
            || !receipt.event_ids.iter().all(|id| uuid_v7(id))
            || !receipt.resource.as_ref().is_some_and(|resource| {
                resource.kind == self.resource_kind
                    && uuid_v7(&resource.id)
                    && resource.id.eq_ignore_ascii_case(&self.resource_id)
            })
        {
            return Err(ApiError::BadGateway);
        }
        Ok(())
    }
}
