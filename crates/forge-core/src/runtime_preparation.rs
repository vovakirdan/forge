//! Host-side composition; provider executables run only inside Supervisor sandboxes.

use crate::{CoreError, CoreService, GatewayHandle, credentials::credential_error};
use forge_domain::runtime::RuntimeLaunchSpec;
use forge_protocol::runtime::{RunnerCredentialFile, RunnerInvocation, RunnerManagedFile};
use forge_provider_common::{PrivateMaterialization, SecretBytes, adapter::PreparedInvocation};
use forge_storage::RunProjection;
use std::{
    collections::HashMap,
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
};
use tokio::sync::Mutex;
use uuid::Uuid;

pub(crate) struct ExecutionRuntime {
    pub root: PathBuf,
    gateways: Mutex<HashMap<Uuid, GatewayHandle>>,
}

impl ExecutionRuntime {
    pub fn new(root: PathBuf) -> Result<Self, CoreError> {
        // Linux sockaddr_un permits 107 pathname bytes plus its terminating NUL.
        // Reject unusable configuration before any Run creates credentials.
        if root
            .join("gateways")
            .join(Uuid::nil().to_string())
            .join("gateway.sock")
            .as_os_str()
            .len()
            >= 108
        {
            return Err(CoreError::InvalidTransport {
                field: "execution_root",
                reason:
                    "path is too long for per-Run Linux Unix sockets; select a shorter state root"
                        .into(),
            });
        }
        secure_directory(&root)?;
        for name in ["grants", "gateways", "runtime"] {
            secure_directory(&root.join(name))?;
        }
        Ok(Self {
            root,
            gateways: Mutex::new(HashMap::new()),
        })
    }
}

impl CoreService {
    pub(crate) fn prepared_traceparent(
        &self,
        run_id: Uuid,
        epoch: u64,
    ) -> Result<String, CoreError> {
        let root = &self.execution.as_ref().ok_or_else(credential_error)?.root;
        let path = root
            .join("grants")
            .join(run_id.to_string())
            .join(epoch.to_string())
            .join("traceparent");
        let bytes = PrivateMaterialization::open(&path)
            .and_then(|file| file.read(64))
            .map_err(|_| credential_error())?;
        let value = std::str::from_utf8(bytes.expose()).map_err(|_| credential_error())?;
        forge_protocol::trace_context::TraceContext::parse(value)
            .map(|context| context.header())
            .ok_or_else(credential_error)
    }
    /// Idempotent immutable materialization. A different existing file fails closed.
    /// This is safe to repeat after reconciliation; it never starts a provider.
    pub(crate) async fn prepare_run(&self, run_id: Uuid) -> Result<(), CoreError> {
        let started = std::time::Instant::now();
        let parent = crate::observability::current_traceparent();
        let result = crate::observability::trace_operation(
            Some(&parent),
            crate::observability::Operation::Adapter,
            self.prepare_run_inner(run_id),
        )
        .await;
        self.record_operation(
            crate::observability::Operation::Adapter,
            result.is_ok(),
            started.elapsed(),
        );
        result
    }

