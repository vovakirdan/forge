//! Message identity is model-facing; delivery identity belongs to the transport.

use forge_protocol::supervisor::v1::{
    CloseRuntimeInput, DeliverRuntimeInput, RuntimeMessageInput, deliver_runtime_input::Action,
};
use forge_provider_common::native_input::{MAX_INPUT_BYTES, addressed_prompt};
use serde_json::json;
use uuid::Uuid;

fn delivery(source: String) -> DeliverRuntimeInput {
    DeliverRuntimeInput {
        command_id: Uuid::now_v7().to_string(),
        run_id: Uuid::now_v7().to_string(),
        lease_fencing_token: 1,
        environment_epoch: 1,
        sequence: 1,
        action: Some(Action::Message(RuntimeMessageInput {
            source_message_json: source,
        })),
    }
}

#[test]
fn addressed_prompt_uses_canonical_message_id_and_preserves_source_json() {
    let message_id = Uuid::now_v7();
    let source = json!({
        "id": message_id,
        "thread_id": Uuid::now_v7(),
        "kind": "instruction",
        "body": "Reply READY. Не редактируй файлы до GO."
    })
    .to_string();
    let source = format!(" \n{source}");
    let input = delivery(source.clone());
    let prompt = addressed_prompt(&input).unwrap();
    let text = std::str::from_utf8(prompt.expose()).unwrap();

    assert!(text.starts_with(&format!("Forge addressed instruction {message_id}.\n")));
    assert!(text.ends_with(&source));
    assert!(!text.contains(&input.command_id));
}

#[test]
fn redelivery_keeps_message_identity_and_go_uses_its_own_identity() {
    let ready_id = Uuid::now_v7();
    let ready = json!({"id": ready_id, "body": "Reply READY, then wait for GO."}).to_string();
    let first = delivery(ready.clone());
    let mut redelivery = first.clone();
    redelivery.command_id = Uuid::now_v7().to_string();
    redelivery.sequence = 2;
    let go_id = Uuid::now_v7();
    let mut go = delivery(json!({"id": go_id, "body": "GO"}).to_string());
    go.sequence = 3;

    let initial_prompt = addressed_prompt(&first).unwrap();
    let repeated_prompt = addressed_prompt(&redelivery).unwrap();
    assert_eq!(initial_prompt.expose(), repeated_prompt.expose());
    let go_prompt = addressed_prompt(&go).unwrap();
    let go_text = std::str::from_utf8(go_prompt.expose()).unwrap();
    assert!(go_text.starts_with(&format!("Forge addressed instruction {go_id}.\n")));
    assert!(!go_text.contains(&ready_id.to_string()));
}

#[test]
fn missing_or_invalid_canonical_id_never_falls_back_to_delivery_id() {
    for source in [
        "",
        "not json",
        "null",
        "[]",
        "[\"01900000-0000-7000-8000-000000000001\"]",
        "{}",
        "{\"id\":null}",
        "{\"id\":123}",
        "{\"id\":\"\"}",
        "{\"id\":\"SYNTHETIC_PRIVATE_INVALID_ID\"}",
        "{\"id\":\"00000000-0000-0000-0000-000000000000\"}",
        "{\"id\":\"01900000-0000-4000-8000-000000000001\"}",
        "{\"id\":\"01900000-0000-7000-8000-000000000001\",\"id\":\"01900000-0000-7000-8000-000000000002\"}",
    ] {
        let error = addressed_prompt(&delivery(source.into())).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert!(!error.to_string().contains("SYNTHETIC_PRIVATE"));
    }
}

#[test]
fn oversized_canonical_source_remains_rejected() {
    let source =
        json!({"id": Uuid::now_v7(), "body": "x".repeat(MAX_INPUT_BYTES as usize)}).to_string();
    assert!(addressed_prompt(&delivery(source)).is_err());
}

#[test]
fn close_control_cannot_be_rendered_as_an_instruction() {
    let mut input = delivery("{}".into());
    input.action = Some(Action::CloseAfterTurn(CloseRuntimeInput {
        reason_code: "assignment_completed".into(),
    }));
    assert!(addressed_prompt(&input).is_err());
    input.action = None;
    assert!(addressed_prompt(&input).is_err());
}
