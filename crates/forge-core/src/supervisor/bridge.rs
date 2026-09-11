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
pub(crate) enum InboundResult {
    ProposalSaved,
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
        let parent = match message.message.as_ref() {
            Some(supervisor_to_core::Message::ObservedRunEvent(event))
                if event.details_json.len() <= 262_144 =>
            {
                serde_json::from_str::<serde_json::Value>(&event.details_json)
                    .ok()
                    .and_then(|details| {
                        details
                            .get("traceparent")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned)
                    })
            }
            _ => None,
        };
        let started = std::time::Instant::now();
        let response = crate::observability::trace_operation(
            parent.as_deref(),
            crate::observability::Operation::Grpc,
            self.handle_supervisor_message_inner(identity, message),
        )
        .await;
        let success = matches!(response.message.as_ref(), Some(forge_protocol::supervisor::v1::core_to_supervisor::Message::Acknowledgement(ack))
            if ack.disposition != AcknowledgementDisposition::Rejected as i32);
        self.record_operation(
            crate::observability::Operation::Grpc,
            success,
            started.elapsed(),
        );
        response
    }

    async fn handle_supervisor_message_inner(
        &self,
        identity: &SupervisorIdentity,
        message: SupervisorToCore,
    ) -> CoreToSupervisor {
        match message.message {
            Some(supervisor_to_core::Message::GitIntegration(result)) => {
                let message_id = result.message_id.clone();
                match self.record_git_integration_result(identity, result).await {
                    Ok(result) => inbound_acknowledgement(&message_id, result),
                    Err(error) => core_error_acknowledgement(&message_id, error),
                }
            }
            Some(supervisor_to_core::Message::RuntimeInputReceipt(receipt)) => {
                let id = receipt.message_id.clone();
                match self.record_runtime_input_receipt(identity, receipt).await {
                    Ok(result) => inbound_acknowledgement(&id, result),
                    Err(error) => core_error_acknowledgement(&id, error),
                }
            }
            Some(supervisor_to_core::Message::GitCandidateInspection(result)) => {
                let message_id = result.message_id.clone();
                match self.record_git_candidate_inspection(identity, result).await {
                    Ok(result) => inbound_acknowledgement(&message_id, result),
                    Err(error) => core_error_acknowledgement(&message_id, error),
                }
            }
            Some(supervisor_to_core::Message::Inventory(inventory)) => {
                let message_id = inventory.message_id.clone();
                match self.record_supervisor_inventory(identity, inventory).await {
                    Ok(result) => inbound_acknowledgement(&message_id, result),
                    Err(error) => core_error_acknowledgement(&message_id, error),
                }
            }
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
                        match self
                            .record_executor_artifact(Some(identity), artifact)
                            .await
                        {
                            Ok(result) => inbound_acknowledgement(&message_id, result),
                            Err(error) => core_error_acknowledgement(&message_id, error),
                        }
                    }
                    Ok(validation::ParsedExecutorSubmission::StageOutcome(outcome)) => {
                        match self
                            .record_executor_stage_outcome(Some(identity), outcome)
                            .await
                        {
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
        mut observation: validation::ParsedObservation,
    ) -> Result<InboundResult, CoreError> {
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
        let observed_wall = Timestamp::now_utc();
        let now = crate::canonical_clock::project_mutation_time_at(&project, observed_wall);
        // Provider failure is not proof that the shell/container has stopped.
        let must_stop = matches!(run.run_spec_version, 2..=6)
            && matches!(
                observation.kind,
                forge_protocol::supervisor::v1::RunEventKind::ProviderFailed
                    | forge_protocol::supervisor::v1::RunEventKind::BudgetExhausted
                    | forge_protocol::supervisor::v1::RunEventKind::PolicyDenied
            );
        if must_stop {
            observation.state = RunObservedState::Stopping;
        }
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
                received_at: observed_wall,
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
        let git_source = super::source::source_descriptor(&run.run_spec, &observation)?;
        if let Some(descriptor) = &git_source {
            transaction
                .record_git_source_selection(run.id, descriptor)
                .await?;
        }
        if matches!(
            observation.kind,
            forge_protocol::supervisor::v1::RunEventKind::Running
                | forge_protocol::supervisor::v1::RunEventKind::Heartbeat
        ) {
            transaction
                .record_run_liveness(run.id, observed_wall)
                .await?;
        }
        if must_stop {
            let _ = transaction
                .request_run_stop(
                    run.id,
                    run.lease_fencing_token,
                    run.environment_epoch,
                    false,
                )
                .await?;
        }
        let terminal_observation = matches!(
            observation.state,
            RunObservedState::Stopped | RunObservedState::Failed | RunObservedState::Lost
        );
        if observation.state == RunObservedState::Stopped {
            self.write_back_run_auth(&mut transaction, &run).await?;
            transaction
                .record_environment_report(&forge_storage::EnvironmentReport {
                    scope: forge_domain::runtime::RunScope {
                        run_id: run.id,
                        fencing_token: run.lease_fencing_token,
                        environment_epoch: run.environment_epoch,
                    },
                    host_id: identity.host_id.clone(),
                    boot_id: identity.boot_id.clone(),
                    environment_id: String::new(),
                    quiescent: true,
                    unknown: false,
                })
                .await?;
        }
        // A terminal observation before a StageOutcome must not leave the
        // Task's leased queue entry behind. Completion moves that entry to
        // `completed` before the terminal observation, so only a still-leased
        // entry represents interrupted work that needs a human/manager decision.
        let pending_git_proposal =
            transaction
                .load_git_proposal(run.id)
                .await?
                .is_some_and(|proposal| {
                    matches!(
                        proposal.state,
                        forge_domain::git_delivery::GitProposalState::AwaitingQuiescence
                            | forge_domain::git_delivery::GitProposalState::Inspecting
                    )
                });
        let interrupted_queue = if terminal_observation && !pending_git_proposal {
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
        if terminal_observation && run.assignment.hook().is_some() {
            self.record_hook_terminal(
                &mut transaction,
                &mut project,
                &run,
                observation.state,
                &observation.details,
            )
            .await?;
        }
        let interrupted_event =
            if run.assignment.task_stage().is_some() && (interrupted_queue || must_stop) {
                pause_interrupted_task(
                    &mut transaction,
                    &mut project,
                    run.require_task_id()?,
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
        if run.assignment.task_stage().is_some()
            && (terminal_observation || must_stop)
            && !pending_git_proposal
            && !transaction.run_has_handoff(run.id).await?
        {
            let (incident_id, inserted) = transaction
                .record_run_incident(
                    project.id(),
                    run.id,
                    forge_storage::IncidentKind::Interrupted,
                    forge_domain::runtime::RecoveryAssessment::Unknown,
                )
                .await?;
            let task = load_scoped_task(&mut transaction, &project, run.require_task_id()?)
                .await?
                .task;
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
            if inserted {
                transaction
                    .append_event_and_outbox(&event(
                        project.id(),
                        AggregateRef::Task(task.id()),
                        task.revision().get(),
                        DomainEventKind::RunIncidentRaised,
                        self.actors.core,
                        CommandId::from(observation.message_id),
                        None,
                        event_payload([
                            ("run_id", json!(run.id)),
                            ("incident_id", json!(incident_id)),
                            ("kind", json!("interrupted")),
                        ]),
                        now,
                    )?)
                    .await?;
            }
        }
        if run.assignment.communication().is_some()
            && (terminal_observation || must_stop)
            && !transaction.communication_run_is_complete(run.id).await?
        {
            transaction.hold_communication_run(run.id).await?;
            let (incident_id, inserted) = transaction
                .record_run_incident(
                    project.id(),
                    run.id,
                    forge_storage::IncidentKind::Interrupted,
                    forge_domain::runtime::RecoveryAssessment::Unknown,
                )
                .await?;
            if inserted {
                transaction
                    .append_event_and_outbox(&event(
                        project.id(),
                        AggregateRef::Project(project.id()),
                        project.revision(),
                        DomainEventKind::RunIncidentRaised,
                        self.actors.core,
                        CommandId::from(observation.message_id),
                        None,
                        event_payload([
                            ("run_id", json!(run.id)),
                            ("incident_id", json!(incident_id)),
                            ("assignment", json!(run.assignment)),
                        ]),
                        now,
                    )?)
                    .await?;
            }
        }
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
                ("git_source", json!(git_source)),
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
        if terminal_observation {
            self.close_run_gateway(run.id).await;
        }
        if (terminal_observation || must_stop)
            && let Err(error) = self.revoke_run_proxy_key(&run).await
        {
            warn!(error=%error,"proxy key revocation deferred; scope is revoked locally");
        }
        if must_stop {
            let _ = self.deliver_pending_stop_requests(project_id).await;
        }
        if terminal_observation && let Err(error) = self.dispatch_available(project_id).await {
            warn!(error = %error, "could not dispatch work after terminal Run observation");
        }
        Ok(InboundResult::Accepted("Run observation recorded"))
    }
}

#[path = "bridge/lifecycle.rs"]
mod lifecycle;
pub(super) use lifecycle::refused_fenced_write;
use lifecycle::{pause_interrupted_task, stale_observation_event};

fn inbound_acknowledgement(message_id: &str, result: InboundResult) -> CoreToSupervisor {
    match result {
        InboundResult::ProposalSaved => acknowledgement(
            message_id,
            AcknowledgementDisposition::Accepted,
            "proposal_saved",
            "Git outcome proposal saved; awaiting quiescent inspection",
        ),
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
        CoreError::Storage(_) | CoreError::Repository(_) => (
            "canonical_storage_unavailable",
            "Core could not persist the Supervisor message",
        ),
        CoreError::SupervisorUnavailable => (
            "supervisor_channel_unavailable",
            "Core control channel is unavailable",
        ),
        CoreError::Application(_)
        | CoreError::Domain(_)
        | CoreError::InvalidTransport { .. }
        | CoreError::Forbidden => (
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
#[path = "bridge/tests.rs"]
mod tests;
