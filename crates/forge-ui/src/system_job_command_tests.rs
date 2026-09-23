use serde_json::{Value, json};

use crate::{command::CommandTarget, system_job_command::SystemJobCommand};

const PROJECT: &str = "01988000-0000-7000-8000-000000000001";
const EMPLOYEE: &str = "01988000-0000-7000-8000-000000000002";
const JOB: &str = "01988000-0000-7000-8000-000000000003";

fn request(payload: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"project_id":PROJECT,"expected_revision":3,"payload":payload}))
        .unwrap()
}

#[test]
fn named_system_job_actions_have_exact_payloads_and_receipts() {
    for (name, target, payload, resource_kind, resource_id) in [
        (
            "request_task_summary",
            CommandTarget::RequestTaskSummary,
            json!({"task_id":EMPLOYEE}),
            "system_job",
            JOB,
        ),
        (
            "request_employee_onboarding",
            CommandTarget::RequestEmployeeOnboarding,
            json!({"employee_id":EMPLOYEE}),
            "system_job",
            JOB,
        ),
        (
            "retry_system_job",
            CommandTarget::RetrySystemJob,
            json!({"job_id":JOB}),
            "system_job",
            JOB,
        ),
        (
            "skip_employee_onboarding",
            CommandTarget::SkipEmployeeOnboarding,
            json!({"employee_id":EMPLOYEE,"reason":"Approved local bypass"}),
            "employee",
            EMPLOYEE,
        ),
    ] {
        assert!(CommandTarget::from_browser_path(&format!("/api/commands/{name}")).is_some());
        assert!(CommandTarget::from_browser_path(&format!("/api/commands/{name}/")).is_none());
        let command = SystemJobCommand::parse(&request(payload), target).unwrap();
        let receipt = serde_json::to_vec(&json!({"command_id":JOB,"status":"applied","project_revision":4,"event_ids":[EMPLOYEE],"resource":{"kind":resource_kind,"id":resource_id}})).unwrap();
        command.validate_receipt(&receipt).unwrap();
    }
}

#[test]
fn system_job_boundary_rejects_forged_operation_and_missing_configuration() {
    assert!(
        SystemJobCommand::parse(
            &request(json!({"employee_id":EMPLOYEE,"operation":"retry"})),
            CommandTarget::RequestEmployeeOnboarding
        )
        .is_err()
    );
    assert!(
        SystemJobCommand::parse(
            &request(json!({"employee_id":EMPLOYEE,"reason":" "})),
            CommandTarget::SkipEmployeeOnboarding
        )
        .is_err()
    );
    assert!(
        SystemJobCommand::parse(
            &request(json!({"expected_settings_revision":0,"policy":{},"binding":{}})),
            CommandTarget::ConfigureSystemJobs
        )
        .is_err()
    );
    let command = SystemJobCommand::parse(
        &request(json!({"job_id":JOB})),
        CommandTarget::RetrySystemJob,
    )
    .unwrap();
    let foreign = serde_json::to_vec(&json!({"command_id":JOB,"status":"applied","project_revision":4,"event_ids":[EMPLOYEE],"resource":{"kind":"system_job","id":EMPLOYEE}})).unwrap();
    assert!(command.validate_receipt(&foreign).is_err());
}
