//! Rootless Podman process boundary. External environment state is inspected,
//! never inferred from a transport disconnect or a successful domain submission.

use forge_domain::runtime::{RuntimeLaunchSpec, SandboxLaunchSpec};
use forge_protocol::{
    runtime::RunnerInvocation,
    supervisor::v1::{ProvisionRun, RunEventKind},
};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{process::Command, sync::Mutex, time::sleep};

use crate::{RunControl, SupervisorConfig, SupervisorError, registry::RunRegistry, surface};

mod capacity;
mod git_inspection;
mod git_integration;
mod hook;
mod monitor;
#[cfg(test)]
mod nonstart_tests;
mod reconcile;
mod sandbox_arguments;
mod version;

#[derive(Clone)]
pub(crate) struct PodmanBackend {
    config: SupervisorConfig,
    provisioning: Arc<Mutex<()>>,
}

#[derive(Deserialize)]
struct Inspection {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "State")]
    state: ContainerState,
    #[serde(rename = "Config")]
    config: ContainerConfig,
}
#[derive(Deserialize)]
struct ContainerState {
    #[serde(rename = "Running")]
    running: bool,
    #[serde(rename = "Status")]
    status: String,
    #[serde(rename = "ExitCode")]
    exit_code: i32,
    #[serde(rename = "StartedAt")]
    started_at: String,
}
#[derive(Deserialize)]
struct ContainerConfig {
    #[serde(rename = "Labels")]
    labels: HashMap<String, String>,
}

impl PodmanBackend {
    pub fn new(config: &SupervisorConfig) -> Self {
        Self {
            config: config.clone(),
            provisioning: Arc::default(),
        }
    }

    pub async fn run(
        &self,
        provision: ProvisionRun,
        registry: RunRegistry,
        control: Arc<RunControl>,
        adopt: bool,
    ) -> Result<(), SupervisorError> {
        crate::execution_assignment::validate(&provision)?;
        let spec = match serde_json::from_str::<SandboxLaunchSpec>(&provision.run_spec_json)
            .ok()
            .filter(|spec| {
                spec.validate().is_ok()
                    && u32::from(spec.schema_version()) == provision.run_spec_version
            }) {
            Some(spec) => spec,
            None if adopt => {
                // Startup reconciliation validates retained specs before any
                // worker is adopted. A corrupt contract is never non-start proof.
                registry.mark_unknown(&provision).await?;
                return Err(SupervisorError::InvalidRunSpec);
            }
            None => {
                self.finish_without_start(&provision, &registry, Some("invalid_sandbox_spec"))
                    .await?;
                return Err(SupervisorError::InvalidRunSpec);
            }
        };
        if !adopt {
            loop {
                if control.stopped() {
                    return self.finish_without_start(&provision, &registry, None).await;
                }
                if registry
                    .emit(
                        &provision,
                        RunEventKind::Provisioning,
                        json!({"executor":"rootless_podman"}),
                    )
                    .await
                    .is_ok()
                {
                    break;
                }
                sleep(Duration::from_secs(1)).await;
            }
            let creation = match &spec {
                SandboxLaunchSpec::Provider(spec) => {
                    self.create(&provision, spec, &registry, &control).await
                }
                SandboxLaunchSpec::Hook(spec) => {
                    self.create_hook(&provision, spec, &registry, &control)
                        .await
                }
            };
            match creation {
                Ok(false) => return self.finish_without_start(&provision, &registry, None).await,
                Ok(true) => {}
                Err(error) => {
                    // A failed CLI call can leave a created or running container.
                    // Only a successful absent check proves this was a non-start.
                    match self.inspect(&provision).await {
                        Ok(None) => {
                            let reason = match &error {
                                SupervisorError::EvidenceCapacity => "evidence_capacity_exhausted",
                                SupervisorError::RootlessUnavailable => {
                                    "rootless_sandbox_unavailable"
                                }
                                SupervisorError::RuntimePreflightFailed => {
                                    "runtime_image_preflight_failed"
                                }
                                SupervisorError::SurfacePreparationFailed
                                | SupervisorError::UnsafeSurface => "unsafe_or_failed_work_surface",
                                _ => "sandbox_provision_failed",
                            };
                            self.finish_without_start(&provision, &registry, Some(reason))
                                .await?;
                        }
                        _ => {
                            let _ = registry
                                .emit(
                                    &provision,
                                    RunEventKind::EnvironmentLost,
                                    json!({"reason_code":"sandbox_provision_unconfirmed"}),
                                )
                                .await;
                            let _ = registry.mark_unknown(&provision).await;
                            tracing::warn!(run_id = %provision.run_id, error = %error,
                            "provision result is uncertain; continuing physical observation");
                            return self.observe(&provision, &spec, &registry, &control).await;
                        }
                    }
                    return Err(error);
                }
            }
        }
        self.observe(&provision, &spec, &registry, &control).await
    }

