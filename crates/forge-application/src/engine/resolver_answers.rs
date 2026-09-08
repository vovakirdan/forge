//! Named resolver controls delegate answer semantics to the shared acceptance path.
use super::{
    CommandError, CommandTransaction, Engine,
    resolver_commands::{audit, human_assignment, refusal, touch_project},
    task_support::load_scoped_task,
};
use crate::command::resolver_input::{RerouteEscalationInput, SubmitHumanResolutionInput};
use forge_domain::{
    ActorKind, CommandId, DomainEvent, DomainEventKind, Project, Timestamp, resolution::*,
};
use serde_json::json;

impl Engine<'_> {
    pub(super) async fn accept_human_resolution(
        &self,
        tx: &mut impl CommandTransaction,
        project: &mut Project,
        input: &SubmitHumanResolutionInput,
        command: CommandId,
        now: Timestamp,
        events: &mut Vec<DomainEvent>,
    ) -> Result<forge_protocol::wire::ResourceReference, CommandError> {
        if self.actors.human.kind() != ActorKind::Human {
            return Err(CommandError::Forbidden);
        }
        let accepted = super::resolver_acceptance::accept_resolution(
            tx,
            project,
            input,
            super::resolver_acceptance::ResolutionAcceptanceContext {
                actor: self.actors.human,
                coordinator: self.actors.core,
                command_id: command,
                now,
                observed_wall: self.clock.now(),
                run_id: None,
            },
        )
        .await?;
        events.extend(accepted.events);
        Ok(super::receipt::resource(
            "escalation",
            accepted.escalation_id,
        ))
    }

    pub(super) async fn reroute_resolution(
        &self,
        tx: &mut impl CommandTransaction,
        project: &mut Project,
        input: &RerouteEscalationInput,
        command: CommandId,
        now: Timestamp,
        events: &mut Vec<DomainEvent>,
    ) -> Result<forge_protocol::wire::ResourceReference, CommandError> {
        let mut escalation = load_current(
            tx,
            project,
            input.escalation_id,
            input.expected_escalation_revision,
        )
        .await?;
        if let Some(source) = escalation.source.task() {
            let stored = load_scoped_task(tx, project, source.task_id).await?;
            validate_source(&escalation, &stored.task)?;
        } else if !tx.communication_escalation_is_current(&escalation).await? {
            return Err(refusal("Communication source is no longer current"));
        }
        if let EscalationState::Assigned { assignment_id } = escalation.state {
            let mut prior = tx
                .load_resolution_assignment(assignment_id)
                .await?
                .ok_or_else(|| refusal("current assignment absent"))?;
            if prior.resolver != Resolver::Human {
                for run in tx.lock_active_runs_for_project(project.id()).await? {
                    if run
                        .assignment
                        .resolution()
                        .is_some_and(|owner| owner.assignment_id == prior.id)
                    {
                        tx.request_run_stop(
                            run.id,
                            run.lease_fencing_token,
                            run.environment_epoch,
                            false,
                        )
                        .await?;
                        tx.revoke_run_lease(run.id, run.lease_fencing_token, run.environment_epoch)
                            .await?;
                        events.push(super::run_control::run_stop_event(
                            project,
                            &run,
                            command,
                            self.actors.human,
                            "resolution_rerouted",
                            now,
                        )?);
                    }
                }
            }
            prior.finish(ResolutionAssignmentState::Retired {
                reason: input.reason.clone(),
            })?;
            tx.update_resolution_assignment(&prior).await?;
        }
        let assignment = human_assignment(&mut escalation, self.actors.core, now)?;
        tx.save_escalation(&escalation, Some(input.expected_escalation_revision))
            .await?;
        tx.insert_resolution_assignment(&assignment).await?;
        touch_project(tx, project, now).await?;
        events.push(audit(
            project,
            DomainEventKind::EscalationRerouted,
            self.actors.human,
            command,
            now,
            json!({"escalation":escalation,"assignment":assignment,"reason":input.reason}),
        )?);
        Ok(super::receipt::resource("escalation", escalation.id))
    }
}

pub(super) async fn load_current(
    tx: &mut impl CommandTransaction,
    project: &Project,
    id: uuid::Uuid,
    expected: u64,
) -> Result<Escalation, CommandError> {
    let escalation = tx
        .load_escalation(id)
        .await?
        .filter(|value| value.project_id == project.id())
        .ok_or_else(|| refusal("escalation absent from Project"))?;
    if escalation.revision != expected {
        return Err(refusal("escalation revision is stale"));
    }
    Ok(escalation)
}
pub(super) fn validate_source(
    escalation: &Escalation,
    task: &forge_domain::Task,
) -> Result<(), CommandError> {
    let source = escalation
        .source
        .task()
        .ok_or_else(|| refusal("Task source required"))?;
    if task.lifecycle() != forge_domain::LifecycleStatus::Waiting
        || task.pipeline().pipeline_version_id() != source.pipeline_version_id
        || task.current_stage_id() != Some(&source.stage_id)
        || task.current_stage_visit() != Some(source.stage_visit)
        || !task.wait_conditions().any(|wait| {
            wait.id() == source.wait_condition_id
                && wait.kind() == &forge_domain::TaskWaitKind::EscalationPending
        })
    {
        return Err(refusal(
            "escalation source wait or stage visit is no longer current",
        ));
    }
    Ok(())
}
