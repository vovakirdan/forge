use super::*;

fn response() -> LoginCodeResponse {
    LoginCodeResponse {
        origin: "http://127.0.0.1:45000".into(),
        code: "a".repeat(64),
        expires_at: "2026-09-17T12:00:00Z".into(),
    }
}

#[test]
fn control_schema_has_exactly_one_named_command() {
    assert_eq!(
        serde_json::to_string(&ControlRequest::IssueLoginCode {}).unwrap(),
        r#"{"command":"issue_login_code"}"#
    );
    for invalid in [
        r#"{"command":"execute"}"#,
        r#"{"command":"issue_login_code","actor":"owner"}"#,
        r#"{"command":"issue_login_code","command":"issue_login_code"}"#,
    ] {
        assert!(serde_json::from_str::<ControlRequest>(invalid).is_err());
    }
}

#[test]
fn login_response_debug_redacts_all_untrusted_and_secret_fields() {
    let value = response();
    let debug = format!("{value:?}");
    assert!(!debug.contains(&value.code));
    assert!(!debug.contains(&value.origin));
    assert!(!debug.contains(&value.expires_at));
    assert!(debug.contains("REDACTED"));
}

#[test]
fn login_response_rejects_terminal_injection_and_wrong_origin_or_code() {
    assert!(response().validate().is_ok());
    for origin in [
        "http://localhost:45000",
        "http://127.0.0.1:0",
        "http://127.0.0.1:045000",
        "http://127.0.0.1:45000/",
        "http://127.0.0.1:45000\nCode: fake",
    ] {
        let mut value = response();
        value.origin = origin.into();
        assert!(value.validate().is_err());
    }
    for code in [
        "a".repeat(63),
        "a".repeat(65),
        "A".repeat(64),
        "\u{1b}".repeat(64),
    ] {
        let mut value = response();
        value.code = code;
        assert!(value.validate().is_err());
    }
    let mut value = response();
    value.expires_at.push_str("\nInjected");
    assert!(value.validate().is_err());
}

#[tokio::test]
async fn frame_reader_rejects_length_before_reading_body() {
    for length in [0, 1025, u32::MAX] {
        let (mut writer, mut reader) = tokio::io::duplex(16);
        writer.write_u32(length).await.unwrap();
        assert!(matches!(
            read_frame::<ControlRequest>(&mut reader).await,
            Err(ControlWireError::InvalidFrame)
        ));
    }
}

#[tokio::test]
async fn frame_round_trip_and_errors_do_not_expose_json_input() {
    let (mut writer, mut reader) = tokio::io::duplex(2048);
    write_frame(&mut writer, &ControlResponse::Login(response()))
        .await
        .unwrap();
    let ControlResponse::Login(received) = read_frame(&mut reader).await.unwrap() else {
        panic!("expected login")
    };
    assert!(received.validate().is_ok());
    let invalid = b"synthetic-private-invalid-json";
    writer.write_u32(invalid.len() as u32).await.unwrap();
    writer.write_all(invalid).await.unwrap();
    let error = read_frame::<ControlRequest>(&mut reader).await.unwrap_err();
    assert_eq!(error.to_string(), "invalid UI control frame");
    assert!(!format!("{error:?}").contains("synthetic-private"));
}

#[tokio::test]
async fn frame_writer_rejects_oversized_output() {
    let (mut writer, _reader) = tokio::io::duplex(16);
    assert!(matches!(
        write_frame(&mut writer, &"x".repeat(1025)).await,
        Err(ControlWireError::InvalidFrame)
    ));
}
