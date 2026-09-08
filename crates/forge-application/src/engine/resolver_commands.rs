//! Named management controls create questions, never synthetic execution Tasks.
use super::{
    CommandError, CommandTransaction, Engine,
    event::{event, event_payload},
    receipt::{finish_command, resource},
};
use crate::{CommandEnvelope, CommandPayload};
use forge_domain::{
    Actor, AggregateRef, CommandId, DomainEvent, DomainEventKind, Project, Timestamp, resolution::*,
};
use forge_protocol::wire::CommandReceipt;
use serde_json::json;
use uuid::Uuid;

impl Engine<'_> {
    pub(super) async fn manage_resolution(
        &self,
        tx: &mut impl CommandTransaction,
        mut project: Project,
        envelope: &CommandEnvelope,
        hash: &str,
        command: CommandId,
        now: Timestamp,
    ) -> Result<CommandReceipt, CommandError> {
        let mut events = Vec::new();
        let target = match &envelope.payload {
            CommandPayload::ConfigureResolverRoute(input) => {
                let old = tx
                    .load_resolver_route(project.id(), &input.route_key)
                    .await?;
                let expected = old.as_ref().map(|route| route.revision);
                let route = ResolverRoute {
                    project_id: project.id(),
                    key: input.route_key.clone(),
                    revision: expected
                        .unwrap_or(0)
                        .checked_add(1)
                        .ok_or_else(|| refusal("route revision exhausted"))?,
                    employee_ids: input.employee_ids.clone(),
                    assignment_timeout_seconds: input.assignment_timeout_seconds,
                    updated_at: now,
                };
                route.validate_snapshot()?;
                for employee in &route.employee_ids {
                    if tx
                        .lock_employee(*employee)
                        .await?
                        .is_none_or(|value| value.project_id() != project.id())
                    {
                        return Err(refusal("route candidate is absent from Project"));
                    }
                }
                tx.save_resolver_route(&route, expected).await?;
                touch_project(tx, &mut project, now).await?;
                events.push(audit(
                    &project,
                    DomainEventKind::ResolverRouteConfigured,
                    self.actors.human,
                    command,
                    now,
                    json!({"route":route}),
                )?);
                resource("project", project.id().as_uuid())
            }
            CommandPayload::RaiseEscalation(input) => {
                let (escalation, raised) = self
                    .raise_task_escalation(
                        tx,
                        &mut project,
                        input,
                        Uuid::now_v7(),
                        forge_domain::WaitConditionId::new(),
                        self.actors.human,
                        command,
                        now,
                    )
                    .await?;
                events.extend(raised);
                resource("escalation", escalation.id)
            }
            CommandPayload::SubmitHumanResolution(input) => {
                self.accept_human_resolution(tx, &mut project, input, command, now, &mut events)
                    .await?
            }
            CommandPayload::RerouteEscalation(input) => {
                self.reroute_resolution(tx, &mut project, input, command, now, &mut events)
                    .await?
            }
            _ => return Err(CommandError::UnsupportedCommand),
        };
        finish_command(
            tx,
            &project,
            envelope,
            hash,
            command,
            self.actors.human,
            events,
            Some(target),
        )
        .await
    }
}

pub(super) fn human_assignment(
    escalation: &mut Escalation,
    coordinator: Actor,
    now: Timestamp,
) -> Result<ResolutionAssignment, CommandError> {
    escalation.generation = escalation
        .generation
        .checked_add(1)
        .ok_or_else(|| refusal("resolution generation exhausted"))?;
    let assignment = ResolutionAssignment {
        id: Uuid::now_v7(),
        project_id: escalation.project_id,
        escalation_id: escalation.id,
        resolver: Resolver::Human,
        lease: ResolutionLease {
            generation: escalation.generation,
            held_by: Actor::system_manager(coordinator.id()),
            expires_at: None,
        },
        allowed_outcomes: escalation.allowed_outcomes.clone(),
        issued_at: now,
        state: ResolutionAssignmentState::Active,
    };
    assignment.validate_snapshot()?;
    escalation.advance(EscalationState::Assigned {
        assignment_id: assignment.id,
    })?;
    Ok(assignment)
}

pub(super) async fn touch_project(
    tx: &mut impl CommandTransaction,
    project: &mut Project,
    now: Timestamp,
) -> Result<(), CommandError> {
    let previous = project.revision();
    project.record_child_mutation(now)?;
    tx.update_project(project, previous).await?;
    Ok(())
}
pub(super) fn audit(
    project: &Project,
    kind: DomainEventKind,
    actor: Actor,
    command: CommandId,
    now: Timestamp,
    value: serde_json::Value,
) -> Result<DomainEvent, CommandError> {
    Ok(event(
        project.id(),
        AggregateRef::Project(project.id()),
        project.revision(),
        kind,
        actor,
        command,
        None,
        event_payload([("resolution", value)]),
        now,
    )?)
}
pub(super) fn refusal(reason: &str) -> CommandError {
    CommandError::InvalidTransport {
        field: "resolution",
        reason: reason.into(),
    }
}
