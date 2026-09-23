use serde_json::json;

use crate::{command::CommandTarget, create_project::CreateProject};

const PROJECT: &str = "01988000-0000-7000-8000-000000000001";

#[test]
fn create_project_uses_reserved_identity_and_zero_revision() {
    let body = serde_json::to_vec(
        &json!({"project_id":PROJECT,"expected_revision":0,"payload":{"name":"New Project"}}),
    )
    .unwrap();
    let command = CreateProject::parse(&body).unwrap();
    let receipt = serde_json::to_vec(&json!({"command_id":"01988000-0000-7000-8000-000000000002","status":"applied","project_revision":1,"event_ids":["01988000-0000-7000-8000-000000000003"],"resource":{"kind":"project","id":PROJECT}})).unwrap();
    command.validate_receipt(&receipt).unwrap();
    assert!(CommandTarget::from_browser_path("/api/commands/create_project").is_some());
    assert!(CommandTarget::from_browser_path("/api/commands/create_project/").is_none());
    let forged = serde_json::to_vec(&json!({"project_id":PROJECT,"expected_revision":0,"payload":{"name":"New Project","actor":"human"}})).unwrap();
    assert!(CreateProject::parse(&forged).is_err());
    let wrong = serde_json::to_vec(&json!({"command_id":"01988000-0000-7000-8000-000000000002","status":"applied","project_revision":1,"event_ids":["01988000-0000-7000-8000-000000000003"],"resource":{"kind":"project","id":"01988000-0000-7000-8000-000000000004"}})).unwrap();
    assert!(command.validate_receipt(&wrong).is_err());
}
