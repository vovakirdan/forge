use std::collections::BTreeSet;

use forge_domain::{
    CredentialBinding, CredentialDeliveryMode, ExecutionProfile, ExecutionProfileInput, ProjectId,
    RuntimeCapability,
};
use forge_provider_common::{
    SecretBytes,
    adapter::{AdapterError, RuntimeFailureKind, RuntimeObservation, StopStrategy},
};
use forge_provider_opencode::{
    EventInterpreter, OpenCodeAdapter, OpenCodeRunInput, PINNED_OPENCODE_VERSION, SessionEvent,
    parse_driver_event, write_driver_event,
};
use serde_json::{Value, json};

fn profile() -> ExecutionProfile {
    let project_id = ProjectId::new();
    ExecutionProfile::try_from(ExecutionProfileInput {
        id: ProjectId::new().as_uuid(),
        revision: 1,
        project_id,
        adapter_id: "opencode_runtime".into(),
        adapter_version: PINNED_OPENCODE_VERSION.into(),
        provider_id: "anthropic".into(),
        model: "forge-claude-explicit".into(),
        credential_binding: CredentialBinding {
            id: ProjectId::new().as_uuid(),
            project_id,
            secret_id: ProjectId::new().as_uuid(),
            account_id: None,
            allowed_delivery_modes: BTreeSet::from([CredentialDeliveryMode::ProxyOnly]),
        },
        credential_delivery: CredentialDeliveryMode::ProxyOnly,
        capability_profile: OpenCodeAdapter::capabilities(),
    })
    .unwrap()
}
fn input(profile: &ExecutionProfile) -> OpenCodeRunInput<'_> {
    OpenCodeRunInput {
        profile,
        prompt: SecretBytes::new(b"private task input".to_vec()),
        workdir: "/workspace/task",
        gateway_url: "http://127.0.0.1:4097/mcp",
        inference_url: "http://127.0.0.1:4098/v1",
        inference_model: "forge-route-exact-binding-revision",
    }
}

#[test]
fn exact_pin_capabilities_and_domain_binding_are_conformant() {
    let profile = profile();
    assert!(OpenCodeAdapter::preflight("1.18.29\n", &profile).is_ok());
    assert_eq!(
        OpenCodeAdapter::preflight("1.18.30", &profile),
        Err(AdapterError::VersionMismatch)
    );
    assert!(
        !OpenCodeAdapter::capabilities()
            .capabilities
            .contains(&RuntimeCapability::SessionResume)
    );
    let binding = forge_domain::runtime::RuntimeBinding {
        execution_profile: profile,
        image: format!("runner@sha256:{}", "a".repeat(64)),
        surface: forge_domain::runtime::SurfaceSpec::FilesystemSandbox,
        access: Default::default(),
        limits: Default::default(),
        budget: Default::default(),
        system_prompt: "system".into(),
        employee_prompt: "role".into(),
    };
    binding.validate().unwrap();
    OpenCodeAdapter::prepare(input(&binding.execution_profile)).unwrap();
}

#[test]
fn managed_environment_has_explicit_provider_model_and_only_key_reference() {
    let profile = profile();
    let prepared = OpenCodeAdapter::prepare(input(&profile)).unwrap();
    assert_eq!(prepared.program, "forge-opencode-driver");
    assert!(prepared.args.is_empty());
    assert_eq!(prepared.stdin.expose(), b"private task input");
    assert!(!format!("{prepared:?}").contains("private task input"));
    assert_eq!(
        prepared.stop,
        StopStrategy::RuntimeApiAbortThenEnvironmentKill
    );
    assert_eq!(
        prepared.credential_files[0].source,
        "/run/forge-secrets/api-key"
    );
    assert_eq!(
        prepared.credential_files[0].target,
        "/run/forge/opencode/virtual-key"
    );
    assert!(!prepared.credential_files[0].writeback);
    for name in [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "FORGE_LITELLM_KEY",
        "LITELLM_MASTER_KEY",
        "OPENCODE_CONFIG",
        "OPENCODE_CONFIG_DIR",
    ] {
        assert!(!prepared.env.contains_key(name));
    }
    assert_eq!(prepared.env["OPENCODE_DISABLE_PROJECT_CONFIG"], "true");
    assert_eq!(prepared.env["HOME"], "/run/forge/home");
    let config: Value = serde_json::from_str(&prepared.env["OPENCODE_CONFIG_CONTENT"]).unwrap();
    assert_eq!(config["model"], "forge/forge-claude-explicit");
    assert_eq!(config["small_model"], config["model"]);
    assert_eq!(
        config["provider"]["forge"]["options"]["apiKey"],
        "{env:FORGE_LITELLM_KEY}"
    );
    assert!(
        config["provider"]["forge"]["models"]
            .get("forge-claude-explicit")
            .is_some()
    );
    assert_eq!(
        config["provider"]["forge"]["models"]["forge-claude-explicit"]["id"],
        "forge-route-exact-binding-revision"
    );
    assert_eq!(config["enabled_providers"], json!(["forge"]));
    assert_eq!(config["mcp"].as_object().unwrap().len(), 1);
    assert_eq!(config["permission"]["bash"], Value::Null);
    assert_eq!(config["permission"]["*"], "allow");
}

