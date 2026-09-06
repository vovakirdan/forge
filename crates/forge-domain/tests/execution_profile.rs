use std::collections::BTreeSet;

use forge_domain::{
    CapabilityProfile, CredentialBinding, CredentialDeliveryMode, ExecutionProfile,
    ExecutionProfileError, ExecutionProfileInput, ProjectId, RuntimeCapability, TransportEngine,
};
use uuid::Uuid;

fn input() -> ExecutionProfileInput {
    let project_id = ProjectId::new();
    ExecutionProfileInput {
        id: Uuid::now_v7(),
        revision: 1,
        project_id,
        adapter_id: "codex_cli".into(),
        adapter_version: "0.153.2".into(),
        provider_id: "openai_chatgpt".into(),
        model: "operator-selected-model".into(),
        credential_binding: CredentialBinding {
            id: Uuid::now_v7(),
            project_id,
            secret_id: Uuid::now_v7(),
            account_id: Some("synthetic-account".into()),
            allowed_delivery_modes: BTreeSet::from([CredentialDeliveryMode::IsolatedRuntimeSecret]),
        },
        credential_delivery: CredentialDeliveryMode::IsolatedRuntimeSecret,
        capability_profile: CapabilityProfile {
            adapter_id: "codex_cli".into(),
            adapter_version: "0.153.2".into(),
            transport_engine: TransportEngine::CliWrapper,
            capabilities: BTreeSet::from([
                RuntimeCapability::StructuredEvents,
                RuntimeCapability::ControlledStop,
                RuntimeCapability::ModelSelection,
            ]),
            credential_exposed_to_run: true,
        },
    }
}

#[test]
fn immutable_profile_roundtrips_and_preserves_pinned_identity() {
    let input = input();
    let profile = ExecutionProfile::try_from(input.clone()).unwrap();
    let encoded = serde_json::to_string(&profile).unwrap();
    let decoded: ExecutionProfile = serde_json::from_str(&encoded).unwrap();
    assert_eq!(profile, decoded);
    assert_eq!(profile.model(), input.model);
    assert_eq!(profile.revision(), 1);
}

#[test]
fn serialized_profile_cannot_bypass_exposure_validation() {
    let mut input = input();
    input.capability_profile.credential_exposed_to_run = false;
    assert!(
        serde_json::from_value::<ExecutionProfile>(serde_json::to_value(&input).unwrap()).is_err()
    );
    assert_eq!(
        ExecutionProfile::try_from(input),
        Err(ExecutionProfileError::ExposureMismatch)
    );
}

#[test]
fn profile_cannot_cross_project_or_change_adapter_capabilities() {
    let mut other_project = input();
    other_project.credential_binding.project_id = ProjectId::new();
    assert_eq!(
        ExecutionProfile::try_from(other_project),
        Err(ExecutionProfileError::ProjectMismatch)
    );
    let mut other_adapter = input();
    other_adapter.capability_profile.adapter_version = "different".into();
    assert_eq!(
        ExecutionProfile::try_from(other_adapter),
        Err(ExecutionProfileError::AdapterMismatch)
    );
}

#[test]
fn proxy_only_requires_declared_gateway_auth_without_upstream_exposure() {
    let mut input = input();
    input.credential_delivery = CredentialDeliveryMode::ProxyOnly;
    input.credential_binding.allowed_delivery_modes =
        BTreeSet::from([CredentialDeliveryMode::ProxyOnly]);
    input.capability_profile.credential_exposed_to_run = false;
    assert_eq!(
        ExecutionProfile::try_from(input.clone()),
        Err(ExecutionProfileError::GatewayAuthRequired)
    );
    input
        .capability_profile
        .capabilities
        .insert(RuntimeCapability::GatewayAuth);
    assert!(ExecutionProfile::try_from(input).is_ok());
}

#[test]
fn unsupported_capabilities_fail_eligibility_without_implicit_emulation() {
    let profile = ExecutionProfile::try_from(input()).unwrap();
    assert!(
        !profile
            .capability_profile()
            .supports(&BTreeSet::from([RuntimeCapability::SessionResume]))
    );
    assert!(
        profile
            .capability_profile()
            .supports(&BTreeSet::from([RuntimeCapability::ControlledStop]))
    );
}

#[test]
fn missing_model_zero_revision_and_unapproved_delivery_are_rejected() {
    let mut missing = input();
    missing.model.clear();
    assert_eq!(
        ExecutionProfile::try_from(missing),
        Err(ExecutionProfileError::MissingSelection)
    );
    let mut zero = input();
    zero.revision = 0;
    assert_eq!(
        ExecutionProfile::try_from(zero),
        Err(ExecutionProfileError::InvalidIdentity)
    );
    let mut unapproved = input();
    unapproved.credential_delivery = CredentialDeliveryMode::TrustedHost;
    assert_eq!(
        ExecutionProfile::try_from(unapproved),
        Err(ExecutionProfileError::DeliveryDenied)
    );
}
