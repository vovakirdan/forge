use forge_protocol::wire::{CommandName, CommandRequest};
use serde_json::json;

use super::{CommandEnvelope, CommandPayload};

#[test]
fn rejects_unknown_typed_create_task_fields() {
    let request = CommandRequest {
        project_id: uuid::Uuid::now_v7().to_string(),
        expected_revision: 1,
        payload: json!({
            "title": "Investigate",
            "kind": "analysis",
            "pipeline_version_id": uuid::Uuid::now_v7().to_string(),
            "priority": "normal",
            "unexpected": true
        })
        .as_object()
        .cloned()
        .unwrap_or_default(),
    };

    let parsed = CommandEnvelope::parse(CommandName::CreateTask, request, "test-key");

    assert!(parsed.is_err());
}

#[test]
fn maps_the_path_to_a_typed_payload() {
    let request = CommandRequest {
        project_id: uuid::Uuid::now_v7().to_string(),
        expected_revision: 0,
        payload: json!({"name": "M0 Demo"})
            .as_object()
            .cloned()
            .unwrap_or_default(),
    };

    let parsed = CommandEnvelope::parse(CommandName::CreateProject, request, "create-project")
        .expect("valid command");

    assert!(matches!(parsed.payload, CommandPayload::CreateProject(_)));
}

#[test]
fn rejects_non_v7_public_project_identity() {
    let request = CommandRequest {
        project_id: uuid::Uuid::nil().to_string(),
        expected_revision: 0,
        payload: json!({"name": "M0 Demo"})
            .as_object()
            .cloned()
            .unwrap_or_default(),
    };

    let parsed = CommandEnvelope::parse(CommandName::CreateProject, request, "create-project");

    assert!(parsed.is_err());
}

#[test]
fn rejects_create_project_with_a_nonzero_revision() {
    let request = CommandRequest {
        project_id: uuid::Uuid::now_v7().to_string(),
        expected_revision: 1,
        payload: json!({"name": "M0 Demo"})
            .as_object()
            .cloned()
            .unwrap_or_default(),
    };

    let parsed = CommandEnvelope::parse(CommandName::CreateProject, request, "create-project");

    assert!(parsed.is_err());
}

#[test]
fn eagerly_rejects_an_invalid_external_stage_key() {
    let request = CommandRequest {
        project_id: uuid::Uuid::now_v7().to_string(),
        expected_revision: 1,
        payload: json!({
            "task_id": uuid::Uuid::now_v7().to_string(),
            "expected_task_revision": 1,
            "stage_id": "Needs Review",
            "outcome": "accept",
            "wait_condition_id": uuid::Uuid::now_v7().to_string()
        })
        .as_object()
        .cloned()
        .unwrap_or_default(),
    };

    let parsed = CommandEnvelope::parse(
        CommandName::SubmitExternalStageOutcome,
        request,
        "submit-outcome",
    );

    assert!(parsed.is_err());
}
