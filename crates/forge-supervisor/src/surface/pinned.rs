//! Exact accepted revision checkout, independent of the mutable writer files.
use super::{PreparedSurface, inspection_worktree, private_directory};
use crate::{SupervisorError, git::GitBackend, journal::GitSourceScope};
use forge_domain::runtime::{RuntimeLaunchSpec, SurfaceAccess, SurfaceSpec};
use forge_protocol::supervisor::v1::ProvisionRun;
use std::{path::Path, time::Duration};

pub(super) async fn prepare(
    root: &Path,
    provision: &ProvisionRun,
    spec: &RuntimeLaunchSpec,
) -> Result<PreparedSurface, SupervisorError> {
    let (original, candidate) = match &spec.binding.surface {
        SurfaceSpec::GitCandidateSnapshot {
            repository,
            base_ref,
            candidate,
        } => (
            SurfaceSpec::GitWorktree {
                repository: repository.clone(),
                base_ref: base_ref.clone(),
            },
            candidate,
        ),
        SurfaceSpec::GitUnbornCandidateSnapshot {
            repository,
            object_format,
            candidate,
        } => (
            SurfaceSpec::GitUnborn {
                repository: repository.clone(),
                object_format: *object_format,
            },
            candidate,
        ),
        _ => return Err(SupervisorError::UnsafeSurface),
    };
    crate::execution_assignment::validate(provision)?;
    spec.validate()
        .map_err(|_| SupervisorError::InvalidRunSpec)?;
    if !matches!(provision.run_spec_version, 2 | 6)
        || !matches!(spec.schema_version, 2 | 6)
        || spec.communication.is_some()
        || spec.resolution.is_some()
        || spec.binding.access != SurfaceAccess::ReadOnly
        || spec.surface_id.get_version_num() != 7
        || provision.lease_fencing_token == 0
        || provision.environment_epoch == 0
    {
        return Err(SupervisorError::UnsafeSurface);
    }
    let run_id = uuid::Uuid::parse_str(&provision.run_id)
        .ok()
        .filter(|id| id.get_version_num() == 7 && id.to_string() == provision.run_id)
        .ok_or(SupervisorError::UnsafeSurface)?;
    let source = GitSourceScope {
        project_id: spec.project_id,
        surface_id: spec.surface_id,
        source: original,
    };
    // This is a readonly lookup. An absent, replaced, or mismatched original
    // writer manifest fails without preparing a new source or repairing it.
    let worktree = inspection_worktree(root, &provision.task_id, &source)?;
    let snapshots = root.join("snapshots");
    private_directory(&snapshots)?;
    let destination = snapshots.join(format!(
        "{run_id}-{}-{}",
        provision.lease_fencing_token, provision.environment_epoch
    ));
    tokio::time::timeout(
        Duration::from_secs(60),
        GitBackend::default().snapshot_candidate(&worktree, candidate, &destination),
    )
    .await
    .map_err(|_| SupervisorError::SurfacePreparationFailed)?
    .map_err(|_| SupervisorError::SurfacePreparationFailed)?;
    Ok(PreparedSurface {
        mount: Some(destination),
        workdir: "/workspace/worktree",
        read_only: true,
    })
}

#[cfg(test)]
pub(crate) mod tests;