    async fn prepare_run_inner(&self, run_id: Uuid) -> Result<(), CoreError> {
        let execution = self.execution.as_ref().ok_or_else(credential_error)?;
        let run = self
            .store
            .load_run(run_id)
            .await?
            .ok_or(CoreError::NotFound { aggregate: "run" })?;
        let mut spec: RuntimeLaunchSpec =
            serde_json::from_value(run.run_spec.clone()).map_err(|_| credential_error())?;
        spec.validate().map_err(|_| credential_error())?;
        self.prepare_file_inputs(&run, &spec).await?;
        // Resolution is one-shot even when this Employee's Task profile supports
        // native input. This is an effective launch copy, never a binding edit.
        apply_purpose_profile(&mut spec)?;
        if !spec.file_inputs.is_empty() {
            spec.instruction.push_str("\n\nForge file inputs: immutable snapshots are mounted read-only at /run/forge-inputs/<artifact_id>/files, with manifest.json beside files. See /run/forge-inputs/inputs.json for identities. These are input materials, not Git candidates or accepted stage results. Copy selected files into your own workspace only when the task calls for it; no automatic overlay is performed.");
        }
        let mut transaction = self.store.begin().await?;
        let scope = forge_domain::runtime::RunScope {
            run_id,
            fencing_token: run.lease_fencing_token,
            environment_epoch: run.environment_epoch,
        };
        if !transaction.gateway_scope_is_active(scope).await? {
            return Err(credential_error());
        }
        let record = transaction
            .run_credential(run_id)
            .await?
            .ok_or_else(credential_error)?;
        transaction.commit().await?;
        let auth = self.open_runtime_credential(
            &record,
            spec.binding.execution_profile.credential_binding(),
            spec.binding.execution_profile.adapter_id(),
        )?;
        let prompt = SecretBytes::new(
            format!(
                "{}\n\n{}\n\n{}",
                spec.binding.system_prompt, spec.binding.employee_prompt, spec.instruction
            )
            .into_bytes(),
        );
        // Match Supervisor's surface layout. CLI directory flags override the
        // container's working directory, so /workspace would start outside the
        // checkout for both Git and filesystem surfaces.
        let workdir = container_workdir(&spec.binding.surface);
        let (prepared, secret, secret_name) = match spec.binding.execution_profile.adapter_id() {
            "codex_cli" => (
                forge_provider_codex::CodexAdapter::prepare(forge_provider_codex::CodexRunInput {
                    profile: &spec.binding.execution_profile,
                    prompt,
                    workdir,
                    gateway_url: "http://127.0.0.1:4097/mcp",
                    proxy_url: "http://127.0.0.1:4098",
                })
                .map_err(|_| credential_error())?,
                auth,
                "auth.json",
            ),
            "opencode_runtime" => {
                let (key, alias) = self.prepare_proxy_key(&run, &spec, &record, &auth).await?;
                let prepared = forge_provider_opencode::OpenCodeAdapter::prepare(
                    forge_provider_opencode::OpenCodeRunInput {
                        profile: &spec.binding.execution_profile,
                        prompt,
                        workdir,
                        gateway_url: "http://127.0.0.1:4097/mcp",
                        inference_url: "http://127.0.0.1:4098/v1",
                        inference_model: &alias,
                    },
                )
                .map_err(|_| credential_error())?;
                (prepared, key, "api-key")
            }
            "claude_code_cli" => (
                forge_provider_claude::ClaudeAdapter::prepare(
                    forge_provider_claude::ClaudeRunInput {
                        profile: &spec.binding.execution_profile,
                        prompt,
                        workdir,
                        gateway_url: "http://127.0.0.1:4097/mcp",
                        proxy_url: "http://127.0.0.1:4098",
                        // Stable per-Run identities make preparation retries byte
                        // identical. This path delivers one turn and then stdin EOF.
                        session_id: run.id,
                        message_id: run.id,
                    },
                )
                .map_err(|_| credential_error())?,
                auth,
                "claude-setup-token",
            ),
            _ => {
                return Err(CoreError::InvalidTransport {
                    field: "runtime",
                    reason: "runtime route is not configured".into(),
                });
            }
        };
        // An external key operation can overlap a stop. Do not materialize or
        // reopen the Gateway after authority was withdrawn; revoke its intent.
        let mut authorization = self.store.begin().await?;
        if !authorization.gateway_scope_is_active(scope).await? {
            authorization.commit().await?;
            self.revoke_run_proxy_key(&run).await?;
            return Err(credential_error());
        }
        authorization.commit().await?;
        materialize(execution, &run, &spec, prepared, &secret, secret_name)?;
        let mut gateways = execution.gateways.lock().await;
        if let std::collections::hash_map::Entry::Vacant(entry) = gateways.entry(run_id) {
            entry.insert(
                self.serve_run_gateway(run_id, &execution.root.join("gateways"))
                    .await?,
            );
        }
        Ok(())
    }

    pub(crate) async fn close_run_gateway(&self, run_id: Uuid) {
        if let Some(execution) = &self.execution {
            let handle = execution.gateways.lock().await.remove(&run_id);
            if let Some(handle) = handle {
                handle.shutdown().await;
            }
        }
    }
}

fn apply_purpose_profile(spec: &mut RuntimeLaunchSpec) -> Result<(), CoreError> {
    if spec.resolution.is_some() {
        let mut profile: forge_domain::ExecutionProfileInput =
            spec.binding.execution_profile.clone().into();
        profile
            .capability_profile
            .capabilities
            .retain(|capability| *capability != forge_domain::RuntimeCapability::LiveInput);
        spec.binding.execution_profile = profile.try_into().map_err(|_| credential_error())?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "runtime_preparation/resolution_tests.rs"]
mod resolution_tests;

fn container_workdir(surface: &forge_domain::runtime::SurfaceSpec) -> &'static str {
    match surface {
        forge_domain::runtime::SurfaceSpec::None => "/workspace",
        forge_domain::runtime::SurfaceSpec::FilesystemSandbox
        | forge_domain::runtime::SurfaceSpec::GitWorktree { .. }
        | forge_domain::runtime::SurfaceSpec::GitCandidateSnapshot { .. } => "/workspace/worktree",
        forge_domain::runtime::SurfaceSpec::GitUnborn { .. }
        | forge_domain::runtime::SurfaceSpec::GitUnbornCandidateSnapshot { .. } => {
            "/workspace/worktree"
        }
    }
}

