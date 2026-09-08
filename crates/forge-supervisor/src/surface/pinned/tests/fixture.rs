use super::super::*;
use crate::surface::{prepare as prepare_surface, private_directory};
use forge_domain::{
    CapabilityProfile, CredentialBinding, CredentialDeliveryMode, ExecutionProfileInput, ProjectId,
    RuntimeCapability, TransportEngine,
    git::{GitCandidate, GitObjectId},
    runtime::{ResourceLimits, RuntimeBinding, SandboxRunSpec},
};
use std::{collections::BTreeSet, fs, path::PathBuf};
use uuid::Uuid;

pub(crate) struct Fixture {
    pub root: PathBuf,
    pub source: PathBuf,
    pub writer: PathBuf,
    pub manifest: PathBuf,
    pub provision: ProvisionRun,
    pub spec: RuntimeLaunchSpec,
    pub candidate: GitCandidate,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    pub async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("forge-pinned-surface-{}", Uuid::now_v7()));
        private_directory(&root).unwrap();
        let source = root.join("source");
        private_directory(&source).unwrap();
        git(&source, &["init", "--initial-branch=main"]).await;
        fs::write(source.join("tracked.txt"), "original\n").unwrap();
        fs::write(source.join(".gitignore"), "ignored.txt\n").unwrap();
        git(&source, &["add", "."]).await;
        git(&source, &["commit", "-m", "Original fixture"]).await;
        let base = git(&source, &["rev-parse", "HEAD"]).await;
        let mut spec = valid_spec();
        spec.binding.surface = SurfaceSpec::GitWorktree {
            repository: source.to_str().unwrap().into(),
            base_ref: base.clone(),
        };
        let mut provision = crate::tests::provision();
        provision.run_spec_version = 2;
        provision.run_spec_json = serde_json::to_string(&spec).unwrap();
        let writer = prepare_surface(&root, &provision, &spec.clone().into())
            .await
            .unwrap()
            .mount
            .unwrap()
            .join("worktree");
        fs::write(writer.join("tracked.txt"), "accepted revision\n").unwrap();
        git(&writer, &["add", "tracked.txt"]).await;
        git(&writer, &["commit", "-m", "Accepted employee work"]).await;
        let candidate = GitCandidate {
            commit: GitObjectId::new(git(&writer, &["rev-parse", "HEAD"]).await).unwrap(),
            tree: GitObjectId::new(git(&writer, &["rev-parse", "HEAD^{tree}"]).await).unwrap(),
        };
        let manifest = root
            .join("surface-manifests")
            .join(format!("{}.json", spec.surface_id));
        spec.binding.surface = SurfaceSpec::GitCandidateSnapshot {
            repository: source.to_str().unwrap().into(),
            base_ref: base,
            candidate: candidate.clone(),
        };
        spec.binding.access = SurfaceAccess::ReadOnly;
        provision.run_id = crate::new_id();
        provision.command_id = crate::new_id();
        provision.stage_id = "review".into();
        provision.run_spec_json = serde_json::to_string(&spec).unwrap();
        Self {
            root,
            source,
            writer,
            manifest,
            provision,
            spec: spec.into(),
            candidate,
        }
    }
    pub async fn prepare(&self) -> Result<PreparedSurface, SupervisorError> {
        prepare_surface(&self.root, &self.provision, &self.spec).await
    }
}

pub(crate) async fn git(path: &Path, args: &[&str]) -> String {
    let output = tokio::process::Command::new("/usr/bin/git")
        .env_clear()
        .envs([
            ("PATH", "/usr/bin:/bin"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_AUTHOR_NAME", "Fixture"),
            ("GIT_AUTHOR_EMAIL", "fixture@example.invalid"),
            ("GIT_COMMITTER_NAME", "Fixture"),
            ("GIT_COMMITTER_EMAIL", "fixture@example.invalid"),
        ])
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .current_dir(path)
        .output()
        .await
        .unwrap();
    assert!(output.status.success(), "synthetic Git command failed");
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end_matches('\n')
        .into()
}

fn valid_spec() -> SandboxRunSpec {
    let project_id = ProjectId::new();
    let mode = CredentialDeliveryMode::IsolatedRuntimeSecret;
    SandboxRunSpec {
        schema_version: 2,
        project_id,
        surface_id: Uuid::now_v7(),
        instruction: "Synthetic pinned snapshot".into(),
        binding: RuntimeBinding {
            surface: SurfaceSpec::None,
            access: SurfaceAccess::ReadWrite,
            limits: ResourceLimits::default(),
            budget: Default::default(),
            image: format!("localhost/synthetic@sha256:{}", "0".repeat(64)),
            system_prompt: "Synthetic fixture".into(),
            employee_prompt: "Review the pinned revision".into(),
            execution_profile: ExecutionProfileInput {
                id: Uuid::now_v7(),
                revision: 1,
                project_id,
                adapter_id: "codex_cli".into(),
                adapter_version: "0.153.2".into(),
                provider_id: "openai".into(),
                model: "synthetic-no-inference".into(),
                credential_delivery: mode,
                credential_binding: CredentialBinding {
                    id: Uuid::now_v7(),
                    project_id,
                    secret_id: Uuid::now_v7(),
                    account_id: Some("synthetic".into()),
                    allowed_delivery_modes: BTreeSet::from([mode]),
                },
                capability_profile: CapabilityProfile {
                    adapter_id: "codex_cli".into(),
                    adapter_version: "0.153.2".into(),
                    transport_engine: TransportEngine::CliWrapper,
                    credential_exposed_to_run: true,
                    capabilities: BTreeSet::from([
                        RuntimeCapability::ControlledStop,
                        RuntimeCapability::NativeMcp,
                    ]),
                },
            }
            .try_into()
            .unwrap(),
        },
    }
}
