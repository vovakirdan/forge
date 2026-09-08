use std::collections::BTreeSet;

use forge_domain::{
    CredentialBinding, CredentialDeliveryMode, ExecutionProfile, ExecutionProfileInput, ProjectId,
    RuntimeCapability,
};
use forge_provider_codex::{
    CODEX_HOME, CodexAdapter, CodexRunInput, PINNED_CODEX_VERSION, parse_jsonl_event,
};
use forge_provider_common::{
    SecretBytes,
    adapter::{
        AdapterError, RuntimeActivityPhase, RuntimeFailureKind, RuntimeObservation,
        RuntimeToolKind, StopStrategy,
    },
};

fn profile() -> ExecutionProfile {
    let project_id = ProjectId::new();
    ExecutionProfile::try_from(ExecutionProfileInput {
        id: ProjectId::new().as_uuid(),
        revision: 1,
        project_id,
        adapter_id: "codex_cli".into(),
        adapter_version: PINNED_CODEX_VERSION.into(),
        provider_id: "openai".into(),
        model: "explicit-test-model".into(),
        credential_binding: CredentialBinding {
            id: ProjectId::new().as_uuid(),
            project_id,
            secret_id: ProjectId::new().as_uuid(),
            account_id: Some("synthetic-account".into()),
            allowed_delivery_modes: BTreeSet::from([CredentialDeliveryMode::IsolatedRuntimeSecret]),
        },
        credential_delivery: CredentialDeliveryMode::IsolatedRuntimeSecret,
        capability_profile: CodexAdapter::capabilities(),
    })
    .unwrap()
}

fn input(profile: &ExecutionProfile) -> CodexRunInput<'_> {
    CodexRunInput {
        profile,
        prompt: SecretBytes::new(b"private task context".to_vec()),
        workdir: "/workspace/task",
        gateway_url: "http://127.0.0.1:4097/mcp",
        proxy_url: "http://127.0.0.1:4098",
    }
}

#[test]
fn preflight_requires_exact_pinned_version_and_never_assumes_resume() {
    let profile = profile();
    assert!(CodexAdapter::preflight("codex-cli 0.153.2\n", &profile).is_ok());
    assert_eq!(
        CodexAdapter::preflight("codex-cli 0.153.3", &profile),
        Err(AdapterError::VersionMismatch)
    );
    assert_eq!(
        CodexAdapter::preflight("codex-cli 0.153.2-modified", &profile),
        Err(AdapterError::VersionMismatch)
    );
    assert!(
        !CodexAdapter::capabilities()
            .capabilities
            .contains(&RuntimeCapability::SessionResume)
    );
}

#[test]
fn explicit_live_input_uses_pinned_app_server_without_exec_only_options() {
    let mut value: ExecutionProfileInput = profile().into();
    value.capability_profile = CodexAdapter::live_capabilities();
    let profile = ExecutionProfile::try_from(value).unwrap();
    let prepared = CodexAdapter::prepare(input(&profile)).unwrap();
    assert_eq!(prepared.program, "forge-codex-driver");
    assert_eq!(
        &prepared.args[..4],
        ["app-server", "--listen", "stdio://", "--strict-config"]
    );
    assert!(!prepared.args.iter().any(|arg| matches!(
        arg.as_str(),
        "--ignore-user-config" | "--ignore-rules" | "exec" | "-"
    )));
    assert!(
        CodexAdapter::preflight("codex-cli 0.153.2", &profile)
            .unwrap()
            .capabilities
            .contains(&RuntimeCapability::LiveInput)
    );
    assert!(
        !CodexAdapter::capabilities()
            .capabilities
            .contains(&RuntimeCapability::LiveInput)
    );
    assert_eq!(prepared.env["FORGE_CODEX_WORKDIR"], "/workspace/task");
    assert_eq!(prepared.credential_files.len(), 1);
}

#[test]
fn domain_valid_native_runtime_binding_can_be_prepared_without_proxy_auth() {
    let profile = profile();
    let binding = forge_domain::runtime::RuntimeBinding {
        budget: Default::default(),
        execution_profile: profile.clone(),
        image: format!("forge/runner@sha256:{}", "a".repeat(64)),
        surface: forge_domain::runtime::SurfaceSpec::FilesystemSandbox,
        access: Default::default(),
        limits: Default::default(),
        system_prompt: "common rules".into(),
        employee_prompt: "worker role".into(),
    };
    binding.validate().unwrap();
    let invocation = CodexAdapter::prepare(input(&binding.execution_profile)).unwrap();
    assert!(
        !profile
            .capability_profile()
            .capabilities
            .contains(&RuntimeCapability::GatewayAuth)
    );
    assert_eq!(
        invocation.credential_files[0].source,
        "/run/forge-secrets/auth.json"
    );
}

