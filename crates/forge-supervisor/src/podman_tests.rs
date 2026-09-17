//! Explicit local container tests. These use a fake executable in a real sandbox,
//! not a provider login or an inference request.

use forge_domain::{
    CapabilityProfile, CredentialBinding, CredentialDeliveryMode, ExecutionProfileInput, ProjectId,
    RuntimeCapability, TransportEngine,
    runtime::{ResourceLimits, RuntimeBinding, SandboxRunSpec, SurfaceAccess, SurfaceSpec},
};
use forge_protocol::{
    runtime::RunnerInvocation,
    supervisor::v1::{EnvironmentPresence, ProvisionRun},
};
use std::{collections::BTreeSet, fs, os::unix::fs::PermissionsExt, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
    process::Command,
    time::{sleep, timeout},
};

use crate::{
    SupervisorConfig,
    journal::Journal,
    new_id,
    podman::{PodmanBackend, scoped_directory},
    registry::RunRegistry,
    surface::{private_directory, private_write},
};

mod fixture;
mod input_acceptance;
mod receipt_admission;
mod ui_boundary;
use fixture::*;

#[tokio::test]
#[ignore = "requires rootless Podman and the rebuilt TASK-12 fixture image; no credentials"]
async fn podman_stop_before_provision_never_launches_the_employee() {
    let fixture = Fixture::new(false).await;
    let backend = PodmanBackend::new(&fixture.config);
    let control = fixture
        .registry
        .register(&fixture.provision, &fixture.config.boot_id)
        .await
        .unwrap()
        .unwrap();
    control.request_stop();
    backend
        .run(
            fixture.provision.clone(),
            fixture.registry.clone(),
            control,
            false,
        )
        .await
        .unwrap();
    assert!(!fixture.config.state_directory.join("surfaces").exists());
    assert_eq!(
        fixture.registry.journal.lock().await.inventory()[0].presence,
        EnvironmentPresence::Quiescent as i32
    );
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires rootless Podman and the rebuilt TASK-12 fixture image; no credentials"]
async fn missing_runtime_fails_before_surface_or_credential_preparation() {
    let mut fixture = Fixture::new(false).await;
    // Cached fixture base has no Codex executable; never pull a replacement.
    fixture.spec.binding.image = "docker.io/library/ubuntu@sha256:1e0a86e57d247923571b75e0aaf48a1449cf8c543d51fb3e07a4a7d7bfa79316".into();
    fixture.provision.run_spec_json = serde_json::to_string(&fixture.spec).unwrap();
    let grant = scoped_directory(&fixture.config.grants_directory, &fixture.provision);
    fs::remove_file(grant.join("invocation.json")).unwrap();
    let backend = PodmanBackend::new(&fixture.config);
    let control = fixture
        .registry
        .register(&fixture.provision, &fixture.config.boot_id)
        .await
        .unwrap()
        .unwrap();
    let result = backend
        .run(
            fixture.provision.clone(),
            fixture.registry.clone(),
            control,
            false,
        )
        .await;
    assert!(matches!(
        result,
        Err(crate::SupervisorError::RuntimePreflightFailed)
    ));
    assert!(!fixture.config.state_directory.join("surfaces").exists());
    assert!(!fixture.config.state_directory.join("runtime").exists());
    assert!(fixture.registry.journal.lock().await.pending().any(|message| {
        matches!(&message.message, Some(forge_protocol::supervisor::v1::supervisor_to_core::Message::ObservedRunEvent(event))
            if event.kind == forge_protocol::supervisor::v1::RunEventKind::Stopped as i32
            && event.details_json.contains("sandbox_not_started_confirmed"))
    }));
    assert_eq!(
        fixture.registry.journal.lock().await.inventory()[0].presence,
        EnvironmentPresence::Quiescent as i32
    );
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires rootless Podman and the rebuilt TASK-12 fixture image; no credentials"]
async fn no_persistent_surface_uses_a_writable_private_tmpfs() {
    let mut fixture = Fixture::new(false).await;
    fixture.spec.binding.surface = SurfaceSpec::None;
    fixture.provision.run_spec_json = serde_json::to_string(&fixture.spec).unwrap();
    let backend = PodmanBackend::new(&fixture.config);
    let control = fixture
        .registry
        .register(&fixture.provision, &fixture.config.boot_id)
        .await
        .unwrap()
        .unwrap();
    timeout(
        Duration::from_secs(30),
        backend.run(
            fixture.provision.clone(),
            fixture.registry.clone(),
            control,
            false,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let evidence = scoped_directory(
        &fixture.config.state_directory.join("evidence"),
        &fixture.provision,
    );
    let exit: forge_protocol::runtime::RunnerExit =
        serde_json::from_slice(&fs::read(evidence.join("exit.json")).unwrap()).unwrap();
    assert_eq!(
        exit.exit_code,
        Some(0),
        "fixture must be able to write its private ephemeral workspace"
    );
    assert!(!fixture.config.state_directory.join("surfaces").exists());
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires rootless Podman and the rebuilt TASK-12 fixture image; no credentials"]
async fn podman_inspection_failure_keeps_stop_control_and_reconciles_without_restart() {
    let mut fixture = Fixture::new(true).await;
    let fault = fixture.config.state_directory.join("inspect-fault");
    let shim = fixture.config.state_directory.join("podman-shim");
    private_write(&shim, format!("#!/bin/sh\nif [ \"$1\" = inspect ] && [ -f '{}' ]; then exit 125; fi\nexec podman \"$@\"\n", fault.display()).as_bytes()).unwrap();
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o700)).unwrap();
    fixture.config.podman_binary = shim;
    let worker = running_fixture(&fixture).await;
    private_write(&fault, b"synthetic failure").unwrap();
    timeout(Duration::from_secs(10), async {
        while fixture.registry.journal.lock().await.inventory()[0].presence
            != EnvironmentPresence::Unknown as i32
        {
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    request_fixture_stop(&fixture).await;
    await_container_exit(&fixture).await;
    fs::remove_file(fault).unwrap();
    timeout(Duration::from_secs(10), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.registry.journal.lock().await.inventory()[0].presence,
        EnvironmentPresence::Quiescent as i32
    );
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires rootless Podman and the rebuilt TASK-12 fixture image; no credentials"]
async fn podman_full_journal_does_not_disable_physical_stop() {
    let fixture = Fixture::new(true).await;
    let worker = running_fixture(&fixture).await;
    fixture.registry.journal.lock().await.set_test_capacity(1);
    request_fixture_stop(&fixture).await;
    await_container_exit(&fixture).await;
    assert!(
        !worker.is_finished(),
        "retains observation until terminal proof is durable"
    );
    fixture
        .registry
        .journal
        .lock()
        .await
        .set_test_capacity(16 * 1024 * 1024);
    timeout(Duration::from_secs(10), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires rootless Podman and the rebuilt TASK-12 fixture image; no credentials"]
async fn podman_gateway_socket_replacement_is_visible_to_a_surviving_run() {
    let mut fixture = Fixture::new(true).await;
    let worker = running_fixture(&fixture).await;
    gateway_roundtrip(&fixture).await;
    drop(fixture._gateway.take());
    let socket = fixture
        .config
        .gateways_directory
        .join(&fixture.provision.run_id)
        .join("gateway.sock");
    fs::remove_file(&socket).unwrap();
    fixture._gateway = Some(UnixListener::bind(socket).unwrap());
    gateway_roundtrip(&fixture).await;
    request_fixture_stop(&fixture).await;
    timeout(Duration::from_secs(10), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires rootless Podman and the rebuilt TASK-12 fixture image; no credentials"]
async fn podman_read_only_surface_denies_a_real_child_write() {
    let mut fixture = Fixture::new(false).await;
    fixture.spec.binding.access = SurfaceAccess::ReadOnly;
    fixture.provision.run_spec_json = serde_json::to_string(&fixture.spec).unwrap();
    let backend = PodmanBackend::new(&fixture.config);
    let control = fixture
        .registry
        .register(&fixture.provision, &fixture.config.boot_id)
        .await
        .unwrap()
        .unwrap();
    timeout(
        Duration::from_secs(30),
        backend.run(
            fixture.provision.clone(),
            fixture.registry.clone(),
            control,
            false,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let evidence = scoped_directory(
        &fixture.config.state_directory.join("evidence"),
        &fixture.provision,
    );
    let exit: forge_protocol::runtime::RunnerExit =
        serde_json::from_slice(&fs::read(evidence.join("exit.json")).unwrap()).unwrap();
    assert_ne!(
        exit.exit_code,
        Some(0),
        "fixture tries to write its read-only working directory"
    );
    assert!(
        fs::read_to_string(evidence.join("stderr.log"))
            .unwrap()
            .contains("Read-only file system")
    );
    let surface = fixture
        .config
        .state_directory
        .join("surfaces")
        .join(fixture.spec.surface_id.to_string());
    assert!(!surface.join("surface.json").exists());
    assert!(!surface.join("worktree/fixture-result.txt").exists());
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires rootless Podman and the built TASK-12 fixture image; no credentials"]
async fn podman_wrapper_preserves_files_and_filters_reasoning_without_fake_outcome() {
    let fixture = Fixture::new(false).await;
    let backend = PodmanBackend::new(&fixture.config);
    let control = fixture
        .registry
        .register(&fixture.provision, &fixture.config.boot_id)
        .await
        .unwrap()
        .unwrap();
    timeout(
        Duration::from_secs(30),
        backend.run(
            fixture.provision.clone(),
            fixture.registry.clone(),
            control,
            false,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let file = fixture
        .config
        .state_directory
        .join("surfaces")
        .join(fixture.spec.surface_id.to_string())
        .join("worktree/fixture-result.txt");
    assert_eq!(fs::read_to_string(file).unwrap(), "fixture work\n");
    let evidence = scoped_directory(
        &fixture.config.state_directory.join("evidence"),
        &fixture.provision,
    );
    assert!(
        !fs::read_to_string(evidence.join("stdout.jsonl"))
            .unwrap()
            .contains("private reasoning")
    );
    assert!(evidence.join("exit.json").is_file());
    assert_eq!(
        fixture.registry.journal.lock().await.inventory()[0].presence,
        EnvironmentPresence::Quiescent as i32
    );
    assert!(
        !fixture
            .registry
            .journal
            .lock()
            .await
            .pending()
            .any(|message| matches!(
                message.message,
                Some(
                    forge_protocol::supervisor::v1::supervisor_to_core::Message::ExecutorSubmission(
                        _
                    )
                )
            ))
    );
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires rootless Podman and the built TASK-12 fixture image; no credentials"]
async fn podman_worker_survives_monitor_abort_and_is_adopted_before_stop() {
    let mut fixture = Fixture::new(true).await;
    let backend = PodmanBackend::new(&fixture.config);
    let control = fixture
        .registry
        .register(&fixture.provision, &fixture.config.boot_id)
        .await
        .unwrap()
        .unwrap();
    let worker_backend = backend.clone();
    let registry = fixture.registry.clone();
    let provision = fixture.provision.clone();
    let worker = tokio::spawn(async move {
        worker_backend
            .run(provision, registry, control, false)
            .await
    });
    let result_file = fixture
        .config
        .state_directory
        .join("surfaces")
        .join(fixture.spec.surface_id.to_string())
        .join("worktree/fixture-result.txt");
    timeout(Duration::from_secs(30), async {
        while !result_file.exists() {
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    worker.abort();
    let _ = worker.await;
    let Fixture {
        config,
        provision,
        spec,
        registry,
        _gateway,
    } = fixture;
    drop(registry);
    let reopened = RunRegistry::new(
        Journal::open(
            &config.state_directory.join("journal"),
            &config.host_id,
            config.journal_max_bytes,
        )
        .unwrap(),
    );
    assert_eq!(
        reopened.journal.lock().await.inventory()[0].presence,
        EnvironmentPresence::Unknown as i32
    );
    fixture = Fixture {
        config,
        provision,
        spec,
        registry: reopened,
        _gateway,
    };
    let mut competing = fixture.provision.clone();
    competing.run_id = new_id();
    competing.command_id = new_id();
    assert!(matches!(
        fixture
            .registry
            .register(&competing, &fixture.config.boot_id)
            .await,
        Err(crate::SupervisorError::ConflictingProvision)
    ));
    let name = format!(
        "forge-run-{}-{}-{}",
        fixture.provision.run_id,
        fixture.provision.lease_fencing_token,
        fixture.provision.environment_epoch
    );
    let isolation=Command::new("podman").args(["exec",&name,"/bin/sh","-c","test ! -e /home/zov && test ! -S /run/podman/podman.sock && test ! -S /var/run/docker.sock && test ! -S /run/forge/core.sock"]).status().await.unwrap();
    assert!(isolation.success());
    let network = timeout(
        Duration::from_secs(3),
        Command::new("podman")
            .args([
                "exec",
                &name,
                "/bin/bash",
                "--noprofile",
                "--norc",
                "-c",
                "exec 3<>/dev/tcp/1.1.1.1/443",
            ])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!network.status.success());
    assert!(String::from_utf8_lossy(&network.stderr).contains("Network is unreachable"));
    let memory = Command::new("podman")
        .args(["exec", &name, "cat", "/sys/fs/cgroup/memory.max"])
        .output()
        .await
        .unwrap();
    assert_eq!(
        String::from_utf8(memory.stdout).unwrap().trim(),
        (128u64 * 1024 * 1024).to_string()
    );
    let adopted = backend.reconcile(&fixture.registry).await.unwrap();
    assert_eq!(adopted.len(), 1);
    let control = fixture.registry.adopt_control(&fixture.provision).await;
    control.request_stop();
    timeout(
        Duration::from_secs(15),
        backend.run(
            fixture.provision.clone(),
            fixture.registry.clone(),
            control,
            true,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(result_file.is_file());
    fixture.cleanup().await;
}
