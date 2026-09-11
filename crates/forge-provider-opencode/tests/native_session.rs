//! Local HTTP/SSE fixture shaped from pinned OpenCode 1.18.29 /doc and SDK.
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::sse::{Event, Sse},
    routing::{get, post},
};
use forge_protocol::{
    runtime::RuntimeInputConfig,
    supervisor::v1::{
        CloseRuntimeInput, DeliverRuntimeInput, RuntimeInputReceipt, RuntimeInputStatus,
        RuntimeMessageInput, deliver_runtime_input::Action,
    },
};
use forge_provider_common::{
    PrivateMaterialization, SecretBytes, native_event::NativeDriverEvent,
    native_input::NativeMailbox,
};
use forge_provider_opencode::{OpenCodeClient, SessionResult};
use serde_json::{Value, json};
use std::{
    convert::Infallible,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, mpsc, watch};
use uuid::Uuid;
type EventSender = mpsc::Sender<Result<Event, Infallible>>;
#[derive(Clone, Default)]
struct Fixture {
    stream: Arc<Mutex<Option<EventSender>>>,
    prompts: Arc<Mutex<Vec<Value>>>,
    changed: Arc<Notify>,
    aborts: Arc<AtomicUsize>,
}
async fn fixture() -> (OpenCodeClient, Fixture, tokio::task::JoinHandle<()>) {
    let state = Fixture::default();
    let router = Router::new()
        .route(
            "/session",
            post(|| async { Json(json!({"id":"ses_fixture"})) }),
        )
        .route("/event", get(events))
        .route("/session/ses_fixture/prompt_async", post(prompt))
        .route("/session/ses_fixture/abort", post(abort))
        .route(
            "/session/ses_fixture/message/{message}",
            get(|| async {
                Json(json!({"info":{"id":"msg_wrong","role":"user","sessionID":"ses_fixture"}}))
            }),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = OpenCodeClient::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        "/workspace/task",
    )
    .unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (client, state, task)
}
async fn events(
    State(state): State<Fixture>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::channel(32);
    tx.send(Ok(Event::default().data(
        json!({"type":"server.connected","properties":{}}).to_string(),
    )))
    .await
    .unwrap();
    *state.stream.lock().unwrap() = Some(tx);
    Sse::new(futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|event| (event, rx))
    }))
}
async fn prompt(State(state): State<Fixture>, Json(body): Json<Value>) -> StatusCode {
    assert!(state.stream.lock().unwrap().is_some());
    state.prompts.lock().unwrap().push(body);
    state.changed.notify_one();
    StatusCode::NO_CONTENT
}
async fn abort(State(state): State<Fixture>) -> Json<bool> {
    state.aborts.fetch_add(1, Ordering::SeqCst);
    Json(true)
}
async fn wait_prompt(state: &Fixture, count: usize) -> Value {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let notified = state.changed.notified();
            if state.prompts.lock().unwrap().len() >= count {
                break;
            }
            notified.await;
        }
        state.prompts.lock().unwrap()[count - 1].clone()
    })
    .await
    .unwrap()
}
async fn event(state: &Fixture, value: Value) {
    let sender = state.stream.lock().unwrap().as_ref().unwrap().clone();
    sender
        .send(Ok(Event::default().data(value.to_string())))
        .await
        .unwrap();
}
async fn user(state: &Fixture, id: &str) {
    event(state,json!({"type":"message.updated","properties":{"info":{"id":id,"sessionID":"ses_fixture","role":"user","time":{}}}})).await;
}
async fn finish(state: &Fixture, parent: &str, assistant: &str) {
    event(state,json!({"type":"session.status","properties":{"sessionID":"ses_fixture","status":{"type":"busy"}}})).await;
    let value = json!({"type":"message.updated","properties":{"info":{"id":assistant,"parentID":parent,"sessionID":"ses_fixture","role":"assistant","time":{"completed":1},"tokens":{"input":7,"output":2,"reasoning":0,"cache":{"read":3,"write":0}}}}});
    event(state, value.clone()).await;
    event(state, value).await;
    event(
        state,
        json!({"type":"session.idle","properties":{"sessionID":"ses_fixture"}}),
    )
    .await;
}
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

