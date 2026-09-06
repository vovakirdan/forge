//! Validated, immutable provider execution profiles without credential bytes.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::ProjectId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialDeliveryMode {
    ProxyOnly,
    IsolatedRuntimeSecret,
    TrustedHost,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialBinding {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub secret_id: Uuid,
    pub account_id: Option<String>,
    pub allowed_delivery_modes: BTreeSet<CredentialDeliveryMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportEngine {
    Acp,
    CliWrapper,
    ApiRuntime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCapability {
    StructuredEvents,
    SessionResume,
    ModelSelection,
    UsageReporting,
    ControlledStop,
    GatewayAuth,
    NativeMcp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityProfile {
    pub adapter_id: String,
    pub adapter_version: String,
    pub transport_engine: TransportEngine,
    pub capabilities: BTreeSet<RuntimeCapability>,
    pub credential_exposed_to_run: bool,
}

impl CapabilityProfile {
    pub fn supports(&self, requirements: &BTreeSet<RuntimeCapability>) -> bool {
        requirements.is_subset(&self.capabilities)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionProfileInput {
    pub id: Uuid,
    pub revision: u64,
    pub project_id: ProjectId,
    pub adapter_id: String,
    pub adapter_version: String,
    pub provider_id: String,
    pub model: String,
    pub credential_binding: CredentialBinding,
    pub credential_delivery: CredentialDeliveryMode,
    pub capability_profile: CapabilityProfile,
}

/// A validated snapshot: changes are represented by a new profile revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ExecutionProfileInput", into = "ExecutionProfileInput")]
pub struct ExecutionProfile(ExecutionProfileInput);

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ExecutionProfileError {
    #[error("execution profile contains an invalid identifier or version")]
    InvalidIdentity,
    #[error("execution profile requires an explicit adapter, provider, and model")]
    MissingSelection,
    #[error("credential binding belongs to another project")]
    ProjectMismatch,
    #[error("credential delivery mode is not allowed by its binding")]
    DeliveryDenied,
    #[error("capability profile does not describe the selected adapter version")]
    AdapterMismatch,
    #[error("credential exposure does not match the selected delivery mode")]
    ExposureMismatch,
    #[error("proxy-only execution requires gateway authentication capability")]
    GatewayAuthRequired,
}

impl TryFrom<ExecutionProfileInput> for ExecutionProfile {
    type Error = ExecutionProfileError;

    fn try_from(input: ExecutionProfileInput) -> Result<Self, Self::Error> {
        if input.id.get_version_num() != 7
            || input.revision == 0
            || input.revision > i64::MAX as u64
            || input.project_id.as_uuid().get_version_num() != 7
            || input.credential_binding.id.get_version_num() != 7
            || input.credential_binding.secret_id.get_version_num() != 7
        {
            return Err(ExecutionProfileError::InvalidIdentity);
        }
        if [
            &input.adapter_id,
            &input.adapter_version,
            &input.provider_id,
            &input.model,
        ]
        .iter()
        .any(|value| value.trim().is_empty() || value.chars().any(char::is_control))
        {
            return Err(ExecutionProfileError::MissingSelection);
        }
        if input.credential_binding.project_id != input.project_id {
            return Err(ExecutionProfileError::ProjectMismatch);
        }
        if input
            .credential_binding
            .account_id
            .as_ref()
            .is_some_and(|account| {
                account.trim().is_empty() || account.chars().any(char::is_control)
            })
        {
            return Err(ExecutionProfileError::InvalidIdentity);
        }
        if !input
            .credential_binding
            .allowed_delivery_modes
            .contains(&input.credential_delivery)
        {
            return Err(ExecutionProfileError::DeliveryDenied);
        }
        let capability = &input.capability_profile;
        if capability.adapter_id != input.adapter_id
            || capability.adapter_version != input.adapter_version
        {
            return Err(ExecutionProfileError::AdapterMismatch);
        }
        let exposed = input.credential_delivery != CredentialDeliveryMode::ProxyOnly;
        if capability.credential_exposed_to_run != exposed {
            return Err(ExecutionProfileError::ExposureMismatch);
        }
        if !exposed
            && !capability
                .capabilities
                .contains(&RuntimeCapability::GatewayAuth)
        {
            return Err(ExecutionProfileError::GatewayAuthRequired);
        }
        Ok(Self(input))
    }
}

impl ExecutionProfile {
    pub fn id(&self) -> Uuid {
        self.0.id
    }
    pub fn revision(&self) -> u64 {
        self.0.revision
    }
    pub fn project_id(&self) -> ProjectId {
        self.0.project_id
    }
    pub fn adapter_id(&self) -> &str {
        &self.0.adapter_id
    }
    pub fn adapter_version(&self) -> &str {
        &self.0.adapter_version
    }
    pub fn provider_id(&self) -> &str {
        &self.0.provider_id
    }
    pub fn model(&self) -> &str {
        &self.0.model
    }
    pub fn credential_binding(&self) -> &CredentialBinding {
        &self.0.credential_binding
    }
    pub fn credential_delivery(&self) -> CredentialDeliveryMode {
        self.0.credential_delivery
    }
    pub fn capability_profile(&self) -> &CapabilityProfile {
        &self.0.capability_profile
    }
}

impl From<ExecutionProfile> for ExecutionProfileInput {
    fn from(profile: ExecutionProfile) -> Self {
        profile.0
    }
}
