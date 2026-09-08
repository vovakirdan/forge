mod common;

use forge_domain::{ExecutionProfile, ExecutionProfileInput, RuntimeCapability};
use forge_provider_claude::{CLAUDE_CONFIG_DIR, CLAUDE_TOKEN_PATH, ClaudeAdapter};
use forge_provider_common::adapter::{AdapterError, StopStrategy};
use serde_json::Value;

use common::{input, profile};

#[test]
fn version_and_capabilities_are_pinned_without_promising_resume() {
    let profile = profile();
    assert!(ClaudeAdapter::preflight("2.1.263 (Claude Code)\n", &profile).is_ok());
    for version in [
        "2.1.264 (Claude Code)",
        "2.1.263",
        "2.1.263-modified (Claude Code)",
    ] {
        assert_eq!(
            ClaudeAdapter::preflight(version, &profile),
            Err(AdapterError::VersionMismatch)
        );
    }
    assert!(
        !ClaudeAdapter::capabilities()
            .capabilities
            .contains(&RuntimeCapability::SessionResume)
    );
    assert!(
        !ClaudeAdapter::capabilities()
            .capabilities
            .contains(&RuntimeCapability::GatewayAuth)
    );
}

#[test]
fn unsupported_declared_capability_is_rejected_before_invocation() {
    let mut profile: ExecutionProfileInput = profile().into();
    profile
        .capability_profile
        .capabilities
        .insert(RuntimeCapability::SessionResume);
    let profile = ExecutionProfile::try_from(profile).unwrap();
    assert_eq!(
        ClaudeAdapter::preflight("2.1.263 (Claude Code)", &profile),
        Err(AdapterError::UnsupportedCapability)
    );
}

#[test]
fn live_input_is_explicit_without_changing_legacy_defaults() {
    assert!(
        !ClaudeAdapter::capabilities()
            .capabilities
            .contains(&RuntimeCapability::LiveInput)
    );
    let mut input: ExecutionProfileInput = profile().into();
    input.capability_profile = ClaudeAdapter::live_capabilities();
    let profile = ExecutionProfile::try_from(input).unwrap();
    assert!(
        ClaudeAdapter::preflight("2.1.263 (Claude Code)", &profile)
            .unwrap()
            .capabilities
            .contains(&RuntimeCapability::LiveInput)
    );
}

#[test]
fn subscription_requires_explicit_account_and_anthropic_provider() {
    for missing_account in [false, true] {
        let mut profile: ExecutionProfileInput = profile().into();
        if missing_account {
            profile.credential_binding.account_id = None;
        } else {
            profile.provider_id = "openai".into();
        }
        let profile = ExecutionProfile::try_from(profile).unwrap();
        assert_eq!(
            ClaudeAdapter::preflight("2.1.263 (Claude Code)", &profile),
            Err(AdapterError::ProfileMismatch)
        );
    }
}

#[test]
fn invocation_separates_private_context_and_subscription_secret_from_arguments() {
    let profile = profile();
    let input = input(&profile);
    let session_id = input.session_id;
    let message_id = input.message_id;
    let invocation = ClaudeAdapter::prepare(input).unwrap();
    let message: Value = serde_json::from_slice(invocation.stdin.expose()).unwrap();
    assert_eq!(message["message"]["content"], "private task context");
    assert_eq!(message["session_id"], session_id.to_string());
    assert_eq!(message["uuid"], message_id.to_string());
    assert!(!format!("{invocation:?}").contains("private task context"));
    assert_eq!(invocation.program, "forge-claude-driver");
    assert_eq!(invocation.env["CLAUDE_CONFIG_DIR"], CLAUDE_CONFIG_DIR);
    assert_eq!(invocation.credential_files[0].target, CLAUDE_TOKEN_PATH);
    assert_eq!(
        invocation.credential_files[0].source,
        "/run/forge-secrets/claude-setup-token"
    );
    assert!(!invocation.credential_files[0].writeback);
    assert_eq!(invocation.stop, StopStrategy::SigintThenEnvironmentKill);
    for key in [
        "CLAUDE_CODE_OAUTH_TOKEN",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "DBUS_SESSION_BUS_ADDRESS",
    ] {
        assert!(!invocation.env.contains_key(key));
    }
    assert!(!invocation.args.contains(&"--bare".into()));
    assert!(!invocation.args.contains(&"--safe-mode".into()));
    assert!(!invocation.args.contains(&"--fallback-model".into()));
}

