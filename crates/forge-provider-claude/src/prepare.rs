use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use forge_domain::{
    CapabilityProfile, CredentialDeliveryMode, ExecutionProfile, RuntimeCapability, TransportEngine,
};
use forge_provider_common::{
    SecretBytes,
    adapter::{
        AdapterError, CredentialFileCopy, ManagedRuntimeFile, PreparedInvocation, StopStrategy,
    },
};
use serde_json::json;
use url::Url;
use uuid::Uuid;

use crate::{CLAUDE_CONFIG_DIR, CLAUDE_TOKEN_PATH, PINNED_CLAUDE_VERSION, encode_user_message};

/// Already-authorized Run context; credentials are materialized separately.
pub struct ClaudeRunInput<'a> {
    /// Immutable execution profile selecting this adapter and subscription lane.
    pub profile: &'a ExecutionProfile,
    /// Initial context, never placed on the command line.
    pub prompt: SecretBytes,
    /// Task surface mount location inside the managed container.
    pub workdir: &'a str,
    /// Run-scoped loopback HTTP MCP relay.
    pub gateway_url: &'a str,
    /// Run-scoped loopback HTTP forward proxy.
    pub proxy_url: &'a str,
    /// New session identity for this physical Run, not an Employee-wide session.
    pub session_id: Uuid,
    /// Correlation identity of the initial input message.
    pub message_id: Uuid,
}

/// Prepares the Claude Code CLI; does not execute it or mutate canonical state.
pub struct ClaudeAdapter;

impl ClaudeAdapter {
    /// Validates a static assignment without claiming authentication or a live
    /// executable probe. Supervisor checks the actual image before credential mount.
    pub fn validate_profile(profile: &ExecutionProfile) -> Result<(), AdapterError> {
        validate_profile(profile)
    }

    /// One-shot defaults preserve existing profiles; live input is explicit opt-in.
    pub fn capabilities() -> CapabilityProfile {
        CapabilityProfile {
            adapter_id: "claude_code_cli".into(),
            adapter_version: PINNED_CLAUDE_VERSION.into(),
            transport_engine: TransportEngine::CliWrapper,
            capabilities: BTreeSet::from([
                RuntimeCapability::StructuredEvents,
                RuntimeCapability::ModelSelection,
                RuntimeCapability::UsageReporting,
                RuntimeCapability::ControlledStop,
                RuntimeCapability::NativeMcp,
            ]),
            credential_exposed_to_run: true,
        }
    }

    /// Optional native streaming input in the same fenced physical Run.
    pub fn live_capabilities() -> CapabilityProfile {
        let mut profile = Self::capabilities();
        profile.capabilities.insert(RuntimeCapability::LiveInput);
        profile
    }

    /// Checks sandbox-local `claude --version` before attaching any credential.
    /// Successful preflight establishes compatibility, not authentication.
    pub fn preflight(
        version_output: &str,
        profile: &ExecutionProfile,
    ) -> Result<CapabilityProfile, AdapterError> {
        if version_output.trim() != format!("{PINNED_CLAUDE_VERSION} (Claude Code)") {
            return Err(AdapterError::VersionMismatch);
        }
        validate_profile(profile)?;
        Ok(
            if profile
                .capability_profile()
                .capabilities
                .contains(&RuntimeCapability::LiveInput)
            {
                Self::live_capabilities()
            } else {
                Self::capabilities()
            },
        )
    }

