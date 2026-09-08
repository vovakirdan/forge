//! Shared answer semantics for authorized Human commands and fenced resolver Gateway calls.
use super::{
    ArtifactLocation, CommandError, CommandTransaction,
    resolver_answers::{load_current, validate_source},
    resolver_commands::{audit, human_assignment, refusal, touch_project},
    scheduler::enqueue_if_employee,
    task_support::{
        current_stage_executor, load_scoped_task, persist_task_and_project, pinned_pipeline_version,
    },
};
pub use crate::command::resolver_input::SubmitHumanResolutionInput as ResolutionSubmission;
use forge_domain::{
    Actor, ActorKind, Artifact, ArtifactBody, ArtifactId, ArtifactKind, ArtifactProducer,
    CommandId, DomainEvent, DomainEventKind, EmployeeId, NewArtifact, Project, Timestamp,
    resolution::*,
};
use serde_json::json;
use uuid::Uuid;

/// Supplied by an already authenticated Human command or exact Run Gateway scope.
pub struct ResolutionAcceptanceContext {
    pub actor: Actor,
    pub coordinator: Actor,
    pub command_id: CommandId,
    pub now: Timestamp,
    pub observed_wall: Timestamp,
    pub run_id: Option<Uuid>,
}
pub struct AcceptedResolution {
    pub escalation_id: Uuid,
    pub artifact_id: ArtifactId,
    pub events: Vec<DomainEvent>,
    pub task_resumed: bool,
    pub communication_resume_pending: bool,
}

