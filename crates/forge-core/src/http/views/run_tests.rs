use forge_domain::{EmployeeId, ProjectId, TaskId};
use forge_storage::{RunDesiredState, RunObservedState, RunProjection};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{ListView, RunDetailView, run_view};

#[test]
fn run_list_and_detail_exclude_private_projection_canaries() {
    let task_id = TaskId::new();
    let projection = RunProjection {
        id: Uuid::now_v7(),
        project_id: ProjectId::new(),
        assignment: serde_json::from_value(json!({
            "purpose":"task_stage", "owner":{
                "task_id":task_id, "queue_entry_id":Uuid::now_v7(), "stage_id":"work"
            }
        }))
        .unwrap(),
        lease_id: Uuid::now_v7(),
        employee_id: Some(EmployeeId::new()),
        attempt_number: 2,
        lease_fencing_token: 7,
        environment_epoch: 3,
        last_sequence: 5,
        desired_state: RunDesiredState::StopRequested,
        observed_state: RunObservedState::Running,
        run_spec_version: 7,
        run_spec: json!({"auth":{"token":"private-auth-canary"},"prompt":"private-prompt-canary"}),
        context_manifest: json!({"memory":"private-context-canary"}),
        observed_details: json!({"raw":"private-observation-canary"}),
    };
    let list = serde_json::to_value(ListView {
        items: vec![run_view(projection.clone())],
        next_cursor: None,
    })
    .unwrap();
    let detail = serde_json::to_value(RunDetailView {
        run: run_view(projection),
        diagnostics: json!({
            "runtime_report":null,"handoff":null,"incidents":[],"evidence":[],
            "streams":[],"proxy_usage":null,"git_source":null
        }),
    })
    .unwrap();
    for view in [&list["items"][0], &detail] {
        assert_eq!(view["task_id"], task_id.to_string());
        assert_eq!(view["desired_state"], "stop_requested");
        assert_eq!(view["observed_state"], "running");
        assert_eq!(view["run_spec_version"], 7);
        for forbidden in [
            "run_spec",
            "context_manifest",
            "observed_details",
            "lease_id",
            "auth",
            "prompt",
        ] {
            assert!(view.get(forbidden).is_none(), "private field {forbidden}");
        }
        let wire = serde_json::to_string(view).unwrap();
        for canary in [
            "private-auth-canary",
            "private-prompt-canary",
            "private-context-canary",
            "private-observation-canary",
        ] {
            assert!(!wire.contains(canary));
        }
    }
    let expected: Value = json!([
        "assignment",
        "attempt",
        "desired_state",
        "employee_id",
        "environment_epoch",
        "id",
        "last_observed_sequence",
        "lease_fencing_token",
        "observed_state",
        "run_spec_version",
        "stage_id",
        "task_id"
    ]);
    let mut keys: Vec<_> = list["items"][0]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    keys.sort();
    assert_eq!(json!(keys), expected);
}
