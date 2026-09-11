use super::{digest, result::SystemJobResult};
use forge_domain::{EmployeeId, ProjectId, TaskId, system_job::SystemJobKind};
use serde_json::{Value, json};
use uuid::Uuid;
#[path = "../../../forge-domain/tests/support/m3_system_job.rs"]
mod fixture;

fn onboarding() -> (forge_domain::runtime::SystemJobRunSpec, Value) {
    let employee = EmployeeId::new();
    let mut spec = fixture::spec(ProjectId::new(), employee);
    let source = json!({"kind":"event","event_id":Uuid::now_v7()});
    spec.input.context = json!({"source_refs":[source.clone()]});
    spec.input.source_digest = digest(&serde_json::to_vec(&spec.input.context).unwrap());
    let result = json!({"source_digest":spec.input.source_digest,"entries":[{"subject":{"kind":"employee_memory_entry","employee_id":employee,"task_id":null},"markdown":"I read the supplied role and rules; this is not proof of comprehension.","source_refs":[source]}]});
    (spec, result)
}

#[test]
fn only_frozen_sources_and_target_can_be_used() {
    let (spec, result) = onboarding();
    assert!(SystemJobResult::decode(result.clone(), &spec).is_ok());
    let mut forged = result.clone();
    forged["entries"][0]["subject"]["employee_id"] = json!(EmployeeId::new());
    assert!(SystemJobResult::decode(forged, &spec).is_err());
    let mut forged = result.clone();
    forged["entries"][0]["source_refs"][0]["event_id"] = json!(Uuid::now_v7());
    assert!(SystemJobResult::decode(forged, &spec).is_err());
    let mut forged = result;
    forged["source_digest"] = json!("0".repeat(64));
    assert!(SystemJobResult::decode(forged, &spec).is_err());
}
#[test]
fn model_cannot_supply_authority_metadata_or_public_onboarding_output() {
    let (spec, result) = onboarding();
    for key in [
        "created_at",
        "revision",
        "accepted",
        "actor",
        "id",
        "verified",
    ] {
        let mut forged = result.clone();
        forged["entries"][0][key] = json!("invented");
        assert!(SystemJobResult::decode(forged, &spec).is_err(), "{key}");
    }
    let mut forged = result;
    forged["entries"][0]["subject"] = json!({"kind":"project_knowledge_entry","task_id":null});
    assert!(SystemJobResult::decode(forged, &spec).is_err());
}
#[test]
fn source_digest_checks_the_actual_frozen_bytes() {
    let (mut spec, result) = onboarding();
    spec.input.context["new_unpinned_text"] = json!(true);
    assert!(SystemJobResult::decode(result, &spec).is_err());
}
#[test]
fn output_is_bounded_nonempty_and_single_subject() {
    let (mut spec, result) = onboarding();
    spec.max_result_bytes = 16;
    assert!(SystemJobResult::decode(result.clone(), &spec).is_err());
    spec.max_result_bytes = 16 * 1024;
    let mut empty = result.clone();
    empty["entries"] = json!([]);
    assert!(SystemJobResult::decode(empty, &spec).is_err());
    let mut duplicate = result.clone();
    duplicate["entries"] = json!([result["entries"][0], result["entries"][0]]);
    assert!(SystemJobResult::decode(duplicate, &spec).is_err());
    let mut duplicate = result.clone();
    duplicate["entries"][0]["source_refs"] = json!([
        result["entries"][0]["source_refs"][0],
        result["entries"][0]["source_refs"][0]
    ]);
    assert!(SystemJobResult::decode(duplicate, &spec).is_err());
    let mut blank = result;
    blank["entries"][0]["markdown"] = json!(" \n");
    assert!(SystemJobResult::decode(blank, &spec).is_err());
}
#[test]
fn summaries_require_task_output_and_do_not_target_unrelated_workers() {
    let (mut spec, mut result) = onboarding();
    let task = TaskId::new();
    let employee = EmployeeId::new();
    spec.assignment.kind = SystemJobKind::Summarization;
    spec.input.target_employee_id = None;
    spec.input.source_task_id = Some(task);
    spec.input.context["allowed_employee_ids"] = json!([employee]);
    spec.input.source_digest = digest(&serde_json::to_vec(&spec.input.context).unwrap());
    result["source_digest"] = json!(spec.input.source_digest);
    result["entries"][0]["subject"] = json!({"kind":"task_summary","task_id":task});
    assert!(SystemJobResult::decode(result.clone(), &spec).is_ok());
    let mut wrong = result.clone();
    wrong["entries"][0]["subject"]["task_id"] = json!(TaskId::new());
    assert!(SystemJobResult::decode(wrong, &spec).is_err());
    let mut private = result.clone();
    let mut note = private["entries"][0].clone();
    note["subject"] = json!({"kind":"employee_memory_entry","employee_id":employee,"task_id":task});
    private["entries"]
        .as_array_mut()
        .unwrap()
        .push(note.clone());
    assert!(SystemJobResult::decode(private.clone(), &spec).is_ok());
    private["entries"][1]["subject"]["employee_id"] = json!(EmployeeId::new());
    assert!(SystemJobResult::decode(private, &spec).is_err());
    result["entries"] = json!([note]);
    assert!(SystemJobResult::decode(result, &spec).is_err());
}
