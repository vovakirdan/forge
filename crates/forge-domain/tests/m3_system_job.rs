#[path = "support/m3_system_job.rs"]
mod fixture;

use forge_domain::{
    EmployeeId, ExecutionAssignment, ProjectId, TaskId,
    runtime::{RuntimeLaunchSpec, SandboxLaunchSpec},
    system_job::SystemJobKind,
};
use serde_json::json;

#[test]
fn system_job_roundtrip_preserves_exact_attempt_without_task_or_employee_ownership() {
    let spec = fixture::spec(ProjectId::new(), EmployeeId::new());
    spec.validate().unwrap();
    let value = serde_json::to_value(&spec).unwrap();
    let decoded: RuntimeLaunchSpec = serde_json::from_value(value.clone()).unwrap();
    let sandbox: SandboxLaunchSpec = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(sandbox.schema_version(), 7);
    assert_eq!(decoded.surface_id, spec.run_id);
    assert!(decoded.communication.is_none() && decoded.resolution.is_none());
    let owner = ExecutionAssignment::SystemJob(spec.assignment.clone());
    assert!(owner.task_stage().is_none() && owner.communication().is_none());
    assert_eq!(owner.system_job(), Some(&spec.assignment));
    assert!(value.get("task_id").is_none() && value.get("employee_id").is_none());
    assert_eq!(
        value["input"]["target_employee_id"],
        json!(spec.input.target_employee_id)
    );
}

#[test]
fn system_job_rejects_fake_task_employee_surface_and_mixed_purpose_payloads() {
    let spec = fixture::spec(ProjectId::new(), EmployeeId::new());
    let base = serde_json::to_value(&spec).unwrap();
    for (field, replacement) in [
        ("task_id", json!(TaskId::new())),
        ("employee_id", json!(EmployeeId::new())),
        ("surface_id", json!(spec.run_id)),
        ("communication", json!({})),
        ("resolution", json!({})),
        ("source_request", json!({})),
        ("file_inputs", json!([])),
        ("schema_version", json!(2)),
    ] {
        let mut invalid = base.clone();
        invalid[field] = replacement;
        assert!(
            serde_json::from_value::<RuntimeLaunchSpec>(invalid).is_err(),
            "{field}"
        );
    }
    for (pointer, replacement) in [
        ("/assignment/generation", json!(0)),
        ("/assignment/generation", json!(u64::MAX)),
        ("/binding/surface", json!({"mode":"filesystem_sandbox"})),
        ("/max_result_bytes", json!(0)),
        ("/input/target_employee_id", json!(null)),
        ("/input/source_task_id", json!(TaskId::new())),
    ] {
        let mut invalid = base.clone();
        *invalid.pointer_mut(pointer).unwrap() = replacement;
        assert!(
            serde_json::from_value::<RuntimeLaunchSpec>(invalid).is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn summary_task_reference_is_context_only_and_cannot_change_runtime_project() {
    let mut spec = fixture::spec(ProjectId::new(), EmployeeId::new());
    spec.assignment.kind = SystemJobKind::Summarization;
    spec.input.source_task_id = Some(TaskId::new());
    spec.input.target_employee_id = None;
    spec.input.covered_sequence = 12;
    spec.validate().unwrap();
    let decoded: RuntimeLaunchSpec =
        serde_json::from_value(serde_json::to_value(&spec).unwrap()).unwrap();
    assert!(decoded.communication.is_none() && decoded.resolution.is_none());
    spec.project_id = ProjectId::new();
    assert!(spec.validate().is_err());
}
