//! Shared Task pause and canonical question creation for Human and fenced Gateway callers.
use super::{
    CommandError, CommandTransaction, Engine,
    event::event_payload,
    resolver_commands::{audit, human_assignment, refusal},
    run_control::run_stop_event,
    task_support::{
        load_scoped_task, persist_task_and_project, pinned_pipeline_version, require_task_revision,
        task_event,
    },
};
use crate::command::RaiseEscalationInput;
use forge_domain::{
    Actor, CommandId, DomainEvent, DomainEventKind, Project, TaskWaitCondition, TaskWaitKind,
    Timestamp, WaitConditionId, resolution::*,
};
use serde_json::json;
use uuid::Uuid;

impl Engine<'_> {
    #[allow(clippy::too_many_arguments)]
    pub async fn raise_task_escalation(
        &self,
        tx: &mut impl CommandTransaction,
        project: &mut Project,
        input: &RaiseEscalationInput,
        escalation_id: Uuid,
        wait_condition_id: WaitConditionId,
        actor: Actor,
        command: CommandId,
        now: Timestamp,
    ) -> Result<(Escalation, Vec<DomainEvent>), CommandError> {
        let mut events = Vec::new();
        let mut stored = load_scoped_task(tx, project, input.task_id).await?;
        require_task_revision(&stored.task, input.expected_task_revision)?;
        let version = pinned_pipeline_version(tx, project, &stored.task).await?;
        let stage_id = stored
            .task
            .current_stage_id()
            .cloned()
            .ok_or_else(|| refusal("escalation requires an approved Task stage"))?;
        let stage_visit = stored
            .task
            .current_stage_visit()
            .ok_or_else(|| refusal("escalation requires a stage visit"))?;
        let stage = version
            .stage(&stage_id)
            .ok_or_else(|| refusal("stage absent from pinned version"))?;
        let route = match &input.route_key {
            Some(key) => Some(
                tx.load_resolver_route(project.id(), key)
                    .await?
                    .ok_or_else(|| refusal("unknown resolver route"))?,
            ),
            None => None,
        };
        let mut escalation = Escalation {
            id: escalation_id,
            project_id: project.id(),
            source: EscalationSource::Task(TaskEscalationSource {
                task_id: input.task_id,
                pipeline_version_id: version.id(),
                stage_id,
                stage_visit,
                wait_condition_id,
            }),
            category: input.category,
            question: input.question.clone(),
            route,
            allowed_outcomes: stage
                .transitions()
                .map(|transition| transition.outcome().as_str().to_owned())
                .collect(),
            created_by: actor,
            created_at: now,
            revision: 1,
            generation: 0,
            next_candidate: 0,
            state: EscalationState::Queued,
        };
        escalation.validate_snapshot()?;
        let source = escalation
            .source
            .task()
            .cloned()
            .ok_or_else(|| refusal("Task source required"))?;
        stored.task.add_wait_condition(
            TaskWaitCondition::new(
                source.wait_condition_id,
                TaskWaitKind::EscalationPending,
                Some(input.question.clone()),
                actor,
                now,
            )?,
            now,
        )?;
        let runs = tx
            .lock_active_runs_for_task(project.id(), input.task_id)
            .await?;
        for run in &runs {
            tx.request_run_stop(
                run.id,
                run.lease_fencing_token,
                run.environment_epoch,
                false,
            )
            .await?;
            tx.cancel_leased_queue_for_fenced_run(
                run.id,
                run.lease_fencing_token,
                run.environment_epoch,
            )
            .await?;
        }
        persist_task_and_project(
            tx,
            project,
            &stored.task,
            stored.persistence,
            input.expected_task_revision,
            now,
        )
        .await?;
        events.push(task_event(
            project,
            &stored.task,
            DomainEventKind::TaskWaiting,
            actor,
            command,
            now,
            event_payload([
                ("wait_kind", json!("escalation_pending")),
                ("wait_condition_id", json!(source.wait_condition_id)),
                ("escalation_id", json!(escalation.id)),
            ]),
        )?);
        for run in &runs {
            events.push(run_stop_event(
                project,
                run,
                command,
                actor,
                "escalation_raised",
                now,
            )?);
        }

        self.record_escalation(
            tx,
            project,
            &mut escalation,
            actor,
            command,
            now,
            &mut events,
        )
        .await?;
        Ok((escalation, events))
    }

    /// Caller has already held its exact source and advanced the Project once.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_escalation(
        &self,
        tx: &mut impl CommandTransaction,
        project: &Project,
        escalation: &mut Escalation,
        actor: Actor,
        command: CommandId,
        now: Timestamp,
        events: &mut Vec<DomainEvent>,
    ) -> Result<(), CommandError> {
        escalation.validate_snapshot()?;
        let assignment = if escalation.requires_human() {
            Some(human_assignment(escalation, self.actors.core, now)?)
        } else {
            None
        };
        tx.save_escalation(escalation, None).await?;
        if let Some(assignment) = assignment {
            tx.insert_resolution_assignment(&assignment).await?;
            events.push(audit(
                project,
                DomainEventKind::ResolutionAssigned,
                self.actors.core,
                command,
                now,
                json!({"assignment":assignment}),
            )?);
        }
        events.push(audit(
            project,
            DomainEventKind::EscalationRaised,
            actor,
            command,
            now,
            json!({"escalation":escalation}),
        )?);

        Ok(())
    }
}