#[tokio::test]
async fn two_turns_share_one_session_and_usage_and_require_native_user_message() {
    let (client, state, server) = fixture().await;
    let session = client.create_session().await.unwrap();
    let (_root, mut mailbox, inputs, receipts) = setup();
    let source_id = Uuid::now_v7();
    let source = json!({"id":source_id,"body":"follow-up"});
    let input = command(
        &mailbox,
        1,
        Action::Message(RuntimeMessageInput {
            source_message_json: source.to_string(),
        }),
    );
    put(&inputs, &input);
    let close = command(
        &mailbox,
        2,
        Action::CloseAfterTurn(CloseRuntimeInput {
            reason_code: "assignment_completed".into(),
        }),
    );
    let (_stop, rx) = watch::channel(false);
    let task = tokio::spawn(async move {
        let mut output = Vec::new();
        let result = client
            .run_live_session(
                &session,
                "fixture-model",
                SecretBytes::new(b"initial".to_vec()),
                rx,
                &mut mailbox,
                |bytes| {
                    output.push(bytes);
                    Ok(())
                },
            )
            .await
            .unwrap();
        (result, output)
    });
    let first = wait_prompt(&state, 1).await;
    let first_id = first["messageID"].as_str().unwrap();
    user(&state, first_id).await;
    tokio::time::sleep(Duration::from_millis(180)).await;
    assert_eq!(state.prompts.lock().unwrap().len(), 1);
    finish(&state, first_id, "msg_assistant_first").await;
    let second = wait_prompt(&state, 2).await;
    let second_id = second["messageID"].as_str().unwrap();
    let text = second["parts"][0]["text"].as_str().unwrap();
    assert!(text.starts_with(&format!("Forge addressed instruction {source_id}.")));
    assert!(!text.contains(&input.command_id));
    assert!(text.ends_with(&source.to_string()));
    assert_eq!(
        second_id,
        format!(
            "msg_{}",
            Uuid::parse_str(&input.command_id).unwrap().simple()
        )
    );
    assert_eq!(
        second["model"],
        json!({"providerID":"forge","modelID":"fixture-model"})
    );
    assert_eq!(
        std::fs::read_dir(&receipts).unwrap().count(),
        0,
        "HTTP 204 is not acceptance"
    );
    user(&state, second_id).await;
    put(&inputs, &close);
    // A late first-turn assistant cannot complete or double-charge the second turn.
    event(&state,json!({"type":"message.updated","properties":{"info":{"id":"msg_assistant_first","parentID":first_id,"sessionID":"ses_fixture","role":"assistant","time":{"completed":1},"tokens":{"input":999,"output":999}}}})).await;
    assert!(!task.is_finished());
    finish(&state, second_id, "msg_assistant_second").await;
    let (result, output) = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result, SessionResult::TurnEnded);
    let metadata: Vec<_> = output
        .iter()
        .filter_map(|b| NativeDriverEvent::decode(b.expose()).ok())
        .collect();
    assert_eq!(metadata.len(), 4);
    assert!(
        matches!(metadata.last().unwrap(),NativeDriverEvent::TurnFinished{usage:Some(usage),..} if usage.input_tokens==20&&usage.output_tokens==4)
    );
    assert!(matches!(
        &metadata[2],
        NativeDriverEvent::InputAccepted { input_id, turn_id, .. }
            if input_id.to_string() == input.command_id && turn_id == second_id
    ));
    assert_eq!(std::fs::read_dir(&receipts).unwrap().count(), 2);
    for (command, status) in [
        (&input, RuntimeInputStatus::RuntimeAccepted),
        (&close, RuntimeInputStatus::InputClosed),
    ] {
        let receipt: RuntimeInputReceipt = serde_json::from_slice(
            &std::fs::read(receipts.join(format!("{}.json", command.command_id))).unwrap(),
        )
        .unwrap();
        assert_eq!(receipt.input_command_id, command.command_id);
        assert_eq!(receipt.run_id, command.run_id);
        assert_eq!(receipt.lease_fencing_token, 7);
        assert_eq!(receipt.environment_epoch, 3);
        assert_eq!(receipt.status, status as i32);
    }
    assert_eq!(state.prompts.lock().unwrap().len(), 2);
    assert_eq!(state.aborts.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn stop_during_unaccepted_http_turn_calls_abort_without_receipt() {
    let (client, state, server) = fixture().await;
    let session = client.create_session().await.unwrap();
    let (_root, mut mailbox, _inputs, receipts) = setup();
    let (stop, rx) = watch::channel(false);
    let task = tokio::spawn(async move {
        client
            .run_live_session(
                &session,
                "model",
                SecretBytes::new(b"prompt".to_vec()),
                rx,
                &mut mailbox,
                |_| Ok(()),
            )
            .await
    });
    wait_prompt(&state, 1).await;
    stop.send(true).unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        SessionResult::Aborted
    );
    assert_eq!(state.aborts.load(Ordering::SeqCst), 1);
    assert_eq!(std::fs::read_dir(receipts).unwrap().count(), 0);
    server.abort();
}

#[tokio::test]
async fn wrong_exact_message_lookup_cannot_prove_http_acceptance() {
    let (client, state, server) = fixture().await;
    let session = client.create_session().await.unwrap();
    let (_root, mut mailbox, _inputs, receipts) = setup();
    let (_stop, rx) = watch::channel(false);
    let task = tokio::spawn(async move {
        client
            .run_live_session(
                &session,
                "model",
                SecretBytes::new(b"prompt".to_vec()),
                rx,
                &mut mailbox,
                |_| Ok(()),
            )
            .await
    });
    let first = wait_prompt(&state, 1).await;
    finish(
        &state,
        first["messageID"].as_str().unwrap(),
        "msg_assistant",
    )
    .await;
    assert!(
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert_eq!(std::fs::read_dir(receipts).unwrap().count(), 0);
    server.abort();
}
