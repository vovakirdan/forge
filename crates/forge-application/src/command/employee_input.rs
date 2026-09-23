//! Employee management payloads share strict identity and revision validation.
use super::{ApplicationError, CommandName, CommandPayload, parse_typed, parse_uuid_v7};
use forge_domain::EmployeeAmendment;
use forge_protocol::wire::MAX_COMMAND_AUDIT_DETAIL_CHARACTERS;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    employee_id: String,
    expected_employee_revision: u64,
    patch: Option<EmployeeAmendment>,
    reason: Option<String>,
}

pub(super) fn parse(name: CommandName, value: Value) -> Result<CommandPayload, ApplicationError> {
    let patch_present = value.get("patch").is_some();
    let reason_present = value.get("reason").is_some();
    let input: Input = parse_typed(name, value)?;
    let employee_id = parse_uuid_v7("employee_id", &input.employee_id)?;
    let expected_employee_revision = input.expected_employee_revision;
    let invalid = |reason: &str| ApplicationError::InvalidPayload {
        command: name,
        reason: reason.to_owned(),
    };
    if expected_employee_revision == 0 {
        return Err(invalid(
            "expected_employee_revision must be greater than zero",
        ));
    }
    if name == CommandName::AmendEmployee {
        if reason_present {
            return Err(invalid(
                "reason is only valid for Employee lifecycle commands",
            ));
        }
        let patch = input.patch.ok_or_else(|| invalid("patch is required"))?;
        patch
            .validate_nonempty()
            .map_err(|error| invalid(&error.to_string()))?;
        return Ok(CommandPayload::AmendEmployee {
            employee_id,
            expected_employee_revision,
            patch,
        });
    }
    if patch_present {
        return Err(invalid("patch is only valid for amend_employee"));
    }
    if input.reason.as_ref().is_some_and(|reason| {
        reason.trim().is_empty()
            || reason.contains('\0')
            || reason.chars().count() > MAX_COMMAND_AUDIT_DETAIL_CHARACTERS
    }) {
        return Err(invalid(
            "reason must be nonblank, contain no NUL and be at most 10000 characters",
        ));
    }
    match name {
        CommandName::EnableEmployee => Ok(CommandPayload::EnableEmployee {
            employee_id,
            expected_employee_revision,
            reason: input.reason,
        }),
        CommandName::DisableEmployee => Ok(CommandPayload::DisableEmployee {
            employee_id,
            expected_employee_revision,
            reason: input.reason,
        }),
        CommandName::RetireEmployee => Ok(CommandPayload::RetireEmployee {
            employee_id,
            expected_employee_revision,
            reason: input.reason,
        }),
        _ => Err(invalid("not an Employee management command")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn employee_command_payload_rejects_zero_revisions_capacity_and_extra_state_fields() {
        let id = uuid::Uuid::now_v7();
        for (name, payload) in [
            (
                CommandName::AmendEmployee,
                json!({"employee_id":id,"expected_employee_revision":0,"patch":{"name":"Bob"}}),
            ),
            (
                CommandName::AmendEmployee,
                json!({"employee_id":id,"expected_employee_revision":1,"patch":{"max_concurrent_runs":0}}),
            ),
            (
                CommandName::AmendEmployee,
                json!({"employee_id":id,"expected_employee_revision":1,"patch":{}}),
            ),
            (
                CommandName::DisableEmployee,
                json!({"employee_id":id,"expected_employee_revision":1,"patch":null}),
            ),
            (
                CommandName::RetireEmployee,
                json!({"employee_id":id,"expected_employee_revision":1,"stop":true}),
            ),
        ] {
            assert!(parse(name, payload).is_err());
        }
    }

    #[test]
    fn lifecycle_reason_is_optional_bounded_and_excluded_from_amendment() {
        let id = uuid::Uuid::now_v7();
        let base = json!({"employee_id":id,"expected_employee_revision":1});
        for name in [
            CommandName::EnableEmployee,
            CommandName::DisableEmployee,
            CommandName::RetireEmployee,
        ] {
            assert!(parse(name, base.clone()).is_ok());
            let with_reason =
                json!({"employee_id":id,"expected_employee_revision":1,"reason":"Operator choice"});
            assert!(parse(name, with_reason).is_ok());
            for reason in [" ".to_owned(), "x\0y".to_owned(), "x".repeat(10_001)] {
                assert!(
                    parse(
                        name,
                        json!({"employee_id":id,"expected_employee_revision":1,"reason":reason})
                    )
                    .is_err()
                );
            }
        }
        assert!(parse(CommandName::AmendEmployee, json!({"employee_id":id,"expected_employee_revision":1,"patch":{"name":"Bob"},"reason":"why"})).is_err());
        assert!(parse(CommandName::AmendEmployee, json!({"employee_id":id,"expected_employee_revision":1,"patch":{"name":"Bob"},"reason":null})).is_err());
    }
}