#[test]
fn unsafe_paths_endpoints_and_unsupported_capabilities_fail_before_launch() {
    let profile = profile();
    for workdir in [
        "/home/user",
        "/workspace/../other",
        "relative",
        "/workspace\n",
    ] {
        let mut input = input(&profile);
        input.workdir = workdir;
        assert!(OpenCodeAdapter::prepare(input).is_err());
    }
    for endpoint in [
        "https://api.anthropic.com/v1",
        "http://user:pass@127.0.0.1:4098/v1",
        "http://127.0.0.1:4098/v1?token=secret",
        "http://127.0.0.1:4098/key/generate",
    ] {
        let mut input = input(&profile);
        input.inference_url = endpoint;
        assert!(OpenCodeAdapter::prepare(input).is_err());
    }
    let mut unsupported: ExecutionProfileInput = profile.into();
    unsupported
        .capability_profile
        .capabilities
        .insert(RuntimeCapability::SessionResume);
    assert!(
        OpenCodeAdapter::prepare(input(&ExecutionProfile::try_from(unsupported).unwrap())).is_err()
    );
}

fn message(completed: bool) -> Value {
    json!({"type":"message.updated","properties":{"sessionID":"ses_test","info":{"id":"msg_a","sessionID":"ses_test","role":"assistant","time":{"completed":completed.then_some(123)},"tokens":{"input":5,"output":3,"reasoning":1,"cache":{"read":2,"write":1}}}}})
}
fn event(interpreter: &mut EventInterpreter, value: Value) -> SessionEvent {
    interpreter.ingest(&value.to_string()).unwrap()
}

#[test]
fn initial_idle_and_other_sessions_cannot_complete_this_run() {
    let mut parser = EventInterpreter::new("ses_test");
    for value in [
        json!({"type":"session.idle","properties":{"sessionID":"ses_test"}}),
        json!({"type":"session.status","properties":{"sessionID":"ses_other","status":{"type":"busy"}}}),
    ] {
        assert!(matches!(
            event(&mut parser, value),
            SessionEvent::Observation(RuntimeObservation::Ignored)
        ));
    }
    event(
        &mut parser,
        json!({"type":"session.status","properties":{"sessionID":"ses_test","status":{"type":"busy"}}}),
    );
    assert!(
        parser
            .ingest(
                &json!({"type":"session.idle","properties":{"sessionID":"ses_test"}}).to_string()
            )
            .is_err()
    );
}

#[test]
fn hidden_reasoning_and_tool_bodies_never_enter_normalized_output() {
    let mut parser = EventInterpreter::new("ses_test");
    event(&mut parser, message(false));
    for part in [
        json!({"id":"p_r","type":"reasoning","text":{"not":"even decoded"}}),
        json!({"id":"p_x","type":"unknown","text":"private raw content"}),
    ] {
        let mut part = part;
        part["sessionID"] = json!("ses_test");
        part["messageID"] = json!("msg_a");
        assert!(matches!(
            event(
                &mut parser,
                json!({"type":"message.part.updated","properties":{"part":part}})
            ),
            SessionEvent::Observation(RuntimeObservation::Ignored)
        ));
    }
    let observed = event(
        &mut parser,
        json!({"type":"message.part.updated","properties":{"part":{"id":"p_t","sessionID":"ses_test","messageID":"msg_a","type":"tool","tool":"bash","state":{"status":"completed","input":{"command":"secret-command"},"output":"secret-output"}}}}),
    );
    assert!(!format!("{observed:?}").contains("secret"));
    assert!(matches!(
        observed,
        SessionEvent::Observation(RuntimeObservation::ToolActivity {
            succeeded: Some(true),
            ..
        })
    ));
}

#[test]
fn completed_usage_is_aggregated_once_and_text_debug_stays_redacted() {
    let mut parser = EventInterpreter::new("ses_test");
    event(&mut parser, message(false));
    let text = json!({"type":"message.part.updated","properties":{"part":{"id":"p_a","sessionID":"ses_test","messageID":"msg_a","type":"text","text":"private\ntext","time":{"end":123}}}});
    let observed = event(&mut parser, text.clone());
    assert!(!format!("{observed:?}").contains("private"));
    assert!(matches!(
        event(&mut parser, text),
        SessionEvent::Observation(RuntimeObservation::Ignored)
    ));
    event(&mut parser, message(true));
    event(&mut parser, message(true));
    let usage = parser.usage().unwrap();
    assert_eq!(usage.input_tokens, 8);
    assert_eq!(usage.output_tokens, 3);
    let SessionEvent::Observation(observed) = observed else {
        panic!("output expected")
    };
    let encoded = write_driver_event(&observed).unwrap();
    let decoded = parse_driver_event(std::str::from_utf8(encoded.expose()).unwrap()).unwrap();
    let RuntimeObservation::AssistantOutput { text } = decoded else {
        panic!("text expected")
    };
    assert_eq!(text.expose(), b"private\ntext");
}

#[test]
fn provider_auth_and_rate_errors_are_static_and_usage_invalid_is_not_invented() {
    for (status, expected) in [
        (401, RuntimeFailureKind::AuthRequired),
        (429, RuntimeFailureKind::RateLimited),
        (503, RuntimeFailureKind::ProviderUnavailable),
    ] {
        let mut parser = EventInterpreter::new("ses_test");
        let observed = event(
            &mut parser,
            json!({"type":"session.error","properties":{"sessionID":"ses_test","error":{"name":"APIError","data":{"statusCode":status,"message":"secret body","responseBody":"private"}}}}),
        );
        assert!(
            matches!(observed,SessionEvent::Observation(RuntimeObservation::Failure{kind}) if kind==expected)
        );
        assert!(!format!("{observed:?}").contains("private"));
    }
    let mut parser = EventInterpreter::new("ses_test");
    let mut broken = message(true);
    broken["properties"]["info"]["tokens"]["input"] = json!(-1);
    assert!(parser.ingest(&broken.to_string()).is_err());
}
