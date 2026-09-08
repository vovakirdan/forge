use super::{
    ApplicationError, CommandPayload, bounded_optional_audit_detail, parse_typed, parse_uuid_v7,
    positive_revision,
};
use forge_protocol::wire::CommandName;
use serde::Deserialize;
use serde_json::Value;

pub(super) fn parse(name: CommandName, value: Value) -> Result<CommandPayload, ApplicationError> {
    match name {
        CommandName::ScheduleTaskResume => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                task_id: String,
                expected_task_revision: u64,
                wait_condition_id: String,
                not_before: String,
                reason: String,
            }
            let input: Input = parse_typed(name, value)?;
            reason(name, Some(&input.reason))?;
            Ok(CommandPayload::ScheduleTaskResume {
                task_id: parse_uuid_v7("task_id", &input.task_id)?,
                expected_task_revision: positive_revision(name, input.expected_task_revision)?,
                wait_condition_id: parse_uuid_v7("wait_condition_id", &input.wait_condition_id)?,
                not_before: forge_domain::Timestamp::from_offset_date_time(
                    time::OffsetDateTime::parse(
                        &input.not_before,
                        &time::format_description::well_known::Rfc3339,
                    )
                    .map_err(|_| ApplicationError::InvalidPayload {
                        command: name,
                        reason: "not_before must be an RFC3339 timestamp".into(),
                    })?,
                ),
                reason: input.reason,
            })
        }
        CommandName::CancelTaskResume => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                schedule_id: String,
                #[serde(default)]
                reason: Option<String>,
            }
            let input: Input = parse_typed(name, value)?;
            reason(name, input.reason.as_deref())?;
            Ok(CommandPayload::CancelTaskResume {
                schedule_id: parse_uuid_v7("schedule_id", &input.schedule_id)?,
                reason: input.reason,
            })
        }
        _ => Err(ApplicationError::InvalidPayload {
            command: name,
            reason: "not a scheduled resume command".into(),
        }),
    }
}

fn reason(name: CommandName, value: Option<&str>) -> Result<(), ApplicationError> {
    bounded_optional_audit_detail(name, "reason", value.map(ToOwned::to_owned))?;
    if value.is_some_and(|value| value.trim().is_empty() || value.contains('\0')) {
        return Err(ApplicationError::InvalidPayload {
            command: name,
            reason: "reason must be nonblank without NUL".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn alarm_parser_requires_rfc3339_reason_and_exact_wait_scope() {
        let valid = json!({"task_id":uuid::Uuid::now_v7(),"expected_task_revision":1,"wait_condition_id":uuid::Uuid::now_v7(),"not_before":"2026-09-09T13:00:00.123456789+03:00","reason":"Continue after reset"});
        assert!(parse(CommandName::ScheduleTaskResume, valid.clone()).is_ok());
        for (field, value) in [
            ("reason", json!(" ")),
            ("expected_task_revision", json!(0)),
            ("not_before", json!("tomorrow")),
            ("wait_condition_id", json!(uuid::Uuid::nil())),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            assert!(parse(CommandName::ScheduleTaskResume, invalid).is_err());
        }
        let mut invalid = valid;
        invalid.as_object_mut().expect("object").remove("reason");
        assert!(parse(CommandName::ScheduleTaskResume, invalid).is_err());
    }
}
