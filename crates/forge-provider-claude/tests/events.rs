use forge_provider_claude::ClaudeTurnEnd;
use forge_provider_claude::{
    MAX_EVENT_BYTES, encode_user_message, parse_jsonl_event, validate_setup_token,
};
use forge_provider_common::{
    SecretBytes,
    adapter::{
        AdapterError, RuntimeActivityPhase, RuntimeFailureKind, RuntimeObservation, RuntimeToolKind,
    },
};
use serde_json::json;
use uuid::Uuid;

const SESSION: &str = "018fa934-3c88-7000-8000-000000000001";

#[test]
fn failed_and_interrupted_results_keep_usage_without_claiming_completion() {
    for reason in ["aborted_streaming", "aborted_tools", "max_turns"] {
        let mut line = json!({"type":"result","subtype":"success","is_error":false,"terminal_reason":reason,"session_id":SESSION,"usage":{"input_tokens":7,"output_tokens":3,"cache_read_input_tokens":2}});
        let parsed = parse_jsonl_event(&line.to_string()).unwrap();
        assert_eq!(parsed.usage.unwrap().input_tokens, 9);
        assert!(
            !parsed
                .observations
                .iter()
                .any(|item| matches!(item, RuntimeObservation::TurnCompleted { .. }))
        );
        line["usage"]["input_tokens"] = json!(u64::MAX);
        assert!(matches!(
            parse_jsonl_event(&line.to_string()),
            Err(AdapterError::InvalidUsage)
        ));
    }
}

#[test]
fn assistant_api_error_flags_are_classified_before_provider_text() {
    for (status, expected) in [
        (401, RuntimeFailureKind::AuthRequired),
        (429, RuntimeFailureKind::RateLimited),
        (529, RuntimeFailureKind::ProviderUnavailable),
    ] {
        let line = json!({"type":"assistant","session_id":SESSION,"is_api_error_message":true,"api_error_status":status,"message":{"role":"assistant","content":[{"type":"text","text":"private provider error"}]}});
        let event = parse_jsonl_event(&line.to_string()).unwrap();
        assert!(
            matches!(event.observations.as_slice(), [RuntimeObservation::Failure { kind }] if *kind == expected)
        );
        assert!(!format!("{event:?}").contains("private provider error"));
    }
}

#[test]
fn aborted_and_budget_ended_loops_are_not_completed_turns() {
    for (reason, expected) in [
        ("aborted_streaming", ClaudeTurnEnd::Interrupted),
        ("aborted_tools", ClaudeTurnEnd::Interrupted),
        ("max_turns", ClaudeTurnEnd::Failed),
    ] {
        let line = json!({"type":"result","subtype":"success","is_error":false,"terminal_reason":reason,"session_id":SESSION});
        let parsed = parse_jsonl_event(&line.to_string()).unwrap();
        assert_eq!(parsed.turn_end, Some(expected));
        assert!(
            !parsed
                .observations
                .iter()
                .any(|value| matches!(value, RuntimeObservation::TurnCompleted { .. }))
        );
    }
}

#[test]
fn conversation_reset_and_injected_turn_results_fail_closed() {
    for line in [
        json!({"type":"conversation_reset","session_id":SESSION}),
        json!({"type":"result","subtype":"success","is_error":false,"session_id":SESSION,"origin":{"kind":"task-notification"}}),
    ] {
        assert!(matches!(
            parse_jsonl_event(&line.to_string()),
            Err(AdapterError::InvalidEvent)
        ));
    }
}

#[test]
fn pinned_synthetic_fixture_normalizes_without_promoting_task_outcomes() {
    let events: Vec<_> = include_str!("fixtures/2.1.263.jsonl")
        .lines()
        .map(|line| parse_jsonl_event(line).unwrap())
        .collect();
    assert!(matches!(
        events[0].observations.as_slice(),
        [RuntimeObservation::ThreadStarted]
    ));
    assert_eq!(
        events[1].replayed_message_id.unwrap().to_string(),
        "018fa934-3c88-7000-8000-000000000011"
    );
    assert_eq!(events[2].observations.len(), 3);
    assert!(matches!(
        events[2].observations[1],
        RuntimeObservation::ToolActivity {
            kind: RuntimeToolKind::Shell,
            phase: RuntimeActivityPhase::Started,
            succeeded: None
        }
    ));
    assert!(matches!(
        events[2].observations[2],
        RuntimeObservation::ToolActivity {
            kind: RuntimeToolKind::Mcp,
            ..
        }
    ));
    let RuntimeObservation::TurnCompleted {
        usage: Some(ref usage),
    } = events[3].observations[0]
    else {
        panic!("turn usage expected");
    };
    assert_eq!(usage.input_tokens, 70);
    assert_eq!(usage.cached_input_tokens, Some(40));
    assert_eq!(usage.cache_write_input_tokens, Some(20));
    assert_eq!(usage.output_tokens, 5);
    assert_eq!(usage.reasoning_output_tokens, None);
    let debug = format!("{events:?}");
    for private in [
        "synthetic private",
        "synthetic hidden",
        "synthetic-secret",
        "synthetic duplicate",
    ] {
        assert!(!debug.contains(private));
    }
}

#[test]
fn additional_input_receipt_correlates_uuid_but_never_implies_employee_ack() {
    let session = Uuid::now_v7();
    let message = Uuid::now_v7();
    let encoded = encode_user_message(
        session,
        message,
        &SecretBytes::new(b"private\n\"text\"".to_vec()),
    )
    .unwrap();
    assert_eq!(
        encoded
            .expose()
            .iter()
            .filter(|byte| **byte == b'\n')
            .count(),
        1
    );
    let receipt = parse_jsonl_event(std::str::from_utf8(encoded.expose()).unwrap()).unwrap();
    assert_eq!(receipt.session_id, Some(session));
    assert_eq!(receipt.replayed_message_id, Some(message));
    assert!(receipt.observations.is_empty());
}