#[test]
fn unsupported_declared_capability_is_rejected_before_preparing_work() {
    let mut input_profile: ExecutionProfileInput = profile().into();
    input_profile
        .capability_profile
        .capabilities
        .insert(RuntimeCapability::SessionResume);
    let profile = ExecutionProfile::try_from(input_profile).unwrap();
    assert_eq!(
        CodexAdapter::preflight("codex-cli 0.153.2", &profile),
        Err(AdapterError::UnsupportedCapability)
    );
}

#[test]
fn prepared_invocation_keeps_prompt_off_arguments_and_uses_private_auth_copy() {
    let profile = profile();
    let prepared = CodexAdapter::prepare(input(&profile)).unwrap();
    assert_eq!(prepared.program, "codex");
    assert_eq!(prepared.stdin.expose(), b"private task context");
    assert!(!format!("{prepared:?}").contains("private task context"));
    assert!(
        prepared
            .args
            .windows(2)
            .any(|pair| pair == ["--model", "explicit-test-model"])
    );
    assert_eq!(prepared.args.last().unwrap(), "-");
    assert_eq!(prepared.env["CODEX_HOME"], CODEX_HOME);
    assert!(!prepared.env.contains_key("OPENAI_API_KEY"));
    assert!(!prepared.env.contains_key("DBUS_SESSION_BUS_ADDRESS"));
    assert_eq!(
        prepared.credential_files[0].source,
        "/run/forge-secrets/auth.json"
    );
    assert_eq!(
        prepared.credential_files[0].target,
        format!("{CODEX_HOME}/auth.json")
    );
    assert!(prepared.credential_files[0].writeback);
    assert_eq!(prepared.stop, StopStrategy::SigintThenEnvironmentKill);
}

#[test]
fn http_and_https_are_both_forced_through_relay_without_external_noproxy() {
    let profile = profile();
    let prepared = CodexAdapter::prepare(input(&profile)).unwrap();
    for key in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
        assert_eq!(prepared.env[key], "http://127.0.0.1:4098/");
    }
    for key in ["NO_PROXY", "no_proxy"] {
        assert_eq!(prepared.env[key], "127.0.0.1,localhost,::1");
    }
    assert!(
        prepared
            .args
            .iter()
            .any(|arg| arg == "features.respect_system_proxy=true")
    );
}

#[test]
fn managed_config_overrides_are_valid_toml_and_disable_unmanaged_context_sources() {
    let profile = profile();
    let mut input = input(&profile);
    input.workdir = "/workspace/project.with spaces";
    let prepared = CodexAdapter::prepare(input).unwrap();
    let settings: Vec<_> = prepared
        .args
        .windows(2)
        .filter(|pair| pair[0] == "-c")
        .map(|pair| &pair[1])
        .collect();
    for setting in &settings {
        let _: toml::Value = toml::from_str(setting).unwrap();
    }
    for required in [
        "project_doc_max_bytes=0",
        "features.plugins=false",
        "features.hooks=false",
        "features.skip_host_skill_discovery=true",
        "forced_login_method=\"chatgpt\"",
        "cli_auth_credentials_store=\"file\"",
        "mcp_servers.forge.required=true",
    ] {
        assert!(settings.iter().any(|setting| setting.as_str() == required));
    }
    assert!(settings.iter().any(|setting| setting.as_str()
        == "projects.\"/workspace/project.with spaces\".trust_level=\"untrusted\""));
    assert!(prepared.args.contains(&"--ignore-user-config".into()));
    assert!(prepared.args.contains(&"--ignore-rules".into()));
}

#[test]
fn remote_credential_urls_and_out_of_surface_workdirs_are_rejected() {
    let profile = profile();
    for invalid in [
        "http://remote.example:4097/mcp",
        "http://token@127.0.0.1:4097/mcp",
        "http://127.0.0.1:4097/mcp?token=secret",
        "https://127.0.0.1:4097/mcp",
    ] {
        let mut input = input(&profile);
        input.gateway_url = invalid;
        assert!(matches!(
            CodexAdapter::prepare(input),
            Err(AdapterError::InvalidEnvironment)
        ));
    }
    for invalid in [
        "/home/operator",
        "/workspace/../secrets",
        "relative",
        "/workspace/invalid\npath",
    ] {
        let mut input = input(&profile);
        input.workdir = invalid;
        assert!(matches!(
            CodexAdapter::prepare(input),
            Err(AdapterError::InvalidEnvironment)
        ));
    }
}

