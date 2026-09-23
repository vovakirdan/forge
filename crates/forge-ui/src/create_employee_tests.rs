use serde_json::{Value, json};

use crate::{command::CommandTarget, create_employee::CreateEmployee};

const PROJECT: &str = "01988000-0000-7000-8000-000000000001";
const VERSION: &str = "01988000-0000-7000-8000-000000000002";

fn request() -> Value {
    json!({"project_id":PROJECT,"expected_revision":3,"payload":{
        "name":"New teammate","role":"reviewer","stage_eligibility":{"mode":"any"}
    }})
}

fn receipt(kind: &str) -> Value {
    json!({
        "command_id":"01988000-0000-7000-8000-000000000003",
        "status":"applied","project_revision":4,
        "event_ids":["01988000-0000-7000-8000-000000000004"],
        "resource":{"kind":kind,"id":"01988000-0000-7000-8000-000000000005"}
    })
}

#[test]
fn creation_accepts_explicit_any_or_unique_scoped_stages() {
    assert!(CreateEmployee::parse(&serde_json::to_vec(&request()).unwrap()).is_ok());
    let mut value = request();
    value["payload"]["stage_eligibility"] = json!({"mode":"only","stages":[
        {"pipeline_version_id":VERSION,"stage_id":"review"},
        {"pipeline_version_id":VERSION,"stage_id":"build"}
    ]});
    let command = CreateEmployee::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(
        command
            .validate_receipt(&serde_json::to_vec(&receipt("employee")).unwrap())
            .is_ok()
    );
    assert!(
        command
            .validate_receipt(&serde_json::to_vec(&receipt("task")).unwrap())
            .is_err()
    );
}

#[test]
fn creation_rejects_unknown_fields_invalid_scopes_and_bad_stage_sets() {
    let mut cases = Vec::new();
    for (pointer, replacement) in [
        ("/project_id", json!("invalid")),
        ("/expected_revision", json!(0)),
        ("/payload/name", json!(" ")),
        ("/payload/name", json!("x".repeat(201))),
        ("/payload/role", json!("")),
        (
            "/payload/stage_eligibility",
            json!({"mode":"only","stages":[]}),
        ),
        ("/payload/stage_eligibility", json!({"mode":"unrestricted"})),
        (
            "/payload/stage_eligibility",
            json!({"mode":"any","token":"secret"}),
        ),
        (
            "/payload/stage_eligibility",
            json!({"mode":"only","stages":[{"pipeline_version_id":PROJECT,"stage_id":"Review"}]}),
        ),
        (
            "/payload/stage_eligibility",
            json!({"mode":"only","stages":[{"pipeline_version_id":"foreign","stage_id":"review"}]}),
        ),
    ] {
        let mut value = request();
        *value.pointer_mut(pointer).unwrap() = replacement;
        cases.push(value);
    }
    let mut duplicate = request();
    duplicate["payload"]["stage_eligibility"] = json!({"mode":"only","stages":[
        {"pipeline_version_id":VERSION,"stage_id":"review"},
        {"pipeline_version_id":VERSION,"stage_id":"review"}
    ]});
    cases.push(duplicate);
    let mut unknown = request();
    unknown["actor"] = json!("owner");
    cases.push(unknown);
    for case in cases {
        assert!(
            CreateEmployee::parse(&serde_json::to_vec(&case).unwrap()).is_err(),
            "{case}"
        );
    }
    assert!(CreateEmployee::parse(b"[]").is_err());
    let duplicated_field = serde_json::to_string(&request()).unwrap().replacen(
        "\"name\":\"New teammate\"",
        "\"name\":\"New teammate\",\"name\":\"Another\"",
        1,
    );
    assert!(CreateEmployee::parse(duplicated_field.as_bytes()).is_err());
    assert!(CommandTarget::from_browser_path("/api/commands/create_employee").is_some());
    assert!(CommandTarget::from_browser_path("/api/commands/create_employee/").is_none());
}