#[test]
fn tool_result_and_injected_user_messages_are_not_input_receipts() {
    for content in [
        json!([{"type":"tool_result","tool_use_id":"toolu_test","content":"private output"}]),
        json!([]),
    ] {
        let line = json!({"type":"user", "session_id": SESSION, "uuid": Uuid::now_v7(), "message":{"role":"user","content":content}});
        assert!(
            parse_jsonl_event(&line.to_string())
                .unwrap()
                .replayed_message_id
                .is_none()
        );
    }
    for context in [
        json!({"parent_tool_use_id":"toolu_test"}),
        json!({"origin":{"kind":"task-notification"}}),
    ] {
        let mut line = json!({"type":"user","session_id":SESSION,"uuid":Uuid::now_v7(),"message":{"role":"user","content":"private"}});
        line.as_object_mut()
            .unwrap()
            .extend(context.as_object().unwrap().clone());
        assert!(
            parse_jsonl_event(&line.to_string())
                .unwrap()
                .replayed_message_id
                .is_none()
        );
    }
}

#[test]
fn failed_result_with_success_subtype_does_not_become_turn_completed() {
    for (status, expected) in [
        (401, RuntimeFailureKind::AuthRequired),
        (429, RuntimeFailureKind::RateLimited),
        (529, RuntimeFailureKind::ProviderUnavailable),
        (400, RuntimeFailureKind::RuntimeError),
    ] {
        let line = json!({"type":"result","subtype":"success","is_error":true,"api_error_status":status,"session_id":SESSION,"errors":["secret token and URL"],"result":"private body"});
        let parsed = parse_jsonl_event(&line.to_string()).unwrap();
        assert!(
            matches!(parsed.observations[0], RuntimeObservation::Failure { kind } if kind == expected)
        );
        assert!(!format!("{parsed:?}").contains("secret"));
    }
}

#[test]
fn partial_thinking_future_events_and_duplicate_result_text_are_not_output() {
    for line in [
        json!({"type":"future","text":"secret"}),
        json!({"type":"stream_event","event":{"delta":{"thinking":"secret"}}}),
        json!({"type":"assistant","session_id":SESSION,"message":{"role":"assistant","content":[{"type":"thinking","thinking":"secret"},{"type":"future","text":"secret"}]}}),
    ] {
        let parsed = parse_jsonl_event(&line.to_string()).unwrap();
        assert!(matches!(
            parsed.observations.as_slice(),
            [RuntimeObservation::Ignored]
        ));
    }
}

#[test]
fn unknown_usage_stays_unknown_and_invalid_counters_are_rejected() {
    let base = json!({"type":"result","subtype":"success","is_error":false,"session_id":SESSION});
    assert!(matches!(
        parse_jsonl_event(&base.to_string()).unwrap().observations[0],
        RuntimeObservation::TurnCompleted { usage: None }
    ));
    for usage in [
        json!({"input_tokens":-1,"output_tokens":2}),
        json!({"input_tokens":1.5,"output_tokens":2}),
        json!({"input_tokens":u64::MAX,"output_tokens":2,"cache_read_input_tokens":1}),
        json!({"output_tokens":2}),
    ] {
        let mut line = base.clone();
        line["usage"] = usage;
        assert!(matches!(
            parse_jsonl_event(&line.to_string()),
            Err(AdapterError::InvalidUsage)
        ));
    }
}

#[test]
fn malformed_missing_session_or_oversized_events_fail_without_echoing_input() {
    for line in [
        "secret malformed".into(),
        r#"{"type":"result","subtype":"success","is_error":false}"#.into(),
        json!({"type":"system","subtype":"init","session_id":Uuid::nil()}).to_string(),
        json!({"type":"result","session_id":SESSION,"subtype":"success","is_error":"false"})
            .to_string(),
        "x".repeat(MAX_EVENT_BYTES + 1),
    ] {
        assert!(matches!(
            parse_jsonl_event(&line),
            Err(AdapterError::InvalidEvent)
        ));
    }
}

#[test]
fn input_uuid_utf8_and_post_escaping_size_are_validated() {
    let session = Uuid::now_v7();
    let message = Uuid::now_v7();
    for text in [
        Vec::new(),
        b" \n ".to_vec(),
        vec![0xff],
        vec![b'"'; MAX_EVENT_BYTES / 2 + 1],
    ] {
        assert!(matches!(
            encode_user_message(session, message, &SecretBytes::new(text)),
            Err(AdapterError::InvalidEvent)
        ));
    }
    assert!(encode_user_message(Uuid::nil(), message, &SecretBytes::new(b"hi".to_vec())).is_err());
    assert!(encode_user_message(session, Uuid::nil(), &SecretBytes::new(b"hi".to_vec())).is_err());
}

#[test]
fn token_shape_rejects_api_keys_whitespace_and_oversized_material() {
    for token in [
        "sk-ant-api03-synthetic".into(),
        "sk-ant-oat01-".into(),
        "sk-ant-oat01-has whitespace".into(),
        "sk-ant-oat01-line\ninjection".into(),
        format!("sk-ant-oat01-{}", "a".repeat(16 * 1024)),
    ] {
        assert!(validate_setup_token(&SecretBytes::new(token.into_bytes())).is_err());
    }
    let token = SecretBytes::new(b"sk-ant-oat01-synthetic-token\n".to_vec());
    assert_eq!(
        validate_setup_token(&token).unwrap(),
        "sk-ant-oat01-synthetic-token"
    );
}
