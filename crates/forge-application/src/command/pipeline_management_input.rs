//! Strict named Pipeline-management payloads.
use super::{ApplicationError, CommandPayload, parse_typed, parse_uuid_v7};
use forge_protocol::wire::CommandName;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Publish {
    pipeline_id: String,
    expected_pipeline_revision: u64,
    definition: crate::PipelineDefinitionInput,
    #[serde(default)]
    make_default: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetDefault {
    pipeline_id: String,
    expected_pipeline_revision: u64,
    pipeline_version_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Delete {
    pipeline_id: String,
    expected_pipeline_revision: u64,
}

pub(super) fn parse(name: CommandName, value: Value) -> Result<CommandPayload, ApplicationError> {
    Ok(match name {
        CommandName::PublishPipelineVersion => {
            let input: Publish = parse_typed(name, value)?;
            CommandPayload::PublishPipelineVersion {
                pipeline_id: parse_uuid_v7("pipeline_id", &input.pipeline_id)?,
                expected_pipeline_revision: positive(name, input.expected_pipeline_revision)?,
                definition: input.definition,
                make_default: input.make_default,
            }
        }
        CommandName::SetPipelineDefaultVersion => {
            let input: SetDefault = parse_typed(name, value)?;
            CommandPayload::SetPipelineDefaultVersion {
                pipeline_id: parse_uuid_v7("pipeline_id", &input.pipeline_id)?,
                expected_pipeline_revision: positive(name, input.expected_pipeline_revision)?,
                pipeline_version_id: parse_uuid_v7(
                    "pipeline_version_id",
                    &input.pipeline_version_id,
                )?,
            }
        }
        CommandName::DeletePipeline => {
            let input: Delete = parse_typed(name, value)?;
            CommandPayload::DeletePipeline {
                pipeline_id: parse_uuid_v7("pipeline_id", &input.pipeline_id)?,
                expected_pipeline_revision: positive(name, input.expected_pipeline_revision)?,
            }
        }
        _ => {
            return Err(ApplicationError::InvalidPayload {
                command: name,
                reason: "not a Pipeline management command".into(),
            });
        }
    })
}

fn positive(command: CommandName, value: u64) -> Result<u64, ApplicationError> {
    if value == 0 {
        Err(ApplicationError::InvalidPayload {
            command,
            reason: "expected_pipeline_revision must be positive".into(),
        })
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pipeline_commands_reject_missing_revision_unknown_fields_and_mixed_selectors() {
        let id = uuid::Uuid::now_v7();
        for value in [
            json!({"pipeline_id":id,"expected_pipeline_revision":0}),
            json!({"pipeline_id":id,"expected_pipeline_revision":1,"hard":true}),
            json!({"pipeline_id":id}),
        ] {
            assert!(parse(CommandName::DeletePipeline, value).is_err());
        }
        let base = json!({"title":"Selection","kind":"delivery","priority":"normal"});
        for selectors in [
            json!({}),
            json!({"pipeline_id":id,"pipeline_version_id":id}),
        ] {
            let mut value = base.clone();
            value
                .as_object_mut()
                .unwrap()
                .extend(selectors.as_object().unwrap().clone());
            let parsed: crate::CreateTaskCommand = serde_json::from_value(value).unwrap();
            assert!(parsed.validate_public_contract().is_err());
        }
        for selector in ["pipeline_id", "pipeline_version_id"] {
            let mut value = base.clone();
            value[selector] = json!(id);
            let parsed: crate::CreateTaskCommand = serde_json::from_value(value).unwrap();
            parsed.validate_public_contract().unwrap();
        }
    }
}
