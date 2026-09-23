use serde_json::{Value, json};

use crate::{command::CommandTarget, employee_lifecycle::EmployeeLifecycle};

const PROJECT: &str = "01988000-0000-7000-8000-000000000001";
const EMPLOYEE: &str = "01988000-0000-7000-8000-000000000002";

fn request() -> Value {
    json!({"project_id":PROJECT,"expected_revision":3,
        "payload":{"employee_id":EMPLOYEE,"expected_employee_revision":2,"reason":"Operator choice"}})
}

#[test]
fn accepts_optional_reason_and_correlated_receipt() {
    let parsed = EmployeeLifecycle::parse(&serde_json::to_vec(&request()).unwrap()).unwrap();
    let receipt = json!({"command_id":"01988000-0000-7000-8000-000000000003",
        "status":"applied","project_revision":4,
        "event_ids":["01988000-0000-7000-8000-000000000004"],
        "resource":{"kind":"employee","id":EMPLOYEE}});
    assert!(
        parsed
            .validate_receipt(&serde_json::to_vec(&receipt).unwrap())
            .is_ok()
    );
    let mut without_reason = request();
    without_reason["payload"]
        .as_object_mut()
        .unwrap()
        .remove("reason");
    assert!(EmployeeLifecycle::parse(&serde_json::to_vec(&without_reason).unwrap()).is_ok());
    for (field, replacement) in [
        ("project_revision", json!(5)),
        ("resource", json!({"kind":"employee","id":PROJECT})),
        ("event_ids", json!([])),
    ] {
        let mut wrong = receipt.clone();
        wrong[field] = replacement;
        assert!(
            parsed
                .validate_receipt(&serde_json::to_vec(&wrong).unwrap())
                .is_err()
        );
    }
}

#[test]
fn rejects_invalid_scope_revisions_and_reason() {
    for (pointer, value) in [
        ("/expected_revision", json!(0)),
        (
            "/payload/employee_id",
            json!(PROJECT.replace("7000", "4000")),
        ),
        ("/payload/expected_employee_revision", json!(0)),
        ("/payload/reason", json!(" ")),
        ("/payload/reason", json!("a\u{0000}b")),
        ("/payload/reason", json!("x".repeat(10_001))),
    ] {
        let mut wrong = request();
        *wrong.pointer_mut(pointer).unwrap() = value;
        assert!(EmployeeLifecycle::parse(&serde_json::to_vec(&wrong).unwrap()).is_err());
    }
    let mut extra = request();
    extra["payload"]["mode"] = json!("force");
    assert!(EmployeeLifecycle::parse(&serde_json::to_vec(&extra).unwrap()).is_err());
    for action in ["enable_employee", "disable_employee", "retire_employee"] {
        assert!(CommandTarget::from_browser_path(&format!("/api/commands/{action}")).is_some());
        assert!(CommandTarget::from_browser_path(&format!("/api/commands/{action}/")).is_none());
    }
}