    async fn create(
        &self,
        provision: &ProvisionRun,
        spec: &RuntimeLaunchSpec,
        registry: &RunRegistry,
        control: &RunControl,
    ) -> Result<bool, SupervisorError> {
        let _serial = self.provisioning.lock().await;
        if control.stopped() {
            return Ok(false);
        }
        self.preflight().await?;
        if self.inspect(provision).await?.is_some() {
            return Err(SupervisorError::ConflictingProvision);
        }
        version::verify(&self.config.podman_binary, spec).await?;
        if control.stopped() {
            return Ok(false);
        }
        let grant = scoped_directory(&self.config.grants_directory, provision);
        let invocation: RunnerInvocation = read_private_json(&grant.join("invocation.json"))?;
        if invocation.adapter_id != spec.binding.execution_profile.adapter_id() {
            return Err(SupervisorError::InvalidRunSpec);
        }
        validate_program(&invocation)?;
        let live = matches!(spec.schema_version, 2 | 3)
            && spec
                .binding
                .execution_profile
                .capability_profile()
                .capabilities
                .contains(&forge_domain::RuntimeCapability::LiveInput);
        if live != invocation.runtime_input.is_some()
            || invocation.runtime_input.as_ref().is_some_and(|input| {
                !matches!(
                    invocation.adapter_id.as_str(),
                    "claude_code_cli" | "codex_cli" | "opencode_runtime"
                ) || input.run_id != provision.run_id
                    || input.fencing_token != provision.lease_fencing_token
                    || input.environment_epoch != provision.environment_epoch
            })
        {
            return Err(SupervisorError::InvalidRunSpec);
        }
        if invocation.max_output_bytes > spec.binding.budget.max_output_bytes {
            return Err(SupervisorError::InvalidRunSpec);
        }
        capacity::check(&self.config, registry).await?;
        let surface = surface::prepare(&self.config.state_directory, provision, spec).await?;
        let runtime = scoped_directory(&self.config.state_directory.join("runtime"), provision);
        let evidence = scoped_directory(&self.config.state_directory.join("evidence"), provision);
        prepare_scoped_directory(&runtime)?;
        prepare_scoped_directory(&evidence)?;
        let gateway = self.config.gateways_directory.join(&provision.run_id);
        surface::private_directory(&gateway)?;
        let limits = &spec.binding.limits;
        let mut args = sandbox_arguments::create(provision, limits, surface.workdir);
        for (key, value) in self.labels(provision, spec.surface_id) {
            args.extend(["--label".into(), format!("{key}={value}")]);
        }
        add_mount(&mut args, &grant, "/run/forge-input", true)?;
        add_mount(&mut args, &runtime, "/run/forge", false)?;
        add_mount(&mut args, &evidence, "/run/forge-evidence", false)?;
        // Socket entries must be replaceable after Core restart. Binding the
        // inode itself strands an adopted container on the old socket forever.
        add_mount(&mut args, &gateway, "/run/forge-gateway", true)?;
        for credential in &invocation.credential_files {
            let file = match (invocation.adapter_id.as_str(), credential.source.as_str()) {
                ("codex_cli", "/run/forge-secrets/auth.json") => "auth.json",
                ("opencode_runtime", "/run/forge-secrets/api-key") => "api-key",
                ("claude_code_cli", "/run/forge-secrets/claude-setup-token")
                    if credential.target == "/run/forge/claude-home/setup-token"
                        && !credential.writeback =>
                {
                    "claude-setup-token"
                }
                _ => return Err(SupervisorError::InvalidRunSpec),
            };
            add_mount(&mut args, &grant.join(file), &credential.source, true)?;
        }
        if let Some(path) = surface.mount {
            add_mount(&mut args, &path, "/workspace", surface.read_only)?;
        } else {
            args.extend([
                "--tmpfs".into(),
                "/workspace:rw,nosuid,nodev,mode=1777,size=268435456".into(),
            ]);
        }
        args.push(spec.binding.image.clone());
        if control.stopped() {
            return Ok(false);
        }
        let output = self.command(&args).await?;
        let id = String::from_utf8(output).map_err(|_| SupervisorError::PodmanFailed)?;
        let id = id.trim();
        if id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(SupervisorError::PodmanFailed);
        }
        registry.attach_environment(provision, id, true).await?;
        if control.stopped() {
            return Ok(false);
        }
        self.command(&["start".into(), id.into()]).await?;
        Ok(true)
    }

    async fn finish_without_start(
        &self,
        provision: &ProvisionRun,
        registry: &RunRegistry,
        failure_reason: Option<&str>,
    ) -> Result<(), SupervisorError> {
        // Retain the control until both the observation and physical proof are
        // durable; a full journal must not forget a requested stop.
        let mut announced = false;
        let mut failure_announced = failure_reason.is_none();
        loop {
            if !failure_announced {
                failure_announced = registry
                    .emit(
                        provision,
                        RunEventKind::StartFailed,
                        json!({"reason_code":failure_reason}),
                    )
                    .await
                    .is_ok();
            }
            if failure_announced && !announced {
                announced = registry
                    .emit(
                        provision,
                        RunEventKind::Stopped,
                        json!({"reason_code":if failure_reason.is_some() {
                            "sandbox_not_started_confirmed"
                        } else {
                            "stop_before_container_start"
                        },"evidence_not_started":true}),
                    )
                    .await
                    .is_ok();
            }
            if announced && registry.finish(provision, true).await.is_ok() {
                return Ok(());
            }
            sleep(Duration::from_secs(1)).await;
        }
    }

    async fn preflight(&self) -> Result<(), SupervisorError> {
        let output = self
            .command(&[
                "info".into(),
                "--format".into(),
                "{{.Host.Security.Rootless}} {{.Host.CgroupsVersion}}".into(),
            ])
            .await?;
        if String::from_utf8_lossy(&output).trim() != "true v2" {
            return Err(SupervisorError::RootlessUnavailable);
        }
        Ok(())
    }

    async fn inspect(
        &self,
        provision: &ProvisionRun,
    ) -> Result<Option<Inspection>, SupervisorError> {
        let name = environment_name(provision);
        let exists = tokio::time::timeout(
            Duration::from_secs(30),
            Command::new(&self.config.podman_binary)
                .args(["container", "exists", &name])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| SupervisorError::PodmanFailed)??;
        if exists.status.code() == Some(1) {
            return Ok(None);
        }
        if !exists.status.success() {
            return Err(SupervisorError::PodmanFailed);
        }
        let bytes = self.command(&["inspect".into(), name]).await?;
        let mut values: Vec<Inspection> = serde_json::from_slice(&bytes)?;
        let value = values.pop().ok_or(SupervisorError::PodmanFailed)?;
        for (key, expected) in [
            ("forge.host", &self.config.host_id),
            ("forge.run", &provision.run_id),
        ] {
            if value.config.labels.get(key) != Some(expected) {
                return Err(SupervisorError::ConflictingProvision);
            }
        }
        if value.config.labels.get("forge.epoch") != Some(&provision.environment_epoch.to_string())
            || value.config.labels.get("forge.fence")
                != Some(&provision.lease_fencing_token.to_string())
        {
            return Err(SupervisorError::ConflictingProvision);
        }
        Ok(Some(value))
    }

    async fn signal(&self, id: &str, signal: &str) -> Result<(), SupervisorError> {
        self.command(&["kill".into(), "--signal".into(), signal.into(), id.into()])
            .await
            .map(|_| ())
    }

    async fn command(&self, args: &[String]) -> Result<Vec<u8>, SupervisorError> {
        let output = tokio::time::timeout(
            Duration::from_secs(30),
            Command::new(&self.config.podman_binary)
                .args(args)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| SupervisorError::PodmanFailed)??;
        if !output.status.success() {
            #[cfg(test)]
            eprintln!(
                "fixture Podman {} failed: {}",
                args.first().map_or("", String::as_str),
                String::from_utf8_lossy(&output.stderr)
            );
            return Err(SupervisorError::PodmanFailed);
        }
        Ok(output.stdout)
    }

    fn labels(
        &self,
        provision: &ProvisionRun,
        surface_id: uuid::Uuid,
    ) -> Vec<(&'static str, String)> {
        vec![
            ("forge.managed", "true".into()),
            ("forge.host", self.config.host_id.clone()),
            ("forge.boot", self.config.boot_id.clone()),
            ("forge.run", provision.run_id.clone()),
            ("forge.fence", provision.lease_fencing_token.to_string()),
            ("forge.epoch", provision.environment_epoch.to_string()),
            ("forge.surface", surface_id.to_string()),
        ]
    }
}

