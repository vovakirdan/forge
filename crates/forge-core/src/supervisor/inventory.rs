//! Inventory proves physical presence, never a successful stage result.

use super::{SupervisorIdentity, bridge::InboundResult, validation};
use crate::{
    CoreError, CoreService,
    event::{event, event_payload},
};
use forge_domain::{AggregateRef, CommandId, DomainEventKind, Timestamp, runtime::RunScope};
use forge_protocol::supervisor::v1::{EnvironmentPresence, SupervisorInventory};
use forge_storage::{EnvironmentReport, IncidentKind, RunDesiredState};
use serde_json::json;
use std::collections::HashSet;

impl CoreService {
    pub(super) async fn record_supervisor_inventory(
        &self,
        identity: &SupervisorIdentity,
        inventory: SupervisorInventory,
    ) -> Result<InboundResult, CoreError> {
        if inventory.host_id != identity.host_id
            || inventory.boot_id != identity.boot_id
            || inventory.entries.len() > forge_protocol::SUPERVISOR_INVENTORY_LIMIT
        {
            return Ok(InboundResult::Rejected(
                "invalid_inventory",
                "inventory identity or size is invalid",
            ));
        }
        let Ok(message_id) =
            validation::parse_uuid_v7("inventory.message_id", &inventory.message_id)
        else {
            return Ok(InboundResult::Rejected(
                "invalid_inventory",
                "inventory message identity is invalid",
            ));
        };
        let mut seen = HashSet::new();
        for entry in &inventory.entries {
            let Ok(run_id) = validation::parse_uuid_v7("inventory.run_id", &entry.run_id) else {
                return Ok(InboundResult::Rejected(
                    "invalid_inventory",
                    "inventory Run identity is invalid",
                ));
            };
            if !seen.insert(run_id) || EnvironmentPresence::try_from(entry.presence).is_err() {
                return Ok(InboundResult::Rejected(
                    "invalid_inventory",
                    "duplicate Run or unknown presence",
                ));
            }
        }
        let requested = self
            .supervisor
            .inventory_is_requested(identity, &inventory.request_command_id)
            .await;
        if requested && !self.fake_runtime_enabled {
            self.reconcile_host_boot(&identity.host_id, &identity.boot_id)
                .await?;
        }
        let mut observed = HashSet::new();
        for entry in &inventory.entries {
            let run_id =
                validation::parse_uuid_v7("inventory.run_id", &entry.run_id).map_err(|_| {
                    CoreError::InvalidTransport {
                        field: "inventory.run_id",
                        reason: "invalid Run identity".into(),
                    }
                })?;
            let Some(snapshot) = self.store.load_run(run_id).await? else {
                continue;
            };
            let mut transaction = self.store.begin().await?;
            let Some(mut project) = transaction.lock_project(snapshot.project_id).await? else {
                continue;
            };
            let Some(run) = transaction.load_run(run_id).await? else {
                continue;
            };
            if run.lease_fencing_token != entry.lease_fencing_token
                || run.environment_epoch != entry.environment_epoch
            {
                continue;
            }
            if matches!(run.run_spec_version, 2..=6) && entry.provision_boot_id != identity.boot_id
            {
                continue;
            }
            if !transaction
                .record_inventory_receipt(message_id, run_id)
                .await?
            {
                continue;
            }
            let state = transaction.run_recovery_state(run_id).await?;
            if state.as_ref().is_some_and(|state| {
                state
                    .boot_id
                    .as_deref()
                    .is_some_and(|boot| boot != identity.boot_id)
                    || state
                        .host_id
                        .as_deref()
                        .is_some_and(|host| host != identity.host_id)
            }) {
                continue;
            }
            observed.insert(run_id);
            let presence = EnvironmentPresence::try_from(entry.presence)
                .unwrap_or(EnvironmentPresence::Unknown);
            let unknown = !matches!(
                presence,
                EnvironmentPresence::Active | EnvironmentPresence::Quiescent
            );
            let quiescent = presence == EnvironmentPresence::Quiescent
                && entry.last_sequence <= run.last_sequence;
            let observed_wall = Timestamp::now_utc();
            let now = crate::canonical_clock::project_mutation_time_at(&project, observed_wall);
            transaction
                .record_environment_report(&EnvironmentReport {
                    scope: RunScope {
                        run_id,
                        fencing_token: entry.lease_fencing_token,
                        environment_epoch: entry.environment_epoch,
                    },
                    host_id: identity.host_id.clone(),
                    boot_id: identity.boot_id.clone(),
                    environment_id: entry.environment_id.clone(),
                    quiescent,
                    unknown,
                })
                .await?;
            if unknown && state.as_ref().is_some_and(|state| state.lease_active) {
                self.quarantine_execution(
                    &mut transaction,
                    &mut project,
                    &run,
                    IncidentKind::EnvironmentUnknown,
                    false,
                    now,
                )
                .await?;
            } else if quiescent && state.as_ref().is_some_and(|state| state.lease_active) {
                self.quarantine_execution(
                    &mut transaction,
                    &mut project,
                    &run,
                    IncidentKind::Interrupted,
                    true,
                    now,
                )
                .await?;
            } else if presence == EnvironmentPresence::Active {
                transaction
                    .record_run_liveness(run_id, observed_wall)
                    .await?;
            }
            // A Hook belongs to the Core instance which admitted its command.
            // Retain physical evidence after a Core restart, but never restore
            // provider grants or continue an old command as newly authorized.
            if presence == EnvironmentPresence::Active
                && run.assignment.hook().is_some()
                && transaction
                    .hook_for_run(run_id)
                    .await?
                    .is_none_or(|hook| hook.core_instance != self.instance_id)
            {
                self.quarantine_execution(
                    &mut transaction,
                    &mut project,
                    &run,
                    IncidentKind::Interrupted,
                    false,
                    now,
                )
                .await?;
            }
            transaction
                .append_event_and_outbox(&event(
                    project.id(),
                    AggregateRef::Project(project.id()),
                    project.revision(),
                    DomainEventKind::RunObserved,
                    identity.actor(),
                    CommandId::from(message_id),
                    None,
                    event_payload([
                        ("run_id", json!(run_id)),
                        ("source", json!("inventory")),
                        ("quiescent", json!(quiescent)),
                        ("unknown", json!(unknown)),
                    ]),
                    now,
                )?)
                .await?;
            transaction.commit().await?;
            if matches!(run.run_spec_version, 2..=4 | 6)
                && presence == EnvironmentPresence::Active
                && state.is_some_and(|state| state.lease_active)
                && matches!(
                    run.desired_state,
                    RunDesiredState::ProvisionRequested | RunDesiredState::Running
                )
                && self.prepare_run(run_id).await.is_err()
            {
                tracing::warn!(%run_id,"active Run gateway restoration failed; watchdog retains recovery responsibility");
            }
        }
        if requested && !self.fake_runtime_enabled {
            for state in self.store.list_recovery_runs().await? {
                if observed.contains(&state.run_id) || !state.lease_active {
                    continue;
                }
                let mut transaction = self.store.begin().await?;
                let Some(mut project) = transaction.lock_project(state.project_id).await? else {
                    continue;
                };
                let Some(run) = transaction.load_run(state.run_id).await? else {
                    continue;
                };
                let now = crate::canonical_clock::project_mutation_time(&project);
                self.quarantine_execution(
                    &mut transaction,
                    &mut project,
                    &run,
                    IncidentKind::EnvironmentUnknown,
                    false,
                    now,
                )
                .await?;
                transaction.commit().await?;
            }
        }
        // Unsolicited inventory, including duplicate Provision replies, cannot
        // trigger provision replay or another recovery-dispatch cycle.
        if self
            .supervisor
            .accept_inventory(identity, &inventory.request_command_id)
            .await
        {
            self.recover_after_supervisor_attach().await?;
        }
        Ok(InboundResult::Accepted("Supervisor inventory reconciled"))
    }
}
