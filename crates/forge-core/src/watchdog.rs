//! Conservative time-based recovery; lack of text/progress is never liveness loss.

use crate::{
    CoreError, CoreService,
    event::{event, event_payload},
    task_support::{load_scoped_task, persist_task_and_project, retained_persistence, task_event},
};
use forge_domain::{
    AggregateRef, CommandId, DomainEventKind, Project, TaskWaitCondition, TaskWaitKind, Timestamp,
    WaitConditionId, runtime::RecoveryAssessment,
};
use forge_protocol::supervisor::v1::{CoreToSupervisor, StopMode, StopRun, core_to_supervisor};
use forge_storage::{
    IncidentKind, RunDesiredState, RunProjection, RunRecoveryState, StorageTransaction,
};
use serde_json::json;
use uuid::Uuid;

/// Independent operational deadlines in seconds, evaluated against canonical time.
#[derive(Clone, Copy, Debug)]
pub struct WatchdogDeadlines {
    /// Maximum wait for a positive Running observation after provision.
    pub start_seconds: u32,
    /// Maximum age of the last Runner heartbeat, not progress.
    pub liveness_seconds: u32,
    /// Grace before a committed graceful stop escalates to forced termination.
    pub stop_grace_seconds: u32,
}

impl Default for WatchdogDeadlines {
    fn default() -> Self {
        Self {
            start_seconds: 120,
            liveness_seconds: 90,
            stop_grace_seconds: 30,
        }
    }
}

/// Bounded counters suitable for metrics, without identifying labels.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WatchdogReport {
    /// Newly quarantined executions.
    pub quarantined: usize,
    /// Forced-stop requests sent or retained as desired state.
    pub force_stops: usize,
    /// External key revocations that remain retryable.
    pub pending_key_revocations: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeadlineFailure {
    Start,
    Heartbeat,
    ForceStop,
}

fn deadline_failure(
    state: &RunRecoveryState,
    desired: RunDesiredState,
    now: Timestamp,
    deadlines: WatchdogDeadlines,
) -> Option<DeadlineFailure> {
    let elapsed = |at: Timestamp, seconds: u32| {
        now.as_offset_date_time() - at.as_offset_date_time()
            >= time::Duration::seconds(i64::from(seconds))
    };
    if !state.reserved {
        return None;
    }
    if matches!(
        desired,
        RunDesiredState::StopRequested | RunDesiredState::ForceStopRequested
    ) {
        return state
            .stop_requested_at
            .filter(|at| elapsed(*at, deadlines.stop_grace_seconds))
            .map(|_| DeadlineFailure::ForceStop);
    }
    if !state.lease_active {
        return None;
    }
    match state.started_at {
        None if elapsed(state.created_at, deadlines.start_seconds) => Some(DeadlineFailure::Start),
        Some(started)
            if elapsed(
                state.liveness_observed_at.unwrap_or(started),
                deadlines.liveness_seconds,
            ) =>
        {
            Some(DeadlineFailure::Heartbeat)
        }
        _ => None,
    }
}

impl CoreService {
    /// Runs one failure-detection pass. The supplied time evaluates deadlines;
    /// canonical mutations get fresh time after the Project lock. Repeated
    /// calls retry physical stop and key revocation.
    pub async fn watchdog_tick(
        &self,
        now: Timestamp,
        deadlines: WatchdogDeadlines,
    ) -> Result<WatchdogReport, CoreError> {
        let started = std::time::Instant::now();
        let parent = crate::observability::current_traceparent();
        let result = crate::observability::trace_operation(
            Some(&parent),
            crate::observability::Operation::Recovery,
            self.watchdog_tick_inner(now, deadlines),
        )
        .await;
        self.record_operation(
            crate::observability::Operation::Recovery,
            result.is_ok(),
            started.elapsed(),
        );
        result
    }

