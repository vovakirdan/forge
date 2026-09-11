//! Export one exact source revision before provisioning; no worker Git metadata is changed.
use super::PodmanBackend;
use crate::{
    SupervisorError,
    git::{GitBackend, GitSourceDescriptor},
    registry::RunRegistry,
    surface,
};
use forge_domain::runtime::RuntimeLaunchSpec;
use forge_protocol::supervisor::v1::{EnvironmentPresence, ProvisionRun, RunEventKind};
use std::{path::PathBuf, time::Duration};

pub(crate) struct PreparedSource {
    pub directory: PathBuf,
    pub descriptor: GitSourceDescriptor,
}

impl PodmanBackend {
    pub(super) async fn prepare_source(
        &self,
        provision: &ProvisionRun,
        spec: &RuntimeLaunchSpec,
        registry: &RunRegistry,
    ) -> Result<Option<PreparedSource>, SupervisorError> {
        let Some(request) = &spec.source_request else {
            return Ok(None);
        };
        self.confirm_previous_environments_stopped(provision, spec, registry)
            .await?;
        let backend = GitBackend::default();
        let retained = registry.journal.lock().await.source_selection(provision);
        let selection = match retained {
            Some(selection) if &selection.request == request => selection,
            Some(_) => return Err(SupervisorError::ConflictingProvision),
            None => {
                let key = backend
                    .source_key(request)
                    .await
                    .map_err(|_| SupervisorError::SurfacePreparationFailed)?;
                let established = registry.journal.lock().await.target_established(&key);
                let selected = tokio::time::timeout(
                    Duration::from_secs(60),
                    backend.select_source(request, established),
                )
                .await
                .map_err(|_| SupervisorError::SurfacePreparationFailed)?
                .map_err(|_| SupervisorError::SurfacePreparationFailed)?;
                registry
                    .journal
                    .lock()
                    .await
                    .record_source_selection(provision, &selected)?;
                selected
            }
        };
        let root = self.config.state_directory.join("git-sources");
        surface::private_directory(&root)?;
        let run = root.join(&provision.run_id);
        surface::private_directory(&run)?;
        let directory = run.join(format!(
            "{}-{}",
            provision.lease_fencing_token, provision.environment_epoch
        ));
        let descriptor = tokio::time::timeout(
            Duration::from_secs(60),
            backend.export_source(&selection, &directory),
        )
        .await
        .map_err(|_| SupervisorError::SurfacePreparationFailed)?
        .map_err(|_| SupervisorError::SurfacePreparationFailed)?;
        registry
            .emit(
                provision,
                RunEventKind::Provisioning,
                serde_json::json!({"git_source":descriptor}),
            )
            .await?;
        Ok(Some(PreparedSource {
            directory,
            descriptor,
        }))
    }

    async fn confirm_previous_environments_stopped(
        &self,
        provision: &ProvisionRun,
        spec: &RuntimeLaunchSpec,
        registry: &RunRegistry,
    ) -> Result<(), SupervisorError> {
        let records = registry.journal.lock().await.records();
        for record in records.iter().filter(|record| {
            record.provision.task_id == provision.task_id
                && (record.provision.run_id != provision.run_id
                    || record.provision.lease_fencing_token != provision.lease_fencing_token
                    || record.provision.environment_epoch != provision.environment_epoch)
        }) {
            if record.presence != EnvironmentPresence::Quiescent {
                return Err(SupervisorError::ConflictingProvision);
            }
            match self.inspect(&record.provision).await? {
                Some(environment)
                    if environment.id == record.environment_id
                        && !environment.state.running
                        && matches!(environment.state.status.as_str(), "exited" | "stopped")
                        && environment.config.labels.get("forge.surface")
                            == Some(&spec.surface_id.to_string())
                        && environment.config.labels.get("forge.boot") == Some(&record.boot_id) => {
                }
                // Core may dispatch after durably accepting Stopped but before
                // its ACK reaches this journal. A fresh exact exited container
                // is sufficient; a removed container still needs retained ACK.
                None if record.quiescence_confirmed => {}
                _ => return Err(SupervisorError::ConflictingProvision),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