fn materialize(
    execution: &ExecutionRuntime,
    run: &RunProjection,
    spec: &RuntimeLaunchSpec,
    prepared: PreparedInvocation,
    secret: &SecretBytes,
    secret_name: &str,
) -> Result<(), CoreError> {
    let parent = execution.root.join("grants").join(run.id.to_string());
    secure_directory(&parent)?;
    let directory = parent.join(run.environment_epoch.to_string());
    secure_directory(&directory)?;
    immutable_file(&directory.join(secret_name), secret)?;
    immutable_file(&directory.join("stdin"), &prepared.stdin)?;
    let trace_path = directory.join("traceparent");
    let traceparent = if trace_path.try_exists().map_err(|_| credential_error())? {
        let bytes = PrivateMaterialization::open(&trace_path)
            .and_then(|file| file.read(64))
            .map_err(|_| credential_error())?;
        forge_protocol::trace_context::TraceContext::parse(
            std::str::from_utf8(bytes.expose()).map_err(|_| credential_error())?,
        )
        .map(|context| context.header())
        .ok_or_else(credential_error)?
    } else {
        let value = crate::observability::current_traceparent();
        immutable_file(&trace_path, &SecretBytes::new(value.as_bytes().to_vec()))?;
        value
    };
    let mut invocation = RunnerInvocation {
        adapter_id: spec.binding.execution_profile.adapter_id().into(),
        program: prepared.program,
        args: prepared.args,
        env: prepared.env,
        managed_files: prepared
            .managed_files
            .into_iter()
            .map(|file| RunnerManagedFile {
                relative_path: file.relative_path,
                contents: file.contents,
            })
            .collect(),
        credential_files: prepared
            .credential_files
            .into_iter()
            .map(|file| RunnerCredentialFile {
                source: file.source,
                target: file.target,
                writeback: file.writeback,
            })
            .collect(),
        max_output_bytes: spec.binding.budget.max_output_bytes,
        stop_grace_seconds: spec.binding.limits.stop_grace_seconds,
        runtime_input: (matches!(spec.schema_version, 2 | 3 | 6)
            && spec
                .binding
                .execution_profile
                .capability_profile()
                .capabilities
                .contains(&forge_domain::RuntimeCapability::LiveInput))
        .then(|| forge_protocol::runtime::RuntimeInputConfig {
            run_id: run.id.to_string(),
            fencing_token: run.lease_fencing_token,
            environment_epoch: run.environment_epoch,
        }),
    };
    invocation
        .env
        .insert("FORGE_TRACEPARENT".into(), traceparent.clone());
    if invocation.adapter_id == "codex_cli" {
        // Codex forwards this non-secret correlation header through its managed
        // MCP client. No additional server or permission is introduced.
        let quoted = serde_json::to_string(&traceparent).map_err(|_| credential_error())?;
        invocation.args.extend([
            "-c".into(),
            format!("mcp_servers.forge.http_headers.traceparent={quoted}"),
        ]);
    } else if invocation.adapter_id == "claude_code_cli" {
        let file = invocation
            .managed_files
            .iter_mut()
            .find(|file| file.relative_path == "claude-mcp.json")
            .ok_or_else(credential_error)?;
        let mut config: serde_json::Value =
            serde_json::from_str(&file.contents).map_err(|_| credential_error())?;
        config["mcpServers"]["forge"]["headers"] = serde_json::json!({"traceparent": traceparent});
        file.contents = serde_json::to_string(&config).map_err(|_| credential_error())?;
    } else if invocation.adapter_id == "opencode_runtime" {
        let raw = invocation
            .env
            .get_mut("OPENCODE_CONFIG_CONTENT")
            .ok_or_else(credential_error)?;
        let mut config: serde_json::Value =
            serde_json::from_str(raw).map_err(|_| credential_error())?;
        config["mcp"]["forge"]["headers"] = serde_json::json!({"traceparent":traceparent});
        config["provider"]["forge"]["options"]["headers"] =
            serde_json::json!({"traceparent":traceparent});
        *raw = serde_json::to_string(&config).map_err(|_| credential_error())?;
    }
    let bytes = serde_json::to_vec(&invocation).map_err(|_| credential_error())?;
    immutable_file(&directory.join("invocation.json"), &SecretBytes::new(bytes))
}

fn immutable_file(path: &Path, bytes: &SecretBytes) -> Result<(), CoreError> {
    if fs::symlink_metadata(path).is_ok() {
        let existing = PrivateMaterialization::open(path)
            .and_then(|file| file.read(2 * 1024 * 1024))
            .map_err(|_| credential_error())?;
        if existing.expose() != bytes.expose() {
            return Err(credential_error());
        }
        return Ok(());
    }
    PrivateMaterialization::create(path, bytes).map_err(|_| credential_error())?;
    Ok(())
}

pub(crate) fn secure_directory(path: &Path) -> Result<(), CoreError> {
    if !path.is_absolute() {
        return Err(credential_error());
    }
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(credential_error()),
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| credential_error())?;
    if !metadata.is_dir()
        || metadata.uid() != nix::unistd::Uid::effective().as_raw()
        || metadata.mode() & 0o777 != 0o700
        || fs::canonicalize(path).map_err(|_| credential_error())? != path
    {
        return Err(credential_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::container_workdir;
    use forge_domain::runtime::SurfaceSpec;

    #[test]
    fn cli_directory_matches_each_supervisor_surface_layout() {
        assert_eq!(container_workdir(&SurfaceSpec::None), "/workspace");
        for surface in [
            SurfaceSpec::FilesystemSandbox,
            SurfaceSpec::GitWorktree {
                repository: "/source".into(),
                base_ref: "main".into(),
            },
        ] {
            assert_eq!(container_workdir(&surface), "/workspace/worktree");
        }
    }
}
