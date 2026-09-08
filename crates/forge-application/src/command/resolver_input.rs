use super::{ApplicationError, CommandPayload, parse_typed, positive_revision};
use forge_domain::{
    EmployeeId, TaskId,
    resolution::{EscalationCategory, ResolutionAnswer},
};
use forge_protocol::wire::CommandName;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigureResolverRouteInput {
    pub route_key: String,
    pub employee_ids: Vec<EmployeeId>,
    pub assignment_timeout_seconds: u32,
}
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaiseEscalationInput {
    pub task_id: TaskId,
    pub expected_task_revision: u64,
    #[serde(default)]
    pub route_key: Option<String>,
    pub category: EscalationCategory,
    pub question: String,
}
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitHumanResolutionInput {
    pub escalation_id: Uuid,
    pub expected_escalation_revision: u64,
    pub assignment_id: Uuid,
    pub lease_generation: u64,
    pub answer: ResolutionAnswer,
}
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RerouteEscalationInput {
    pub escalation_id: Uuid,
    pub expected_escalation_revision: u64,
    pub reason: String,
}

pub(super) fn parse(name: CommandName, value: Value) -> Result<CommandPayload, ApplicationError> {
    match name {
        CommandName::ConfigureResolverRoute => Ok(CommandPayload::ConfigureResolverRoute(
            parse_typed(name, value)?,
        )),
        CommandName::RaiseEscalation => {
            let input: RaiseEscalationInput = parse_typed(name, value)?;
            positive_revision(name, input.expected_task_revision)?;
            valid_id(name, input.task_id.as_uuid())?;
            bounded(name, &input.question, 20_000)?;
            Ok(CommandPayload::RaiseEscalation(input))
        }
        CommandName::SubmitHumanResolution => {
            let input: SubmitHumanResolutionInput = parse_typed(name, value)?;
            valid_id(name, input.escalation_id)?;
            valid_id(name, input.assignment_id)?;
            positive_revision(name, input.expected_escalation_revision)?;
            positive_revision(name, input.lease_generation)?;
            bounded(name, &input.answer.summary, 20_000)?;
            Ok(CommandPayload::SubmitHumanResolution(input))
        }
        CommandName::RerouteEscalation => {
            let input: RerouteEscalationInput = parse_typed(name, value)?;
            valid_id(name, input.escalation_id)?;
            positive_revision(name, input.expected_escalation_revision)?;
            bounded(name, &input.reason, 10_000)?;
            Ok(CommandPayload::RerouteEscalation(input))
        }
        _ => Err(invalid(name)),
    }
}
fn invalid(command: CommandName) -> ApplicationError {
    ApplicationError::InvalidPayload {
        command,
        reason: "invalid resolver command scope or bounded text".into(),
    }
}
fn valid_id(name: CommandName, id: Uuid) -> Result<(), ApplicationError> {
    if id.get_version_num() != 7 {
        Err(invalid(name))
    } else {
        Ok(())
    }
}
fn bounded(name: CommandName, value: &str, max: usize) -> Result<(), ApplicationError> {
    if value.trim().is_empty() || value.contains('\0') || value.chars().count() > max {
        Err(invalid(name))
    } else {
        Ok(())
    }
}
