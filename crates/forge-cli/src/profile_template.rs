//! Offline profile templates reuse pinned adapter declarations; no credential or network I/O.
use anyhow::{Result, ensure};
use forge_domain::{
    CredentialBinding, CredentialDeliveryMode, ExecutionProfileInput, ProjectId,
    runtime::{ResourceLimits, RunBudget, RuntimeBinding, SurfaceAccess, SurfaceSpec},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileLane {
    CodexCli,
    ClaudeCli,
    #[serde(rename = "openrouter_api")]
    OpenRouterApi,
    #[serde(rename = "openai_api")]
    OpenAiApi,
}

/// Credential IDs refer to a separately enrolled secret; this input contains no key.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileTemplateInput {
    pub lane: ProfileLane,
    pub project_id: ProjectId,
    pub employee_id: Uuid,
    pub binding_id: Uuid,
    pub secret_id: Uuid,
    pub account_id: Option<String>,
    pub model: String,
    pub image: String,
    pub system_prompt: String,
    pub employee_prompt: String,
    pub limits: ResourceLimits,
    pub budget: RunBudget,
    #[serde(default)]
    pub live_input: bool,
    #[serde(default)]
    pub access: SurfaceAccess,
}

#[derive(Debug, Serialize)]
pub struct ConfigureRuntimeTemplate {
    pub employee_id: Uuid,
    pub binding: RuntimeBinding,
}

pub fn build(input: ProfileTemplateInput) -> Result<ConfigureRuntimeTemplate> {
    ensure!(
        input.employee_id.get_version_num() == 7,
        "employee_id must be UUIDv7"
    );
    let (capability, provider, delivery) = match input.lane {
        ProfileLane::CodexCli => (
            if input.live_input {
                forge_provider_codex::CodexAdapter::live_capabilities()
            } else {
                forge_provider_codex::CodexAdapter::capabilities()
            },
            "openai",
            CredentialDeliveryMode::IsolatedRuntimeSecret,
        ),
        ProfileLane::ClaudeCli => (
            if input.live_input {
                forge_provider_claude::ClaudeAdapter::live_capabilities()
            } else {
                forge_provider_claude::ClaudeAdapter::capabilities()
            },
            "anthropic",
            CredentialDeliveryMode::IsolatedRuntimeSecret,
        ),
        ProfileLane::OpenRouterApi | ProfileLane::OpenAiApi => (
            if input.live_input {
                forge_provider_opencode::OpenCodeAdapter::live_capabilities()
            } else {
                forge_provider_opencode::OpenCodeAdapter::capabilities()
            },
            if matches!(input.lane, ProfileLane::OpenRouterApi) {
                "openrouter"
            } else {
                "openai"
            },
            CredentialDeliveryMode::ProxyOnly,
        ),
    };
    ensure!(
        (delivery == CredentialDeliveryMode::IsolatedRuntimeSecret) == input.account_id.is_some(),
        "subscription lanes require an explicit account_id; API lanes omit it"
    );
    ensure!(
        input.model.trim() == input.model,
        "model cannot have surrounding whitespace"
    );
    let binding = RuntimeBinding {
        budget: input.budget,
        execution_profile: ExecutionProfileInput {
            id: Uuid::now_v7(),
            revision: 1,
            project_id: input.project_id,
            adapter_id: capability.adapter_id.clone(),
            adapter_version: capability.adapter_version.clone(),
            provider_id: provider.into(),
            model: input.model,
            credential_binding: CredentialBinding {
                id: input.binding_id,
                project_id: input.project_id,
                secret_id: input.secret_id,
                account_id: input.account_id,
                allowed_delivery_modes: [delivery].into(),
            },
            credential_delivery: delivery,
            capability_profile: capability,
        }
        .try_into()?,
        image: input.image,
        // Git source/base belong to Task, never a provider template.
        surface: SurfaceSpec::FilesystemSandbox,
        access: input.access,
        limits: input.limits,
        system_prompt: input.system_prompt,
        employee_prompt: input.employee_prompt,
    };
    binding.validate()?;
    Ok(ConfigureRuntimeTemplate {
        employee_id: input.employee_id,
        binding,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_domain::RuntimeCapability;
    fn input(lane: ProfileLane) -> ProfileTemplateInput {
        ProfileTemplateInput {
            lane,
            project_id: ProjectId::new(),
            employee_id: Uuid::now_v7(),
            binding_id: Uuid::now_v7(),
            secret_id: Uuid::now_v7(),
            account_id: matches!(lane, ProfileLane::CodexCli | ProfileLane::ClaudeCli)
                .then(|| "explicit-account".into()),
            model: "explicit-model".into(),
            image: format!("localhost/fixture@sha256:{}", "0".repeat(64)),
            system_prompt: "Stay inside the assigned sandbox.".into(),
            employee_prompt: "Follow the assigned stage and its Forge tools.".into(),
            limits: ResourceLimits::default(),
            budget: RunBudget::default(),
            live_input: false,
            access: SurfaceAccess::ReadWrite,
        }
    }

    #[test]
    fn four_lanes_share_validated_contract_without_selecting_task_source() -> Result<()> {
        for lane in [
            ProfileLane::CodexCli,
            ProfileLane::ClaudeCli,
            ProfileLane::OpenRouterApi,
            ProfileLane::OpenAiApi,
        ] {
            let template = build(input(lane))?;
            template.binding.validate()?;
            let profile = &template.binding.execution_profile;
            assert_eq!(template.binding.surface, SurfaceSpec::FilesystemSandbox);
            assert_eq!(
                profile.credential_binding().project_id,
                profile.project_id()
            );
            assert!(
                !profile
                    .capability_profile()
                    .capabilities
                    .contains(&RuntimeCapability::LiveInput)
            );
            let mut live = input(lane);
            live.live_input = true;
            assert!(
                build(live)?
                    .binding
                    .execution_profile
                    .capability_profile()
                    .capabilities
                    .contains(&RuntimeCapability::LiveInput)
            );
        }
        Ok(())
    }

    #[test]
    fn template_refuses_implicit_account_identity_or_mutable_image() {
        let mut missing_account = input(ProfileLane::CodexCli);
        missing_account.account_id = None;
        assert!(build(missing_account).is_err());
        let mut api_account = input(ProfileLane::OpenAiApi);
        api_account.account_id = Some("not-a-subscription".into());
        assert!(build(api_account).is_err());
        let mut tag = input(ProfileLane::ClaudeCli);
        tag.image = "localhost/forge-runtime:latest".into();
        assert!(build(tag).is_err());
        let mut identity = input(ProfileLane::OpenRouterApi);
        identity.employee_id = Uuid::nil();
        assert!(build(identity).is_err());
    }
}
