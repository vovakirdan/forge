//! Synthetic bidirectional transport, shaped from pinned 0.153.2 generated schema.
use forge_protocol::{
    runtime::RuntimeInputConfig,
    supervisor::v1::{
        CloseRuntimeInput, DeliverRuntimeInput, RuntimeInputReceipt, RuntimeInputStatus,
        RuntimeMessageInput, deliver_runtime_input::Action,
    },
};
use forge_provider_codex::{
    app_server::{self, AppServerTurn},
    driver,
};
use forge_provider_common::{
    PrivateMaterialization, SecretBytes, native_event::NativeDriverEvent,
    native_input::NativeMailbox,
};
use serde_json::{Value, json};
use std::{
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

fn setup() -> (tempfile::TempDir, NativeMailbox, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let private = root.path().join("private");
    let inputs = root.path().join("inputs");
    let receipts = root.path().join("receipts");
    for path in [&private, &inputs] {
        std::fs::DirBuilder::new().mode(0o700).create(path).unwrap();
    }
    let mailbox = NativeMailbox::open(
        RuntimeInputConfig {
            run_id: Uuid::now_v7().to_string(),
            fencing_token: 7,
            environment_epoch: 3,
        },
        &private,
        inputs.clone(),
        receipts.clone(),
    )
    .unwrap();
    (root, mailbox, inputs, receipts)
}
fn command(mailbox: &NativeMailbox, seq: u64, action: Action) -> DeliverRuntimeInput {
    DeliverRuntimeInput {
        command_id: Uuid::now_v7().to_string(),
        run_id: mailbox.scope().run_id.clone(),
        lease_fencing_token: 7,
        environment_epoch: 3,
        sequence: seq,
        action: Some(action),
    }
}
fn put(path: &Path, input: &DeliverRuntimeInput) {
    PrivateMaterialization::create(
        &path.join(format!("{:020}-{}.json", input.sequence, input.command_id)),
        &SecretBytes::new(serde_json::to_vec(input).unwrap()),
    )
    .unwrap();
}
async fn send(writer: &mut (impl AsyncWriteExt + Unpin), value: Value) {
    writer
        .write_all((value.to_string() + "\n").as_bytes())
        .await
        .unwrap();
    writer.flush().await.unwrap();
}
async fn read(reader: &mut (impl AsyncBufReadExt + Unpin)) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    serde_json::from_str(&line).unwrap()
}
fn thread_response(thread: Uuid) -> Value {
    json!({"id":"forge-thread","result":{"model":"fixture-model","modelProvider":"openai","cwd":"/workspace/task","approvalPolicy":"never","sandbox":{"type":"dangerFullAccess"},"thread":{"id":thread,"sessionId":thread,"ephemeral":true,"cliVersion":"0.153.2"}}})
}
async fn handshake(
    reader: &mut (impl AsyncBufReadExt + Unpin),
    writer: &mut (impl AsyncWriteExt + Unpin),
    thread: Uuid,
) {
    assert_eq!(read(reader).await["method"], "initialize");
    send(writer, json!({"id":"forge-initialize","result":{}})).await;
    assert_eq!(read(reader).await["method"], "initialized");
    assert_eq!(read(reader).await["method"], "thread/start");
    send(writer, thread_response(thread)).await;
}
async fn finished(writer: &mut (impl AsyncWriteExt + Unpin), thread: Uuid, turn: Uuid, input: u64) {
    send(writer,json!({"method":"thread/tokenUsage/updated","params":{"threadId":thread,"turnId":turn,"tokenUsage":{"total":{"inputTokens":input,"outputTokens":2,"cachedInputTokens":0,"reasoningOutputTokens":0,"totalTokens":input+2}}}})).await;
    send(writer,json!({"method":"turn/completed","params":{"threadId":thread,"turn":{"id":turn,"status":"completed","error":null}}})).await;
}

