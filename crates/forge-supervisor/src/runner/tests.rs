use super::*;

#[test]
fn new_private_credential_parents_are_owner_only() {
    use std::os::unix::fs::MetadataExt;
    let root = std::env::temp_dir().join(format!("forge-private-parent-{}", crate::new_id()));
    create_parent(&root.join("claude-home/setup-token")).unwrap();
    assert_eq!(
        fs::metadata(root.join("claude-home")).unwrap().mode() & 0o777,
        0o700
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn claude_invocation_requires_exact_non_writeback_subscription_delivery() {
    let mut invocation = RunnerInvocation {
        runtime_input: None,
        adapter_id: "claude_code_cli".into(),
        program: "forge-claude-driver".into(),
        args: vec![],
        env: Default::default(),
        managed_files: vec![],
        credential_files: vec![forge_protocol::runtime::RunnerCredentialFile {
            source: "/run/forge-secrets/claude-setup-token".into(),
            target: "/run/forge/claude-home/setup-token".into(),
            writeback: false,
        }],
        max_output_bytes: 4096,
        stop_grace_seconds: 2,
    };
    assert!(validate(&invocation).is_ok());
    invocation.credential_files[0].writeback = true;
    assert!(validate(&invocation).is_err());
    invocation.credential_files[0].writeback = false;
    invocation.credential_files[0].source = "/run/forge-secrets/api-key".into();
    assert!(validate(&invocation).is_err());
    invocation.credential_files.clear();
    assert!(validate(&invocation).is_err());
}
#[test]
fn secret_crossing_read_chunks_is_redacted_as_one_complete_line() {
    let token = b"synthetic-refresh-secret".to_vec();
    assert_eq!(
        redact(b"before synthetic-refresh-secret after", &[token]),
        b"before [REDACTED] after"
    );
}
#[test]
fn reasoning_items_are_not_persisted_as_raw_fallback() {
    assert!(contains_reasoning(
        &serde_json::json!({"type":"item.completed","item":{"type":"reasoning","text":"private"}})
    ));
    assert!(!contains_reasoning(
        &serde_json::json!({"type":"turn.completed","usage":{"reasoning_output_tokens":12}})
    ));
}
#[test]
fn runtime_files_cannot_escape_private_mount() {
    assert!(private_path("../auth.json", false).is_err());
    assert!(private_path("/workspace/auth.json", true).is_err());
}

#[test]
fn rotated_opaque_credentials_are_redacted_before_raw_persistence() {
    let root = std::env::temp_dir().join(format!("forge-redactor-{}", crate::new_id()));
    crate::surface::private_directory(&root).unwrap();
    let auth = root.join("auth.json");
    write_private(
        &auth,
        br#"{"tokens":{"refresh_token":"opaque-new-refresh"}}"#,
    )
    .unwrap();
    let policy = RedactionPolicy {
        initial: vec![b"opaque-old-refresh".to_vec()],
        credential_paths: vec![auth],
        codex_errors: true,
    };
    assert_eq!(
        redact(b"opaque-new-refresh", &policy.current_secrets().unwrap()),
        b"[REDACTED]"
    );
    assert!(contains_credential_field(
        br#"{"item":{"aggregated_output":"{\"refresh_token\":\"new-before-writeback\"}"}}"#
    ));
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn captured_codex_failures_keep_their_kind_without_raw_error_secrets() {
    use forge_provider_common::adapter::{RuntimeFailureKind, RuntimeObservation};
    let root = std::env::temp_dir().join(format!("forge-failure-capture-{}", crate::new_id()));
    crate::surface::private_directory(&root).unwrap();
    for (index, (message, expected)) in [
        (
            "refresh_token_reused",
            RuntimeFailureKind::AuthRefreshReused,
        ),
        ("refresh_token_expired", RuntimeFailureKind::AuthExpired),
        (
            "refresh_token_invalidated",
            RuntimeFailureKind::AuthInvalidated,
        ),
        (
            "Authentication required 401",
            RuntimeFailureKind::AuthRequired,
        ),
        ("Usage limit reached 429", RuntimeFailureKind::RateLimited),
        (
            "Connection failed 503",
            RuntimeFailureKind::ProviderUnavailable,
        ),
        (
            "unexpected provider failure",
            RuntimeFailureKind::RuntimeError,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let message = format!("{message} opaque-new-refresh access_token=synthetic-secret");
        let mut input = [
            serde_json::json!({"type":"error","message":message}),
            serde_json::json!({"type":"turn.failed","error":{"message":message}}),
            serde_json::json!({"type":"item.completed","item":{"type":"error","message":message}}),
            serde_json::json!({"tokens":{"refresh_token":"opaque-new-refresh"}}),
        ]
        .map(|value| value.to_string())
        .join("\n");
        input.push('\n');
        let path = root.join(index.to_string());
        let budget = Arc::new(OutputBudget {
            remaining: AtomicU64::new(4096),
            incomplete: AtomicBool::new(false),
            failed: Notify::new(),
        });
        capture(
            input.as_bytes(),
            path.to_str().unwrap().into(),
            Arc::clone(&budget),
            RedactionPolicy {
                initial: vec![b"synthetic-secret".to_vec()],
                credential_paths: Vec::new(),
                codex_errors: true,
            },
        )
        .await
        .unwrap();
        let persisted = fs::read_to_string(&path).unwrap();
        assert_eq!(persisted.lines().count(), 3);
        assert!(
            !persisted.contains("synthetic-secret") && !persisted.contains("opaque-new-refresh")
        );
        assert!(!budget.incomplete.load(Ordering::Acquire));
        for line in persisted.lines() {
            assert!(
                matches!(forge_provider_codex::parse_jsonl_event(line).unwrap(), RuntimeObservation::Failure { kind } if kind == expected)
            );
        }
    }
    fs::remove_dir_all(root).unwrap();
}
