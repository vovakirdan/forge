use super::*;
use crate::{SupervisorConfig, journal::Journal, new_id};
use forge_protocol::supervisor::v1::supervisor_to_core;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

fn valid_spec() -> forge_domain::runtime::SandboxRunSpec {
    use forge_domain::{
        CapabilityProfile, CredentialBinding, CredentialDeliveryMode, ExecutionProfileInput,
        ProjectId, RuntimeCapability, TransportEngine,
        runtime::{ResourceLimits, RuntimeBinding, SandboxRunSpec, SurfaceAccess, SurfaceSpec},
    };
    use std::collections::BTreeSet;
    let project_id = ProjectId::new();
    let mode = CredentialDeliveryMode::IsolatedRuntimeSecret;
    let profile = ExecutionProfileInput {
        id: uuid::Uuid::now_v7(),
        revision: 1,
        project_id,
        adapter_id: "codex_cli".into(),
        adapter_version: "0.153.2".into(),
        provider_id: "openai".into(),
        model: "synthetic-no-inference".into(),
        credential_binding: CredentialBinding {
            id: uuid::Uuid::now_v7(),
            project_id,
            secret_id: uuid::Uuid::now_v7(),
            account_id: Some("synthetic".into()),
            allowed_delivery_modes: BTreeSet::from([mode]),
        },
        credential_delivery: mode,
        capability_profile: CapabilityProfile {
            adapter_id: "codex_cli".into(),
            adapter_version: "0.153.2".into(),
            transport_engine: TransportEngine::CliWrapper,
            capabilities: BTreeSet::from([
                RuntimeCapability::ControlledStop,
                RuntimeCapability::NativeMcp,
            ]),
            credential_exposed_to_run: true,
        },
    }
    .try_into()
    .expect("valid profile");
    SandboxRunSpec {
        schema_version: 2,
        project_id,
        surface_id: uuid::Uuid::now_v7(),
        instruction: "Synthetic retained execution".into(),
        binding: RuntimeBinding {
            execution_profile: profile,
            image: format!("localhost/synthetic@sha256:{}", "0".repeat(64)),
            surface: SurfaceSpec::FilesystemSandbox,
            access: SurfaceAccess::ReadWrite,
            limits: ResourceLimits::default(),
            budget: Default::default(),
            system_prompt: "Synthetic".into(),
            employee_prompt: "No execution".into(),
        },
    }
}

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

async fn scenario(failed_inspection: bool) {
    let directory = Directory(std::env::temp_dir().join(format!("forge-inspection-{}", new_id())));
    fs::create_dir(&directory.0).expect("fixture directory");
    fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o700))
        .expect("private directory");
    let stub = directory.0.join("podman-stub");
    let script = if failed_inspection {
        "#!/bin/sh\nexit 125\n"
    } else {
        "#!/bin/sh\ncase \"$1\" in\ncontainer) exit 0;;\ninspect) sleep 1; exec /bin/cat \"$(dirname \"$0\")/inspection.json\";;\n*) exit 125;;\nesac\n"
    };
    fs::write(&stub, script).expect("inspection-only stub");
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o700)).expect("private executable");
    let mut config = SupervisorConfig::new(
        directory.0.join("core.sock"),
        "test-host".into(),
        "boot".into(),
    );
    config.podman_binary = stub;
    let mut provision = crate::tests::provision();
    provision.run_spec_version = 2;
    provision.run_spec_json =
        serde_json::to_string(&valid_spec()).expect("valid retained contract");
    let inspection = serde_json::json!([{
        "Id":"a".repeat(64),
        "State":{"Running":true,"Status":"running","ExitCode":0,"StartedAt":"2026-09-06T10:00:00Z"},
        "Config":{"Labels":{
            "forge.host":config.host_id,"forge.run":provision.run_id,
            "forge.epoch":provision.environment_epoch.to_string(),
            "forge.fence":provision.lease_fencing_token.to_string()
        }}
    }]);
    fs::write(
        directory.0.join("inspection.json"),
        serde_json::to_vec(&inspection).expect("encode"),
    )
    .expect("fixture inspection");
    let mut journal = Journal::open(
        &config.state_directory,
        &config.host_id,
        config.journal_max_bytes,
    )
    .expect("journal");
    journal
        .register(&provision, &config.boot_id)
        .expect("register running scope");
    drop(journal);
    let registry = RunRegistry::new(
        Journal::open(
            &config.state_directory,
            &config.host_id,
            config.journal_max_bytes,
        )
        .expect("restart journal"),
    );
    assert_eq!(
        registry.journal.lock().await.inventory()[0].presence,
        EnvironmentPresence::Unknown as i32
    );
    let backend = PodmanBackend::new(&config);
    let started = tokio::time::Instant::now();
    let adopted = backend
        .reconcile(&registry)
        .await
        .expect("initial physical barrier");
    assert_eq!(adopted, vec![provision]);
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    crate::send_inventory(&sender, &config, &registry, new_id())
        .await
        .expect("first requested inventory");
    let Some(supervisor_to_core::Message::Inventory(inventory)) =
        receiver.recv().await.expect("inventory message").message
    else {
        panic!("expected inventory");
    };
    assert_eq!(inventory.entries.len(), 1);
    if failed_inspection {
        assert_eq!(
            inventory.entries[0].presence,
            EnvironmentPresence::Unknown as i32
        );
    } else {
        assert!(
            started.elapsed() >= Duration::from_secs(1),
            "inventory waited for physical inspect"
        );
        assert_eq!(
            inventory.entries[0].presence,
            EnvironmentPresence::Active as i32
        );
        assert_eq!(inventory.entries[0].environment_id, "a".repeat(64));
    }
    assert_eq!(
        registry.journal.lock().await.pending().count(),
        0,
        "inspection does not launch or synthesize worker observations"
    );
}

#[tokio::test]
async fn initial_inventory_waits_for_delayed_inspection_of_surviving_run() {
    scenario(false).await;
}

#[tokio::test]
async fn failed_initial_inspection_preserves_unknown_identity_for_monitor_and_stop() {
    scenario(true).await;
}

#[tokio::test]
async fn invalid_retained_contract_fails_before_physical_adoption_or_inventory() {
    let directory =
        Directory(std::env::temp_dir().join(format!("forge-invalid-adopt-{}", new_id())));
    fs::create_dir(&directory.0).expect("fixture root");
    fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o700)).expect("private root");
    let mut config = SupervisorConfig::new(
        directory.0.join("core.sock"),
        "test-host".into(),
        "boot".into(),
    );
    config.podman_binary = directory.0.join("must-not-be-invoked");
    let mut provision = crate::tests::provision();
    provision.run_spec_version = 2;
    provision.run_spec_json = serde_json::json!({"surface_id":new_id()}).to_string();
    let mut journal = Journal::open(
        &config.state_directory,
        &config.host_id,
        config.journal_max_bytes,
    )
    .expect("journal");
    journal
        .register(&provision, &config.boot_id)
        .expect("retained scope");
    drop(journal);
    let registry = RunRegistry::new(
        Journal::open(
            &config.state_directory,
            &config.host_id,
            config.journal_max_bytes,
        )
        .expect("restart"),
    );
    assert!(matches!(
        PodmanBackend::new(&config).reconcile(&registry).await,
        Err(SupervisorError::InvalidRunSpec)
    ));
    let journal = registry.journal.lock().await;
    assert_eq!(
        journal.inventory()[0].presence,
        EnvironmentPresence::Unknown as i32
    );
    assert_eq!(journal.pending().count(), 0);
}
