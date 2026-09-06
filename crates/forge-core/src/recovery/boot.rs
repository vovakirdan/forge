use crate::{
    CoreError, CoreService,
    event::{event, event_payload},
};
use forge_domain::{AggregateRef, CommandId, DomainEventKind, runtime::BootRecoveryPolicy};
use forge_storage::IncidentKind;
use serde_json::json;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostChange {
    First,
    SameBoot,
    Reboot,
    Replacement,
}

fn host_change(previous: Option<(&str, &str)>, host: &str, boot: &str) -> HostChange {
    match previous {
        None => HostChange::First,
        Some((previous_host, _)) if previous_host != host => HostChange::Replacement,
        Some((_, previous_boot)) if previous_boot != boot => HostChange::Reboot,
        _ => HostChange::SameBoot,
    }
}

impl CoreService {
    pub(crate) async fn reconcile_host_boot(
        &self,
        host: &str,
        boot: &str,
    ) -> Result<(), CoreError> {
        let previous = self.store.execution_host().await?;
        let change = host_change(
            previous
                .as_ref()
                .map(|(host, boot)| (host.as_str(), boot.as_str())),
            host,
            boot,
        );
        if change == HostChange::SameBoot {
            return Ok(());
        }
        if matches!(change, HostChange::Reboot | HostChange::Replacement) {
            let candidates = self.store.list_recovery_runs().await?;
            // All Projects enter their configured recovery gate, including an
            // idle Project whose queued work had not yet received a Lease.
            for project_id in self.store.list_project_ids().await? {
                let mut transaction = self.store.begin().await?;
                let Some(mut project) = transaction.lock_project(project_id).await? else {
                    continue;
                };
                if !transaction
                    .begin_project_boot_reconciliation(project_id, host, boot)
                    .await?
                {
                    continue;
                }
                let policy = transaction.boot_recovery_policy(project_id).await?;
                transaction
                    .set_recovery_hold(
                        project_id,
                        policy != BootRecoveryPolicy::ReconcileThenResumeQueue,
                    )
                    .await?;
                let now = crate::canonical_clock::project_mutation_time(&project);
                for candidate in candidates
                    .iter()
                    .filter(|candidate| candidate.project_id == project_id)
                {
                    let Some(state) = transaction.run_recovery_state(candidate.run_id).await?
                    else {
                        continue;
                    };
                    if !state.reserved {
                        continue;
                    }
                    let Some(run) = transaction.load_run(candidate.run_id).await? else {
                        continue;
                    };
                    // A changed boot on the same host proves the old process
                    // cannot still execute. A different host proves no such thing.
                    let quiescent = change == HostChange::Reboot
                        && state.host_id.as_deref() == Some(host)
                        && state
                            .boot_id
                            .as_deref()
                            .is_some_and(|old_boot| old_boot != boot);
                    self.quarantine_execution(
                        &mut transaction,
                        &mut project,
                        &run,
                        IncidentKind::HostReboot,
                        quiescent,
                        now,
                    )
                    .await?;
                }
                let previous_revision = project.revision();
                project.record_child_mutation(now)?;
                transaction
                    .update_project(&project, previous_revision)
                    .await?;
                transaction
                    .append_event_and_outbox(&event(
                        project_id,
                        AggregateRef::Project(project_id),
                        project.revision(),
                        DomainEventKind::ProjectRecoveryReconciled,
                        self.actors.core,
                        CommandId::new(),
                        None,
                        event_payload([
                            ("host_id", json!(host)),
                            ("boot_id", json!(boot)),
                            ("boot_policy", json!(policy)),
                            ("host_replacement", json!(change == HostChange::Replacement)),
                        ]),
                        now,
                    )?)
                    .await?;
                transaction.commit().await?;
            }
        }
        let mut transaction = self.store.begin().await?;
        transaction.record_execution_host(host, boot).await?;
        transaction.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn core_restart_on_same_boot_is_not_a_reboot() {
        assert_eq!(
            host_change(Some(("host", "boot")), "host", "boot"),
            HostChange::SameBoot
        );
    }
    #[test]
    fn new_boot_revokes_old_execution_scope() {
        assert_eq!(
            host_change(Some(("host", "boot1")), "host", "boot2"),
            HostChange::Reboot
        );
    }
    #[test]
    fn new_host_is_not_positive_exit_evidence_for_old_host() {
        assert_eq!(
            host_change(Some(("host1", "boot")), "host2", "boot"),
            HostChange::Replacement
        );
    }
}
