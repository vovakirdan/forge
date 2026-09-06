use std::{
    convert::Infallible,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::sse::{Event, Sse},
    routing::{get, post},
};
use forge_provider_common::{SecretBytes, adapter::RuntimeObservation};
use forge_provider_opencode::{OpenCodeClient, OpenCodeError, SessionResult};
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

type EventSender = mpsc::Sender<Result<Event, Infallible>>;

#[derive(Clone)]
struct Fixture {
    stream: Arc<Mutex<Option<EventSender>>>,
    mode: &'static str,
    prompts: Arc<Mutex<Vec<Value>>>,
    aborts: Arc<AtomicUsize>,
}
async fn fixture(mode: &'static str) -> (OpenCodeClient, Fixture, tokio::task::JoinHandle<()>) {
    let state = Fixture {
        stream: Arc::new(Mutex::new(None)),
        mode,
        prompts: Arc::new(Mutex::new(Vec::new())),
        aborts: Arc::new(AtomicUsize::new(0)),
    };
    let router = Router::new()
        .route(
            "/global/health",
            get(|| async { Json(json!({"healthy":true,"version":"1.18.29"})) }),
        )
        .route("/session", post(create))
        .route("/event", get(events))
        .route("/session/ses_test/prompt_async", post(prompt))
        .route("/session/ses_test/abort", post(abort))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = OpenCodeClient::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        "/workspace/task",
    )
    .unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (client, state, server)
}
async fn create(State(state): State<Fixture>) -> Json<Value> {
    Json(json!({"id":if state.mode=="bad_id"{"ses_../../other"}else{"ses_test"}}))
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
    assert!(
        state.stream.lock().unwrap().is_some(),
        "subscription must precede prompt"
    );
    state.prompts.lock().unwrap().push(body);
    let tx = state.stream.lock().unwrap().as_ref().unwrap().clone();
    if state.mode == "hang" {
        return StatusCode::NO_CONTENT;
    }
    if state.mode == "eof" {
        *state.stream.lock().unwrap() = None;
        return StatusCode::NO_CONTENT;
    }
    if state.mode == "oversized" {
        tx.send(Ok(Event::default().data("x".repeat(1024 * 1024 + 1))))
            .await
            .unwrap();
        return StatusCode::NO_CONTENT;
    }
    let events = if state.mode == "auth" {
        vec![
            json!({"type":"session.error","properties":{"sessionID":"ses_test","error":{"name":"ProviderAuthError","data":{"message":"secret-upstream-key"}}}}),
        ]
    } else {
        vec![
            json!({"type":"session.idle","properties":{"sessionID":"ses_test"}}),
            json!({"type":"session.status","properties":{"sessionID":"ses_test","status":{"type":"busy"}}}),
            json!({"type":"message.updated","properties":{"info":{"sessionID":"ses_test","id":"msg_a","role":"assistant","time":{}}}}),
            json!({"type":"message.part.updated","properties":{"part":{"sessionID":"ses_test","messageID":"msg_a","id":"part_r","type":"reasoning","text":"hidden-private-reasoning"}}}),
            json!({"type":"message.part.updated","properties":{"part":{"sessionID":"ses_test","messageID":"msg_a","id":"part_a","type":"text","text":"visible assistant output","time":{"end":1}}}}),
            json!({"type":"message.updated","properties":{"info":{"sessionID":"ses_test","id":"msg_a","role":"assistant","time":{"completed":1},"tokens":{"input":7,"output":2,"cache":{"read":3,"write":0},"reasoning":0}}}}),
            json!({"type":"session.status","properties":{"sessionID":"ses_test","status":{"type":"idle"}}}),
        ]
    };
    for event in events {
        tx.send(Ok(Event::default().data(event.to_string())))
            .await
            .unwrap();
    }
    StatusCode::NO_CONTENT
}
async fn abort(State(state): State<Fixture>) -> Json<bool> {
    state.aborts.fetch_add(1, Ordering::SeqCst);
    Json(true)
}

