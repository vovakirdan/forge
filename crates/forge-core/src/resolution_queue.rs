//! Management-owned resolution routing and stale-source cleanup.
use crate::{
    CoreError, CoreService,
    event::{event, event_payload},
};
use forge_domain::{
    Actor, AggregateRef, CommandId, DomainEventKind, LifecycleStatus, Project, Timestamp,
    resolution::*,
};
use forge_storage::{RunDesiredState, RunObservedState, StorageTransaction};
use serde_json::json;
use uuid::Uuid;

pub(crate) fn invalid() -> CoreError {
    CoreError::InvalidTransport {
        field: "resolution",
        reason: "resolution scope or generation is no longer current".into(),
    }
}

pub(crate) async fn source_is_current(
    tx: &mut StorageTransaction<'_>,
    value: &Escalation,
) -> Result<bool, CoreError> {
    if let Some(source) = value.source.task() {
        let Some(stored) = tx.lock_task(source.task_id).await? else {
            return Ok(false);
        };
        let task = stored.task;
        Ok(task.project_id() == value.project_id
            && task.pipeline().pipeline_version_id() == source.pipeline_version_id
            && task.lifecycle() == LifecycleStatus::Waiting
            && task.current_stage_id() == Some(&source.stage_id)
            && task.current_stage_visit() == Some(source.stage_visit)
            && task.wait_conditions().any(|wait| {
                wait.id() == source.wait_condition_id
                    && wait.kind() == &forge_domain::TaskWaitKind::EscalationPending
            }))
    } else {
        Ok(tx.communication_escalation_is_current(value).await?)
    }
}

pub(crate) fn assign(
    value: &mut Escalation,
    resolver: Resolver,
    coordinator: Actor,
    now: Timestamp,
) -> Result<ResolutionAssignment, CoreError> {
    value.generation = value.generation.checked_add(1).ok_or_else(invalid)?;
    let expires_at = match resolver {
        Resolver::Human => None,
        Resolver::Employee { .. } => Some(Timestamp::from_offset_date_time(
            now.as_offset_date_time()
                + time::Duration::seconds(i64::from(
                    value
                        .route
                        .as_ref()
                        .ok_or_else(invalid)?
                        .assignment_timeout_seconds,
                )),
        )),
    };
    let assignment = ResolutionAssignment {
        id: Uuid::now_v7(),
        project_id: value.project_id,
        escalation_id: value.id,
        resolver,
        lease: ResolutionLease {
            generation: value.generation,
            held_by: Actor::system_manager(coordinator.id()),
            expires_at,
        },
        allowed_outcomes: value.allowed_outcomes.clone(),
        issued_at: now,
        state: ResolutionAssignmentState::Active,
    };
    value.advance(EscalationState::Assigned {
        assignment_id: assignment.id,
    })?;
    assignment.validate_owner(value)?;
    Ok(assignment)
}

pub(crate) async fn audit(
    tx: &mut StorageTransaction<'_>,
    project: &mut Project,
    actor: Actor,
    kind: DomainEventKind,
    payload: serde_json::Value,
) -> Result<(), CoreError> {
    let now = crate::canonical_clock::project_mutation_time(project);
    let old = project.revision();
    project.record_child_mutation(now)?;
    tx.update_project(project, old).await?;
    tx.append_event_and_outbox(&event(
        project.id(),
        AggregateRef::Project(project.id()),
        project.revision(),
        kind,
        actor,
        CommandId::new(),
        None,
        event_payload([("resolution", payload)]),
        now,
    )?)
    .await?;
    Ok(())
}

impl CoreService {
    /// Does not launch work. Stale authority is withdrawn even while Project is stopped.
    pub async fn reconcile_resolutions(&self) -> Result<(), CoreError> {
        self.reconcile_resolutions_at(Timestamp::now_utc()).await
    }
    /// Trusted clock entry point for the watchdog and deterministic recovery tests.
    pub async fn reconcile_resolutions_at(
        &self,
        observed_wall: Timestamp,
    ) -> Result<(), CoreError> {
        let mut cursor = self.resolution_cursor.lock().await;
        for id in self.store.resolution_queue_ids(None, *cursor).await? {
            let mut tx = self.store.begin().await?;
            // Read the immutable project identity before acquiring the mutation gate.
            let Some(project_id) = tx.escalation_project(id).await? else {
                continue;
            };
            let Some(mut project) = tx.lock_project(project_id).await? else {
                continue;
            };
            let Some(mut value) = tx.load_escalation(id).await? else {
                continue;
            };
            let obsolete = !source_is_current(&mut tx, &value).await?;
            let prior = value.revision;
            let assignment = match value.state {
                EscalationState::Assigned { assignment_id } => {
                    tx.load_resolution_assignment(assignment_id).await?
                }
                _ => None,
            };
            let mut retire = obsolete;
            let mut run = None;
            if let Some(assignment) = &assignment {
                run = tx.resolution_run(assignment.id).await?;
                retire |= assignment
                    .lease
                    .expires_at
                    .is_some_and(|deadline| deadline <= observed_wall);
                if let Some(run) = &run {
                    retire |= !matches!(
                        run.desired_state,
                        RunDesiredState::ProvisionRequested | RunDesiredState::Running
                    ) || matches!(
                        run.observed_state,
                        RunObservedState::Stopped
                            | RunObservedState::Failed
                            | RunObservedState::Lost
                    );
                } else if matches!(assignment.resolver, Resolver::Employee { .. }) {
                    retire = true;
                }
            }
            if retire {
                if let Some(mut assignment) = assignment
                    && assignment.state == ResolutionAssignmentState::Active
                {
                    assignment.finish(ResolutionAssignmentState::Retired {
                        reason: if obsolete {
                            "source_superseded"
                        } else {
                            "resolver_unavailable"
                        }
                        .into(),
                    })?;
                    tx.update_resolution_assignment(&assignment).await?;
                }
                if let Some(run) = run {
                    tx.request_run_stop(
                        run.id,
                        run.lease_fencing_token,
                        run.environment_epoch,
                        false,
                    )
                    .await?;
                    tx.revoke_run_lease(run.id, run.lease_fencing_token, run.environment_epoch)
                        .await?;
                }
                value.advance(if obsolete {
                    EscalationState::Superseded {
                        reason: "source_no_longer_current".into(),
                    }
                } else {
                    EscalationState::Queued
                })?;
                tx.save_escalation(&value, Some(prior)).await?;
                audit(&mut tx,&mut project,self.actors.core,DomainEventKind::EscalationRerouted,json!({"escalation":value,"reason":if obsolete{"source_superseded"}else{"resolver_unavailable"}})).await?;
            }
            tx.commit().await?;
            *cursor = Some(id);
            let _ = self.deliver_pending_stop_requests(project_id).await;
        }
        Ok(())
    }
}
