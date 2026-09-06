//! Shared synthetic Run sandbox fixture and physical observation helpers.

use super::*;

pub(super) struct Fixture {
    pub(super) config: SupervisorConfig,
    pub(super) provision: ProvisionRun,
    pub(super) spec: SandboxRunSpec,
    pub(super) registry: RunRegistry,
    pub(super) _gateway: Option<UnixListener>,
}

pub(super) async fn running_fixture(
    fixture: &Fixture,
) -> tokio::task::JoinHandle<Result<(), crate::SupervisorError>> {
    let backend = PodmanBackend::new(&fixture.config);
    let control = fixture
        .registry
        .register(&fixture.provision, &fixture.config.boot_id)
        .await
        .unwrap()
        .unwrap();
    let registry = fixture.registry.clone();
    let provision = fixture.provision.clone();
    let worker =
        tokio::spawn(async move { backend.run(provision, registry, control, false).await });
    let result = fixture
        .config
        .state_directory
        .join("surfaces")
        .join(fixture.spec.surface_id.to_string())
        .join("worktree/fixture-result.txt");
    timeout(Duration::from_secs(30), async {
        while !result.exists() {
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    worker
}

pub(super) async fn request_fixture_stop(fixture: &Fixture) {
    assert!(
        fixture
            .registry
            .request_stop(&forge_protocol::supervisor::v1::StopRun {
                command_id: new_id(),
                run_id: fixture.provision.run_id.clone(),
                lease_fencing_token: fixture.provision.lease_fencing_token,
                environment_epoch: fixture.provision.environment_epoch,
                mode: forge_protocol::supervisor::v1::StopMode::Graceful as i32,
                grace_period_ms: 1000,
                reason_code: "fixture_stop".into(),
            })
            .await
    );
}

pub(super) async fn await_container_exit(fixture: &Fixture) {
    let name = format!(
        "forge-run-{}-{}-{}",
        fixture.provision.run_id,
        fixture.provision.lease_fencing_token,
        fixture.provision.environment_epoch
    );
    timeout(Duration::from_secs(15), async {
        loop {
            let output = Command::new("podman")
                .args(["inspect", "--format", "{{.State.Running}}", &name])
                .output()
                .await
                .unwrap();
            if output.status.success() && output.stdout == b"false\n" {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
}

pub(super) async fn gateway_roundtrip(fixture: &Fixture) {
    let name = format!(
        "forge-run-{}-{}-{}",
        fixture.provision.run_id,
        fixture.provision.lease_fencing_token,
        fixture.provision.environment_epoch
    );
    let mut client = Command::new("podman").args(["exec", &name, "/bin/bash", "--noprofile", "--norc", "-c",
        "exec 3<>/dev/tcp/127.0.0.1/4097; printf ping >&3; IFS= read -r reply <&3; test \"$reply\" = pong"])
        .kill_on_drop(true).spawn().unwrap();
    let (mut stream, _) = timeout(
        Duration::from_secs(5),
        fixture._gateway.as_ref().unwrap().accept(),
    )
    .await
    .unwrap()
    .unwrap();
    let mut bytes = [0; 4];
    stream.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"ping");
    stream.write_all(b"pong\n").await.unwrap();
    assert!(
        timeout(Duration::from_secs(5), client.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
}

impl Fixture {
    pub(super) async fn new(hold: bool) -> Self {
        let root = std::env::temp_dir().join(format!("fp-{}", new_id()));
        private_directory(&root).unwrap();
        let mut config = SupervisorConfig::new(
            root.join("core.sock"),
            "podman-test".into(),
            crate::current_boot_id().unwrap(),
        );
        config.state_directory = root.clone();
        config.grants_directory = root.join("grants");
        config.gateways_directory = root.join("gateways");
        let project_id = ProjectId::new();
        let mode = CredentialDeliveryMode::IsolatedRuntimeSecret;
        let profile = ExecutionProfileInput {
            id: uuid::Uuid::now_v7(),
            revision: 1,
            project_id,
            adapter_id: "codex_cli".into(),
            adapter_version: "0.153.2".into(),
            provider_id: "openai".into(),
            model: "fixture-no-inference".into(),
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
        .unwrap();
        let image = Command::new("podman")
            .args([
                "image",
                "inspect",
                "localhost/forge-runner-fixture:m1",
                "--format",
                "{{.Digest}}",
            ])
            .output()
            .await
            .unwrap();
        assert!(
            image.status.success(),
            "build infra/runtime/Containerfile.fixture first"
        );
        let digest = String::from_utf8(image.stdout).unwrap();
        let spec = SandboxRunSpec {
            schema_version: 2,
            project_id,
            surface_id: uuid::Uuid::now_v7(),
            instruction: "synthetic fixture task".into(),
            binding: RuntimeBinding {
                execution_profile: profile,
                image: format!("localhost/forge-runner-fixture@{}", digest.trim()),
                surface: SurfaceSpec::FilesystemSandbox,
                access: SurfaceAccess::ReadWrite,
                limits: ResourceLimits {
                    cpu_millis: 1000,
                    memory_bytes: 128 * 1024 * 1024,
                    pids: 64,
                    wall_seconds: 60,
                    stop_grace_seconds: 1,
                },
                budget: Default::default(),
                system_prompt: "fixture rules".into(),
                employee_prompt: "fixture employee".into(),
            },
        };
        spec.validate().unwrap();
        let mut provision = crate::tests::provision();
        provision.run_spec_version = 2;
        provision.run_spec_json = serde_json::to_string(&spec).unwrap();
        for directory in [&config.grants_directory, &config.gateways_directory] {
            private_directory(directory).unwrap();
        }
        let grant = scoped_directory(&config.grants_directory, &provision);
        private_directory(grant.parent().unwrap()).unwrap();
        private_directory(&grant).unwrap();
        let invocation = RunnerInvocation {
            adapter_id: "codex_cli".into(),
            program: "codex".into(),
            args: Vec::new(),
            env: [
                ("PATH".into(), "/usr/local/bin:/usr/bin:/bin".into()),
                (
                    "FORGE_FIXTURE_HOLD".into(),
                    if hold { "1" } else { "0" }.into(),
                ),
            ]
            .into(),
            managed_files: Vec::new(),
            credential_files: Vec::new(),
            max_output_bytes: 1024 * 1024,
            stop_grace_seconds: 1,
        };
        private_write(
            &grant.join("invocation.json"),
            &serde_json::to_vec(&invocation).unwrap(),
        )
        .unwrap();
        private_write(&grant.join("stdin"), b"fixture work\n").unwrap();
        let gateway_dir = config.gateways_directory.join(&provision.run_id);
        private_directory(&gateway_dir).unwrap();
        let gateway = UnixListener::bind(gateway_dir.join("gateway.sock")).unwrap();
        let registry = RunRegistry::new(
            Journal::open(
                &root.join("journal"),
                &config.host_id,
                config.journal_max_bytes,
            )
            .unwrap(),
        );
        Self {
            config,
            provision,
            spec,
            registry,
            _gateway: Some(gateway),
        }
    }

    pub(super) async fn cleanup(self) {
        let name = format!(
            "forge-run-{}-{}-{}",
            self.provision.run_id,
            self.provision.lease_fencing_token,
            self.provision.environment_epoch
        );
        let _ = Command::new("podman")
            .args(["rm", "--force", &name])
            .output()
            .await;
        // Only this test's UUID-scoped synthetic files are removed.
        let _ = fs::remove_dir_all(self.config.state_directory);
    }
}
