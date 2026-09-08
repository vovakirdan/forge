//! Management stop requests require an explicit policy and optimistic scope.
use forge_domain::ExecutionStopMode;
use forge_protocol::wire::CommandName;
use serde::Deserialize;
use serde_json::Value;

use super::{
    ApplicationError, CommandPayload, bounded_optional_audit_detail, parse_typed, parse_uuid_v7,
    positive_revision,
};

pub(super) fn parse(name: CommandName, value: Value) -> Result<CommandPayload, ApplicationError> {
    match name {
        CommandName::SetNextRunEmployee => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                task_id: String,
                expected_task_revision: u64,
                employee_id: String,
                #[serde(default)]
                reason: Option<String>,
            }
            let input: Input = parse_typed(name, value)?;
            validate_reason(name, input.reason.as_deref())?;
            Ok(CommandPayload::SetNextRunEmployee {
                task_id: parse_uuid_v7("task_id", &input.task_id)?,
                expected_task_revision: positive_revision(name, input.expected_task_revision)?,
                employee_id: parse_uuid_v7("employee_id", &input.employee_id)?,
                reason: input.reason,
            })
        }
        CommandName::ClearNextRunEmployee => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                task_id: String,
                expected_task_revision: u64,
                #[serde(default)]
                reason: Option<String>,
            }
            let input: Input = parse_typed(name, value)?;
            validate_reason(name, input.reason.as_deref())?;
            Ok(CommandPayload::ClearNextRunEmployee {
                task_id: parse_uuid_v7("task_id", &input.task_id)?,
                expected_task_revision: positive_revision(name, input.expected_task_revision)?,
                reason: input.reason,
            })
        }
        CommandName::StopEmployee => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                employee_id: String,
                expected_employee_revision: u64,
                mode: ExecutionStopMode,
                #[serde(default)]
                reason: Option<String>,
            }
            let input: Input = parse_typed(name, value)?;
            validate_reason(name, input.reason.as_deref())?;
            Ok(CommandPayload::StopEmployee {
                employee_id: parse_uuid_v7("employee_id", &input.employee_id)?,
                expected_employee_revision: positive_revision(
                    name,
                    input.expected_employee_revision,
                )?,
                mode: input.mode,
                reason: input.reason,
            })
        }
        CommandName::PauseTask => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                task_id: String,
                expected_task_revision: u64,
                mode: ExecutionStopMode,
                #[serde(default)]
                reason: Option<String>,
            }
            let input: Input = parse_typed(name, value)?;
            validate_reason(name, input.reason.as_deref())?;
            Ok(CommandPayload::PauseTask {
                task_id: parse_uuid_v7("task_id", &input.task_id)?,
                expected_task_revision: positive_revision(name, input.expected_task_revision)?,
                mode: input.mode,
                reason: input.reason,
            })
        }
        _ => Err(ApplicationError::InvalidPayload {
            command: name,
            reason: "not an execution management command".into(),
        }),
    }
}

fn validate_reason(name: CommandName, value: Option<&str>) -> Result<(), ApplicationError> {
    bounded_optional_audit_detail(name, "reason", value.map(ToOwned::to_owned))?;
    if value.is_some_and(|value| value.trim().is_empty() || value.contains('\0')) {
        return Err(ApplicationError::InvalidPayload {
            command: name,
            reason: "reason must be nonblank and contain no NUL when supplied".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn future_employee_requires_exact_valid_identifiers_and_no_extra_authority() {
        let task = uuid::Uuid::now_v7();
        let employee = uuid::Uuid::now_v7();
        for value in [
            json!({"task_id":task,"expected_task_revision":1}),
            json!({"task_id":task,"expected_task_revision":0,"employee_id":employee}),
            json!({"task_id":task,"expected_task_revision":1,"employee_id":uuid::Uuid::nil()}),
            json!({"task_id":task,"expected_task_revision":1,"employee_id":employee,"stop_current":true}),
        ] {
            assert!(parse(CommandName::SetNextRunEmployee, value).is_err());
        }
        assert!(
            parse(
                CommandName::SetNextRunEmployee,
                json!({"task_id":task,"expected_task_revision":1,"employee_id":employee})
            )
            .is_ok()
        );
        assert!(
            parse(
                CommandName::ClearNextRunEmployee,
                json!({"task_id":task,"expected_task_revision":1,"employee_id":employee})
            )
            .is_err()
        );
    }

    #[test]
    fn stop_requests_require_explicit_mode_and_exact_scope() {
        let employee = uuid::Uuid::now_v7();
        for value in [
            json!({"employee_id":employee,"expected_employee_revision":1}),
            json!({"employee_id":employee,"expected_employee_revision":0,"mode":"force"}),
            json!({"employee_id":employee,"expected_employee_revision":1,"mode":"kill"}),
            json!({"employee_id":employee,"expected_employee_revision":1,"mode":"force","reason":" "}),
            json!({"employee_id":employee,"expected_employee_revision":1,"mode":"force","task_id":uuid::Uuid::now_v7()}),
        ] {
            assert!(parse(CommandName::StopEmployee, value).is_err());
        }
        assert!(
            parse(
                CommandName::StopEmployee,
                json!({"employee_id":employee,"expected_employee_revision":1,"mode":"graceful"})
            )
            .is_ok()
        );
        assert!(
            parse(
                CommandName::PauseTask,
                json!({"task_id":uuid::Uuid::now_v7(),"expected_task_revision":1,"mode":"force"})
            )
            .is_ok()
        );
    }
}
