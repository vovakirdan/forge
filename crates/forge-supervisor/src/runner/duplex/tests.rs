use super::*;
use forge_domain::{
    Actor, ActorId, EmployeeId, PipelineVersionId, ProjectId, StageId, TaskId, Timestamp,
    communication::{
        DeliveryRequirement, MessageInput, MessageKind, MessageTarget, TaskMessageContext,
    },
};
use forge_protocol::supervisor::v1::{CloseRuntimeInput, RuntimeInputReceipt, RuntimeMessageInput};
use std::{
    os::unix::fs::DirBuilderExt,
    sync::atomic::{AtomicBool, AtomicU64},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    sync::Notify,
};

fn fixture() -> (RuntimeInputConfig, Paths, Arc<OutputBudget>) {
    let root = std::env::temp_dir().join(format!("forge-native-input-{}", Uuid::now_v7()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .unwrap();
    for name in ["mailbox", "private", "receipts"] {
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(root.join(name))
            .unwrap();
    }
    let config = RuntimeInputConfig {
        run_id: Uuid::now_v7().to_string(),
        fencing_token: 7,
        environment_epoch: 3,
    };
    let initial = encode_user_message(
        config.run_id.parse().unwrap(),
        config.run_id.parse().unwrap(),
        &SecretBytes::new(b"initial".to_vec()),
    )
    .unwrap();
    write_private(&root.join("initial"), initial.expose()).unwrap();
    (
        config,
        Paths {
            initial: root.join("initial"),
            mailbox: root.join("mailbox"),
            private: root.join("private"),
            receipts: root.join("receipts"),
        },
        Arc::new(OutputBudget {
            remaining: AtomicU64::new(4096),
            incomplete: AtomicBool::new(false),
            failed: Notify::new(),
        }),
    )
}
fn input(config: &RuntimeInputConfig, seq: u64) -> DeliverRuntimeInput {
    let source = EmployeeMessage::new(MessageInput {
        id: Uuid::now_v7(),
        thread_id: Uuid::now_v7(),
        project_id: ProjectId::new(),
        employee_id: EmployeeId::new(),
        sequence: seq,
        sender: Actor::human(ActorId::new()),
        target: MessageTarget::TaskExecution {
            context: TaskMessageContext {
                task_id: TaskId::new(),
                pipeline_version_id: PipelineVersionId::new(),
                stage_id: StageId::new("work").unwrap(),
                stage_visit: 1,
            },
        },
        kind: MessageKind::Instruction,
        requirement: DeliveryRequirement::Acknowledged,
        body: "Keep the public interface unchanged.".into(),
        reply_to: None,
        created_at: Timestamp::now_utc(),
    })
    .unwrap();
    DeliverRuntimeInput {
        command_id: Uuid::now_v7().to_string(),
        run_id: config.run_id.clone(),
        lease_fencing_token: config.fencing_token,
        environment_epoch: config.environment_epoch,
        sequence: seq,
        action: Some(Action::Message(RuntimeMessageInput {
            source_message_json: serde_json::to_string(&source).unwrap(),
        })),
    }
}
fn publish(paths: &Paths, input: &DeliverRuntimeInput) {
    crate::runtime_input::immutable(
        &paths
            .mailbox
            .join(format!("{:020}-{}.json", input.sequence, input.command_id)),
        &SecretBytes::new(serde_json::to_vec(input).unwrap()),
    )
    .unwrap();
}
async fn echo(tx: &mpsc::Sender<Frame>, session: &str, id: &str) {
    forward(serde_json::json!({"type":"user","session_id":session,"uuid":id,"parent_tool_use_id":null,"message":{"role":"user","content":"echoed input"}}).to_string().as_bytes(),tx).await.unwrap();
}
async fn end(tx: &mpsc::Sender<Frame>, session: &str) {
    forward(serde_json::json!({"type":"result","subtype":"success","session_id":session,"uuid":Uuid::now_v7(),"is_error":false,"usage":{"input_tokens":10,"output_tokens":3}}).to_string().as_bytes(),tx).await.unwrap();
}

#[tokio::test]
async fn queued_input_waits_for_turn_boundary_and_native_echo_is_not_stdin_success() {
    let (config, paths, budget) = fixture();
    let (tx, rx) = mpsc::channel(8);
    let (write, read) = tokio::io::duplex(65536);
    let message = input(&config, 1);
    publish(&paths, &message);
    let receipt_path = paths.receipts.join(format!("{}.json", message.command_id));
    let mut close = input(&config, 2);
    close.action = Some(Action::CloseAfterTurn(CloseRuntimeInput {
        reason_code: "assignment_completed".into(),
    }));
    let close_path = paths.receipts.join(format!("{}.json", close.command_id));
    let close_mailbox = paths.mailbox.clone();
    let task = tokio::spawn(pump(write, config.clone(), paths, rx, Arc::clone(&budget)));
    let mut read = BufReader::new(read);
    let mut line = String::new();
    read.read_line(&mut line).await.unwrap();
    assert!(line.contains("initial"));
    line.clear();
    assert!(
        tokio::time::timeout(Duration::from_millis(150), read.read_line(&mut line))
            .await
            .is_err()
    );
    assert!(!receipt_path.exists());
    echo(&tx, &config.run_id, &config.run_id).await;
    end(&tx, &config.run_id).await;
    tokio::time::timeout(Duration::from_secs(2), read.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    assert!(line.contains(&message.command_id));
    assert!(
        !receipt_path.exists(),
        "stdin success is not native acceptance"
    );
    echo(&tx, &config.run_id, &message.command_id).await;
    end(&tx, &config.run_id).await;
    crate::runtime_input::immutable(
        &close_mailbox.join(format!("{:020}-{}.json", close.sequence, close.command_id)),
        &SecretBytes::new(serde_json::to_vec(&close).unwrap()),
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    // A normal input close may precede the provider's final diagnostic frame.
    forward(serde_json::json!({"type":"system","subtype":"status","session_id":config.run_id,"uuid":Uuid::now_v7()}).to_string().as_bytes(),&tx).await.unwrap();
    assert!(!budget.incomplete.load(std::sync::atomic::Ordering::Acquire));
    let receipt: RuntimeInputReceipt =
        serde_json::from_slice(&read_private(&receipt_path).unwrap()).unwrap();
    assert_eq!(receipt.status, Status::RuntimeAccepted as i32);
    assert!(close_path.exists());
    let mut remaining = Vec::new();
    read.read_to_end(&mut remaining).await.unwrap();
    assert!(
        remaining.is_empty(),
        "duplicate mailbox input and Close are not extra turns"
    );
    assert_eq!(
        budget.remaining.load(std::sync::atomic::Ordering::Acquire),
        4096,
        "input cannot replenish output budget"
    );
}

#[tokio::test]
async fn wrong_session_echo_fails_without_runtime_acceptance() {
    let (config, paths, budget) = fixture();
    let (tx, rx) = mpsc::channel(8);
    let (write, _read) = tokio::io::duplex(65536);
    let task = tokio::spawn(pump(write, config.clone(), paths, rx, budget));
    echo(&tx, &Uuid::now_v7().to_string(), &config.run_id).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
}

#[tokio::test]
async fn driver_restart_cannot_reinject_an_uncertain_previous_session() {
    let (config, paths, budget) = fixture();
    write_private(&paths.private.join("native-input-started"), b"1\n").unwrap();
    let (_tx, rx) = mpsc::channel(8);
    let (write, mut read) = tokio::io::duplex(65536);
    assert!(pump(write, config, paths, rx, budget).await.is_err());
    let mut data = Vec::new();
    read.read_to_end(&mut data).await.unwrap();
    assert!(data.is_empty());
}
