//! Snapshot reads and input mounts are separate from writable TaskWorkSurfaces.

use forge_domain::{
    file_snapshot::{FileCaptureRequest, MAX_SNAPSHOT_BYTES, validate_selected_paths},
    runtime::RuntimeLaunchSpec,
};
use forge_protocol::supervisor::v1::{CaptureFileSnapshot, EnvironmentPresence, ProvisionRun};
use forge_provider_common::{
    PrivateMaterialization, SecretBytes,
    selected_file::{capture_selected_files, read_capture, read_selected_file},
};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::{PodmanBackend, add_mount};
use crate::{SupervisorError, journal::Journal, registry::RunRegistry, surface};

#[cfg(test)]
mod capture_tests;
#[cfg(test)]
mod input_tests;
#[cfg(test)]
mod tests;

impl PodmanBackend {
    pub(crate) async fn capture_file_snapshot(
        &self,
        message: CaptureFileSnapshot,
        registry: RunRegistry,
    ) -> Result<(), SupervisorError> {
        if message.request_json.len() > 128 * 1024 {
            return Err(SupervisorError::InvalidRunSpec);
        }
        let request: FileCaptureRequest = serde_json::from_str(&message.request_json)?;
        if [
            request.operation_id,
            request.project_id.as_uuid(),
            request.task_id.as_uuid(),
            request.run_id,
            request.surface_id,
        ]
        .iter()
        .any(|id| id.get_version_num() != 7)
            || request.fencing_token == 0
            || request.environment_epoch == 0
            || validate_selected_paths(&request.paths).is_err()
        {
            return Err(SupervisorError::InvalidRunSpec);
        }
        let Ok(reservation) = Arc::clone(&self.provisioning).try_lock_owned() else {
            return Ok(());
        };
        let captures = self.config.state_directory.join("file-captures");
        surface::private_directory(&captures)?;
        let target = captures.join(request.operation_id.to_string());
        let identity = serde_json::to_vec(&request)?;
        if read_capture(&target, &identity, MAX_SNAPSHOT_BYTES)
            .ok()
            .flatten()
            .is_some()
        {
            return Ok(());
        }
        let failed = captures.join(format!("{}.failed", request.operation_id));
        if failed.try_exists()? {
            return Ok(());
        }
        let result = self
            .capture_stopped_surface(&request, &registry, &target, &identity, reservation)
            .await;
        if result.is_err() {
            // This marker follows completion of all blocking reads, so Core can
            // release the reservation without racing a detached capture worker.
            PrivateMaterialization::create(&failed, &SecretBytes::new(identity))
                .map_err(|_| SupervisorError::UnsafeSurface)?;
        }
        result
    }

    async fn capture_stopped_surface(
        &self,
        request: &FileCaptureRequest,
        registry: &RunRegistry,
        target: &Path,
        identity: &[u8],
        reservation: OwnedMutexGuard<()>,
    ) -> Result<(), SupervisorError> {
        let records = registry.journal.lock().await.records();
        let record = records
            .iter()
            .find(|record| {
                record.provision.run_id == request.run_id.to_string()
                    && record.provision.lease_fencing_token == request.fencing_token
                    && record.provision.environment_epoch == request.environment_epoch
            })
            .ok_or(SupervisorError::UnsafeSurface)?;
        if record.provision.task_id != request.task_id.to_string()
            || record.presence != EnvironmentPresence::Quiescent
            || !record.quiescence_confirmed
            || records.iter().any(|other| {
                other.provision.task_id == record.provision.task_id
                    && other.presence != EnvironmentPresence::Quiescent
            })
        {
            return Err(SupervisorError::UnsafeSurface);
        }
        let inspection =
            tokio::time::timeout(Duration::from_secs(30), self.inspect(&record.provision))
                .await
                .map_err(|_| SupervisorError::PodmanFailed)??;
        if let Some(environment) = inspection
            && (environment.id != record.environment_id
                || environment.state.running
                || !matches!(environment.state.status.as_str(), "stopped" | "exited")
                || environment.config.labels.get("forge.surface")
                    != Some(&request.surface_id.to_string())
                || environment.config.labels.get("forge.boot") != Some(&record.boot_id))
        {
            return Err(SupervisorError::UnsafeSurface);
        }
        let worktree = surface::capture_worktree(
            &self.config.state_directory,
            &request.task_id.to_string(),
            request.surface_id,
        )?;
        let (paths, target, identity) =
            (request.paths.clone(), target.to_owned(), identity.to_vec());
        protected_capture(reservation, Arc::clone(&registry.journal), move || {
            capture_selected_files(&worktree, &paths, &target, &identity, MAX_SNAPSHOT_BYTES)
                .map(|_| ())
                .map_err(|_| SupervisorError::UnsafeSurface)
        })
        .await
    }

    pub(super) fn mount_file_inputs(
        &self,
        provision: &ProvisionRun,
        spec: &RuntimeLaunchSpec,
        args: &mut Vec<String>,
    ) -> Result<(), SupervisorError> {
        if spec.file_inputs.is_empty() {
            return Ok(());
        }
        let relative = PathBuf::from("file-inputs")
            .join(&provision.run_id)
            .join(provision.environment_epoch.to_string());
        let root = self.config.state_directory.join(&relative);
        let expected = serde_json::to_vec(&spec.file_inputs)?;
        let actual = PrivateMaterialization::read_beneath(
            &self.config.state_directory,
            &relative.join("inputs.json"),
            1024 * 1024,
        )
        .map_err(|_| SupervisorError::UnsafeSurface)?;
        if actual.expose() != expected {
            return Err(SupervisorError::InvalidRunSpec);
        }
        for input in &spec.file_inputs {
            input
                .validate(spec.project_id)
                .map_err(|_| SupervisorError::InvalidRunSpec)?;
            let manifest_path = relative
                .join(input.artifact_id.to_string())
                .join("manifest.json");
            let manifest = PrivateMaterialization::read_beneath(
                &self.config.state_directory,
                &manifest_path,
                1024 * 1024,
            )
            .map_err(|_| SupervisorError::UnsafeSurface)?;
            if manifest.expose() != serde_json::to_vec(input)? {
                return Err(SupervisorError::InvalidRunSpec);
            }
            let files = root.join(input.artifact_id.to_string()).join("files");
            for file in &input.manifest.files {
                let body = read_selected_file(&files, Path::new(&file.path), file.size_bytes)
                    .map_err(|_| SupervisorError::UnsafeSurface)?;
                if body.bytes.len() as u64 != file.size_bytes
                    || body.executable != file.executable
                    || format!("{:x}", Sha256::digest(&body.bytes)) != file.sha256
                {
                    return Err(SupervisorError::UnsafeSurface);
                }
            }
        }
        add_mount(args, &root, "/run/forge-inputs", true)
    }
}

async fn protected_capture(
    reservation: OwnedMutexGuard<()>,
    journal: Arc<Mutex<Journal>>,
    capture: impl FnOnce() -> Result<(), SupervisorError> + Send + 'static,
) -> Result<(), SupervisorError> {
    tokio::task::spawn_blocking(move || {
        // Aborting the async worker does not stop blocking filesystem reads.
        // Keep both in-process provisioning and replacement-Supervisor admission
        // fenced until the actual read finishes, including during shutdown.
        let _reservation = reservation;
        let _journal = journal;
        capture()
    })
    .await
    .map_err(|_| SupervisorError::UnsafeSurface)?
}
