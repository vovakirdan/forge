use serde_json::{Value, json};

use crate::{amend_employee::AmendEmployee, command::CommandTarget};

const PROJECT: &str = "01988000-0000-7000-8000-000000000001";
const EMPLOYEE: &str = "01988000-0000-7000-8000-000000000002";
const VERSION: &str = "01988000-0000-7000-8000-000000000003";

fn request() -> Value {
    json!({
        "project_id": PROJECT,
        "expected_revision": 3,
        "payload": {"employee_id": EMPLOYEE, "expected_employee_revision": 2,
            "patch": {"name": "New name", "role": "reviewer", "max_concurrent_runs": 2,
                "stage_eligibility": {"mode": "only", "stages": [{"pipeline_version_id": VERSION, "stage_id": "work"}]}}}
    })
}

#[test]
fn accepts_all_supported_changes_and_matching_receipt() {
    let parsed = AmendEmployee::parse(&serde_json::to_vec(&request()).unwrap()).unwrap();
    let receipt = json!({"command_id": "01988000-0000-7000-8000-000000000004",
        "status": "applied", "project_revision": 4,
        "event_ids": ["01988000-0000-7000-8000-000000000005"],
        "resource": {"kind": "employee", "id": EMPLOYEE}});
    assert!(
        parsed
            .validate_receipt(&serde_json::to_vec(&receipt).unwrap())
            .is_ok()
    );
    for changed in [
        json!({"project_revision": 5}),
        json!({"resource": {"kind": "task", "id": EMPLOYEE}}),
        json!({"resource": {"kind": "employee", "id": PROJECT}}),
    ] {
        let mut invalid = receipt.clone();
        for (key, value) in changed.as_object().unwrap() {
            invalid[key] = value.clone();
        }
        assert!(
            parsed
                .validate_receipt(&serde_json::to_vec(&invalid).unwrap())
                .is_err()
        );
    }
}

#[test]
fn rejects_empty_nullable_extra_and_invalid_patches() {
    for patch in [
        json!({}),
        json!({"name": null}),
        json!({"role":" "}),
        json!({"max_concurrent_runs":0}),
        json!({"max_concurrent_runs":65536}),
        json!({"state":"disabled"}),
        json!({"stage_eligibility":{"mode":"any","stages":[]}}),
        json!({"stage_eligibility":{"mode":"only","stages":[]}}),
        json!({"stage_eligibility":{"mode":"only","stages":[{"pipeline_version_id":VERSION,"stage_id":"work"},{"pipeline_version_id":VERSION,"stage_id":"work"}]}}),
    ] {
        let mut invalid = request();
        invalid["payload"]["patch"] = patch;
        assert!(
            AmendEmployee::parse(&serde_json::to_vec(&invalid).unwrap()).is_err(),
            "{invalid}"
        );
    }
    for path in [
        "/api/commands/amend_employee/",
        "/api/commands/amend_employee?actor=owner",
    ] {
        assert!(CommandTarget::from_browser_path(path).is_none());
    }
    assert!(CommandTarget::from_browser_path("/api/commands/amend_employee").is_some());
}
