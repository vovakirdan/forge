use crate::{command::CommandTarget, resolver_command::ResolverCommand};
use serde_json::json;
const PROJECT: &str = "01988000-0000-7000-8000-000000000001";
const TASK: &str = "01988000-0000-7000-8000-000000000002";
const EMPLOYEE: &str = "01988000-0000-7000-8000-000000000003";
const ESCALATION: &str = "01988000-0000-7000-8000-000000000004";
fn request(payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"project_id":PROJECT,"expected_revision":3,"payload":payload}))
        .unwrap()
}
#[test]
fn resolver_commands_are_exact_and_receipts_scoped() {
    for (path, target, payload, kind, id, revision) in [
        (
            "configure_resolver_route",
            CommandTarget::ConfigureResolverRoute,
            json!({"route_key":"review","employee_ids":[EMPLOYEE],"assignment_timeout_seconds":300}),
            "project",
            PROJECT,
            4,
        ),
        (
            "raise_escalation",
            CommandTarget::RaiseEscalation,
            json!({"task_id":TASK,"expected_task_revision":2,"route_key":"review","category":"clarification","question":"Choose a direction"}),
            "escalation",
            ESCALATION,
            4,
        ),
    ] {
        assert!(CommandTarget::from_browser_path(&format!("/api/commands/{path}")).is_some());
        let command = ResolverCommand::parse(&request(payload), target).unwrap();
        let receipt = serde_json::to_vec(&json!({"command_id":EMPLOYEE,"status":"applied","project_revision":revision,"event_ids":[TASK],"resource":{"kind":kind,"id":id}})).unwrap();
        command.validate_receipt(&receipt).unwrap();
    }
    let invalid = request(
        json!({"route_key":"review","employee_ids":[EMPLOYEE,EMPLOYEE],"assignment_timeout_seconds":300}),
    );
    assert!(ResolverCommand::parse(&invalid, CommandTarget::ConfigureResolverRoute).is_err());
}
