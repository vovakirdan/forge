use forge_protocol::{
    runtime::RuntimeInputConfig,
    supervisor::v1::{
        DeliverRuntimeInput, RuntimeInputStatus, RuntimeMessageInput, deliver_runtime_input::Action,
    },
};
use forge_provider_common::{
    PrivateMaterialization, SecretBytes,
    native_input::{NativeMailbox, persist_receipt},
};
use std::os::unix::fs::DirBuilderExt;
use uuid::Uuid;
#[test]
fn native_marker_scope_and_immutable_receipt_are_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let private = root.path().join("private");
    let inputs = root.path().join("inputs");
    let receipts = root.path().join("receipts");
    for path in [&private, &inputs] {
        std::fs::DirBuilder::new().mode(0o700).create(path).unwrap();
    }
    let scope = RuntimeInputConfig {
        run_id: Uuid::now_v7().to_string(),
        fencing_token: 1,
        environment_epoch: 1,
    };
    let mut mailbox =
        NativeMailbox::open(scope.clone(), &private, inputs.clone(), receipts.clone()).unwrap();
    assert!(
        NativeMailbox::open(scope.clone(), &private, inputs.clone(), receipts.clone()).is_err()
    );
    let input = DeliverRuntimeInput {
        command_id: Uuid::now_v7().to_string(),
        run_id: scope.run_id,
        lease_fencing_token: 1,
        environment_epoch: 1,
        sequence: 1,
        action: Some(Action::Message(RuntimeMessageInput {
            source_message_json: "{}".into(),
        })),
    };
    PrivateMaterialization::create(
        &inputs.join(format!("{:020}-{}.json", input.sequence, input.command_id)),
        &SecretBytes::new(serde_json::to_vec(&input).unwrap()),
    )
    .unwrap();
    assert_eq!(mailbox.next().unwrap().unwrap(), input);
    mailbox.mark_sent(&input).unwrap();
    assert!(mailbox.next().unwrap().is_none());
    mailbox.accepted(&input).unwrap();
    mailbox.accepted(&input).unwrap();
    assert!(persist_receipt(&receipts, &input, RuntimeInputStatus::DeliveryUnknown).is_err());
    let mut foreign = input;
    foreign.environment_epoch = 2;
    PrivateMaterialization::create(
        &inputs.join(format!("{:020}-{}.json", 2, foreign.command_id)),
        &SecretBytes::new(serde_json::to_vec(&foreign).unwrap()),
    )
    .unwrap();
    assert!(mailbox.next().is_err());
}
