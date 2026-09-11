//! Durable native input is observational transport, never Employee acknowledgement.
use super::{SupervisorIdentity, bridge::InboundResult};
use crate::{
    CoreError, CoreService,
    event::{event, event_payload},
};
use forge_domain::{
    ActorKind, AggregateRef, CommandId, DomainEventKind, ProjectId, RuntimeCapability,
    communication::{
        DeliveryReceipt, DeliveryReceiptKind, RuntimeInputAction, RuntimeInputCloseReason,
        RuntimeInputOutcome,
    },
    runtime::{RunScope, RuntimeLaunchSpec},
};
use forge_protocol::supervisor::v1::{
    CloseRuntimeInput, CoreToSupervisor, DeliverRuntimeInput, RuntimeInputReceipt,
    RuntimeInputStatus, RuntimeMessageInput, core_to_supervisor, deliver_runtime_input,
};
use forge_storage::{RunProjection, StoredRuntimeInput};
use serde_json::json;
use uuid::Uuid;

impl CoreService {
    pub(crate) async fn reconcile_runtime_inputs(
        &self,
        project: Option<ProjectId>,
    ) -> Result<(), CoreError> {
        if self.supervisor.reconciled_identity().await.is_none() {
            return Ok(());
        }
        let runs = {
            // Round-robin admission prevents permanently busy/idle early Runs
            // starving later ones. Cursor is operational, not canonical state.
            let mut cursor = self.runtime_input_cursor.lock().await;
            let runs = self.store.runtime_input_runs(project, *cursor).await?;
            if let Some(last) = runs.last() {
                *cursor = Some(*last);
            }
            runs
        };
        for run_id in runs {
            let mut tx = self.store.begin().await?;
            let Some(run) = tx.load_run(run_id).await? else {
                continue;
            };
            let Some(mut project) = tx.lock_project(run.project_id).await? else {
                continue;
            };
            let spec: RuntimeLaunchSpec = serde_json::from_value(run.run_spec.clone())
                .map_err(|_| invalid("invalid native input profile"))?;
            if !matches!(spec.schema_version, 2 | 3 | 6)
                || !matches!(
                    spec.binding.execution_profile.adapter_id(),
                    "claude_code_cli" | "codex_cli" | "opencode_runtime"
                )
                || !spec
                    .binding
                    .execution_profile
                    .capability_profile()
                    .capabilities
                    .contains(&RuntimeCapability::LiveInput)
            {
                continue;
            }
            let now = crate::canonical_clock::project_mutation_time(&project);
            let scope = scope(&run);
            let active = tx.gateway_scope_is_active(scope).await?;
            let task_scope = if active && run.assignment.task_stage().is_some() {
                crate::gateway::task_scope(&mut tx, &run).await.ok()
            } else {
                None
            };
            let completed = run.assignment.communication().is_some()
                && tx.communication_run_is_complete(run.id).await?;
            let close = !active
                || completed
                || (run.assignment.task_stage().is_some() && task_scope.is_none());
            let mut inserted = Vec::new();
            if close {
                let action = RuntimeInputAction::CloseAfterTurn {
                    reason: if completed {
                        RuntimeInputCloseReason::AssignmentCompleted
                    } else {
                        RuntimeInputCloseReason::AuthorityLost
                    },
                };
                if let Some(id) = tx.create_runtime_input(&run, &action, now).await? {
                    inserted.push(id);
                }
            } else if let Some(mut delivery) = task_scope {
                let messages = tx.messages_for_native_input(&delivery).await?;
                for source in messages {
                    delivery.thread_id = source.data().thread_id;
                    delivery
                        .validate_message(&source)
                        .map_err(|_| invalid("invalid native input source"))?;
                    if source.data().sender.kind() == ActorKind::Employee
                        && source.data().sender.id().as_uuid()
                            == run.require_employee_id()?.as_uuid()
                    {
                        continue;
                    }
                    let action = RuntimeInputAction::Message {
                        source: Box::new(source),
                    };
                    if let Some(id) = tx.create_runtime_input(&run, &action, now).await? {
                        inserted.push(id);
                    }
                }
            }
            if !inserted.is_empty() {
                let previous = project.revision();
                project.record_child_mutation(now)?;
                tx.update_project(&project, previous).await?;
                tx.append_event_and_outbox(&event(
                    project.id(),
                    AggregateRef::Project(project.id()),
                    project.revision(),
                    DomainEventKind::RuntimeInputRequested,
                    self.actors.core,
                    CommandId::new(),
                    None,
                    event_payload([
                        ("run_id", json!(run.id)),
                        ("input_ids", json!(inserted)),
                        ("closed", json!(close)),
                    ]),
                    now,
                )?)
                .await?;
            }
            let inputs = tx.runtime_inputs(run.id).await?;
            tx.commit().await?;
            for input in inputs
                .iter()
                .filter(|input| {
                    input.outcome.is_none()
                        && (!close
                            || matches!(input.action, RuntimeInputAction::CloseAfterTurn { .. }))
                })
                .take(16)
            {
                let mut gate = self.store.begin().await?;
                gate.lock_project(run.project_id).await?;
                if matches!(input.action, RuntimeInputAction::Message { .. })
                    && (!gate.gateway_scope_is_active(scope).await?
                        || crate::gateway::task_scope(&mut gate, &run).await.is_err())
                {
                    continue;
                }
                // Keep the final authority lock bounded even if transport backs up.
                tokio::time::timeout(
                    std::time::Duration::from_millis(100),
                    self.supervisor.send(CoreToSupervisor {
                        message: Some(core_to_supervisor::Message::DeliverRuntimeInput(to_wire(
                            input,
                        )?)),
                    }),
                )
                .await
                .map_err(|_| CoreError::SupervisorUnavailable)??;
                gate.commit().await?;
            }
        }
        Ok(())
    }

