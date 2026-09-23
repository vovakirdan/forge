use serde_json::json;

use crate::{command::CommandTarget, git_recovery_command::GitRecoveryCommand};

const PROJECT: &str = "01988000-0000-7000-8000-000000000001";
const OPERATION: &str = "01988000-0000-7000-8000-000000000002";

fn body() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "project_id": PROJECT,
        "expected_revision": 5,
        "payload": {
            "operation_id": OPERATION,
            "expected_task_revision": 9,
            "reason": "Physical state reconciled; review the held integration"
        }
    }))
    .unwrap()
}

#[test]
fn exact_git_recovery_paths_and_receipts() {
    for (path, target) in [
        (
            "/api/commands/retry_git_integration",
            CommandTarget::RetryGitIntegration,
        ),
        (
            "/api/commands/accept_git_integration_result",
            CommandTarget::AcceptGitIntegrationResult,
        ),
    ] {
        assert!(CommandTarget::from_browser_path(path).is_some());
        assert!(CommandTarget::from_browser_path(&format!("{path}/")).is_none());
        let command = GitRecoveryCommand::parse(&body(), target).unwrap();
        let receipt = serde_json::to_vec(&json!({
            "command_id": "01988000-0000-7000-8000-000000000003",
            "status": "applied",
            "project_revision": 6,
            "event_ids": ["01988000-0000-7000-8000-000000000004"],
            "resource": {"kind":"git_integration", "id":OPERATION}
        }))
        .unwrap();
        command.validate_receipt(&receipt).unwrap();
        let mut wrong: serde_json::Value = serde_json::from_slice(&receipt).unwrap();
        wrong["resource"]["kind"] = json!("task");
        assert!(
            command
                .validate_receipt(&serde_json::to_vec(&wrong).unwrap())
                .is_err()
        );
    }
}

#[test]
fn malformed_or_extra_intent_is_rejected() {
    let mut request: serde_json::Value = serde_json::from_slice(&body()).unwrap();
    request["payload"]["reason"] = json!("  ");
    assert!(
        GitRecoveryCommand::parse(
            &serde_json::to_vec(&request).unwrap(),
            CommandTarget::RetryGitIntegration
        )
        .is_err()
    );
    request["payload"]["reason"] = json!("review");
    request["payload"]["force"] = json!(true);
    assert!(
        GitRecoveryCommand::parse(
            &serde_json::to_vec(&request).unwrap(),
            CommandTarget::AcceptGitIntegrationResult
        )
        .is_err()
    );
}
