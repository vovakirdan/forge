use super::super::PodmanBackend;
use crate::{
    SupervisorConfig,
    journal::Journal,
    registry::RunRegistry,
    surface::{prepare, private_directory},
};
use forge_domain::{
    CapabilityProfile, CredentialBinding, CredentialDeliveryMode, ExecutionProfileInput, ProjectId,
    RuntimeCapability, TransportEngine,
    runtime::{ResourceLimits, RuntimeBinding, SandboxRunSpec, SurfaceAccess, SurfaceSpec},
};
use forge_protocol::supervisor::v1::{
    AcknowledgementDisposition, CoreAcknowledgement, GitCandidateInspectionResult,
    InspectGitCandidate, ProvisionRun, RunEventKind, supervisor_to_core,
};
use serde_json::json;
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use uuid::Uuid;

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(in crate::podman) struct Fixture {
    _directory: Directory,
    pub config: SupervisorConfig,
    pub registry: RunRegistry,
    pub backend: PodmanBackend,
    pub provision: ProvisionRun,
    pub spec: SandboxRunSpec,
    pub worktree: PathBuf,
    pub request: InspectGitCandidate,
}

impl Fixture {
    pub async fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("forge-candidate-inspection-{}", Uuid::now_v7()));
        private_directory(&root).unwrap();
        let source = root.join("source");
        private_directory(&source).unwrap();
        git(&source, &["init", "--initial-branch=main"]).await;
        fs::write(source.join("README.md"), "Synthetic initial project\n").unwrap();
        git(&source, &["add", "README.md"]).await;
        git(&source, &["commit", "-m", "Initial"]).await;
        let base = git(&source, &["rev-parse", "HEAD"]).await;
        let stub = root.join("podman-inspection-stub");
        fs::write(&stub, b"#!/bin/sh\nroot=$(dirname \"$0\")\n[ ! -e \"$root/error\" ] || exit 125\ncase \"$1\" in\ncontainer) test ! -e \"$root/absent\";;\ninspect) [ ! -e \"$root/slow\" ] || sleep 0.2; exec /bin/cat \"$root/inspection.json\";;\n*) exit 125;;\nesac\n").unwrap();
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o700)).unwrap();
        let mut config = SupervisorConfig::new(
            root.join("core.sock"),
            "test-host".into(),
            "test-boot".into(),
        );
        config.podman_binary = stub;
        let mut spec = valid_spec();
        spec.binding.surface = SurfaceSpec::GitWorktree {
            repository: source.to_str().unwrap().into(),
            base_ref: base,
        };
        let mut provision = crate::tests::provision();
        provision.run_spec_version = 2;
        provision.run_spec_json = serde_json::to_string(&spec).unwrap();
        let registry = RunRegistry::new(
            Journal::open(
                &config.state_directory,
                &config.host_id,
                config.journal_max_bytes,
            )
            .unwrap(),
        );
        registry
            .register(&provision, &config.boot_id)
            .await
            .unwrap()
            .unwrap();
        prepare(&config.state_directory, &provision, &spec.clone().into())
            .await
            .unwrap();
        let worktree = config
            .state_directory
            .join("surfaces")
            .join(spec.surface_id.to_string())
            .join("worktree");
        fs::write(worktree.join("result.txt"), "Employee implementation\n").unwrap();
        git(&worktree, &["add", "result.txt"]).await;
        git(&worktree, &["commit", "-m", "Employee work"]).await;
        let commit = git(&worktree, &["rev-parse", "HEAD"]).await;
        let request = InspectGitCandidate {
            command_id: crate::new_id(),
            run_id: provision.run_id.clone(),
            lease_fencing_token: provision.lease_fencing_token,
            environment_epoch: provision.environment_epoch,
            project_id: spec.project_id.to_string(),
            surface_id: spec.surface_id.to_string(),
            expected_commit: commit,
            host_id: config.host_id.clone(),
            boot_id: config.boot_id.clone(),
        };
        let fixture = Self {
            _directory: Directory(root),
            backend: PodmanBackend::new(&config),
            config,
            registry,
            provision,
            spec,
            worktree,
            request,
        };
        fixture.physical_state(false, "exited", &"a".repeat(64));
        fixture
            .registry
            .attach_environment(&fixture.provision, &"a".repeat(64), true)
            .await
            .unwrap();
        fixture
            .registry
            .emit(
                &fixture.provision,
                RunEventKind::Stopped,
                json!({"reason_code":"fixture_exited"}),
            )
            .await
            .unwrap();
        fixture
            .registry
            .finish(&fixture.provision, true)
            .await
            .unwrap();
        let messages: Vec<_> = fixture
            .registry
            .journal
            .lock()
            .await
            .pending()
            .cloned()
            .collect();
        for message in messages {
            fixture
                .acknowledge(crate::journal::message_id(&message).unwrap())
                .await;
        }
        fixture
    }

    pub fn physical_state(&self, running: bool, status: &str, id: &str) {
        let value = json!([{"Id":id,"State":{"Running":running,"Status":status,"ExitCode":0,"StartedAt":"2026-09-06T10:00:00Z"},"Config":{"Labels":{
            "forge.host":self.config.host_id,"forge.boot":self.config.boot_id,"forge.run":self.provision.run_id,
            "forge.epoch":self.provision.environment_epoch.to_string(),"forge.fence":self.provision.lease_fencing_token.to_string(),"forge.surface":self.spec.surface_id.to_string()
        }}}]);
        fs::write(
            self._directory.0.join("inspection.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
    }

    pub fn mark(&self, file: &str) {
        fs::write(self._directory.0.join(file), b"fixture").unwrap();
    }

    pub async fn inspect(&self, request: InspectGitCandidate) -> GitCandidateInspectionResult {
        self.backend
            .inspect_git_candidate(request.clone(), self.registry.clone())
            .await
            .unwrap();
        self.registry
            .journal
            .lock()
            .await
            .pending()
            .find_map(|message| match &message.message {
                Some(supervisor_to_core::Message::GitCandidateInspection(result))
                    if result.request_command_id == request.command_id =>
                {
                    Some(result.clone())
                }
                _ => None,
            })
            .expect("durable inspection result")
    }

    pub async fn acknowledge(&self, message_id: &str) {
        self.registry
            .journal
            .lock()
            .await
            .acknowledge(&CoreAcknowledgement {
                message_id: crate::new_id(),
                acknowledged_message_id: message_id.into(),
                disposition: AcknowledgementDisposition::Accepted as i32,
                reason_code: String::new(),
                message: String::new(),
            })
            .unwrap();
    }

    pub async fn restart(&mut self) {
        // Drop every old journal lock before reopening the same retained operational state.
        let replacement = RunRegistry::new(
            Journal::open(
                &self._directory.0.join("temporary-journal"),
                &self.config.host_id,
                self.config.journal_max_bytes,
            )
            .unwrap(),
        );
        drop(std::mem::replace(&mut self.registry, replacement));
        self.registry = RunRegistry::new(
            Journal::open(
                &self.config.state_directory,
                &self.config.host_id,
                self.config.journal_max_bytes,
            )
            .unwrap(),
        );
    }
}

pub(in crate::podman) async fn git(directory: &Path, args: &[&str]) -> String {
    let output = tokio::process::Command::new("/usr/bin/git")
        .env_clear()
        .envs([
            ("PATH", "/usr/bin:/bin"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ])
        .current_dir(directory)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgSign=false",
        ])
        .args(args)
        .output()
        .await
        .unwrap();
    assert!(output.status.success(), "synthetic Git command failed");
    String::from_utf8(output.stdout).unwrap().trim().into()
}

fn valid_spec() -> SandboxRunSpec {
    let project_id = ProjectId::new();
    let mode = CredentialDeliveryMode::IsolatedRuntimeSecret;
    let execution_profile = ExecutionProfileInput {
        id: Uuid::now_v7(),
        revision: 1,
        project_id,
        adapter_id: "codex_cli".into(),
        adapter_version: "0.153.2".into(),
        provider_id: "openai".into(),
        model: "synthetic-no-inference".into(),
        credential_binding: CredentialBinding {
            id: Uuid::now_v7(),
            project_id,
            secret_id: Uuid::now_v7(),
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
    SandboxRunSpec {
        schema_version: 2,
        project_id,
        surface_id: Uuid::now_v7(),
        instruction: "Synthetic no-inference task".into(),
        binding: RuntimeBinding {
            execution_profile,
            image: format!("localhost/synthetic@sha256:{}", "a".repeat(64)),
            surface: SurfaceSpec::FilesystemSandbox,
            access: SurfaceAccess::ReadWrite,
            limits: ResourceLimits::default(),
            budget: Default::default(),
            system_prompt: "Synthetic rules".into(),
            employee_prompt: "Synthetic Employee".into(),
        },
    }
}
