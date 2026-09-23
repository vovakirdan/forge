use super::{command::CommandTarget, management_command::ManagementCommand};
use serde_json::json;
const PROJECT: &str = "01988000-0000-7000-8000-000000000001";
const TASK: &str = "01988000-0000-7000-8000-000000000002";
const EMPLOYEE: &str = "01988000-0000-7000-8000-000000000003";
const WAIT: &str = "01988000-0000-7000-8000-000000000004";
fn body(payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"project_id":PROJECT,"expected_revision":3,"payload":payload}))
        .unwrap()
}
#[test]
fn management_commands_reject_unscoped_bodies_and_validate_exact_resource() {
    let cases = [
        (
            CommandTarget::StartProjectExecution,
            json!({"reason":"operator"}),
            "project",
            PROJECT,
            4,
        ),
        (
            CommandTarget::StopProjectExecution,
            json!({"reason":"operator"}),
            "project",
            PROJECT,
            5,
        ),
        (
            CommandTarget::PauseTask,
            json!({"task_id":TASK,"expected_task_revision":2,"mode":"graceful","reason":null}),
            "task",
            TASK,
            5,
        ),
        (
            CommandTarget::ResumeTask,
            json!({"task_id":TASK,"expected_task_revision":3,"wait_condition_id":WAIT}),
            "task",
            TASK,
            4,
        ),
        (
            CommandTarget::StopEmployee,
            json!({"employee_id":EMPLOYEE,"expected_employee_revision":2,"mode":"force","reason":"operator"}),
            "employee",
            EMPLOYEE,
            5,
        ),
        (
            CommandTarget::SubmitHumanResolution,
            json!({"escalation_id":TASK,"expected_escalation_revision":2,"assignment_id":EMPLOYEE,"lease_generation":1,"answer":{"disposition":"needs_management_change","summary":"Needs policy change","recommended_outcome_key":null}}),
            "escalation",
            TASK,
            5,
        ),
        (
            CommandTarget::RerouteEscalation,
            json!({"escalation_id":TASK,"expected_escalation_revision":2,"reason":"Human fallback required"}),
            "escalation",
            TASK,
            5,
        ),
        (
            CommandTarget::AcceptRunRecoveryAssessment,
            json!({"run_id":TASK,"assessment":"not_started_confirmed"}),
            "run",
            TASK,
            5,
        ),
    ];
    for (target, payload, kind, id, revision) in cases {
        let command = ManagementCommand::parse(&body(payload), target).unwrap();
        let receipt = json!({"command_id":WAIT,"status":"applied","project_revision":revision,"event_ids":[WAIT],"resource":{"kind":kind,"id":id}});
        assert!(
            command
                .validate_receipt(&serde_json::to_vec(&receipt).unwrap())
                .is_ok()
        );
        let wrong = json!({"command_id":WAIT,"status":"applied","project_revision":revision,"event_ids":[WAIT],"resource":{"kind":kind,"id":WAIT}});
        assert!(
            command
                .validate_receipt(&serde_json::to_vec(&wrong).unwrap())
                .is_err()
        );
    }
    assert!(
        ManagementCommand::parse(
            &body(json!({"task_id":TASK,"expected_task_revision":2,"mode":"instant"})),
            CommandTarget::PauseTask
        )
        .is_err()
    );
    assert!(
        ManagementCommand::parse(
            &body(json!({"run_id":TASK,"assessment":"unknown"})),
            CommandTarget::AcceptRunRecoveryAssessment
        )
        .is_err()
    );
    assert!(ManagementCommand::parse(
        &body(json!({"escalation_id":TASK,"expected_escalation_revision":2,"assignment_id":EMPLOYEE,"lease_generation":0,"answer":{"disposition":"continue_stage","summary":"yes","recommended_outcome_key":null}})),
        CommandTarget::SubmitHumanResolution
    ).is_err());
    assert!(
        ManagementCommand::parse(
            &body(json!({"task_id":TASK,"expected_task_revision":2,"mode":"force","reason":" "})),
            CommandTarget::PauseTask
        )
        .is_err()
    );
}