fn assert_addressed_turn(request: &Value, input: &DeliverRuntimeInput, source: &Value) {
    assert_eq!(request["method"], "turn/start");
    assert_eq!(request["id"], input.command_id);
    assert_eq!(request["params"]["clientUserMessageId"], input.command_id);
    let text = request["params"]["input"][0]["text"].as_str().unwrap();
    assert!(text.starts_with(&format!(
        "Forge addressed instruction {}.",
        source["id"].as_str().unwrap()
    )));
    assert!(!text.contains(&input.command_id));
    assert!(text.ends_with(&source.to_string()));
}

#[tokio::test]
async fn bootstrap_ready_redelivery_then_go_preserves_message_and_transport_identities() {
    let (_root, mut mailbox, inputs, receipts) = setup();
    let run = Uuid::parse_str(&mailbox.scope().run_id).unwrap();
    let ready_id = Uuid::now_v7();
    let go_id = Uuid::now_v7();
    let ready_source = json!({"id":ready_id,"body":"Reply READY, then wait for GO."});
    let go_source = json!({"id":go_id,"body":"GO: begin the assigned work."});
    let ready = command(
        &mailbox,
        1,
        Action::Message(RuntimeMessageInput {
            source_message_json: ready_source.to_string(),
        }),
    );
    let go = command(
        &mailbox,
        2,
        Action::Message(RuntimeMessageInput {
            source_message_json: go_source.to_string(),
        }),
    );
    let close = command(
        &mailbox,
        3,
        Action::CloseAfterTurn(CloseRuntimeInput {
            reason_code: "assignment_completed".into(),
        }),
    );
    let (client, server) = tokio::io::duplex(16384);
    let (reader, mut writer) = tokio::io::split(server);
    let mut reader = BufReader::new(reader);
    let (_stop, rx) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(client);
        let mut reader = BufReader::new(reader);
        let mut output = Vec::new();
        driver::session(
            &mut writer,
            &mut reader,
            &mut mailbox,
            "fixture-model",
            "/workspace/task",
            SecretBytes::new(b"Read Forge Inbox and reply READY before starting work.".to_vec()),
            rx,
            |bytes| {
                output.push(bytes);
                Ok(())
            },
        )
        .await
        .unwrap();
        output
    });
    let thread = Uuid::now_v7();
    handshake(&mut reader, &mut writer, thread).await;
    let first = read(&mut reader).await;
    assert_eq!(first["id"], run.to_string());
    assert_eq!(first["params"]["clientUserMessageId"], run.to_string());
    let turn1 = Uuid::now_v7();
    send(
        &mut writer,
        json!({"id":run,"result":{"turn":{"id":turn1,"status":"inProgress","error":null}}}),
    )
    .await;
    // Simulate a completed bootstrap Inbox reply at the provider protocol boundary;
    // this fixture does not execute the Gateway or prove a canonical acknowledgement.
    send(&mut writer,json!({"method":"item/completed","params":{"threadId":thread,"turnId":turn1,"item":{"id":"bootstrap-ready-reply","type":"mcpToolCall","server":"forge","tool":"forge_reply_instruction","arguments":{"message_id":Uuid::now_v7(),"target_message_id":ready_id,"body":"READY"},"status":"completed"}}})).await;
    put(&inputs, &ready);
    assert!(
        tokio::time::timeout(Duration::from_millis(180), read(&mut reader))
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_dir(&receipts).unwrap().count(), 0);
    finished(&mut writer, thread, turn1, 10).await;
    let second = read(&mut reader).await;
    assert_addressed_turn(&second, &ready, &ready_source);
    assert_eq!(second["params"]["threadId"], thread.to_string());
    let turn2 = Uuid::now_v7();
    send(&mut writer,json!({"id":ready.command_id,"result":{"turn":{"id":turn2,"status":"inProgress","error":null}}})).await;
    put(&inputs, &go);
    assert!(
        tokio::time::timeout(Duration::from_millis(180), read(&mut reader))
            .await
            .is_err()
    );
    finished(&mut writer, thread, turn2, 25).await;
    let third = read(&mut reader).await;
    assert_addressed_turn(&third, &go, &go_source);
    assert_eq!(third["params"]["threadId"], thread.to_string());
    let turn3 = Uuid::now_v7();
    send(&mut writer,json!({"id":go.command_id,"result":{"turn":{"id":turn3,"status":"inProgress","error":null}}})).await;
    put(&inputs, &close);
    assert!(!task.is_finished());
    finished(&mut writer, thread, turn3, 40).await;
    let output = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    let metadata: Vec<_> = output
        .iter()
        .filter_map(|bytes| NativeDriverEvent::decode(bytes.expose()).ok())
        .collect();
    assert_eq!(metadata.len(), 6);
    let accepted: Vec<_> = metadata
        .iter()
        .filter_map(|event| match event {
            NativeDriverEvent::InputAccepted { input_id, .. } => Some(*input_id),
            _ => None,
        })
        .collect();
    assert_eq!(
        accepted,
        [
            run,
            Uuid::parse_str(&ready.command_id).unwrap(),
            Uuid::parse_str(&go.command_id).unwrap()
        ]
    );
    assert!(
        matches!(metadata.last().unwrap(),NativeDriverEvent::TurnFinished{usage:Some(usage),..} if usage.input_tokens==40)
    );
    assert_eq!(std::fs::read_dir(&receipts).unwrap().count(), 3);
    for (input, status) in [
        (&ready, RuntimeInputStatus::RuntimeAccepted),
        (&go, RuntimeInputStatus::RuntimeAccepted),
        (&close, RuntimeInputStatus::InputClosed),
    ] {
        let receipt: RuntimeInputReceipt = serde_json::from_slice(
            &std::fs::read(receipts.join(format!("{}.json", input.command_id))).unwrap(),
        )
        .unwrap();
        assert_eq!(receipt.input_command_id, input.command_id);
        assert_eq!(receipt.run_id, run.to_string());
        assert_eq!(receipt.lease_fencing_token, 7);
        assert_eq!(receipt.environment_epoch, 3);
        assert_eq!(receipt.status, status as i32);
    }
    let mut extra = String::new();
    assert_eq!(
        reader.read_line(&mut extra).await.unwrap(),
        0,
        "close must not become a fourth prompt"
    );
}

