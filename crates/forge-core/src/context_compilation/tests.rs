use super::add_knowledge_instruction;
use forge_domain::{EmployeeId, ProjectId, knowledge::KnowledgeContextBundle};
use serde_json::{Value, json};

#[test]
fn each_prompt_revision_tracks_actual_text_independently_of_the_profile() {
    let mut run = json!({"binding":{"execution_profile":{"id":"unchanged","revision":1},"system_prompt":"System A","employee_prompt":"Role A"}});
    let original = super::prompt_revisions(&run);
    run["binding"]["employee_prompt"] = json!("Role B");
    let employee_changed = super::prompt_revisions(&run);
    assert_eq!(employee_changed.0, original.0);
    assert_ne!(employee_changed.1, original.1);
    run["binding"]["system_prompt"] = json!("System B");
    let system_changed = super::prompt_revisions(&run);
    assert_ne!(system_changed.0, employee_changed.0);
    assert_eq!(system_changed.1, employee_changed.1);
    assert!(system_changed.0.starts_with("sha256:"));
    assert_eq!(run["binding"]["execution_profile"]["revision"], 1);
}

#[test]
fn empty_projects_preserve_existing_task_contract() {
    let contract = json!({"task":{"title":"Original intent"},"previous_handoff":null});
    let instruction = add_knowledge_instruction(contract.clone(), None).expect("instruction");
    assert_eq!(
        serde_json::from_str::<Value>(&instruction).expect("contract"),
        contract
    );
}

#[test]
fn instruction_serializes_the_same_owned_bundle_as_the_manifest() {
    let bundle = KnowledgeContextBundle {
        schema_version: 1,
        project_id: ProjectId::new(),
        employee_id: EmployeeId::new(),
        required_pages: vec![],
        optional_pages: vec![],
        derived_memory: vec![],
        omitted_optional_candidates: 1,
    };
    let instruction =
        add_knowledge_instruction(json!({"task":"Unchanged task contract"}), Some(&bundle))
            .expect("instruction");
    let parsed: Value = serde_json::from_str(&instruction).expect("JSON");
    assert_eq!(
        parsed["knowledge_context"],
        serde_json::to_value(bundle).expect("manifest")
    );
    assert_eq!(parsed["task"], "Unchanged task contract");
    assert!(
        parsed["knowledge_authority"]
            .as_str()
            .expect("authority distinction")
            .contains("not instructions or authority")
    );
}
