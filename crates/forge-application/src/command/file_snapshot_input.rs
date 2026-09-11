use forge_domain::{
    ArtifactId, TaskId,
    file_snapshot::{forbidden_component, validate_selected_paths},
};
use forge_protocol::wire::CommandName;
use serde::Deserialize;
use serde_json::Value;

use super::parse_typed;
use crate::{ApplicationError, CommandPayload};

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportFileSnapshotInput {
    pub task_id: TaskId,
    pub expected_task_revision: u64,
    pub title: String,
    pub root: String,
    pub paths: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureFileSnapshotInput {
    pub task_id: TaskId,
    pub expected_task_revision: u64,
    pub title: String,
    pub paths: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachFileInput {
    pub task_id: TaskId,
    pub expected_task_revision: u64,
    pub artifact_id: ArtifactId,
}

pub(super) fn parse(name: CommandName, value: Value) -> Result<CommandPayload, ApplicationError> {
    let invalid = || ApplicationError::InvalidPayload {
        command: name,
        reason: "invalid selected-file snapshot request".into(),
    };
    match name {
        CommandName::ImportTaskFileSnapshot => {
            let input: ImportFileSnapshotInput = parse_typed(name, value)?;
            validate(
                name,
                input.task_id,
                input.expected_task_revision,
                &input.title,
                &input.paths,
            )?;
            let root = std::path::Path::new(&input.root);
            if !root.is_absolute()
                || root.parent().is_none()
                || input.root.len() > 4096
                || input.root.chars().any(char::is_control)
                || root.components().any(|part| match part {
                    std::path::Component::RootDir => false,
                    std::path::Component::Normal(name) => {
                        name.to_str().is_none_or(forbidden_component)
                    }
                    _ => true,
                })
            {
                return Err(invalid());
            }
            Ok(CommandPayload::ImportTaskFileSnapshot(input))
        }
        CommandName::CaptureTaskFileSnapshot => {
            let input: CaptureFileSnapshotInput = parse_typed(name, value)?;
            validate(
                name,
                input.task_id,
                input.expected_task_revision,
                &input.title,
                &input.paths,
            )?;
            Ok(CommandPayload::CaptureTaskFileSnapshot(input))
        }
        CommandName::AttachTaskFileInput => {
            let input: AttachFileInput = parse_typed(name, value)?;
            if input.task_id.as_uuid().get_version_num() != 7
                || input.expected_task_revision == 0
                || input.artifact_id.as_uuid().get_version_num() != 7
            {
                return Err(invalid());
            }
            Ok(CommandPayload::AttachTaskFileInput(input))
        }
        _ => Err(invalid()),
    }
}

fn validate(
    name: CommandName,
    task: TaskId,
    revision: u64,
    title: &str,
    paths: &[String],
) -> Result<(), ApplicationError> {
    if task.as_uuid().get_version_num() != 7
        || revision == 0
        || title.trim().is_empty()
        || title.chars().count() > 240
        || validate_selected_paths(paths).is_err()
    {
        return Err(ApplicationError::InvalidPayload {
            command: name,
            reason: "snapshot requires Task revision, title and safe explicit file paths".into(),
        });
    }
    Ok(())
}
