use super::apply_purpose_profile;
use forge_domain::{
    CredentialBinding, CredentialDeliveryMode, ExecutionProfileInput, ProjectId,
    ResolutionAssignmentRef, RuntimeCapability,
    runtime::{ResolutionRunSpec, RuntimeBinding, RuntimeLaunchSpec, SurfaceAccess, SurfaceSpec},
};
use forge_provider_codex::{CodexAdapter, CodexRunInput, PINNED_CODEX_VERSION};
use forge_provider_common::SecretBytes;
use std::collections::BTreeSet;
use uuid::Uuid;

#[test]
fn resolution_from_live_codex_profile_launches_one_shot_codex_not_app_server_driver() {
    let project_id = ProjectId::new();
    let mode = CredentialDeliveryMode::IsolatedRuntimeSecret;
    let spec = ResolutionRunSpec {
        schema_version: 4,
        project_id,
        run_id: Uuid::now_v7(),
        assignment: ResolutionAssignmentRef {
            assignment_id: Uuid::now_v7(),
            escalation_id: Uuid::now_v7(),
            lease_generation: 1,
        },
        instruction: "Answer only the assigned question".into(),
        binding: RuntimeBinding {
            execution_profile: ExecutionProfileInput {
                id: Uuid::now_v7(),
                revision: 1,
                project_id,
                adapter_id: "codex_cli".into(),
                adapter_version: PINNED_CODEX_VERSION.into(),
                provider_id: "openai".into(),
                model: "synthetic".into(),
                credential_binding: CredentialBinding {
                    id: Uuid::now_v7(),
                    project_id,
                    secret_id: Uuid::now_v7(),
                    account_id: Some("synthetic-resolution-account".into()),
                    allowed_delivery_modes: BTreeSet::from([mode]),
                },
                credential_delivery: mode,
                capability_profile: CodexAdapter::live_capabilities(),
            }
            .try_into()
            .unwrap(),
            system_prompt: "System".into(),
            employee_prompt: "Resolver".into(),
            surface: SurfaceSpec::None,
            access: SurfaceAccess::ReadWrite,
            image: format!("localhost/synthetic@sha256:{}", "0".repeat(64)),
            budget: Default::default(),
            limits: Default::default(),
        },
    };
    spec.validate().unwrap();
    let mut effective: RuntimeLaunchSpec =
        serde_json::from_value(serde_json::to_value(&spec).unwrap()).unwrap();
    apply_purpose_profile(&mut effective).unwrap();
    assert!(
        spec.binding
            .execution_profile
            .capability_profile()
            .capabilities
            .contains(&RuntimeCapability::LiveInput)
    );
    assert!(
        !effective
            .binding
            .execution_profile
            .capability_profile()
            .capabilities
            .contains(&RuntimeCapability::LiveInput)
    );
    let prepared = CodexAdapter::prepare(CodexRunInput {
        profile: &effective.binding.execution_profile,
        prompt: SecretBytes::new(b"question".to_vec()),
        workdir: "/workspace",
        gateway_url: "http://127.0.0.1:4097/mcp",
        proxy_url: "http://127.0.0.1:4098",
    })
    .unwrap();
    assert_eq!(prepared.program, "codex");
    assert_eq!(prepared.args.first().map(String::as_str), Some("exec"));
}
