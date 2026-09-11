//! A named route determines management authority; the payload never selects its operation.

use forge_domain::system_job::SystemJobManagement;
use forge_protocol::wire::CommandName;
use serde_json::Value;

use super::{ApplicationError, CommandPayload, parse_typed};

pub(super) fn parse(
    name: CommandName,
    mut value: Value,
) -> Result<CommandPayload, ApplicationError> {
    let operation = match name {
        CommandName::ConfigureSystemJobs => "configure",
        CommandName::RequestTaskSummary => "request_summary",
        CommandName::RequestEmployeeOnboarding => "request_onboarding",
        CommandName::RetrySystemJob => "retry",
        CommandName::SkipEmployeeOnboarding => "skip_onboarding",
        _ => {
            return Err(ApplicationError::InvalidPayload {
                command: name,
                reason: "not a SystemJob management command".into(),
            });
        }
    };
    let fields = value
        .as_object_mut()
        .ok_or_else(|| ApplicationError::InvalidPayload {
            command: name,
            reason: "SystemJob payload must be an object".into(),
        })?;
    if fields.contains_key("operation") {
        return Err(ApplicationError::InvalidPayload {
            command: name,
            reason: "operation is selected by the named command".into(),
        });
    }
    fields.insert("operation".into(), operation.into());
    let input: SystemJobManagement = parse_typed(name, value)?;
    input
        .validate()
        .map_err(|error| ApplicationError::InvalidPayload {
            command: name,
            reason: error.to_string(),
        })?;
    Ok(match name {
        CommandName::ConfigureSystemJobs => CommandPayload::ConfigureSystemJobs(input),
        CommandName::RequestTaskSummary => CommandPayload::RequestTaskSummary(input),
        CommandName::RequestEmployeeOnboarding => CommandPayload::RequestEmployeeOnboarding(input),
        CommandName::RetrySystemJob => CommandPayload::RetrySystemJob(input),
        CommandName::SkipEmployeeOnboarding => CommandPayload::SkipEmployeeOnboarding(input),
        _ => {
            return Err(ApplicationError::InvalidPayload {
                command: name,
                reason: "not a SystemJob management command".into(),
            });
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn skip_requires_explicit_nonblank_reason_and_does_not_accept_actor() {
        let id = uuid::Uuid::now_v7();
        assert!(
            parse(
                CommandName::SkipEmployeeOnboarding,
                json!({"employee_id":id,"reason":" ",})
            )
            .is_err()
        );
        assert!(
            parse(
                CommandName::SkipEmployeeOnboarding,
                json!({"employee_id":id,"reason":"Known project owner","actor":"manager"})
            )
            .is_err()
        );
        assert!(
            parse(
                CommandName::SkipEmployeeOnboarding,
                json!({"employee_id":id,"reason":"Known project owner"})
            )
            .is_ok()
        );
    }
    #[test]
    fn request_summary_rejects_non_v7_task_and_forged_operation() {
        assert!(
            parse(
                CommandName::RequestTaskSummary,
                json!({"task_id":uuid::Uuid::nil()})
            )
            .is_err()
        );
        assert!(
            parse(
                CommandName::RequestTaskSummary,
                json!({"task_id":uuid::Uuid::now_v7(),"operation":"retry"})
            )
            .is_err()
        );
    }
}
