use forge_application::{ApplicationError, CommandEnvelope, CommandPayload};
use forge_protocol::wire::{
    CommandName, CommandRequest, MAX_ARTIFACT_METADATA_ENTRIES, MAX_COMMAND_AUDIT_DETAIL_CHARACTERS,
};
use serde_json::{Map, Value, json};

fn request(expected_revision: u64, payload: Value) -> CommandRequest {
    CommandRequest {
        project_id: uuid::Uuid::now_v7().to_string(),
        expected_revision,
        payload: payload
            .as_object()
            .expect("test payload must be an object")
            .clone(),
    }
}

fn tagged_task_properties() -> Map<String, Value> {
    json!({
        "contains_migrations": {"type": "boolean", "value": true},
        "runbook": {"type": "text", "value": "Update the operator guide"},
        "estimated_days": {"type": "number", "value": 1.5},
        "target_date": {"type": "date", "value": [2026, 247]},
        "strategy": {"type": "enum", "value": "online"},
        "environments": {"type": "multi_enum", "value": ["staging", "production"]},
        "related_task": {
            "type": "reference",
            "value": {
                "reference_type": "task",
                "reference_id": uuid::Uuid::now_v7().to_string()
            }
        }
    })
    .as_object()
    .expect("test properties must be an object")
    .clone()
}

#[test]
fn create_task_parser_accepts_each_tagged_property_value_shape() {
    let parsed = CommandEnvelope::parse(
        CommandName::CreateTask,
        request(
            1,
            json!({
                "title": "Plan migration",
                "kind": "analysis",
                "pipeline_version_id": uuid::Uuid::now_v7().to_string(),
                "priority": "normal",
                "properties": tagged_task_properties()
            }),
        ),
        "tagged-properties",
    );

    assert!(
        matches!(
            &parsed,
            Ok(CommandEnvelope {
                payload: CommandPayload::CreateTask(_),
                ..
            })
        ),
        "tagged property fixture should parse: {parsed:?}"
    );
}

#[test]
fn create_task_parser_rejects_an_untyped_property_value() {
    let parsed = CommandEnvelope::parse(
        CommandName::CreateTask,
        request(
            1,
            json!({
                "title": "Plan migration",
                "kind": "analysis",
                "pipeline_version_id": uuid::Uuid::now_v7().to_string(),
                "priority": "normal",
                "properties": {"contains_migrations": true}
            }),
        ),
        "untyped-property",
    );

    assert!(matches!(
        parsed,
        Err(ApplicationError::InvalidPayload {
            command: CommandName::CreateTask,
            ..
        })
    ));
}

#[test]
fn amend_draft_parser_rejects_an_untyped_property_value() {
    let parsed = CommandEnvelope::parse(
        CommandName::AmendDraft,
        request(
            1,
            json!({
                "task_id": uuid::Uuid::now_v7().to_string(),
                "expected_task_revision": 1,
                "patch": {"properties": {"contains_migrations": true}}
            }),
        ),
        "untyped-amend-property",
    );

    assert!(matches!(
        parsed,
        Err(ApplicationError::InvalidPayload {
            command: CommandName::AmendDraft,
            ..
        })
    ));
}

#[test]
fn cancel_note_uses_the_shared_character_limit_at_parse_time() {
    let accepted = CommandEnvelope::parse(
        CommandName::CancelTask,
        request(
            1,
            json!({
                "task_id": uuid::Uuid::now_v7().to_string(),
                "expected_task_revision": 1,
                "cancellation_reason_key": "unspecified",
                "note": "x".repeat(MAX_COMMAND_AUDIT_DETAIL_CHARACTERS)
            }),
        ),
        "cancel-boundary-accepted",
    );
    let rejected = CommandEnvelope::parse(
        CommandName::CancelTask,
        request(
            1,
            json!({
                "task_id": uuid::Uuid::now_v7().to_string(),
                "expected_task_revision": 1,
                "cancellation_reason_key": "unspecified",
                "note": "x".repeat(MAX_COMMAND_AUDIT_DETAIL_CHARACTERS + 1)
            }),
        ),
        "cancel-boundary-rejected",
    );

    assert!(accepted.is_ok());
    assert!(matches!(
        rejected,
        Err(ApplicationError::InvalidPayload {
            command: CommandName::CancelTask,
            ..
        })
    ));
}

#[test]
fn project_execution_reason_rejects_an_overlong_value_for_both_commands() {
    for name in [
        CommandName::StartProjectExecution,
        CommandName::StopProjectExecution,
    ] {
        let accepted = CommandEnvelope::parse(
            name,
            request(
                1,
                json!({"reason": "x".repeat(MAX_COMMAND_AUDIT_DETAIL_CHARACTERS)}),
            ),
            "execution-reason-boundary-accepted",
        );
        let parsed = CommandEnvelope::parse(
            name,
            request(
                1,
                json!({"reason": "x".repeat(MAX_COMMAND_AUDIT_DETAIL_CHARACTERS + 1)}),
            ),
            "execution-reason-boundary",
        );

        assert!(accepted.is_ok());
        assert!(matches!(
            parsed,
            Err(ApplicationError::InvalidPayload { command, .. }) if command == name
        ));
    }
}

#[test]
fn external_outcome_parser_rejects_metadata_beyond_the_shared_entry_budget() {
    let metadata = (0..=MAX_ARTIFACT_METADATA_ENTRIES)
        .map(|index| (format!("field_{index}"), json!(index)))
        .collect::<Map<_, _>>();
    let parsed = CommandEnvelope::parse(
        CommandName::SubmitExternalStageOutcome,
        request(
            1,
            json!({
                "task_id": uuid::Uuid::now_v7().to_string(),
                "expected_task_revision": 1,
                "stage_id": "review",
                "outcome": "accept",
                "wait_condition_id": uuid::Uuid::now_v7().to_string(),
                "artifacts": [{
                    "kind": "review_result",
                    "title": "Review result",
                    "metadata": metadata,
                    "body": {"verdict": "accepted"}
                }]
            }),
        ),
        "metadata-entry-boundary",
    );

    assert!(matches!(
        parsed,
        Err(ApplicationError::InvalidPayload {
            command: CommandName::SubmitExternalStageOutcome,
            ..
        })
    ));
}
#[test]
fn draft_dod_patch_distinguishes_missing_null_and_text() {
    use forge_application::DraftTaskPatch;
    let missing: DraftTaskPatch =
        serde_json::from_value(serde_json::json!({"title":"Keep DoD"})).unwrap();
    assert_eq!(missing.definition_of_done, None);
    let clear: DraftTaskPatch =
        serde_json::from_value(serde_json::json!({"definition_of_done":null})).unwrap();
    assert_eq!(clear.definition_of_done, Some(None));
    assert!(clear.validate_nonempty().is_ok());
    let text: DraftTaskPatch =
        serde_json::from_value(serde_json::json!({"definition_of_done":"Acceptance"})).unwrap();
    assert_eq!(text.definition_of_done, Some(Some("Acceptance".to_owned())));
}
