use std::{
    collections::BTreeSet,
    process::{Command, Stdio},
    time::Duration,
};

use forge_domain::{
    CredentialBinding, CredentialDeliveryMode, ExecutionProfile, ExecutionProfileInput, ProjectId,
};
use forge_provider_common::{SecretBytes, adapter::RuntimeObservation};
use forge_provider_litellm::{LiteLlmClient, RunKeySpec, generate_virtual_key};
use forge_provider_opencode::{
    OpenCodeAdapter, OpenCodeClient, OpenCodeRunInput, PINNED_OPENCODE_VERSION, SessionResult,
};
use serde_json::{Value, json};
use time::OffsetDateTime;
use tokio::sync::watch;

struct Container(String);
impl Drop for Container {
    fn drop(&mut self) {
        // Only this test's uniquely named ephemeral container is removed.
        let _ = Command::new("podman")
            .args(["rm", "--force", &self.0])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn profile() -> ExecutionProfile {
    let project_id = ProjectId::new();
    ExecutionProfile::try_from(ExecutionProfileInput {
        id: ProjectId::new().as_uuid(),
        revision: 1,
        project_id,
        adapter_id: "opencode_runtime".into(),
        adapter_version: PINNED_OPENCODE_VERSION.into(),
        provider_id: "openai".into(),
        model: "fixture-model".into(),
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

#[tokio::test]
#[ignore = "requires actual pinned OpenCode runtime image and internal-only LiteLLM fixture"]
async fn actual_opencode_session_roundtrip_uses_litellm_virtual_key_and_stub() {
    assert_eq!(std::env::var("FORGE_LITELLM_CONTRACT").as_deref(), Ok("1"));
    let image = std::env::var("FORGE_OPENCODE_FIXTURE_IMAGE")
        .expect("provide the built runtime image digest");
    assert!(
        image.contains("@sha256:"),
        "runtime image must be digest pinned"
    );
    let profile = profile();
    let spec = RunKeySpec {
        run_id: ProjectId::new().as_uuid(),
        project_id: profile.project_id().as_uuid(),
        environment_epoch: 1,
        fencing_token: 1,
        execution_profile_id: profile.id(),
        execution_profile_revision: 1,
        models: ["forge-contract-stub".to_owned()].into(),
        expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(10),
        requests_per_minute: 100,
        tokens_per_minute: 100000,
        max_budget_usd: Some(100.0),
    };
    let proxy = LiteLlmClient::new(
        "http://127.0.0.1:4007",
        &SecretBytes::new(b"sk-forge-contract-master-not-a-real-key".to_vec()),
    )
    .unwrap();
    let key = generate_virtual_key().unwrap();
    let issued = proxy
        .create_or_reconcile(&spec, &key, OffsetDateTime::now_utc())
        .await
        .unwrap();
    let prepared = OpenCodeAdapter::prepare(OpenCodeRunInput {
        profile: &profile,
        prompt: SecretBytes::new(b"Reply with fixture response.".to_vec()),
        workdir: "/workspace/task",
        gateway_url: "http://127.0.0.1:4097/mcp",
        inference_url: "http://127.0.0.1:4098/v1",
        inference_model: "forge-contract-stub",
    })
    .unwrap();
    let mut environment = prepared.env;
    let mut config: Value = serde_json::from_str(&environment["OPENCODE_CONFIG_CONTENT"]).unwrap();
    // This is a provider transport test, not the Core/sandbox policy gate. Its
    // private network exposes only the fixture proxy; no real upstream exists.
    config["provider"]["forge"]["options"]["baseURL"] = json!("http://litellm:4000/v1");
    config["mcp"] = json!({});
    environment.insert("OPENCODE_CONFIG_CONTENT".into(), config.to_string());
    let name = format!("forge-opencode-fixture-{}", spec.run_id);
    let container = Container(name.clone());
    let mut command = Command::new("podman");
    command.args([
        "run",
        "--detach",
        "--name",
        &name,
        "--network",
        "forge-litellm-fixture_fixture",
        "--publish",
        "127.0.0.1::4096",
        "--entrypoint",
        "sh",
    ]);
    for (name, value) in &environment {
        // Non-secret container settings must not replace Podman's host HOME.
        command.args(["--env", &format!("{name}={value}")]);
    }
    command.args(["--env", "FORGE_LITELLM_KEY"]).env(
        "FORGE_LITELLM_KEY",
        std::str::from_utf8(key.expose()).unwrap(),
    );
    let output = command
        .args([&image, "-c", "mkdir -p /workspace/task && cd /workspace/task && exec opencode serve --hostname 0.0.0.0 --port 4096"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "fixture container failed to start: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let port = Command::new("podman")
        .args(["port", &container.0, "4096"])
        .output()
        .unwrap();
    assert!(port.status.success());
    let endpoint = format!(
        "http://{}",
        std::str::from_utf8(&port.stdout).unwrap().trim()
    );
    let client = OpenCodeClient::new(&endpoint, "/workspace/task").unwrap();
    let mut healthy = false;
    for _ in 0..100 {
        if client.healthy().await.unwrap_or(false) {
            healthy = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert!(
        healthy,
        "actual pinned OpenCode server never became healthy"
    );
    let session = client.create_session().await.unwrap();
    let (_stop, rx) = watch::channel(false);
    let mut observations = Vec::new();
    let result = tokio::time::timeout(
        Duration::from_secs(90),
        client.run_session(&session, "fixture-model", &prepared.stdin, rx, |event| {
            observations.push(event);
            Ok(())
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result, SessionResult::TurnEnded);
    assert!(observations.iter().any(|event| matches!(event, RuntimeObservation::AssistantOutput { text } if text.expose() == b"fixture response")));
    assert!(observations.iter().any(|event| matches!(event, RuntimeObservation::TurnCompleted { usage: Some(usage) } if usage.input_tokens == 1000 && usage.output_tokens == 1000)));
    let pending = client.create_session().await.unwrap();
    let (stop, rx) = watch::channel(false);
    let mut completed = false;
    let stopped = tokio::time::timeout(
        Duration::from_secs(90),
        client.run_session(
            &pending,
            "fixture-model",
            &SecretBytes::new(b"FORGE_FIXTURE_WAIT".to_vec()),
            rx,
            |event| {
                if matches!(event, RuntimeObservation::TurnStarted) {
                    stop.send(true).unwrap();
                }
                completed |= matches!(event, RuntimeObservation::TurnCompleted { .. });
                Ok(())
            },
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(stopped, SessionResult::Aborted);
    assert!(
        !completed,
        "an aborted session must not imply a completed turn"
    );
    proxy.revoke(&spec, &issued.key_hash).await.unwrap();
    drop(container);
}
