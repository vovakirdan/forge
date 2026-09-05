use serde_json::{Value, json};

/// Returns the four-stage employee-only delivery graph used by the happy path.
#[must_use]
pub fn four_stage_pipeline() -> Value {
    json!({
        "name": "M0 delivery acceptance",
        "task_kinds": ["delivery"],
        "entry_stage_id": "implementation",
        "stages": [
            {"id": "implementation", "name": "Implementation", "executor_kind": "employee", "outcomes": ["implemented"]},
            {"id": "verification", "name": "Verification", "executor_kind": "employee", "outcomes": ["verified"]},
            {"id": "review", "name": "Review", "executor_kind": "employee", "outcomes": ["reviewed"]},
            {"id": "integration", "name": "Integration", "executor_kind": "employee", "outcomes": ["integrated"]}
        ],
        "transitions": [
            stage_transition("implementation", "implemented", json!({"kind": "stage", "stage_id": "verification"})),
            stage_transition("verification", "verified", json!({"kind": "stage", "stage_id": "review"})),
            task_history_transition("review", "reviewed", json!({"kind": "stage", "stage_id": "integration"})),
            stage_transition("integration", "integrated", json!({"kind": "done"}))
        ]
    })
}

/// Returns a one-stage employee Pipeline for contention and dependency tests.
#[must_use]
pub fn single_stage_pipeline() -> Value {
    json!({
        "name": "M0 single-stage acceptance",
        "task_kinds": ["delivery"],
        "entry_stage_id": "work",
        "stages": [
            {"id": "work", "name": "Work", "executor_kind": "employee", "outcomes": ["completed"]}
        ],
        "transitions": [stage_transition("work", "completed", json!({"kind": "done"}))]
    })
}

/// Returns a finite retry graph whose lexical first outcome always retries in M0.
#[must_use]
pub fn retry_pipeline() -> Value {
    json!({
        "name": "M0 retry acceptance",
        "task_kinds": ["delivery"],
        "entry_stage_id": "work",
        "max_stage_visits": 2,
        "stages": [
            {"id": "work", "name": "Work", "executor_kind": "employee", "outcomes": ["again", "done"]}
        ],
        "transitions": [
            stage_transition("work", "again", json!({"kind": "stage", "stage_id": "work"})),
            stage_transition("work", "done", json!({"kind": "done"}))
        ]
    })
}

/// Returns a finite human-stage retry graph with no employee Run involved.
#[must_use]
pub fn human_retry_pipeline() -> Value {
    json!({
        "name": "M0 human retry acceptance",
        "task_kinds": ["delivery"],
        "entry_stage_id": "decision",
        "max_stage_visits": 2,
        "stages": [
            {"id": "decision", "name": "Human decision", "executor_kind": "human", "outcomes": ["again", "done"]}
        ],
        "transitions": [
            {"from_stage_id": "decision", "outcome": "again", "target": {"kind": "stage", "stage_id": "decision"}},
            {"from_stage_id": "decision", "outcome": "done", "target": {"kind": "done"}}
        ]
    })
}

fn stage_transition(from_stage_id: &str, outcome: &str, target: Value) -> Value {
    json!({
        "from_stage_id": from_stage_id,
        "outcome": outcome,
        "target": target,
        "artifact_requirements": [{
            "kind": "stage_evidence",
            "minimum_count": 1,
            "scope": "current_stage"
        }]
    })
}

fn task_history_transition(from_stage_id: &str, outcome: &str, target: Value) -> Value {
    json!({
        "from_stage_id": from_stage_id,
        "outcome": outcome,
        "target": target,
        "artifact_requirements": [{
            "kind": "stage_evidence",
            "minimum_count": 1,
            "scope": "task_history"
        }]
    })
}
