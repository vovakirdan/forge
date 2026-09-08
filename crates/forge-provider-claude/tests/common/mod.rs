use std::collections::BTreeSet;

use forge_domain::{
    CredentialBinding, CredentialDeliveryMode, ExecutionProfile, ExecutionProfileInput, ProjectId,
};
use forge_provider_claude::{ClaudeAdapter, ClaudeRunInput, PINNED_CLAUDE_VERSION};
use forge_provider_common::SecretBytes;
use uuid::Uuid;

pub fn profile() -> ExecutionProfile {
    let project_id = ProjectId::new();
    ExecutionProfile::try_from(ExecutionProfileInput {
        id: Uuid::now_v7(),
        revision: 1,
        project_id,
        adapter_id: "claude_code_cli".into(),
        adapter_version: PINNED_CLAUDE_VERSION.into(),
        provider_id: "anthropic".into(),
        model: "explicit-test-model".into(),
        credential_binding: CredentialBinding {
            id: Uuid::now_v7(),
            project_id,
            secret_id: Uuid::now_v7(),
            account_id: Some("synthetic-account".into()),
            allowed_delivery_modes: BTreeSet::from([CredentialDeliveryMode::IsolatedRuntimeSecret]),
        },
        credential_delivery: CredentialDeliveryMode::IsolatedRuntimeSecret,
        capability_profile: ClaudeAdapter::capabilities(),
    })
    .unwrap()
}

pub fn input(profile: &ExecutionProfile) -> ClaudeRunInput<'_> {
    ClaudeRunInput {
        profile,
        prompt: SecretBytes::new(b"private task context".to_vec()),
        workdir: "/workspace/task",
        gateway_url: "http://127.0.0.1:4097/mcp",
        proxy_url: "http://127.0.0.1:4098",
        session_id: Uuid::now_v7(),
        message_id: Uuid::now_v7(),
    }
}
