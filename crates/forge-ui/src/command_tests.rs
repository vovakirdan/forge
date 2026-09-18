use serde_json::{Value, json};

use crate::command::AmendDraft;

pub(crate) const PROJECT: &str = "01988000-0000-7000-8000-000000000001";
pub(crate) const TASK: &str = "01988000-0000-7000-8000-000000000002";

pub(crate) fn request() -> Value {
    json!({"project_id":PROJECT,"expected_revision":3,"payload":{
        "task_id":TASK,"expected_task_revision":1,"patch":{"title":"Edited"}
    }})
}

pub(crate) fn receipt(status: &str) -> Value {
    json!({
        "command_id":"01988000-0000-7000-8000-000000000003",
        "status":status,"project_revision":4,
        "event_ids":["01988000-0000-7000-8000-000000000004"],
        "resource":{"kind":"task","id":TASK}
    })
}

#[test]
fn amend_draft_accepts_each_text_patch_without_changing_text() {
    for patch in [
        json!({"title":"  🚀  "}),
        json!({"description":""}),
        json!({"title":"Name","description":"Line 1\nLine 2"}),
    ] {
        let mut value = request();
        value["payload"]["patch"] = patch;
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(AmendDraft::parse(&bytes).is_ok());
    }
}

#[test]
fn amend_draft_rejects_unknown_fields_null_types_empty_patch_and_invalid_ids_revisions() {
    let cases = [
        ("/actor", json!("owner")),
        ("/payload/actor", json!("owner")),
        ("/payload/patch/lifecycle", json!("ready")),
        ("/payload/patch", json!({})),
        ("/payload/patch/title", Value::Null),
        ("/payload/patch/description", Value::Null),
        ("/payload/patch/title", json!(42)),
        ("/project_id", json!("00000000-0000-4000-8000-000000000001")),
        (
            "/payload/task_id",
            json!("01988000-0000-7000-0000-000000000002"),
        ),
        ("/expected_revision", json!(0)),
        ("/expected_revision", json!(9_007_199_254_740_992_u64)),
        ("/expected_revision", json!(1.5)),
        ("/payload/expected_task_revision", json!(-1)),
        ("/payload/expected_task_revision", json!(0)),
    ];
    for (pointer, replacement) in cases {
        let mut value = request();
        let (parent, field) = pointer.rsplit_once('/').unwrap();
        value.pointer_mut(parent).unwrap()[field] = replacement;
        assert!(
            AmendDraft::parse(&serde_json::to_vec(&value).unwrap()).is_err(),
            "{pointer}"
        );
    }
    let duplicate = serde_json::to_string(&request()).unwrap().replacen(
        "\"expected_revision\":3",
        "\"expected_revision\":3,\"expected_revision\":4",
        1,
    );
    assert!(AmendDraft::parse(duplicate.as_bytes()).is_err());
}

#[test]
fn amend_draft_only_accepts_correlated_committed_receipts() {
    let command = AmendDraft::parse(&serde_json::to_vec(&request()).unwrap()).unwrap();
    for status in ["applied", "replayed"] {
        assert!(
            command
                .validate_receipt(&serde_json::to_vec(&receipt(status)).unwrap())
                .is_ok()
        );
    }
    for (pointer, replacement) in [
        ("/status", json!("queued")),
        ("/command_id", json!("invalid")),
        ("/project_revision", json!(3)),
        ("/project_revision", json!(5)),
        ("/event_ids", json!([])),
        ("/event_ids", json!(["invalid"])),
        ("/resource", Value::Null),
        ("/resource/kind", json!("project")),
        ("/resource/id", json!(PROJECT)),
        ("/extra", json!(true)),
    ] {
        let mut value = receipt("applied");
        let (parent, field) = pointer.rsplit_once('/').unwrap();
        value.pointer_mut(parent).unwrap()[field] = replacement;
        assert!(
            command
                .validate_receipt(&serde_json::to_vec(&value).unwrap())
                .is_err(),
            "{pointer}"
        );
    }
}
