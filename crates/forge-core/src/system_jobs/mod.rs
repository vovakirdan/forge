//! Ownerless semantic attempts: durable sources, one-shot output and fenced recovery.
mod commands;
mod dispatch;
mod reconcile;
pub(crate) mod result;
#[cfg(test)]
mod result_tests;

use crate::{CoreError, CoreService};
use sha2::{Digest, Sha256};

pub(crate) fn invalid(reason: &str) -> CoreError {
    CoreError::InvalidTransport {
        field: "system_job",
        reason: reason.into(),
    }
}
pub(crate) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub(crate) async fn audit(
    tx: &mut forge_storage::StorageTransaction<'_>,
    project: &mut forge_domain::Project,
    actor: forge_domain::Actor,
    kind: forge_domain::DomainEventKind,
    payload: serde_json::Value,
) -> Result<(), CoreError> {
    let now = crate::canonical_clock::project_mutation_time(project);
    let previous = project.revision();
    project.record_child_mutation(now)?;
    tx.update_project(project, previous).await?;
    tx.append_event_and_outbox(&crate::event::event(
        project.id(),
        forge_domain::AggregateRef::Project(project.id()),
        project.revision(),
        kind,
        actor,
        forge_domain::CommandId::new(),
        None,
        crate::event::event_payload([("system_job", payload)]),
        now,
    )?)
    .await?;
    Ok(())
}

impl CoreService {
    /// Canonical ingestion continues while inference is stopped or unavailable.
    /// Each stage commits independently; a provider problem cannot roll back evidence.
    pub async fn system_job_tick(&self) -> Result<usize, CoreError> {
        let mut count = 0;
        for project in self.store.system_job_project_ids().await? {
            let mut tx = self.store.begin().await?;
            if tx.lock_project(project).await?.is_none() {
                continue;
            }
            let Some(settings) = tx.system_job_settings(project).await? else {
                continue;
            };
            count += tx
                .ingest_system_job_events(project, settings.policy.coalesce_seconds)
                .await?;
            count += tx.enqueue_pending_onboarding(project).await?;
            tx.commit().await?;
        }
        count += self.reconcile_system_jobs().await?;
        for project in self.store.system_job_project_ids().await? {
            count += self.dispatch_available(project).await?;
        }
        Ok(count)
    }
}