/// Caller holds Project lock and commits the answer with its own durable receipt.
pub async fn accept_resolution(
    tx: &mut impl CommandTransaction,
    project: &mut Project,
    input: &ResolutionSubmission,
    context: ResolutionAcceptanceContext,
) -> Result<AcceptedResolution, CommandError> {
    let mut escalation = load_current(
        tx,
        project,
        input.escalation_id,
        input.expected_escalation_revision,
    )
    .await?;
    let mut assignment = tx
        .load_resolution_assignment(input.assignment_id)
        .await?
        .ok_or_else(|| refusal("resolution assignment absent"))?;
    assignment.validate_owner(&escalation)?;
    let permitted = match assignment.resolver {
        Resolver::Human => context.actor.kind() == ActorKind::Human && context.run_id.is_none(),
        Resolver::Employee { employee_id } => {
            context.actor.kind() == ActorKind::Employee
                && context.actor.id().as_uuid() == employee_id.as_uuid()
                && context.run_id.is_some()
                && assignment
                    .lease
                    .expires_at
                    .is_some_and(|deadline| deadline > context.observed_wall)
        }
    };
    if !permitted
        || assignment.state != ResolutionAssignmentState::Active
        || assignment.lease.generation != input.lease_generation
    {
        return Err(CommandError::Forbidden);
    }
    input.answer.validate(&assignment.allowed_outcomes)?;
    let source = escalation.source.task().cloned();
    let mut stored = if let Some(source) = &source {
        let stored = load_scoped_task(tx, project, source.task_id).await?;
        validate_source(&escalation, &stored.task)?;
        if input.answer.disposition == ResolutionDisposition::ContinueStage
            && !tx
                .lock_active_runs_for_task(project.id(), source.task_id)
                .await?
                .is_empty()
        {
            return Err(refusal("source execution is not physically quiescent"));
        }
        Some(stored)
    } else {
        if !tx.communication_escalation_is_current(&escalation).await? {
            return Err(refusal(
                "Communication source is no longer the held attempt",
            ));
        }
        None
    };
    let artifact = Artifact::new(
        ArtifactId::new(),
        NewArtifact {
            project_id: project.id(),
            kind: ArtifactKind::new(ArtifactKind::DECISION_RECORD)?,
            title: "Escalation resolution".into(),
            body: ArtifactBody::inline_json(json!(input.answer)),
            metadata: json!({"escalation_id":escalation.id,"assignment_id":assignment.id,"lease_generation":assignment.lease.generation,"authority":"question_resolution_only"}),
            created_by: context.actor,
            created_at: context.now,
        },
    )?;
    let previous = stored.as_ref().map(|stored| stored.task.revision().get());
    if let Some(stored) = &mut stored {
        let employee = (context.actor.kind() == ActorKind::Employee)
            .then(|| EmployeeId::from(context.actor.id().as_uuid()));
        stored.task.attach_artifact(
            &artifact,
            ArtifactProducer::ResolutionAssignment,
            context.actor,
            employee,
            context.now,
        )?;
    }
    tx.insert_artifact(&artifact,&ArtifactLocation{task_id:source.as_ref().map(|source|source.task_id),run_id:context.run_id,stage_id:source.as_ref().map(|source|source.stage_id.clone()),producer:ArtifactProducer::ResolutionAssignment,producer_id:Some(assignment.id),producer_data:json!({"escalation_id":escalation.id,"generation":assignment.lease.generation})}).await?;
    assignment.finish(ResolutionAssignmentState::Answered {
        artifact_id: artifact.id(),
    })?;
    tx.update_resolution_assignment(&assignment).await?;
    let mut resumed = false;
    let successor = match input.answer.disposition {
        ResolutionDisposition::ContinueStage => {
            if let (Some(stored), Some(source)) = (&mut stored, &source) {
                resumed = stored
                    .task
                    .resolve_wait_condition(source.wait_condition_id, context.now)?;
            }
            escalation.advance(EscalationState::Resolved {
                artifact_id: artifact.id(),
            })?;
            None
        }
        ResolutionDisposition::NeedsManagementChange => {
            escalation.advance(EscalationState::NeedsManagementChange {
                artifact_id: artifact.id(),
            })?;
            None
        }
        ResolutionDisposition::ForwardToHuman => Some(human_assignment(
            &mut escalation,
            context.coordinator,
            context.now,
        )?),
    };
    let mut events = Vec::new();
    if let Some(stored) = stored {
        persist_task_and_project(
            tx,
            project,
            &stored.task,
            stored.persistence,
            previous.ok_or_else(|| refusal("missing original Task revision"))?,
            context.now,
        )
        .await?;
        events.push(super::task_support::task_event(
            project,
            &stored.task,
            DomainEventKind::TaskArtifactAttached,
            context.actor,
            context.command_id,
            context.now,
            super::event::event_payload([
                ("artifact_id", json!(artifact.id())),
                ("producer", json!(ArtifactProducer::ResolutionAssignment)),
            ]),
        )?);
        if resumed {
            let version = pinned_pipeline_version(tx, project, &stored.task).await?;
            enqueue_if_employee(
                tx,
                project,
                &stored.task,
                &stored.persistence,
                current_stage_executor(&version, &stored.task)?,
                context.observed_wall,
            )
            .await?;
            events.push(super::task_support::task_event(
                project,
                &stored.task,
                DomainEventKind::TaskResumed,
                context.actor,
                context.command_id,
                context.now,
                super::event::event_payload([("escalation_id", json!(escalation.id))]),
            )?);
        }
    } else {
        touch_project(tx, project, context.now).await?;
    }
    tx.save_escalation(&escalation, Some(input.expected_escalation_revision))
        .await?;
    if let Some(successor) = successor {
        tx.insert_resolution_assignment(&successor).await?;
        events.push(audit(
            project,
            DomainEventKind::ResolutionAssigned,
            context.coordinator,
            context.command_id,
            context.now,
            json!({"assignment":successor}),
        )?);
    }
    events.push(super::event::event(
        project.id(),
        forge_domain::AggregateRef::Artifact(artifact.id()),
        1,
        DomainEventKind::ArtifactCreated,
        context.actor,
        context.command_id,
        None,
        super::event::event_payload([
            ("kind", json!(artifact.kind())),
            ("assignment_id", json!(assignment.id)),
        ]),
        context.now,
    )?);
    let communication_resume_pending =
        source.is_none() && input.answer.disposition == ResolutionDisposition::ContinueStage;
    events.push(audit(project,DomainEventKind::ResolutionSubmitted,context.actor,context.command_id,context.now,json!({"escalation":escalation,"assignment":assignment,"artifact_id":artifact.id(),"answer":input.answer,"task_resumed":resumed,"communication_resume_pending":communication_resume_pending,"stage_completed":false}))?);
    Ok(AcceptedResolution {
        escalation_id: escalation.id,
        artifact_id: artifact.id(),
        events,
        task_resumed: resumed,
        communication_resume_pending,
    })
}
