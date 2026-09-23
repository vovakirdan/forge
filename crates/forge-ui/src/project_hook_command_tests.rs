use serde_json::json;

use crate::{command::CommandTarget, project_hook_command::ConfigureProjectHook};

const PROJECT: &str = "01988000-0000-7000-8000-000000000001";
const VERSION: &str = "01988000-0000-7000-8000-000000000002";

fn body() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "project_id": PROJECT,
        "expected_revision": 3,
        "payload": {
            "name": "lint",
            "image": format!("runner@sha256:{}", "a".repeat(64)),
            "command": ["lint", "--check"],
            "workdir": ".",
            "limits": {"cpu_millis": 2000, "memory_bytes": 1073741824, "pids": 64, "wall_seconds": 300, "stop_grace_seconds": 10},
            "max_output_bytes": 1048576,
            "applicable_task_kinds": ["delivery"],
            "required": false
        }
    })).unwrap()
}

#[test]
fn configure_hook_accepts_exact_shape_and_receipt() {
    let command = ConfigureProjectHook::parse(&body()).unwrap();
    let receipt = serde_json::to_vec(&json!({
        "command_id": "01988000-0000-7000-8000-000000000003",
        "status": "applied",
        "project_revision": 4,
        "event_ids": ["01988000-0000-7000-8000-000000000004"],
        "resource": {"kind":"project_hook_version", "id":VERSION}
    }))
    .unwrap();
    command.validate_receipt(&receipt).unwrap();
    assert!(CommandTarget::from_browser_path("/api/commands/configure_project_hook").is_some());
    assert!(CommandTarget::from_browser_path("/api/commands/configure_project_hook/").is_none());
    let mut forged: serde_json::Value = serde_json::from_slice(&body()).unwrap();
    forged["payload"]["secret_id"] = json!(VERSION);
    assert!(ConfigureProjectHook::parse(&serde_json::to_vec(&forged).unwrap()).is_err());
    let mut invalid: serde_json::Value = serde_json::from_slice(&body()).unwrap();
    invalid["payload"]["image"] = json!("runner:latest");
    assert!(ConfigureProjectHook::parse(&serde_json::to_vec(&invalid).unwrap()).is_err());
}
