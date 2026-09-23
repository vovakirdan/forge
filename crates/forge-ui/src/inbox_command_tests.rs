use super::{command::CommandTarget, inbox_command::InboxCommand};
use serde_json::json;
const PROJECT: &str = "01988000-0000-7000-8000-000000000001";
const EMPLOYEE: &str = "01988000-0000-7000-8000-000000000002";
const THREAD: &str = "01988000-0000-7000-8000-000000000003";
const MESSAGE: &str = "01988000-0000-7000-8000-000000000004";
fn bytes(payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"project_id":PROJECT,"expected_revision":3,"payload":payload}))
        .unwrap()
}
#[test]
fn inbox_command_shapes_and_receipts_are_closed() {
    let cases = [
        (
            CommandTarget::OpenEmployeeThread,
            json!({"employee_id":EMPLOYEE,"task_id":null}),
            "employee_thread",
            THREAD,
        ),
        (
            CommandTarget::SendEmployeeMessage,
            json!({"thread_id":THREAD,"expected_thread_revision":1,"target":{"kind":"inbox"},"kind":"question","requirement":"answered","body":"Hello?","reply_to":null}),
            "employee_message",
            MESSAGE,
        ),
        (
            CommandTarget::WaiveMessageRequirement,
            json!({"message_id":MESSAGE,"reason":"obsolete"}),
            "message_requirement_waiver",
            MESSAGE,
        ),
    ];
    for (target, payload, kind, id) in cases {
        let command = InboxCommand::parse(&bytes(payload), target).unwrap();
        let receipt = json!({"command_id":THREAD,"status":"applied","project_revision":4,"event_ids":[MESSAGE],"resource":{"kind":kind,"id":id}});
        assert!(
            command
                .validate_receipt(&serde_json::to_vec(&receipt).unwrap())
                .is_ok()
        );
        let bad = json!({"command_id":THREAD,"status":"applied","project_revision":4,"event_ids":[MESSAGE],"resource":{"kind":"task","id":id}});
        assert!(
            command
                .validate_receipt(&serde_json::to_vec(&bad).unwrap())
                .is_err()
        );
    }
    assert!(InboxCommand::parse(&bytes(json!({"thread_id":THREAD,"expected_thread_revision":1,"target":{"kind":"inbox"},"kind":"reply","requirement":"answered","body":"x","reply_to":null})),CommandTarget::SendEmployeeMessage).is_err());
    assert!(
        InboxCommand::parse(
            &bytes(json!({"message_id":MESSAGE,"reason":" "})),
            CommandTarget::WaiveMessageRequirement
        )
        .is_err()
    );
    let retry = InboxCommand::parse(
        &bytes(json!({"run_id":THREAD,"reason":"physical quiescence observed"})),
        CommandTarget::RetryCommunication,
    )
    .unwrap();
    let receipt = json!({"command_id":THREAD,"status":"applied","project_revision":4,"event_ids":[MESSAGE],"resource":{"kind":"run","id":THREAD}});
    assert!(
        retry
            .validate_receipt(&serde_json::to_vec(&receipt).unwrap())
            .is_ok()
    );
}
