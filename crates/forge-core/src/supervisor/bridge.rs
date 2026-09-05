//! Sequential Supervisor ingress that enters canonical state only through Core.

use forge_domain::{
    AggregateRef, CommandId, DomainEvent, DomainEventKind, Project, TaskId, TaskWaitCondition,
    TaskWaitKind, Timestamp, WaitConditionId,
};
use forge_protocol::supervisor::v1::{
    AcknowledgementDisposition, CoreToSupervisor, SupervisorToCore, supervisor_to_core,
};
use forge_storage::{FencedWrite, ObservedRunUpdate, RunObservedState};
use serde_json::json;
use tracing::warn;

use crate::{
    CoreError, CoreService,
    event::{event, event_payload},
    task_support::{load_scoped_task, persist_task_and_project, retained_persistence, task_event},
};

use super::{SupervisorIdentity, TransportRejection, acknowledgement, validation};

/// Result of one validated inbound message that can be exposed safely to a Supervisor.
pub(super) enum InboundResult {
    Accepted(&'static str),
    Ignored(&'static str),
    Rejected(&'static str, &'static str),
}

impl CoreService {
    /// Handles exactly one post-Hello Supervisor message in caller order.
    ///
    /// The gRPC service awaits this method before reading the next message, so
    /// Run fence sequences cannot be reordered by spawned tasks.
    pub(crate) async fn handle_supervisor_message(
        &self,
        identity: &SupervisorIdentity,
        message: SupervisorToCore,
    ) -> CoreToSupervisor {
        match message.message {
            Some(supervisor_to_core::Message::Hello(hello)) => acknowledgement(
                &hello.message_id,
                AcknowledgementDisposition::Rejected,
                "hello_already_completed",
                "SupervisorHello is only valid as the first stream message",
            ),
            Some(supervisor_to_core::Message::ObservedRunEvent(observation)) => {
                let message_id = observation.message_id.clone();
                match validation::parse_observation(observation) {
                    Ok(observation) => match self
                        .record_supervisor_observation(identity, observation)
                        .await
                    {
                        Ok(result) => inbound_acknowledgement(&message_id, result),
                        Err(error) => core_error_acknowledgement(&message_id, error),
                    },
                    Err(error) => transport_rejection_acknowledgement(&message_id, error),
                }
            }
            Some(supervisor_to_core::Message::ExecutorSubmission(submission)) => {
                let message_id = submission.message_id.clone();
                match validation::parse_executor_submission(submission) {
                    Ok(validation::ParsedExecutorSubmission::Artifact(artifact)) => {
                        match self.record_executor_artifact(identity, artifact).await {
                            Ok(result) => inbound_acknowledgement(&message_id, result),
                            Err(error) => core_error_acknowledgement(&message_id, error),
                        }
                    }
                    Ok(validation::ParsedExecutorSubmission::StageOutcome(outcome)) => {
                        match self.record_executor_stage_outcome(identity, outcome).await {
                            Ok(result) => inbound_acknowledgement(&message_id, result),
                            Err(error) => core_error_acknowledgement(&message_id, error),
                        }
                    }
                    Err(error) => transport_rejection_acknowledgement(&message_id, error),
                }
            }
            None => acknowledgement(
                "",
                AcknowledgementDisposition::Rejected,
                "empty_supervisor_message",
                "Supervisor stream message has no payload",
            ),
        }
    }

    async fn record_supervisor_observation(
        &self,
        identity: &SupervisorIdentity,
        observation: validation::ParsedObservation,
    ) -> Result<InboundResult, CoreError> {
        let now = Timestamp::now_utc();
        let mut transaction = self.store.begin().await?;
        let Some(run) = transaction.load_run(observation.run_id).await? else {
            return Ok(InboundResult::Rejected(
                "run_not_found",
                "Run does not exist in canonical state",
            ));
        };
        let Some(mut project) = transaction.lock_project(run.project_id).await? else {
            return Ok(InboundResult::Rejected(
                "project_not_found",
                "Run Project does not exist in canonical state",
            ));
        };
        let stale_fence_or_epoch = run.lease_fencing_token != observation.lease_fencing_token
            || run.environment_epoch != observation.environment_epoch;
        let result = transaction
            .record_observed_run_event(&ObservedRunUpdate {
                run_id: observation.run_id,
                lease_fencing_token: observation.lease_fencing_token,
                environment_epoch: observation.environment_epoch,
                sequence: observation.sequence,
                observed_state: observation.state,
                details: observation.details.clone(),
                reported_at: observation.reported_at,
                received_at: now,
            })
            .await?;
        if result != FencedWrite::Applied {
            if result == FencedWrite::Ignored && stale_fence_or_epoch {
                let audit = stale_observation_event(
                    &project,
                    identity,
                    &observation,
                    run.lease_fencing_token,
                    run.environment_epoch,
                    now,
                )?;
                transaction.append_event_and_outbox(&audit).await?;
                transaction.commit().await?;
            }
            return Ok(refused_fenced_write(result));
        }
        let terminal_observation = matches!(
            observation.state,
            RunObservedState::Stopped | RunObservedState::Failed | RunObservedState::Lost
        );
        // A terminal observation before a StageOutcome must not leave the
        // Task's leased queue entry behind. Completion moves that entry to
        // `completed` before the terminal observation, so only a still-leased
        // entry represents interrupted work that needs a human/manager decision.
        let interrupted_queue = if terminal_observation {
            transaction
                .cancel_leased_queue_for_fenced_run(
                    observation.run_id,
                    observation.lease_fencing_token,
                    observation.environment_epoch,
                )
                .await?
                == FencedWrite::Applied
        } else {
            false
        };
        if terminal_observation {
            let _ = transaction
                .release_lease_after_terminal_observation(
                    observation.run_id,
                    observation.lease_fencing_token,
                    observation.environment_epoch,
                )
                .await?;
        }
        let interrupted_event = if interrupted_queue {
            pause_interrupted_task(
                &mut transaction,
                &mut project,
                run.task_id,
                observation.run_id,
                observation.state,
                self.actors.core,
                CommandId::from(observation.message_id),
                now,
            )
            .await?
        } else {
            None
        };
        let audit = event(
            project.id(),
            AggregateRef::Project(project.id()),
            project.revision(),
            DomainEventKind::RunObserved,
            identity.actor(),
            CommandId::from(observation.message_id),
            None,
            event_payload([
                ("run_id", json!(observation.run_id)),
                ("state", json!(observed_state_name(observation.state))),
                ("sequence", json!(observation.sequence)),
                (
                    "lease_fencing_token",
                    json!(observation.lease_fencing_token),
                ),
                ("environment_epoch", json!(observation.environment_epoch)),
            ]),
            now,
        )?;
        transaction.append_event_and_outbox(&audit).await?;
        if let Some(interrupted_event) = interrupted_event {
            transaction
                .append_event_and_outbox(&interrupted_event)
                .await?;
        }
        let project_id = project.id();
        transaction.commit().await?;
        if terminal_observation && let Err(error) = self.dispatch_available(project_id).await {
            warn!(error = %error, "could not dispatch work after terminal Run observation");
        }
        Ok(InboundResult::Accepted("Run observation recorded"))
    }
}

fn stale_observation_event(
    project: &Project,
    identity: &SupervisorIdentity,
    observation: &validation::ParsedObservation,
    expected_lease_fencing_token: u64,
    expected_environment_epoch: u64,
    now: Timestamp,
) -> Result<DomainEvent, CoreError> {
    Ok(event(
        project.id(),
        AggregateRef::Project(project.id()),
        project.revision(),
        DomainEventKind::RunObservationIgnored,
        identity.actor(),
        CommandId::from(observation.message_id),
        None,
        event_payload([
            ("reason_code", json!("stale_fence_or_environment_epoch")),
            ("run_id", json!(observation.run_id)),
            ("state", json!(observed_state_name(observation.state))),
            ("sequence", json!(observation.sequence)),
            (
                "lease_fencing_token",
                json!(observation.lease_fencing_token),
            ),
            ("environment_epoch", json!(observation.environment_epoch)),
            (
                "expected_lease_fencing_token",
                json!(expected_lease_fencing_token),
            ),
            (
                "expected_environment_epoch",
                json!(expected_environment_epoch),
            ),
        ]),
        now,
    )?)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the fenced Run fact and Core audit identity must remain explicit at this canonical reconciliation boundary"
)]
async fn pause_interrupted_task(
    transaction: &mut forge_storage::StorageTransaction<'_>,
    project: &mut Project,
    task_id: TaskId,
    run_id: uuid::Uuid,
    observed_state: RunObservedState,
    actor: forge_domain::Actor,
    command_id: CommandId,
    now: Timestamp,
) -> Result<Option<DomainEvent>, CoreError> {
    let stored = load_scoped_task(transaction, project, task_id).await?;
    let previous_task_revision = stored.task.revision().get();
    let mut task = stored.task;
    if task.lifecycle().is_terminal() {
        return Ok(None);
    }
    task.add_wait_condition(
        TaskWaitCondition::new(
            WaitConditionId::new(),
            TaskWaitKind::Interrupted,
            Some(format!(
                "run_id={run_id};observed_state={}",
                observed_state_name(observed_state)
            )),
            actor,
            now,
        )?,
        now,
    )?;
    let persistence = retained_persistence(project, &task, stored.persistence)?;
    persist_task_and_project(
        transaction,
        project,
        &task,
        persistence,
        previous_task_revision,
        now,
    )
    .await?;
    Ok(Some(task_event(
        project,
        &task,
        DomainEventKind::TaskWaiting,
        actor,
        command_id,
        now,
        event_payload([
            ("reason_code", json!("run_terminated_before_stage_outcome")),
            ("run_id", json!(run_id)),
            ("observed_state", json!(observed_state_name(observed_state))),
        ]),
    )?))
}

pub(super) fn refused_fenced_write(result: FencedWrite) -> InboundResult {
    match result {
        FencedWrite::Applied => InboundResult::Accepted("Run fence accepted"),
        FencedWrite::Ignored => InboundResult::Ignored("stale_or_duplicate_run_message"),
        FencedWrite::SequenceGap { .. } => {
            InboundResult::Rejected("run_sequence_gap", "Run message sequence is not contiguous")
        }
        FencedWrite::InvalidObservedStateTransition { .. } => InboundResult::Rejected(
            "invalid_observed_state_transition",
            "Observed Run state would regress its canonical lifecycle",
        ),
        FencedWrite::Missing => {
            InboundResult::Rejected("run_not_found", "Run does not exist in canonical state")
        }
    }
}

fn inbound_acknowledgement(message_id: &str, result: InboundResult) -> CoreToSupervisor {
    match result {
        InboundResult::Accepted(message) => acknowledgement(
            message_id,
            AcknowledgementDisposition::Accepted,
            "",
            message,
        ),
        InboundResult::Ignored(reason_code) => acknowledgement(
            message_id,
            AcknowledgementDisposition::IgnoredStale,
            reason_code,
            "Supervisor message was stale or already recorded",
        ),
        InboundResult::Rejected(reason_code, message) => acknowledgement(
            message_id,
            AcknowledgementDisposition::Rejected,
            reason_code,
            message,
        ),
    }
}

fn transport_rejection_acknowledgement(
    message_id: &str,
    error: TransportRejection,
) -> CoreToSupervisor {
    acknowledgement(
        message_id,
        AcknowledgementDisposition::Rejected,
        error.reason_code,
        error.message,
    )
}

fn core_error_acknowledgement(message_id: &str, error: CoreError) -> CoreToSupervisor {
    let (reason_code, message) = match error {
        CoreError::NotFound { .. } => (
            "canonical_resource_not_found",
            "Referenced canonical resource was not found",
        ),
        CoreError::Storage(_) => (
            "canonical_storage_unavailable",
            "Core could not persist the Supervisor message",
        ),
        CoreError::SupervisorUnavailable => (
            "supervisor_channel_unavailable",
            "Core control channel is unavailable",
        ),
        CoreError::Application(_) | CoreError::Domain(_) | CoreError::InvalidTransport { .. } => (
            "canonical_validation_failed",
            "Core rejected the Supervisor message",
        ),
        CoreError::IdempotencyConflict => (
            "idempotency_conflict",
            "Core rejected the Supervisor message",
        ),
    };
    acknowledgement(
        message_id,
        AcknowledgementDisposition::Rejected,
        reason_code,
        message,
    )
}

const fn observed_state_name(state: RunObservedState) -> &'static str {
    match state {
        RunObservedState::Unknown => "unknown",
        RunObservedState::Provisioning => "provisioning",
        RunObservedState::Running => "running",
        RunObservedState::Stopping => "stopping",
        RunObservedState::Stopped => "stopped",
        RunObservedState::Failed => "failed",
        RunObservedState::Lost => "lost",
    }
}

#[cfg(test)]
mod tests {
    use forge_storage::{FencedWrite, RunObservedState};

    use super::{InboundResult, refused_fenced_write};

    #[test]
    fn sequence_gap_is_rejected_instead_of_silently_ignored() {
        let result = refused_fenced_write(FencedWrite::SequenceGap {
            expected: 2,
            received: 3,
        });

        assert!(matches!(
            result,
            InboundResult::Rejected("run_sequence_gap", _)
        ));
    }

    #[test]
    fn invalid_state_regression_is_rejected() {
        let result = refused_fenced_write(FencedWrite::InvalidObservedStateTransition {
            from: RunObservedState::Stopped,
            to: RunObservedState::Running,
        });

        assert!(matches!(
            result,
            InboundResult::Rejected("invalid_observed_state_transition", _)
        ));
    }
}
