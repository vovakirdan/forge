//! Initial physical observation barrier before the first Core inventory.

use super::{PodmanBackend, valid_environment_id};
use crate::{SupervisorError, registry::RunRegistry};
use forge_protocol::supervisor::v1::{EnvironmentPresence, ProvisionRun};
use std::{sync::Arc, time::Duration};
use tokio::{sync::Semaphore, task::JoinSet};

const INITIAL_INSPECTION_BUDGET: Duration = Duration::from_secs(10);
const INITIAL_INSPECTION_PARALLELISM: usize = 16;

impl PodmanBackend {
    pub async fn reconcile(
        &self,
        registry: &RunRegistry,
    ) -> Result<Vec<ProvisionRun>, SupervisorError> {
        let provisions: Vec<_> = registry
            .journal
            .lock()
            .await
            .records()
            .into_iter()
            .filter(|record| {
                record.provision.run_spec_version == 2
                    && record.presence != EnvironmentPresence::Quiescent
            })
            .map(|record| record.provision)
            .collect();
        // A corrupted retained contract cannot safely restore a stop controller
        // with guessed limits. Fail startup before any identity becomes Active.
        for provision in &provisions {
            let spec: forge_domain::runtime::SandboxRunSpec =
                serde_json::from_str(&provision.run_spec_json)
                    .map_err(|_| SupervisorError::InvalidRunSpec)?;
            spec.validate()
                .map_err(|_| SupervisorError::InvalidRunSpec)?;
        }
        let parallelism = Arc::new(Semaphore::new(INITIAL_INSPECTION_PARALLELISM));
        let mut inspections = JoinSet::new();
        for provision in &provisions {
            let provision = provision.clone();
            let backend = self.clone();
            let registry = registry.clone();
            let parallelism = parallelism.clone();
            inspections.spawn(async move {
                let Ok(_permit) = parallelism.acquire_owned().await else {
                    return;
                };
                // The journal's previous Active bit is not evidence. Restore
                // only an inspected, label-checked, currently running identity.
                if let Ok(Some(inspection)) = backend.inspect(&provision).await
                    && inspection.state.running
                    && valid_environment_id(&inspection.id)
                {
                    let _ = registry
                        .attach_environment(&provision, &inspection.id, true)
                        .await;
                }
            });
        }
        if tokio::time::timeout(INITIAL_INSPECTION_BUDGET, async {
            while inspections.join_next().await.is_some() {}
        })
        .await
        .is_err()
        {
            inspections.abort_all();
            while inspections.join_next().await.is_some() {}
            tracing::warn!(
                "initial physical inspection deadline expired; unresolved environments remain unknown"
            );
        }
        // Unknown environments retain monitors and stop controls too. Neither
        // timeout nor an absent result can authorize a replacement execution.
        Ok(provisions)
    }
}

#[cfg(test)]
mod tests;