    async fn watchdog_tick_inner(
        &self,
        now: Timestamp,
        deadlines: WatchdogDeadlines,
    ) -> Result<WatchdogReport, CoreError> {
        if deadlines.start_seconds == 0
            || deadlines.liveness_seconds == 0
            || deadlines.stop_grace_seconds == 0
        {
            return Err(CoreError::InvalidTransport {
                field: "watchdog.deadlines",
                reason: "deadlines must be positive".into(),
            });
        }
        let mut report = WatchdogReport::default();
        self.resume_due_tasks(now).await?;
        if self.fake_runtime_enabled {
            return Ok(report);
        }
        // Give an existing Supervisor time to reattach and prove liveness after
        // Core restart. This bounded grace cannot mask permanent channel loss.
        if !self.supervisor.is_reconciled().await
            && self
                .supervisor
                .startup_reconciliation_grace(deadlines.liveness_seconds)
        {
            return Ok(report);
        }
        for candidate in self.store.list_recovery_runs().await? {
            let mut transaction = self.store.begin().await?;
            let Some(mut project) = transaction.lock_project(candidate.project_id).await? else {
                continue;
            };
            // The supplied clock evaluates deadlines, including deterministic
            // future-time failure injection. Canonical mutation time must be
            // captured after serialization, never before waiting for this lock.
            let mutation_time = crate::canonical_clock::project_mutation_time(&project);
            let Some(state) = transaction.run_recovery_state(candidate.run_id).await? else {
                continue;
            };
            let Some(run) = transaction.load_run(candidate.run_id).await? else {
                continue;
            };
            let deadlines = WatchdogDeadlines {
                stop_grace_seconds: crate::run_control::stop_grace_seconds(
                    &run,
                    deadlines.stop_grace_seconds,
                )?,
                ..deadlines
            };
            let failure = deadline_failure(&state, run.desired_state, now, deadlines);
            if let Some(failure) = failure {
                if failure == DeadlineFailure::ForceStop {
                    let changed = transaction
                        .request_run_stop(
                            run.id,
                            run.lease_fencing_token,
                            run.environment_epoch,
                            true,
                        )
                        .await?;
                    if changed == forge_storage::FencedWrite::Applied {
                        transaction
                            .append_event_and_outbox(&crate::run_control::run_stop_event(
                                &project,
                                &run,
                                CommandId::new(),
                                self.actors.core,
                                "watchdog_stop_grace_expired",
                                mutation_time,
                            )?)
                            .await?;
                    }
                    report.force_stops += 1;
                } else {
                    let kind = if failure == DeadlineFailure::Start {
                        IncidentKind::StartFailed
                    } else {
                        IncidentKind::HeartbeatLost
                    };
                    self.quarantine_execution(
                        &mut transaction,
                        &mut project,
                        &run,
                        kind,
                        false,
                        mutation_time,
                    )
                    .await?;
                    report.quarantined += 1;
                }
            }
            transaction.commit().await?;
            if failure.is_some()
                || matches!(
                    run.desired_state,
                    RunDesiredState::StopRequested | RunDesiredState::ForceStopRequested
                )
            {
                let force = failure == Some(DeadlineFailure::ForceStop)
                    || run.desired_state == RunDesiredState::ForceStopRequested;
                let _ = self
                    .supervisor
                    .send(recovery_stop(&run, force, deadlines.stop_grace_seconds))
                    .await;
            }
        }
        self.reconcile_git_proposals().await?;
        self.reconcile_git_integrations().await?;
        self.reconcile_runtime_inputs(None).await?;
        self.reconcile_resolutions().await?;
        self.reconcile_hook_results().await?;
        // Quiescent retired Runs are absent from the physical-watchdog query,
        // but their proxy revocation must still be retried after broker downtime.
        for run_id in self.store.pending_retired_proxy_revocations().await? {
            if let Some(run) = self.store.load_run(run_id).await?
                && self.revoke_run_proxy_key(&run).await.is_err()
            {
                report.pending_key_revocations += 1;
            }
        }
        Ok(report)
    }

