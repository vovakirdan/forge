use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
};

use forge_domain::{
    CapabilityProfile, CredentialDeliveryMode, ExecutionProfile, RuntimeCapability, TransportEngine,
};
use forge_provider_common::{
    SecretBytes,
    adapter::{AdapterError, CredentialFileCopy, PreparedInvocation, StopStrategy},
};
use url::Url;

use crate::{CODEX_HOME, PINNED_CODEX_VERSION};

pub struct CodexRunInput<'a> {
    pub profile: &'a ExecutionProfile,
    pub prompt: SecretBytes,
    pub workdir: &'a str,
    pub gateway_url: &'a str,
    pub proxy_url: &'a str,
}

/// Builds a pinned invocation. It never starts the Codex executable itself.
pub struct CodexAdapter;

impl CodexAdapter {
    pub fn live_capabilities() -> CapabilityProfile {
        let mut profile = Self::capabilities();
        profile.capabilities.insert(RuntimeCapability::LiveInput);
        profile
    }

    pub fn validate_profile(profile: &ExecutionProfile) -> Result<(), AdapterError> {
        validate_profile(profile)
    }
    pub fn capabilities() -> CapabilityProfile {
        CapabilityProfile {
            adapter_id: "codex_cli".into(),
            adapter_version: PINNED_CODEX_VERSION.into(),
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

    /// `version_output` must come from sandbox-local `codex --version`, before
    /// attaching the prompt or invoking a provider. This performs no login/API call.
    pub fn preflight(
        version_output: &str,
        profile: &ExecutionProfile,
    ) -> Result<CapabilityProfile, AdapterError> {
        if version_output.trim() != format!("codex-cli {PINNED_CODEX_VERSION}") {
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

    pub fn prepare(input: CodexRunInput<'_>) -> Result<PreparedInvocation, AdapterError> {
        validate_profile(input.profile)?;
        validate_workdir(input.workdir)?;
        let gateway = loopback_url(input.gateway_url, false)?;
        let proxy = loopback_url(input.proxy_url, true)?;
        let live = input
            .profile
            .capability_profile()
            .capabilities
            .contains(&RuntimeCapability::LiveInput);
        let mut args = vec![
            "exec".into(),
            "--json".into(),
            "--color".into(),
            "never".into(),
            "--ephemeral".into(),
            "--ignore-user-config".into(),
            "--ignore-rules".into(),
            "--strict-config".into(),
            "--skip-git-repo-check".into(),
            // The outer Podman environment is the security boundary. Shell
            // remains functional inside that environment without nested bwrap.
            "--dangerously-bypass-approvals-and-sandbox".into(),
            "--model".into(),
            input.profile.model().into(),
            "--cd".into(),
            input.workdir.into(),
        ];
        if live {
            args = vec![
                "app-server".into(),
                "--listen".into(),
                "stdio://".into(),
                "--strict-config".into(),
            ];
        }
        for setting in settings(input.workdir, &gateway)? {
            args.extend(["-c".into(), setting]);
        }
        if !live {
            args.push("-".into());
        }
        let mut env = BTreeMap::from([
            ("PATH".into(), "/usr/local/bin:/usr/bin:/bin".into()),
            ("HOME".into(), "/run/forge/home".into()),
            ("CODEX_HOME".into(), CODEX_HOME.into()),
            ("XDG_CONFIG_HOME".into(), "/run/forge/config".into()),
            ("XDG_CACHE_HOME".into(), "/run/forge/cache".into()),
            ("XDG_DATA_HOME".into(), "/run/forge/data".into()),
            ("TMPDIR".into(), "/tmp".into()),
            ("LANG".into(), "C.UTF-8".into()),
            ("NO_COLOR".into(), "1".into()),
        ]);
        for key in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
            env.insert(key.into(), proxy.clone());
        }
        for key in ["NO_PROXY", "no_proxy"] {
            env.insert(key.into(), "127.0.0.1,localhost,::1".into());
        }
        if live {
            env.insert("FORGE_CODEX_MODEL".into(), input.profile.model().into());
            env.insert("FORGE_CODEX_WORKDIR".into(), input.workdir.into());
        }
        Ok(PreparedInvocation {
            program: if live { "forge-codex-driver" } else { "codex" }.into(),
            args,
            env,
            stdin: input.prompt,
            managed_files: Vec::new(),
            credential_files: vec![CredentialFileCopy {
                source: "/run/forge-secrets/auth.json".into(),
                target: format!("{CODEX_HOME}/auth.json"),
                writeback: true,
            }],
            stop: StopStrategy::SigintThenEnvironmentKill,
        })
    }
}

fn validate_profile(profile: &ExecutionProfile) -> Result<(), AdapterError> {
    if profile.adapter_id() != "codex_cli"
        || profile.adapter_version() != PINNED_CODEX_VERSION
        || profile.provider_id() != "openai"
        || profile.credential_binding().account_id.is_none()
        || profile.credential_delivery() != CredentialDeliveryMode::IsolatedRuntimeSecret
        || profile.capability_profile().transport_engine != TransportEngine::CliWrapper
    {
        return Err(AdapterError::ProfileMismatch);
    }
    if !profile
        .capability_profile()
        .capabilities
        .is_subset(&CodexAdapter::live_capabilities().capabilities)
    {
        return Err(AdapterError::UnsupportedCapability);
    }
    Ok(())
}

fn settings(workdir: &str, gateway: &str) -> Result<Vec<String>, AdapterError> {
    let quoted_workdir =
        serde_json::to_string(workdir).map_err(|_| AdapterError::InvalidEnvironment)?;
    let quoted_gateway =
        serde_json::to_string(gateway).map_err(|_| AdapterError::InvalidEnvironment)?;
    let mut settings = vec![
        "cli_auth_credentials_store=\"file\"".into(),
        "forced_login_method=\"chatgpt\"".into(),
        "model_provider=\"openai\"".into(),
        "web_search=\"disabled\"".into(),
        "history.persistence=\"none\"".into(),
        "hide_agent_reasoning=true".into(),
        "show_raw_agent_reasoning=false".into(),
        "model_reasoning_summary=\"none\"".into(),
        "analytics.enabled=false".into(),
        "project_doc_max_bytes=0".into(),
        "skills.include_instructions=false".into(),
        "skills.bundled.enabled=false".into(),
        "allow_login_shell=false".into(),
        "features.respect_system_proxy=true".into(),
        "features.skip_host_skill_discovery=true".into(),
        // Session overrides participate in project trust discovery before any
        // project .codex/config.toml can add a second MCP server or a hook.
        format!("projects.{quoted_workdir}.trust_level=\"untrusted\""),
        format!("mcp_servers.forge.url={quoted_gateway}"),
        "mcp_servers.forge.required=true".into(),
        "mcp_servers.forge.startup_timeout_sec=20".into(),
        "mcp_servers.forge.tool_timeout_sec=120".into(),
    ];
    for feature in [
        "plugins",
        "plugin_hooks",
        "codex_hooks",
        "hooks",
        "apps",
        "connectors",
        "multi_agent",
        "multi_agent_mode",
        "memories",
        "memory_tool",
        "external_agent_memory_import",
        "remote_control",
        "remote_plugin",
        "shell_snapshot",
        "shell_snapshot_v2",
    ] {
        settings.push(format!("features.{feature}=false"));
    }
    Ok(settings)
}

fn validate_workdir(path: &str) -> Result<(), AdapterError> {
    let path = Path::new(path);
    if !path.is_absolute()
        || !path.starts_with("/workspace")
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        || path
            .as_os_str()
            .as_encoded_bytes()
            .iter()
            .any(|byte| byte.is_ascii_control())
    {
        return Err(AdapterError::InvalidEnvironment);
    }
    Ok(())
}

fn loopback_url(value: &str, proxy: bool) -> Result<String, AdapterError> {
    let parsed = Url::parse(value).map_err(|_| AdapterError::InvalidEnvironment)?;
    let loopback = parsed
        .host_str()
        .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "[::1]"));
    if parsed.scheme() != "http"
        || !loopback
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.port().is_none()
        || (proxy && parsed.path() != "/")
    {
        return Err(AdapterError::InvalidEnvironment);
    }
    Ok(parsed.into())
}