#[test]
fn correlation_usage_and_thread_configuration_fail_closed() {
    assert_eq!(
        app_server::request_failure(
            &json!({"error":{"code":-32000,"message":"429 quota exceeded: secret-provider-body"}})
        ),
        Some(forge_provider_common::adapter::RuntimeFailureKind::RateLimited)
    );
    let pinned: Value =
        serde_json::from_str(include_str!("fixtures/app-server-0.153.2-thread.json")).unwrap();
    assert!(app_server::validate_thread(&pinned, "gpt-5.4", "/tmp").is_ok());
    let thread = Uuid::now_v7();
    let turn = Uuid::now_v7();
    assert!(
        app_server::validate_thread(&thread_response(thread), "fixture-model", "/workspace/task")
            .is_ok()
    );
    assert!(
        app_server::validate_thread(&thread_response(thread), "wrong-model", "/workspace/task")
            .is_err()
    );
    let mut state = AppServerTurn::new(thread.to_string(), turn.to_string());
    assert!(state.ingest(&json!({"method":"turn/completed","params":{"threadId":Uuid::now_v7(),"turn":{"id":turn,"status":"completed"}}})).is_err());
    assert!(state.ingest(&json!({"method":"thread/tokenUsage/updated","params":{"threadId":thread,"turnId":turn,"tokenUsage":{"total":{"inputTokens":1,"outputTokens":2,"cachedInputTokens":9,"reasoningOutputTokens":0,"totalTokens":3}}}})).is_err());
    assert!(
        app_server::accepted_turn(
            &json!({"id":Uuid::now_v7(),"result":{"turn":{"id":turn,"status":"inProgress"}}}),
            Uuid::now_v7()
        )
        .is_err()
    );
}
