use serde_json::json;

use crate::{command::CommandTarget, employee_runtime_command::ConfigureEmployeeRuntime};

const PROJECT: &str = "01988000-0000-7000-8000-000000000001";
const EMPLOYEE: &str = "01988000-0000-7000-8000-000000000002";
const PROFILE: &str = "01988000-0000-7000-8000-000000000003";
const CREDENTIAL: &str = "01988000-0000-7000-8000-000000000004";
const SECRET: &str = "01988000-0000-7000-8000-000000000005";

fn request() -> serde_json::Value {
    json!({
      "project_id": PROJECT, "expected_revision": 3,
      "payload": {"employee_id": EMPLOYEE, "binding": {
        "execution_profile": {
          "id": PROFILE, "revision": 1, "project_id": PROJECT,
          "adapter_id": "opencode_runtime", "adapter_version": "1", "provider_id": "openai", "model": "test",
          "credential_binding": {"id": CREDENTIAL, "project_id": PROJECT, "secret_id": SECRET, "account_id": null, "allowed_delivery_modes": ["proxy_only"]},
          "credential_delivery": "proxy_only",
          "capability_profile": {"adapter_id": "opencode_runtime", "adapter_version": "1", "transport_engine": "api_runtime", "capabilities": ["controlled_stop", "gateway_auth"], "credential_exposed_to_run": false}
        },
        "image": format!("runner@sha256:{}", "a".repeat(64)),
        "surface": {"mode":"none"},
        "system_prompt": "System", "employee_prompt": "Employee"
      }}
    })
}

#[test]
fn configure_runtime_rejects_credentials_and_correlates_employee_receipt() {
    let request = request();
    let command = ConfigureEmployeeRuntime::parse(&serde_json::to_vec(&request).unwrap()).unwrap();
    assert!(CommandTarget::from_browser_path("/api/commands/configure_employee_runtime").is_some());
    let receipt = serde_json::to_vec(&json!({"command_id":PROFILE,"status":"applied","project_revision":4,"event_ids":[SECRET],"resource":{"kind":"employee","id":EMPLOYEE}})).unwrap();
    command.validate_receipt(&receipt).unwrap();
    let mut secret = request.clone();
    secret["payload"]["binding"]["api_key"] = json!("hidden");
    assert!(ConfigureEmployeeRuntime::parse(&serde_json::to_vec(&secret).unwrap()).is_err());
    let mut foreign = request;
    foreign["payload"]["binding"]["execution_profile"]["project_id"] = json!(EMPLOYEE);
    assert!(ConfigureEmployeeRuntime::parse(&serde_json::to_vec(&foreign).unwrap()).is_err());
}
