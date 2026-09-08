//! A failed local preparation has not crossed the Supervisor boundary.

use crate::{
    CoreError, CoreService,
    event::{event, event_payload},
    task_support::{load_scoped_task, persist_task_and_project, retained_persistence},
};
use forge_domain::{
    AggregateRef, CommandId, DomainEventKind, TaskWaitCondition, TaskWaitKind, Timestamp,
    WaitConditionId, runtime::RecoveryAssessment,
};
use forge_storage::{IncidentKind, ObservedRunUpdate, RunObservedState};
use serde_json::json;
use uuid::Uuid;

impl CoreService {
    /// Only called by the dispatcher for its newly-created, never-sent Run.
    pub(crate) async fn fail_preparation(&self, run_id: Uuid) -> Result<(), CoreError> {
        let mut transaction = self.store.begin().await?;
        let run = transaction
            .load_run(run_id)
            .await?
            .ok_or(CoreError::NotFound { aggregate: "run" })?;
        let mut project =
            transaction
                .lock_project(run.project_id)
                .await?
                .ok_or(CoreError::NotFound {
                    aggregate: "project",
                })?;
        if run.last_sequence != 0 || run.observed_state != RunObservedState::Unknown {
            return Err(CoreError::InvalidTransport {
                field: "run",
                reason: "preparation failure cannot resolve an observed environment".into(),
            });
        }
        let observed_wall = Timestamp::now_utc();
        let now = crate::canonical_clock::project_mutation_time_at(&project, observed_wall);
        let (incident_id, _) = transaction
            .record_run_incident(
                project.id(),
                run.id,
                IncidentKind::StartFailed,
                RecoveryAssessment::NotStartedConfirmed,
            )
            .await?;
        let _ = transaction
            .record_observed_run_event(&ObservedRunUpdate {
                run_id: run.id,
                lease_fencing_token: run.lease_fencing_token,
                environment_epoch: run.environment_epoch,
                sequence: 1,
                observed_state: RunObservedState::Failed,
                details: json!({"reason_code":"core_preparation_failed","not_forwarded":true}),
                reported_at: observed_wall,
                received_at: observed_wall,
            })
            .await?;
        let _ = transaction
            .cancel_leased_queue_for_fenced_run(
                run.id,
                run.lease_fencing_token,
                run.environment_epoch,
            )
            .await?;
        let _ = transaction
            .release_lease_after_terminal_observation(
                run.id,
                run.lease_fencing_token,
                run.environment_epoch,
            )
            .await?;
        transaction
            .record_environment_report(&forge_storage::EnvironmentReport {
                scope: forge_domain::runtime::RunScope {
                    run_id: run.id,
                    fencing_token: run.lease_fencing_token,
                    environment_epoch: run.environment_epoch,
                },
                host_id: String::new(),
                boot_id: String::new(),
                environment_id: String::new(),
                quiescent: true,
                unknown: false,
            })
            .await?;
        let (aggregate, revision) = if let Some(task_id) = run.task_id() {
            let stored = load_scoped_task(&mut transaction, &project, task_id).await?;
            let mut task = stored.task;
            let previous = task.revision().get();
            if !matches!(
                task.lifecycle(),
                forge_domain::LifecycleStatus::Done | forge_domain::LifecycleStatus::Cancelled
            ) {
                task.add_wait_condition(
                    TaskWaitCondition::new(
                        WaitConditionId::new(),
                        TaskWaitKind::ManualPause,
                        Some(format!("recovery_run={}", run.id)),
                        self.actors.core,
                        now,
                    )?,
                    now,
                )?;
                let persistence = retained_persistence(&project, &task, stored.persistence)?;
                persist_task_and_project(
                    &mut transaction,
                    &mut project,
                    &task,
                    persistence,
                    previous,
                    now,
                )
                .await?;
            }
            crate::handoff::persist_handoff(
                &mut transaction,
                &run,
                &task,
                self.actors.core,
                None,
                Some(incident_id),
                now,
            )
            .await?;
            (AggregateRef::Task(task.id()), task.revision().get())
        } else {
            transaction.hold_communication_run(run.id).await?;
            (AggregateRef::Project(project.id()), project.revision())
        };
        transaction
            .append_event_and_outbox(&event(
                project.id(),
                aggregate,
                revision,
                DomainEventKind::RunIncidentRaised,
                self.actors.core,
                CommandId::new(),
                None,
                event_payload([
                    ("run_id", json!(run.id)),
                    ("incident_id", json!(incident_id)),
                    ("kind", json!("start_failed")),
                    ("assessment", json!("not_started_confirmed")),
                ]),
                now,
            )?)
            .await?;
        transaction.commit().await?;
        self.close_run_gateway(run.id).await;
        if let Err(error) = self.revoke_run_proxy_key(&run).await {
            tracing::warn!(error=%error,"proxy revocation awaits reconciliation");
        }
        Ok(())
    }
}