#[tokio::test]
async fn real_http_client_uses_session_sse_prompt_and_allowlisted_observations() {
    let (client, state, server) = fixture("normal").await;
    assert!(client.healthy().await.unwrap());
    let session = client.create_session().await.unwrap();
    let (_stop, rx) = watch::channel(false);
    let mut observations = Vec::new();
    let result = client
        .run_session(
            &session,
            "explicit-model",
            &SecretBytes::new(b"private prompt".to_vec()),
            rx,
            |event| {
                observations.push(event);
                Ok(())
            },
        )
        .await
        .unwrap();
    assert_eq!(result, SessionResult::TurnEnded);
    let prompts = state.prompts.lock().unwrap();
    assert_eq!(
        prompts[0]["model"],
        json!({"providerID":"forge","modelID":"explicit-model"})
    );
    assert_eq!(prompts[0]["parts"][0]["text"], "private prompt");
    assert!(!format!("{observations:?}").contains("hidden-private"));
    assert!(observations.iter().any(|event|matches!(event,RuntimeObservation::TurnCompleted{usage:Some(usage)} if usage.input_tokens==10 && usage.output_tokens==2)));
    assert_eq!(state.aborts.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn stop_calls_abort_api_and_does_not_claim_completion() {
    let (client, state, server) = fixture("hang").await;
    let session = client.create_session().await.unwrap();
    let (stop, rx) = watch::channel(false);
    let checking = state.clone();
    let signal = tokio::spawn(async move {
        for _ in 0..100 {
            if !checking.prompts.lock().unwrap().is_empty() {
                stop.send(true).unwrap();
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("prompt did not start");
    });
    let mut completed = false;
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        client.run_session(
            &session,
            "explicit-model",
            &SecretBytes::new(b"prompt".to_vec()),
            rx,
            |event| {
                completed |= matches!(event, RuntimeObservation::TurnCompleted { .. });
                Ok(())
            },
        ),
    )
    .await
    .unwrap()
    .unwrap();
    signal.await.unwrap();
    assert_eq!(result, SessionResult::Aborted);
    assert!(!completed);
    assert_eq!(state.aborts.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn errors_eof_oversized_frames_and_untrusted_session_ids_fail_closed() {
    for mode in ["auth", "eof", "oversized", "bad_id"] {
        let (client, state, server) = fixture(mode).await;
        let session = client.create_session().await;
        if mode == "bad_id" {
            assert!(matches!(session, Err(OpenCodeError::InvalidResponse)));
            server.abort();
            continue;
        }
        let session = session.unwrap();
        let (_stop, rx) = watch::channel(false);
        let mut complete = false;
        let result = client
            .run_session(
                &session,
                "explicit-model",
                &SecretBytes::new(b"prompt".to_vec()),
                rx,
                |event| {
                    complete |= matches!(event, RuntimeObservation::TurnCompleted { .. });
                    Ok(())
                },
            )
            .await;
        match mode {
            "auth" => {
                assert_eq!(result.unwrap(), SessionResult::ProviderFailed);
                assert_eq!(state.aborts.load(Ordering::SeqCst), 1);
            }
            "eof" => assert!(matches!(result, Err(OpenCodeError::UnexpectedEof))),
            _ => assert!(result.is_err()),
        }
        assert!(!complete);
        server.abort();
    }
}

#[tokio::test]
async fn stop_before_submission_never_sends_prompt() {
    let (client, state, server) = fixture("normal").await;
    let session = client.create_session().await.unwrap();
    let (_stop, rx) = watch::channel(true);
    assert_eq!(
        client
            .run_session(
                &session,
                "explicit-model",
                &SecretBytes::new(b"prompt".to_vec()),
                rx,
                |_| Ok(())
            )
            .await
            .unwrap(),
        SessionResult::Aborted
    );
    assert!(state.prompts.lock().unwrap().is_empty());
    server.abort();
}