#[test]
fn reasoning_and_future_item_bodies_are_not_returned_as_fallback_output() {
    for line in [
        r#"{"type":"item.completed","item":{"type":"reasoning","text":"hidden reasoning text"}}"#,
        r#"{"type":"item.completed","item":{"type":"future_private_kind","text":"secret text"}}"#,
    ] {
        assert!(matches!(
            parse_jsonl_event(line).unwrap(),
            RuntimeObservation::Ignored
        ));
    }
}

#[test]
fn escaped_assistant_text_is_retained_only_in_redacted_output() {
    let event = parse_jsonl_event(r#"{"type":"item.completed","item":{"type":"agent_message","text":"Line one\nLine \"two\""}}"#).unwrap();
    assert!(!format!("{event:?}").contains("Line one"));
    let RuntimeObservation::AssistantOutput { text } = event else {
        panic!("assistant output expected")
    };
    assert_eq!(text.expose(), b"Line one\nLine \"two\"");
}

#[test]
fn shell_and_mcp_observations_do_not_expose_arguments_or_outputs() {
    let shell = parse_jsonl_event(r#"{"type":"item.completed","item":{"type":"command_execution","command":"echo private-token","aggregated_output":"private-token","status":"completed","exit_code":0}}"#).unwrap();
    assert!(matches!(
        shell,
        RuntimeObservation::ToolActivity {
            kind: RuntimeToolKind::Shell,
            phase: RuntimeActivityPhase::Completed,
            succeeded: Some(true)
        }
    ));
    assert!(!format!("{shell:?}").contains("private-token"));
    let mcp = parse_jsonl_event(r#"{"type":"item.started","item":{"type":"mcp_tool_call","server":"forge","tool":"task.read","arguments":{"token":"private-token"},"status":"in_progress"}}"#).unwrap();
    assert!(matches!(
        mcp,
        RuntimeObservation::ToolActivity {
            kind: RuntimeToolKind::Mcp,
            phase: RuntimeActivityPhase::Started,
            succeeded: None
        }
    ));
}

#[test]
fn completed_turn_has_usage_not_an_implicit_stage_outcome() {
    let completed = parse_jsonl_event(r#"{"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":8,"reasoning_output_tokens":3}}"#).unwrap();
    let RuntimeObservation::TurnCompleted { usage: Some(usage) } = completed else {
        panic!("turn usage expected")
    };
    assert_eq!(usage.input_tokens, 100);
    assert_eq!(usage.output_tokens, 8);
    assert_eq!(usage.cache_write_input_tokens, None);
    assert!(matches!(
        parse_jsonl_event(r#"{"type":"turn.completed"}"#).unwrap(),
        RuntimeObservation::TurnCompleted { usage: None }
    ));
}

#[test]
fn malformed_or_negative_usage_is_not_reported_as_zero() {
    assert!(
        parse_jsonl_event(
            r#"{"type":"turn.completed","usage":{"input_tokens":-1,"output_tokens":2}}"#
        )
        .is_err()
    );
    assert!(matches!(
        parse_jsonl_event(
            r#"{"type":"turn.completed","usage":{"input_tokens":1,"cached_input_tokens":2,"output_tokens":0}}"#
        ),
        Err(AdapterError::InvalidUsage)
    ));
    assert!(parse_jsonl_event("not JSON").is_err());
    assert!(parse_jsonl_event(&"x".repeat(1024 * 1024 + 1)).is_err());
}

#[test]
fn refresh_failures_are_typed_without_returning_sensitive_error_text() {
    for (message, expected) in [
        (
            "refresh_token_reused secret-token",
            RuntimeFailureKind::AuthRefreshReused,
        ),
        (
            "refresh_token_expired secret-token",
            RuntimeFailureKind::AuthExpired,
        ),
        (
            "refresh_token_invalidated secret-token",
            RuntimeFailureKind::AuthInvalidated,
        ),
        (
            "Authentication required 401 secret-token",
            RuntimeFailureKind::AuthRequired,
        ),
        (
            "Usage limit reached 429 secret-token",
            RuntimeFailureKind::RateLimited,
        ),
        (
            "Connection failed 503 secret-token",
            RuntimeFailureKind::ProviderUnavailable,
        ),
    ] {
        let line =
            serde_json::json!({"type":"turn.failed","error":{"message":message}}).to_string();
        let observation = parse_jsonl_event(&line).unwrap();
        assert!(matches!(observation,RuntimeObservation::Failure{kind} if kind==expected));
        assert!(!format!("{observation:?}").contains("secret-token"));
    }
}