    pub(crate) async fn quarantine_execution(
        &self,
        transaction: &mut StorageTransaction<'_>,
        project: &mut Project,
        run: &RunProjection,
        kind: IncidentKind,
        quiescent: bool,
        now: Timestamp,
    ) -> Result<(), CoreError> {
        let kind_name = serde_json::to_value(kind).map_err(|_| CoreError::InvalidTransport {
            field: "incident.kind",
            reason: "invalid typed kind".into(),
        })?;
        let reason = kind_name
            .as_str()
            .ok_or(CoreError::InvalidTransport {
                field: "incident.kind",
                reason: "invalid typed kind".into(),
            })?
            .to_owned();
        let _ = transaction
            .request_run_stop(
                run.id,
                run.lease_fencing_token,
                run.environment_epoch,
                false,
            )
            .await?;
        transaction
            .quarantine_run(run.id, &reason, quiescent)
            .await?;
        let (incident_id, inserted) = transaction
            .record_run_incident(project.id(), run.id, kind, RecoveryAssessment::Unknown)
            .await?;
        let command_id = CommandId::new();
        if let Some(task_id) = run.task_id() {
            let stored = load_scoped_task(transaction, project, task_id).await?;
            let mut task = stored.task;
            let detail = format!("recovery_run={}", run.id);
            // An already accepted stage handoff is immutable; process retirement
            // must not pause the next stage or replace accepted evidence.
            let has_handoff = transaction.run_has_handoff(run.id).await?;
            if !task.lifecycle().is_terminal()
                && !has_handoff
                && !task
                    .wait_conditions()
                    .any(|wait| wait.detail() == Some(detail.as_str()))
            {
                let previous = task.revision().get();
                task.add_wait_condition(
                    TaskWaitCondition::new(
                        WaitConditionId::new(),
                        TaskWaitKind::Interrupted,
                        Some(detail),
                        self.actors.core,
                        now,
                    )?,
                    now,
                )?;
                let persistence = retained_persistence(project, &task, stored.persistence)?;
                persist_task_and_project(transaction, project, &task, persistence, previous, now)
                    .await?;
                transaction
                    .append_event_and_outbox(&task_event(
                        project,
                        &task,
                        DomainEventKind::TaskWaiting,
                        self.actors.core,
                        command_id,
                        now,
                        event_payload([("run_id", json!(run.id)), ("reason_code", json!(reason))]),
                    )?)
                    .await?;
            }
            if !has_handoff {
                crate::handoff::persist_handoff(
                    transaction,
                    run,
                    &task,
                    self.actors.core,
                    None,
                    Some(incident_id),
                    now,
                )
                .await?;
            }
        } else if run.assignment.communication().is_some() {
            transaction.hold_communication_run(run.id).await?;
        }
        if inserted {
            transaction
                .append_event_and_outbox(&event(
                    project.id(),
                    AggregateRef::Project(project.id()),
                    project.revision(),
                    DomainEventKind::RunIncidentRaised,
                    self.actors.core,
                    command_id,
                    None,
                    event_payload([
                        ("run_id", json!(run.id)),
                        ("incident_id", json!(incident_id)),
                        ("kind", kind_name),
                        ("assessment", json!("unknown")),
                        ("quiescent", json!(quiescent)),
                    ]),
                    now,
                )?)
                .await?;
            transaction
                .append_event_and_outbox(&crate::run_control::run_stop_event(
                    project,
                    run,
                    command_id,
                    self.actors.core,
                    &reason,
                    now,
                )?)
                .await?;
        }
        Ok(())
    }
}

fn recovery_stop(run: &RunProjection, force: bool, grace_seconds: u32) -> CoreToSupervisor {
    CoreToSupervisor {
        message: Some(core_to_supervisor::Message::StopRun(StopRun {
            command_id: Uuid::now_v7().to_string(),
            run_id: run.id.to_string(),
            lease_fencing_token: run.lease_fencing_token,
            environment_epoch: run.environment_epoch,
            mode: if force {
                StopMode::Force
            } else {
                StopMode::Graceful
            } as i32,
            reason_code: "core_recovery_stop".into(),
            grace_period_ms: u64::from(grace_seconds) * 1000,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> RunRecoveryState {
        let now = Timestamp::from_offset_date_time(time::OffsetDateTime::UNIX_EPOCH);
        RunRecoveryState {
            run_id: Uuid::now_v7(),
            project_id: forge_domain::ProjectId::new(),
            created_at: now,
            started_at: Some(now),
            liveness_observed_at: Some(now),
            stop_requested_at: None,
            host_id: None,
            boot_id: None,
            reserved: true,
            lease_active: true,
        }
    }
    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_offset_date_time(
            time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(seconds),
        )
    }
    #[test]
    fn progress_silence_is_not_an_input_to_loss_detection() {
        let mut state = state();
        state.liveness_observed_at = Some(at(1000));
        assert_eq!(
            deadline_failure(
                &state,
                RunDesiredState::Running,
                at(1050),
                WatchdogDeadlines::default()
            ),
            None
        );
    }
    #[test]
    fn heartbeat_loss_quarantines_after_deadline() {
        assert_eq!(
            deadline_failure(
                &state(),
                RunDesiredState::Running,
                at(90),
                WatchdogDeadlines::default()
            ),
            Some(DeadlineFailure::Heartbeat)
        );
    }
    #[test]
    fn never_started_execution_has_separate_deadline() {
        let mut state = state();
        state.started_at = None;
        state.liveness_observed_at = Some(at(115));
        assert_eq!(
            deadline_failure(
                &state,
                RunDesiredState::ProvisionRequested,
                at(120),
                WatchdogDeadlines::default()
            ),
            Some(DeadlineFailure::Start)
        );
    }
    #[test]
    fn revoked_scope_still_requires_forced_physical_stop() {
        let mut state = state();
        state.lease_active = false;
        state.stop_requested_at = Some(at(10));
        assert_eq!(
            deadline_failure(
                &state,
                RunDesiredState::StopRequested,
                at(40),
                WatchdogDeadlines::default()
            ),
            Some(DeadlineFailure::ForceStop)
        );
    }
}
