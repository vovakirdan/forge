use forge_domain::git::{GitBranchRef, GitObjectId, LocalGitPath};
use forge_protocol::wire::CommandName;
use serde::Deserialize;
use serde_json::Value;

use super::{ApplicationError, CommandPayload, parse_typed, parse_uuid_v7, positive_revision};

pub(super) fn parse(name: CommandName, value: Value) -> Result<CommandPayload, ApplicationError> {
    match name {
        CommandName::RegisterProjectRepository => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                name: String,
                source: LocalGitPath,
                target_ref: GitBranchRef,
            }
            let input: Input = parse_typed(name, value)?;
            if input.name.trim().is_empty()
                || input.name.len() > 128
                || input.name.chars().any(char::is_control)
            {
                return Err(ApplicationError::InvalidPayload {
                    command: name,
                    reason: "repository name must be nonblank, bounded and printable".into(),
                });
            }
            Ok(CommandPayload::RegisterProjectRepository {
                name: input.name,
                source: input.source,
                target_ref: input.target_ref,
            })
        }
        CommandName::BindTaskGitRepository => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                task_id: String,
                expected_task_revision: u64,
                repository_id: String,
                initial_base: GitObjectId,
            }
            let input: Input = parse_typed(name, value)?;
            Ok(CommandPayload::BindTaskGitRepository {
                task_id: parse_uuid_v7("task_id", &input.task_id)?,
                expected_task_revision: positive_revision(name, input.expected_task_revision)?,
                repository_id: parse_uuid_v7("repository_id", &input.repository_id)?,
                initial_base: input.initial_base,
            })
        }
        _ => Err(ApplicationError::InvalidPayload {
            command: name,
            reason: "not a repository command".into(),
        }),
    }
}