    pub(super) async fn record_runtime_input_receipt(
        &self,
        identity: &SupervisorIdentity,
        receipt: RuntimeInputReceipt,
    ) -> Result<InboundResult, CoreError> {
        if self.supervisor.reconciled_identity().await.as_ref() != Some(identity) {
            return Ok(InboundResult::Ignored("runtime_input_stale_supervisor"));
        }
        let id = parse(&receipt.input_command_id)?;
        let receipt_id = parse(&receipt.message_id)?;
        let run_id = parse(&receipt.run_id)?;
        let outcome = from_wire(receipt.status)?;
        let mut tx = self.store.begin().await?;
        let run = tx
            .load_run(run_id)
            .await?
            .ok_or_else(|| invalid("unknown input Run"))?;
        let mut project = tx
            .lock_project(run.project_id)
            .await?
            .ok_or_else(|| invalid("unknown input Project"))?;
        if receipt.lease_fencing_token != run.lease_fencing_token
            || receipt.environment_epoch != run.environment_epoch
        {
            return Ok(InboundResult::Ignored("runtime_input_stale_scope"));
        }
        let input = tx
            .runtime_input(id)
            .await?
            .filter(|input| input.scope == scope(&run))
            .ok_or_else(|| invalid("unknown input identity"))?;
        if let Some((prior_input, prior_outcome)) = tx.runtime_input_observation(receipt_id).await?
        {
            return if prior_input == id && prior_outcome == outcome {
                Ok(InboundResult::Accepted("runtime_input_replayed"))
            } else {
                Err(invalid("conflicting receipt identity"))
            };
        }
        if let Some(prior) = input.outcome
            && prior != outcome
            && !matches!(
                (prior, outcome),
                (
                    RuntimeInputOutcome::DeliveryUnknown,
                    RuntimeInputOutcome::RuntimeAccepted
                ) | (
                    RuntimeInputOutcome::RuntimeAccepted,
                    RuntimeInputOutcome::DeliveryUnknown
                )
            )
        {
            return Err(invalid("conflicting input observation"));
        }
        let now = crate::canonical_clock::project_mutation_time(&project);
        match (&input.action, outcome) {
            (RuntimeInputAction::Message { source }, RuntimeInputOutcome::RuntimeAccepted) => {
                // This is a late-arriving historical observation, not permission
                // for a current Employee action. Validate the frozen assignment.
                let mut delivery = frozen_task_scope(&run)?;
                delivery.thread_id = source.data().thread_id;
                let receipt = DeliveryReceipt::new(
                    source,
                    delivery,
                    DeliveryReceiptKind::RuntimeAccepted,
                    None,
                    now,
                )?;
                tx.insert_delivery_receipt(&receipt).await?;
            }
            (RuntimeInputAction::CloseAfterTurn { .. }, RuntimeInputOutcome::InputClosed) => {}
            (_, RuntimeInputOutcome::RuntimeAccepted | RuntimeInputOutcome::InputClosed) => {
                return Err(invalid("input outcome does not match control"));
            }
            _ => {}
        }
        if !tx
            .observe_runtime_input(id, receipt_id, outcome, now)
            .await?
        {
            return Err(invalid("input receipt identity conflict"));
        }
        let mut retired = Vec::new();
        if matches!(input.action, RuntimeInputAction::CloseAfterTurn { .. }) {
            // A terminal close observation cannot prove whether queued bytes were
            // consumed before shutdown. Preserve that uncertainty, not fake ACKs.
            for pending in tx.runtime_inputs(run.id).await? {
                if matches!(pending.action, RuntimeInputAction::Message { .. })
                    && tx
                        .observe_runtime_input(
                            pending.id,
                            Uuid::now_v7(),
                            RuntimeInputOutcome::DeliveryUnknown,
                            now,
                        )
                        .await?
                {
                    retired.push(pending.id);
                }
            }
        }
        let previous = project.revision();
        project.record_child_mutation(now)?;
        tx.update_project(&project, previous).await?;
        tx.append_event_and_outbox(&event(
            project.id(),
            AggregateRef::Project(project.id()),
            project.revision(),
            DomainEventKind::RuntimeInputObserved,
            identity.actor(),
            CommandId::from(receipt_id),
            None,
            event_payload([
                ("run_id", json!(run.id)),
                ("input_id", json!(id)),
                (
                    "source_message_id",
                    json!(match &input.action {
                        RuntimeInputAction::Message { source } => Some(source.data().id),
                        RuntimeInputAction::CloseAfterTurn { .. } => None,
                    }),
                ),
                ("fencing_token", json!(run.lease_fencing_token)),
                ("environment_epoch", json!(run.environment_epoch)),
                ("outcome", json!(outcome)),
                ("undelivered_inputs_retired", json!(retired)),
            ]),
            now,
        )?)
        .await?;
        tx.commit().await?;
        Ok(InboundResult::Accepted("runtime_input_observed"))
    }
}

