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
use serde_json::json;
use url::Url;

use crate::{PINNED_OPENCODE_VERSION, VIRTUAL_KEY_PATH};

pub struct OpenCodeRunInput<'a> {
    pub profile: &'a ExecutionProfile,
    pub prompt: SecretBytes,
    pub workdir: &'a str,
    pub gateway_url: &'a str,
    /// Run-scoped reverse relay to LiteLLM, including its /v1 prefix.
    pub inference_url: &'a str,
    /// Immutable LiteLLM route alias for the exact credential binding revision.
    pub inference_model: &'a str,
}

pub struct OpenCodeAdapter;

impl OpenCodeAdapter {
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
            adapter_id: "opencode_runtime".into(),
            adapter_version: PINNED_OPENCODE_VERSION.into(),
            transport_engine: TransportEngine::ApiRuntime,
            capabilities: BTreeSet::from([
                RuntimeCapability::StructuredEvents,
                RuntimeCapability::ModelSelection,
                RuntimeCapability::UsageReporting,
                RuntimeCapability::ControlledStop,
                RuntimeCapability::GatewayAuth,
                RuntimeCapability::NativeMcp,
            ]),
            credential_exposed_to_run: false,
        }
    }

    pub fn preflight(
        version_output: &str,
        profile: &ExecutionProfile,
    ) -> Result<CapabilityProfile, AdapterError> {
        if version_output.trim() != PINNED_OPENCODE_VERSION {
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

    pub fn prepare(input: OpenCodeRunInput<'_>) -> Result<PreparedInvocation, AdapterError> {
        validate_profile(input.profile)?;
        validate_workdir(input.workdir)?;
        let gateway = local_url(input.gateway_url)?;
        let inference = local_url(input.inference_url)?;
        if inference.path() != "/v1" && inference.path() != "/v1/" {
            return Err(AdapterError::InvalidEnvironment);
        }
        let model = input.profile.model();
        if input.inference_model.trim().is_empty()
            || input.inference_model.chars().any(char::is_control)
        {
            return Err(AdapterError::ProfileMismatch);
        }
        // Only an environment reference: the child receives a per-Run virtual
        // key, never an upstream credential or a management key.
        let config = json!({
            "model":format!("forge/{model}"),"small_model":format!("forge/{model}"),
            "enabled_providers":["forge"],"provider":{"forge":{
                "npm":"@ai-sdk/openai-compatible","name":"Forge Provider Gateway",
                "options":{"baseURL":inference.as_str().trim_end_matches('/'),"apiKey":"{env:FORGE_LITELLM_KEY}"},
                "models":{model:{"id":input.inference_model,"name":model,"tool_call":true}}
            }},
            "mcp":{"forge":{"type":"remote","url":gateway.as_str(),"enabled":true,"oauth":false,"timeout":120000}},
            "plugin":[],"instructions":[],"share":"disabled","autoupdate":false,
            "lsp":false,"formatter":false,"compaction":{"auto":false,"prune":false},
            "permission":{"*":"allow","question":"deny","task":"deny","skill":"deny"},
            "agent":{"title":{"disable":true},"summary":{"disable":true},"compaction":{"disable":true}},
            "experimental":{"openTelemetry":false},"logLevel":"ERROR"
        });
        let env = BTreeMap::from([
            ("PATH".into(), "/usr/local/bin:/usr/bin:/bin".into()),
            ("HOME".into(), "/run/forge/home".into()),
            ("XDG_CONFIG_HOME".into(), "/run/forge/config".into()),
            ("XDG_CACHE_HOME".into(), "/run/forge/cache".into()),
            ("XDG_DATA_HOME".into(), "/run/forge/data".into()),
            ("XDG_STATE_HOME".into(), "/run/forge/state".into()),
            ("TMPDIR".into(), "/tmp".into()),
            ("LANG".into(), "C.UTF-8".into()),
            ("OPENCODE_DISABLE_PROJECT_CONFIG".into(), "true".into()),
            ("OPENCODE_DISABLE_MODELS_FETCH".into(), "true".into()),
            ("OPENCODE_DISABLE_AUTOUPDATE".into(), "true".into()),
            ("OPENCODE_DISABLE_AUTOCOMPACT".into(), "true".into()),
            ("OPENCODE_DISABLE_TERMINAL_TITLE".into(), "true".into()),
            ("OPENCODE_CONFIG_CONTENT".into(), config.to_string()),
            ("FORGE_OPENCODE_MODEL".into(), model.into()),
            ("FORGE_OPENCODE_WORKDIR".into(), input.workdir.into()),
            ("NO_PROXY".into(), "127.0.0.1,localhost,::1".into()),
            ("no_proxy".into(), "127.0.0.1,localhost,::1".into()),
        ]);
        Ok(PreparedInvocation {
            program: "forge-opencode-driver".into(),
            args: Vec::new(),
            env,
            stdin: input.prompt,
            managed_files: Vec::new(),
            credential_files: vec![CredentialFileCopy {
                source: "/run/forge-secrets/api-key".into(),
                target: VIRTUAL_KEY_PATH.into(),
                writeback: false,
            }],
            stop: StopStrategy::RuntimeApiAbortThenEnvironmentKill,
        })
    }
}

fn validate_profile(profile: &ExecutionProfile) -> Result<(), AdapterError> {
    if profile.adapter_id() != "opencode_runtime"
        || profile.adapter_version() != PINNED_OPENCODE_VERSION
        || profile.credential_delivery() != CredentialDeliveryMode::ProxyOnly
        || profile.capability_profile().transport_engine != TransportEngine::ApiRuntime
    {
        return Err(AdapterError::ProfileMismatch);
    }
    if !profile
        .capability_profile()
        .capabilities
        .is_subset(&OpenCodeAdapter::live_capabilities().capabilities)
    {
        return Err(AdapterError::UnsupportedCapability);
    }
    Ok(())
}

pub(crate) fn validate_workdir(path: &str) -> Result<(), AdapterError> {
    if !Path::new(path).is_absolute()
        || !Path::new(path).starts_with("/workspace")
        || Path::new(path)
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
        || path.chars().any(char::is_control)
    {
        return Err(AdapterError::InvalidEnvironment);
    }
    Ok(())
}

pub(crate) fn local_url(value: &str) -> Result<Url, AdapterError> {
    let url = Url::parse(value).map_err(|_| AdapterError::InvalidEnvironment)?;
    if url.scheme() != "http"
        || !url
            .host_str()
            .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "[::1]"))
        || url.port().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AdapterError::InvalidEnvironment);
    }
    Ok(url)
}