fn environment_name(provision: &ProvisionRun) -> String {
    format!(
        "forge-run-{}-{}-{}",
        provision.run_id, provision.lease_fencing_token, provision.environment_epoch
    )
}

pub(crate) fn scoped_directory(root: &Path, provision: &ProvisionRun) -> PathBuf {
    root.join(&provision.run_id)
        .join(provision.environment_epoch.to_string())
}

fn prepare_scoped_directory(path: &Path) -> Result<(), SupervisorError> {
    let parent = path.parent().ok_or(SupervisorError::UnsafeSurface)?;
    let root = parent.parent().ok_or(SupervisorError::UnsafeSurface)?;
    surface::private_directory(root)?;
    surface::private_directory(parent)?;
    surface::private_directory(path)
}

fn add_mount(
    args: &mut Vec<String>,
    source: &Path,
    target: &str,
    read_only: bool,
) -> Result<(), SupervisorError> {
    let source = source
        .to_str()
        .filter(|path| path.starts_with('/') && !path.contains([',', '\n', '\0']))
        .ok_or(SupervisorError::UnsafeSurface)?;
    args.extend([
        "--mount".into(),
        format!(
            "type=bind,src={source},target={target},{}",
            if read_only { "ro" } else { "rw" }
        ),
    ]);
    Ok(())
}

fn read_private_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, SupervisorError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != nix::unistd::Uid::effective().as_raw()
        || metadata.len() > 1024 * 1024
    {
        return Err(SupervisorError::UnsafeSurface);
    }
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

fn validate_program(invocation: &RunnerInvocation) -> Result<(), SupervisorError> {
    if invocation.adapter_id == "claude_code_cli" && invocation.program != "forge-claude-driver" {
        return Err(SupervisorError::InvalidRunSpec);
    }
    let expected = match invocation.adapter_id.as_str() {
        "codex_cli" if invocation.runtime_input.is_some() => "forge-codex-driver",
        "codex_cli" => "codex",
        "opencode_runtime" => "forge-opencode-driver",
        "claude_code_cli" => "forge-claude-driver",
        _ => return Err(SupervisorError::InvalidRunSpec),
    };
    if Path::new(&invocation.program)
        .file_name()
        .and_then(|name| name.to_str())
        != Some(expected)
        || invocation.max_output_bytes == 0
        || invocation.stop_grace_seconds == 0
    {
        return Err(SupervisorError::InvalidRunSpec);
    }
    Ok(())
}

fn valid_environment_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}