fn frozen_task_scope(
    run: &RunProjection,
) -> Result<forge_domain::communication::DeliveryScope, CoreError> {
    let snapshot: forge_domain::ContextSnapshot =
        serde_json::from_value(run.context_manifest.clone())
            .map_err(|_| invalid("invalid pinned input context"))?;
    let data = snapshot.data();
    let task = run.require_task_stage()?;
    if data.run_id != run.id
        || data.project_id != run.project_id
        || Some(data.employee_id) != run.employee_id
        || data.task_id != task.task_id
        || data.stage_id.as_str() != task.stage_id.as_str()
    {
        return Err(invalid("input context does not match assignment"));
    }
    Ok(forge_domain::communication::DeliveryScope {
        project_id: run.project_id,
        employee_id: run.require_employee_id()?,
        thread_id: run.id,
        run_id: run.id,
        fencing_token: run.lease_fencing_token,
        environment_epoch: run.environment_epoch,
        task_context: Some(forge_domain::communication::TaskMessageContext {
            task_id: task.task_id,
            pipeline_version_id: data.pipeline_version_id,
            stage_id: data.stage_id.clone(),
            stage_visit: data
                .stage_visit
                .ok_or_else(|| invalid("input requires pinned stage visit"))?,
        }),
    })
}

fn scope(run: &RunProjection) -> RunScope {
    RunScope {
        run_id: run.id,
        fencing_token: run.lease_fencing_token,
        environment_epoch: run.environment_epoch,
    }
}
fn parse(value: &str) -> Result<Uuid, CoreError> {
    Uuid::parse_str(value)
        .ok()
        .filter(|id| id.get_version_num() == 7)
        .ok_or_else(|| invalid("invalid input identity"))
}
fn invalid(reason: &str) -> CoreError {
    CoreError::InvalidTransport {
        field: "runtime_input",
        reason: reason.into(),
    }
}
fn from_wire(code: i32) -> Result<RuntimeInputOutcome, CoreError> {
    Ok(
        match RuntimeInputStatus::try_from(code).map_err(|_| invalid("unknown input status"))? {
            RuntimeInputStatus::RuntimeAccepted => RuntimeInputOutcome::RuntimeAccepted,
            RuntimeInputStatus::InputClosed => RuntimeInputOutcome::InputClosed,
            RuntimeInputStatus::Unsupported => RuntimeInputOutcome::Unsupported,
            RuntimeInputStatus::StaleScope => RuntimeInputOutcome::StaleScope,
            RuntimeInputStatus::Conflict => RuntimeInputOutcome::Conflict,
            RuntimeInputStatus::DeliveryUnknown => RuntimeInputOutcome::DeliveryUnknown,
            RuntimeInputStatus::Unspecified => return Err(invalid("missing input status")),
        },
    )
}
fn to_wire(input: &StoredRuntimeInput) -> Result<DeliverRuntimeInput, CoreError> {
    let action = match &input.action {
        RuntimeInputAction::Message { source } => {
            deliver_runtime_input::Action::Message(RuntimeMessageInput {
                source_message_json: serde_json::to_string(source)
                    .map_err(|_| invalid("input encoding failed"))?,
            })
        }
        RuntimeInputAction::CloseAfterTurn { reason } => {
            deliver_runtime_input::Action::CloseAfterTurn(CloseRuntimeInput {
                reason_code: match reason {
                    RuntimeInputCloseReason::AssignmentCompleted => "assignment_completed",
                    RuntimeInputCloseReason::AuthorityLost => "authority_lost",
                }
                .into(),
            })
        }
    };
    Ok(DeliverRuntimeInput {
        command_id: input.id.to_string(),
        run_id: input.scope.run_id.to_string(),
        lease_fencing_token: input.scope.fencing_token,
        environment_epoch: input.scope.environment_epoch,
        sequence: input.sequence,
        action: Some(action),
    })
}
