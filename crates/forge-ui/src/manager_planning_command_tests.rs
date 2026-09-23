use crate::{command::CommandTarget, manager_planning_command::ManagerPlanningCommand};
use serde_json::json;
const PROJECT: &str = "01988000-0000-7000-8000-000000000001";
const TASK: &str = "01988000-0000-7000-8000-000000000002";
const EMPLOYEE: &str = "01988000-0000-7000-8000-000000000003";
const SCHEDULE: &str = "01988000-0000-7000-8000-000000000004";
fn request(payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"project_id":PROJECT,"expected_revision":3,"payload":payload}))
        .unwrap()
}
#[test]
fn named_planning_actions_have_exact_scope_and_receipts() {
    for (path, target, payload, kind, id) in [
        (
            "set_next_run_employee",
            CommandTarget::SetNextRunEmployee,
            json!({"task_id":TASK,"expected_task_revision":2,"employee_id":EMPLOYEE}),
            "task",
            TASK,
        ),
        (
            "clear_next_run_employee",
            CommandTarget::ClearNextRunEmployee,
            json!({"task_id":TASK,"expected_task_revision":2}),
            "task",
            TASK,
        ),
        (
            "schedule_task_resume",
            CommandTarget::ScheduleTaskResume,
            json!({"task_id":TASK,"expected_task_revision":2,"wait_condition_id":EMPLOYEE,"not_before":"2030-01-01T00:00:00Z","reason":"After maintenance"}),
            "task_resume_schedule",
            SCHEDULE,
        ),
        (
            "cancel_task_resume",
            CommandTarget::CancelTaskResume,
            json!({"schedule_id":SCHEDULE}),
            "task_resume_schedule",
            SCHEDULE,
        ),
    ] {
        assert!(CommandTarget::from_browser_path(&format!("/api/commands/{path}")).is_some());
        let command = ManagerPlanningCommand::parse(&request(payload), target).unwrap();
        let receipt = serde_json::to_vec(&json!({"command_id":EMPLOYEE,"status":"applied","project_revision":4,"event_ids":[SCHEDULE],"resource":{"kind":kind,"id":id}})).unwrap();
        command.validate_receipt(&receipt).unwrap();
    }
    let invalid = request(
        json!({"task_id":TASK,"expected_task_revision":2,"employee_id":EMPLOYEE,"stop_current":true}),
    );
    assert!(ManagerPlanningCommand::parse(&invalid, CommandTarget::SetNextRunEmployee).is_err());
}