    /// Builds a secret-free invocation manifest and redacted input stream.
    /// The caller must use `env_clear`, mount a fresh private runtime directory,
    /// and set the process cwd to `workdir` inside the outer Podman sandbox.
    pub fn prepare(input: ClaudeRunInput<'_>) -> Result<PreparedInvocation, AdapterError> {
        validate_profile(input.profile)?;
        validate_workdir(input.workdir)?;
        let gateway = local_url(input.gateway_url, false)?;
        let proxy = local_url(input.proxy_url, true)?;
        let stdin = encode_user_message(input.session_id, input.message_id, &input.prompt)?;
        let args = [
            "--print",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--replay-user-messages",
            "--no-session-persistence",
            "--setting-sources",
            "",
            "--settings",
            "/run/forge/claude-settings.json",
            "--strict-mcp-config",
            "--mcp-config",
            "/run/forge/claude-mcp.json",
            "--disable-slash-commands",
            "--no-chrome",
            "--prompt-suggestions",
            "false",
            // The outer sandbox is the security boundary; shell stays available.
            "--dangerously-skip-permissions",
            "--tools",
            "Bash,Read,Write,Edit,Glob,Grep",
            "--model",
            input.profile.model(),
            "--session-id",
            &input.session_id.to_string(),
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let mut env = BTreeMap::from([
            ("PATH".into(), "/usr/local/bin:/usr/bin:/bin".into()),
            ("HOME".into(), "/run/forge/home".into()),
            ("CLAUDE_CONFIG_DIR".into(), CLAUDE_CONFIG_DIR.into()),
            ("XDG_CONFIG_HOME".into(), "/run/forge/config".into()),
            ("XDG_CACHE_HOME".into(), "/run/forge/cache".into()),
            ("XDG_DATA_HOME".into(), "/run/forge/data".into()),
            ("TMPDIR".into(), "/tmp".into()),
            ("LANG".into(), "C.UTF-8".into()),
            ("NO_COLOR".into(), "1".into()),
            ("ENABLE_CLAUDEAI_MCP_SERVERS".into(), "false".into()),
            (
                "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".into(),
                "1".into(),
            ),
            ("CLAUDE_CODE_DISABLE_AUTO_MEMORY".into(), "1".into()),
            ("CLAUDE_CODE_DISABLE_BACKGROUND_TASKS".into(), "1".into()),
            (
                "CLAUDE_CODE_DISABLE_OFFICIAL_MARKETPLACE_AUTOINSTALL".into(),
                "1".into(),
            ),
            ("DISABLE_LOGIN_COMMAND".into(), "1".into()),
        ]);
        for key in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
            env.insert(key.into(), proxy.clone());
        }
        for key in ["NO_PROXY", "no_proxy"] {
            env.insert(key.into(), "127.0.0.1,localhost,::1".into());
        }
        Ok(PreparedInvocation {
            program: "forge-claude-driver".into(),
            args,
            env,
            stdin,
            managed_files: vec![
                ManagedRuntimeFile {
                    relative_path: "claude-settings.json".into(),
                    contents: json!({
                        "disableAllHooks": true,
                        "autoMemoryEnabled": false,
                        "enableAllProjectMcpServers": false,
                        "enabledPlugins": {},
                        "forceLoginMethod": "claudeai",
                        "claudeMdExcludes": ["**"],
                    })
                    .to_string(),
                },
                ManagedRuntimeFile {
                    relative_path: "claude-mcp.json".into(),
                    contents: json!({"mcpServers":{"forge":{"type":"http","url":gateway}}})
                        .to_string(),
                },
            ],
            credential_files: vec![CredentialFileCopy {
                source: "/run/forge-secrets/claude-setup-token".into(),
                target: CLAUDE_TOKEN_PATH.into(),
                // setup-token is an externally rotated, long-lived access token.
                writeback: false,
            }],
            stop: StopStrategy::SigintThenEnvironmentKill,
        })
    }
}

fn validate_profile(profile: &ExecutionProfile) -> Result<(), AdapterError> {
    if profile.adapter_id() != "claude_code_cli"
        || profile.adapter_version() != PINNED_CLAUDE_VERSION
        || profile.provider_id() != "anthropic"
        || profile.credential_binding().account_id.is_none()
        || profile.credential_delivery() != CredentialDeliveryMode::IsolatedRuntimeSecret
        || profile.capability_profile().transport_engine != TransportEngine::CliWrapper
        || profile.model().starts_with('-')
    {
        return Err(AdapterError::ProfileMismatch);
    }
    if !profile
        .capability_profile()
        .capabilities
        .is_subset(&ClaudeAdapter::live_capabilities().capabilities)
    {
        return Err(AdapterError::UnsupportedCapability);
    }
    Ok(())
}

fn validate_workdir(path: &str) -> Result<(), AdapterError> {
    if !Path::new(path).is_absolute()
        || !Path::new(path).starts_with("/workspace")
        || path.split('/').any(|part| matches!(part, "." | ".."))
        || path.chars().any(char::is_control)
    {
        return Err(AdapterError::InvalidEnvironment);
    }
    Ok(())
}

fn local_url(value: &str, proxy: bool) -> Result<String, AdapterError> {
    let url = Url::parse(value).map_err(|_| AdapterError::InvalidEnvironment)?;
    if url.scheme() != "http"
        || !url
            .host_str()
            .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "[::1]"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port().is_none_or(|port| port == 0)
        || value.chars().any(char::is_control)
        || (proxy && url.path() != "/")
    {
        return Err(AdapterError::InvalidEnvironment);
    }
    Ok(url.into())
}
