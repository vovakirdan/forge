//! Short canonical CAS after physical exit; never serializes provider execution.

use crate::{
    CoreError, CoreService,
    credentials::{CredentialRecord, credential_error},
    event::{event, event_payload},
};
use forge_domain::{
    AggregateRef, CommandId, DomainEventKind, Timestamp, runtime::RecoveryAssessment,
};
use forge_provider_common::{
    AuthSnapshotLease, AuthWriteback, PrivateMaterialization, evaluate_auth_writeback,
};
use forge_storage::{IncidentKind, RunProjection, StorageTransaction, StoredCredential};
use serde_json::json;

impl CoreService {
    pub(crate) async fn write_back_run_auth(
        &self,
        transaction: &mut StorageTransaction<'_>,
        run: &RunProjection,
    ) -> Result<(), CoreError> {
        if !matches!(run.run_spec_version, 2..=4)
            || transaction.auth_writeback_recorded(run.id).await?
        {
            return Ok(());
        }
        let Some(execution) = &self.execution else {
            return Ok(());
        };
        let Some(base) = transaction.run_credential(run.id).await? else {
            return Ok(());
        };
        let CredentialRecord::CodexChatgpt {
            snapshot: base_snapshot,
        } = serde_json::from_value(base.sealed_record.clone()).map_err(|_| credential_error())?
        else {
            return Ok(());
        };
        let path = execution
            .root
            .join("runtime")
            .join(run.id.to_string())
            .join(run.environment_epoch.to_string())
            .join("codex-home/auth.json");
        // A confirmed non-start has no writable auth home. Never import host auth.
        if !path.try_exists().map_err(|_| credential_error())? {
            transaction
                .record_auth_writeback(run.id, "not_materialized")
                .await?;
            return Ok(());
        }
        let relative = path
            .strip_prefix(&execution.root)
            .map_err(|_| credential_error())?;
        let candidate =
            match PrivateMaterialization::read_beneath(&execution.root, relative, 1024 * 1024) {
                Ok(value) => value,
                Err(_) => {
                    self.auth_incident(transaction, run, "unsafe_returned_auth")
                        .await?;
                    transaction
                        .record_auth_writeback(run.id, "unsafe_returned_auth_retained")
                        .await?;
                    return Ok(());
                }
            };
        let current = transaction
            .load_credential(run.project_id, base.secret_id)
            .await?
            .ok_or_else(credential_error)?;
        let CredentialRecord::CodexChatgpt {
            snapshot: current_snapshot,
        } = serde_json::from_value(current.sealed_record.clone())
            .map_err(|_| credential_error())?
        else {
            return Err(credential_error());
        };
        let store = self.secret_store.as_ref().ok_or_else(credential_error)?;
        let lease = AuthSnapshotLease {
            run_id: run.id,
            environment_epoch: run.environment_epoch,
            base: base_snapshot,
        };
        let decision = evaluate_auth_writeback(store, &current_snapshot, &lease, &candidate)
            .map_err(|_| credential_error())?;
        let outcome = match decision {
            AuthWriteback::Unchanged => "unchanged",
            AuthWriteback::Replace {
                expected_version,
                snapshot,
            } => {
                let updated = StoredCredential {
                    version: snapshot.auth.scope.version,
                    sealed_record: serde_json::to_value(CredentialRecord::CodexChatgpt {
                        snapshot,
                    })
                    .map_err(|_| credential_error())?,
                    ..current
                };
                if !transaction
                    .replace_credential(&updated, expected_version)
                    .await?
                {
                    return Err(credential_error());
                }
                let project = transaction
                    .lock_project(run.project_id)
                    .await?
                    .ok_or_else(credential_error)?;
                transaction
                    .append_event_and_outbox(&event(
                        project.id(),
                        AggregateRef::Project(project.id()),
                        project.revision(),
                        DomainEventKind::CredentialUpdated,
                        self.actors.core,
                        CommandId::new(),
                        None,
                        event_payload([
                            ("run_id", json!(run.id)),
                            ("secret_id", json!(updated.secret_id)),
                            ("version", json!(updated.version)),
                            ("reason_code", json!("runtime_auth_writeback")),
                        ]),
                        Timestamp::now_utc(),
                    )?)
                    .await?;
                "updated"
            }
            AuthWriteback::Conflict { reason, recovery } => {
                let reason = serde_json::to_value(reason).map_err(|_| credential_error())?;
                let reason = reason.as_str().ok_or_else(credential_error)?;
                transaction
                    .retain_credential_conflict(
                        run.id,
                        reason,
                        &serde_json::to_value(recovery).map_err(|_| credential_error())?,
                    )
                    .await?;
                self.auth_incident(transaction, run, reason).await?;
                "conflict_retained"
            }
        };
        transaction.record_auth_writeback(run.id, outcome).await?;
        // The evidence worker removes private delivery copies only after this
        // writeback and both redacted output imports have committed durably.
        Ok(())
    }

    async fn auth_incident(
        &self,
        transaction: &mut StorageTransaction<'_>,
        run: &RunProjection,
        reason: &str,
    ) -> Result<(), CoreError> {
        let (id, inserted) = transaction
            .record_run_incident(
                run.project_id,
                run.id,
                IncidentKind::Authentication,
                RecoveryAssessment::Unknown,
            )
            .await?;
        if inserted {
            let project = transaction
                .lock_project(run.project_id)
                .await?
                .ok_or_else(credential_error)?;
            transaction
                .append_event_and_outbox(&event(
                    run.project_id,
                    AggregateRef::Project(run.project_id),
                    project.revision(),
                    DomainEventKind::RunIncidentRaised,
                    self.actors.core,
                    CommandId::new(),
                    None,
                    event_payload([
                        ("run_id", json!(run.id)),
                        ("incident_id", json!(id)),
                        ("kind", json!("authentication")),
                        ("reason_code", json!(reason)),
                    ]),
                    Timestamp::now_utc(),
                )?)
                .await?;
        }
        Ok(())
    }
}