#[test]
fn settings_preserve_shell_and_only_forge_mcp_without_project_hooks() {
    let profile = profile();
    let invocation = ClaudeAdapter::prepare(input(&profile)).unwrap();
    for expected in [
        "--replay-user-messages",
        "--strict-mcp-config",
        "--dangerously-skip-permissions",
        "--no-session-persistence",
        "--disable-slash-commands",
    ] {
        assert!(invocation.args.iter().any(|arg| arg == expected));
    }
    assert!(
        invocation
            .args
            .windows(2)
            .any(|pair| pair == ["--setting-sources", ""])
    );
    assert!(
        invocation
            .args
            .windows(2)
            .any(|pair| pair == ["--tools", "Bash,Read,Write,Edit,Glob,Grep"])
    );
    assert!(
        invocation
            .args
            .windows(2)
            .any(|pair| pair == ["--model", "explicit-test-model"])
    );
    let settings: Value = serde_json::from_str(&invocation.managed_files[0].contents).unwrap();
    assert_eq!(settings["disableAllHooks"], true);
    assert_eq!(settings["autoMemoryEnabled"], false);
    assert_eq!(settings["forceLoginMethod"], "claudeai");
    assert_eq!(settings["claudeMdExcludes"], serde_json::json!(["**"]));
    let mcp: Value = serde_json::from_str(&invocation.managed_files[1].contents).unwrap();
    assert_eq!(mcp["mcpServers"].as_object().unwrap().len(), 1);
    assert_eq!(
        mcp["mcpServers"]["forge"]["url"],
        "http://127.0.0.1:4097/mcp"
    );
}

#[test]
fn both_proxy_cases_are_forced_through_loopback_without_external_noproxy() {
    let profile = profile();
    let invocation = ClaudeAdapter::prepare(input(&profile)).unwrap();
    for key in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
        assert_eq!(invocation.env[key], "http://127.0.0.1:4098/");
    }
    for key in ["NO_PROXY", "no_proxy"] {
        assert_eq!(invocation.env[key], "127.0.0.1,localhost,::1");
    }
}

#[test]
fn unsafe_workdirs_and_relay_urls_fail_closed() {
    let profile = profile();
    for value in [
        "/home/operator",
        "/workspace/../secrets",
        "relative",
        "/workspace/./task",
        "/workspace/invalid\npath",
        "/workspaces/task",
    ] {
        let mut input = input(&profile);
        input.workdir = value;
        assert!(matches!(
            ClaudeAdapter::prepare(input),
            Err(AdapterError::InvalidEnvironment)
        ));
    }
    for value in [
        "http://outside:4097/mcp",
        "http://token@127.0.0.1:4097/mcp",
        "http://127.0.0.1:4097/mcp?secret=x",
        "https://127.0.0.1:4097/mcp",
        "http://127.0.0.1:0/mcp",
        "http://127.0.0.1/mcp",
    ] {
        let mut input = input(&profile);
        input.gateway_url = value;
        assert!(matches!(
            ClaudeAdapter::prepare(input),
            Err(AdapterError::InvalidEnvironment)
        ));
    }
    let mut input = input(&profile);
    input.proxy_url = "http://127.0.0.1:4098/path";
    assert!(matches!(
        ClaudeAdapter::prepare(input),
        Err(AdapterError::InvalidEnvironment)
    ));
}
