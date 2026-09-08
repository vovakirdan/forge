//! Provider-free commands in a Run-private candidate checkout. No host command lane.
use super::{
    PodmanBackend, add_mount, capacity, prepare_scoped_directory, read_private_json,
    sandbox_arguments, scoped_directory,
};
use crate::{
    RunControl, SupervisorError,
    git::GitBackend,
    journal::{GitSourceScope, scope_key},
    registry::RunRegistry,
    surface,
};
use forge_domain::{
    HookExecutionResult, HookVerdict,
    runtime::{HookRunSpec, SurfaceSpec},
};
use forge_protocol::{
    runtime::{RunnerExit, RunnerInvocation},
    supervisor::v1::ProvisionRun,
};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};

impl PodmanBackend {
    pub(super) async fn create_hook(
        &self,
        provision: &ProvisionRun,
        spec: &HookRunSpec,
        registry: &RunRegistry,
        control: &RunControl,
    ) -> Result<bool, SupervisorError> {
        let _serial = self.provisioning.lock().await;
        if control.stopped() {
            return Ok(false);
        }
        crate::execution_assignment::validate(provision)?;
        spec.validate()
            .map_err(|_| SupervisorError::InvalidRunSpec)?;
        self.preflight().await?;
        if self.inspect(provision).await?.is_some() {
            return Err(SupervisorError::ConflictingProvision);
        }
        capacity::check(&self.config, registry).await?;
        let snapshot = self.hook_snapshot(provision, spec).await?;
        let input = scoped_directory(&self.config.grants_directory, provision);
        let runtime = scoped_directory(&self.config.state_directory.join("runtime"), provision);
        let evidence = scoped_directory(&self.config.state_directory.join("evidence"), provision);
        for directory in [&input, &runtime, &evidence] {
            prepare_scoped_directory(directory)?;
        }
        let invocation = invocation(spec);
        // Solely derived from the immutable Core-owned RunSpec; no credentials,
        // provider invocation or Employee configuration are consulted.
        surface::private_write(
            &input.join("invocation.json"),
            &serde_json::to_vec(&invocation)?,
        )?;
        surface::private_write(&input.join("stdin"), b"")?;
        let workdir = if spec.hook.workdir == "." {
            "/workspace/worktree".into()
        } else {
            format!("/workspace/worktree/{}", spec.hook.workdir)
        };
        let mut args = sandbox_arguments::create(provision, &spec.hook.limits, &workdir);
        for (key, value) in self.labels(provision, spec.run_id) {
            args.extend(["--label".into(), format!("{key}={value}")]);
        }
        add_mount(&mut args, &input, "/run/forge-input", true)?;
        add_mount(&mut args, &runtime, "/run/forge", false)?;
        add_mount(&mut args, &evidence, "/run/forge-evidence", false)?;
        add_mount(&mut args, &snapshot, "/workspace", false)?;
        args.push(spec.hook.image.clone());
        if control.stopped() {
            return Ok(false);
        }
        let bytes = self.command(&args).await?;
        let id = std::str::from_utf8(&bytes)
            .map_err(|_| SupervisorError::PodmanFailed)?
            .trim();
        if !super::valid_environment_id(id) {
            return Err(SupervisorError::PodmanFailed);
        }
        registry.attach_environment(provision, id, true).await?;
        if control.stopped() {
            return Ok(false);
        }
        self.command(&["start".into(), id.into()]).await?;
        Ok(true)
    }

    async fn hook_snapshot(
        &self,
        provision: &ProvisionRun,
        spec: &HookRunSpec,
    ) -> Result<PathBuf, SupervisorError> {
        let source = GitSourceScope {
            project_id: spec.project_id,
            surface_id: spec.binding.surface_id,
            source: SurfaceSpec::GitWorktree {
                repository: spec
                    .binding
                    .source
                    .as_path()
                    .to_str()
                    .ok_or(SupervisorError::UnsafeSurface)?
                    .into(),
                base_ref: spec.binding.initial_base.as_str().into(),
            },
        };
        let worktree = surface::inspection_worktree(
            &self.config.state_directory,
            &spec.assignment.task_id.to_string(),
            &source,
        )?;
        let snapshots = self.config.state_directory.join("hook-snapshots");
        surface::private_directory(&snapshots)?;
        let destination = snapshots.join(format!(
            "{}-{}-{}",
            spec.run_id, provision.lease_fencing_token, provision.environment_epoch
        ));
        tokio::time::timeout(
            Duration::from_secs(60),
            GitBackend::default().snapshot_candidate(&worktree, &spec.candidate, &destination),
        )
        .await
        .map_err(|_| SupervisorError::SurfacePreparationFailed)?
        .map_err(|_| SupervisorError::SurfacePreparationFailed)?;
        Ok(destination)
    }

    pub(super) async fn hook_result(
        &self,
        provision: &ProvisionRun,
        spec: &HookRunSpec,
        registry: &RunRegistry,
        control: &RunControl,
        container_exit: i32,
    ) -> HookExecutionResult {
        let stop_reason = registry
            .journal
            .lock()
            .await
            .records()
            .into_iter()
            .find(|record| scope_key(&record.provision) == scope_key(provision))
            .and_then(|record| record.stop_reason);
        let evidence = scoped_directory(&self.config.state_directory.join("evidence"), provision);
        let report = read_private_json::<RunnerExit>(&evidence.join("exit.json")).ok();
        result(
            spec,
            report.as_ref(),
            container_exit,
            stop_reason.as_deref(),
            control.stopped(),
        )
    }
}

fn invocation(spec: &HookRunSpec) -> RunnerInvocation {
    RunnerInvocation {
        adapter_id: "project_hook".into(),
        program: spec.hook.command[0].clone(),
        args: spec.hook.command[1..].to_vec(),
        env: BTreeMap::from([
            ("PATH".into(), "/usr/local/bin:/usr/bin:/bin".into()),
            ("HOME".into(), "/run/forge/home".into()),
            ("XDG_CONFIG_HOME".into(), "/run/forge/config".into()),
            ("XDG_CACHE_HOME".into(), "/run/forge/cache".into()),
            ("XDG_DATA_HOME".into(), "/run/forge/data".into()),
        ]),
        managed_files: vec![],
        credential_files: vec![],
        max_output_bytes: spec.hook.max_output_bytes,
        stop_grace_seconds: spec.hook.limits.stop_grace_seconds,
        runtime_input: None,
    }
}
fn result(
    spec: &HookRunSpec,
    report: Option<&RunnerExit>,
    container_exit: i32,
    stop_reason: Option<&str>,
    stopped: bool,
) -> HookExecutionResult {
    let verdict = if stop_reason == Some("wall_limit") {
        HookVerdict::TimedOut
    } else if stopped || stop_reason.is_some() || report.is_some_and(|exit| exit.stop_requested) {
        HookVerdict::Interrupted
    } else if container_exit == 0
        && report.is_some_and(|exit| exit.exit_code == Some(0) && !exit.output_incomplete)
    {
        HookVerdict::Passed
    } else {
        HookVerdict::Failed
    };
    HookExecutionResult {
        invocation_id: spec.assignment.invocation_id,
        candidate_proposal_id: spec.assignment.candidate_proposal_id,
        candidate: spec.candidate.clone(),
        verdict,
        exit_code: report.and_then(|exit| exit.exit_code),
        output_incomplete: report.is_none_or(|exit| exit.output_incomplete),
    }
}

#[cfg(test)]
mod tests;
