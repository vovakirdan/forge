//! The named path selects the operation; caller JSON cannot substitute a different mutation.

use forge_domain::knowledge::KnowledgePageMutation;
use forge_protocol::wire::CommandName;
use serde_json::Value;

use super::{ApplicationError, CommandPayload, parse_typed};

pub(super) fn parse(
    name: CommandName,
    mut value: Value,
) -> Result<CommandPayload, ApplicationError> {
    let operation = match name {
        CommandName::AuthorKnowledgePage => "author",
        CommandName::PublishKnowledgePage => "publish",
        CommandName::SupersedeKnowledgePage => "supersede",
        CommandName::WithdrawKnowledgePage => "withdraw",
        _ => {
            return Err(ApplicationError::InvalidPayload {
                command: name,
                reason: "not a knowledge command".into(),
            });
        }
    };
    let fields = value
        .as_object_mut()
        .ok_or_else(|| ApplicationError::InvalidPayload {
            command: name,
            reason: "knowledge payload must be an object".into(),
        })?;
    if fields.contains_key("operation") {
        return Err(ApplicationError::InvalidPayload {
            command: name,
            reason: "operation is selected by the named command".into(),
        });
    }
    fields.insert("operation".into(), operation.into());
    let input: KnowledgePageMutation = parse_typed(name, value)?;
    input
        .validate()
        .map_err(|error| ApplicationError::InvalidPayload {
            command: name,
            reason: error.to_string(),
        })?;
    Ok(match name {
        CommandName::AuthorKnowledgePage => CommandPayload::AuthorKnowledgePage(input),
        CommandName::PublishKnowledgePage => CommandPayload::PublishKnowledgePage(input),
        CommandName::SupersedeKnowledgePage => CommandPayload::SupersedeKnowledgePage(input),
        CommandName::WithdrawKnowledgePage => CommandPayload::WithdrawKnowledgePage(input),
        _ => {
            return Err(ApplicationError::InvalidPayload {
                command: name,
                reason: "not a knowledge command".into(),
            });
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn path_cannot_be_replaced_and_unknown_fields_are_rejected() {
        let input = json!({"page_id":Uuid::now_v7(),"expected_page_revision":1});
        assert!(parse(CommandName::PublishKnowledgePage, input.clone()).is_ok());
        let mut forged = input.clone();
        forged["operation"] = json!("withdraw");
        assert!(parse(CommandName::PublishKnowledgePage, forged).is_err());
        let mut forged = input;
        forged["actor"] = json!("human");
        assert!(parse(CommandName::PublishKnowledgePage, forged).is_err());
    }
}
