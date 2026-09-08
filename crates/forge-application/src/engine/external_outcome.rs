//! Human and external Pipeline outcome submission.

use std::collections::BTreeSet;

use super::{ArtifactLocation, CommandTransaction};
use crate::{CommandEnvelope, ExternalStageOutcomeCommand};
use forge_domain::{
    AggregateRef, Artifact, ArtifactId, ArtifactProducer, CommandId, DomainError, DomainEvent,
    DomainEventKind, PipelineTransitionTarget, Project, StageOutcomeSubmission,
    StageTransitionEffect, TaskWaitCondition, TaskWaitKind, Timestamp,
};
use serde_json::json;

use super::{
    CommandError, Engine,
    event::{event, event_payload},
    receipt::{finish_command, resource},
    scheduler::enqueue_if_employee,
    task_support::{
        current_stage_executor, load_scoped_task, persist_task_and_project,
        pinned_pipeline_version, require_task_revision, retained_persistence, stage_payload,
        task_event,
    },
};

impl Engine<'_> {
    #[allow(clippy::too_many_arguments)]
    pub async fn apply_external_outcome_command(
        &self,
        transaction: &mut impl CommandTransaction,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
        input: &ExternalStageOutcomeCommand,
    ) -> Result<forge_protocol::wire::CommandReceipt, CommandError> {
        let (task_id, wait_condition_id) = input.identifiers()?;
        let expected_stage_id = input.stage_id()?;
        let outcome = input.outcome()?;
        let mut outcome_artifact_ids = input.outcome_artifact_ids()?;
        let cancellation_reason_id = input.cancellation_reason_id()?;
        let stored = load_scoped_task(transaction, &project, task_id).await?;
        require_task_revision(&stored.task, input.expected_task_revision)?;
        if stored
            .task
            .wait_conditions()
            .any(|condition| matches!(condition.kind(), TaskWaitKind::RetryExhausted))
        {
            return Err(CommandError::InvalidTransport {
                field: "stage_id",
                reason: "retry-exhausted task accepts only cancellation in M0".to_owned(),
            });
        }
        if stored.task.current_stage_id() != Some(&expected_stage_id) {
            return Err(CommandError::InvalidTransport {
                field: "stage_id",
                reason: "does not match the task current stage".to_owned(),
            });
        }
        let version = pinned_pipeline_version(transaction, &project, &stored.task).await?;
        let current_executor = current_stage_executor(&version, &stored.task)?;
        if !current_executor.requires_wait() {
            return Err(CommandError::InvalidTransport {
                field: "stage_id",
                reason: "does not accept a human/external outcome".to_owned(),
            });
        }
        let next_wait = next_stage_wait(&version, &stored.task, &outcome, self.actors.core, now)?;
        verify_existing_artifacts(transaction, &project, &stored.task, &outcome_artifact_ids)
            .await?;

        let previous_revision = stored.task.revision().get();
        let mut task = stored.task;
        let mut events = Vec::<DomainEvent>::new();
        for input_artifact in &input.artifacts {
            let artifact = Artifact::new(
                ArtifactId::new(),
                input_artifact.to_new_artifact(project.id(), self.actors.human, now)?,
            )?;
            task.attach_artifact(
                &artifact,
                ArtifactProducer::Human,
                self.actors.human,
                None,
                now,
            )?;
            let task_revision = task.revision().get();
            transaction
                .insert_artifact(
                    &artifact,
                    &ArtifactLocation {
                        task_id: Some(task.id()),
                        run_id: None,
                        stage_id: Some(expected_stage_id.clone()),
                        producer: ArtifactProducer::Human,
                        producer_id: None,
                        producer_data: json!({}),
                    },
                )
                .await?;
            outcome_artifact_ids.insert(artifact.id());
            events.push(event(
                project.id(),
                AggregateRef::Artifact(artifact.id()),
                1,
                DomainEventKind::ArtifactCreated,
                self.actors.human,
                command_id,
                None,
                event_payload([("kind", json!(artifact.kind().as_str()))]),
                now,
            )?);
            events.push(event(
                project.id(),
                AggregateRef::Task(task.id()),
                task_revision,
                DomainEventKind::TaskArtifactAttached,
                self.actors.human,
                command_id,
                None,
                event_payload([("artifact_id", json!(artifact.id().as_uuid()))]),
                now,
            )?);
        }

        let submission = StageOutcomeSubmission {
            outcome: outcome.clone(),
            outcome_artifact_ids: outcome_artifact_ids.clone(),
            submitted_by: self.actors.human,
            submitted_at: now,
            cancellation_reason_id,
            cancellation_note: None,
        };
        let effect = match version.apply_outcome(
            &mut task,
            submission,
            project.cancellation_reasons(),
            Some(wait_condition_id),
            next_wait,
        ) {
            Ok(effect) => Some(effect),
            Err(DomainError::StageVisitLimitExceeded { maximum }) => {
                task.add_wait_condition(
                    TaskWaitCondition::new(
                        forge_domain::WaitConditionId::new(),
                        TaskWaitKind::RetryExhausted,
                        Some(format!("pipeline max_stage_visits={maximum} exhausted")),
                        self.actors.core,
                        now,
                    )?,
                    now,
                )?;
                events.push(task_event(
                    &project,
                    &task,
                    DomainEventKind::TaskRetryExhausted,
                    self.actors.core,
                    command_id,
                    now,
                    event_payload([("max_stage_visits", json!(maximum))]),
                )?);
                // Keep the public audit projection symmetric with the
                // employee-run path: retry exhaustion leaves the Task waiting
                // even when its current stage is human/external.
                events.push(task_event(
                    &project,
                    &task,
                    DomainEventKind::TaskWaiting,
                    self.actors.core,
                    command_id,
                    now,
                    event_payload([
                        ("wait_kind", json!("retry_exhausted")),
                        ("max_stage_visits", json!(maximum)),
                    ]),
                )?);
                None
            }
            Err(error) => return Err(error.into()),
        };
        if task.lifecycle() == forge_domain::LifecycleStatus::Done
            && !transaction
                .required_hooks_satisfied(&task, &version, None)
                .await?
        {
            return Err(CommandError::InvalidTransport{field:"outcome",reason:"applicable required project hooks have no passing result for the current candidate".into()});
        }
        let persistence = retained_persistence(&project, &task, stored.persistence)?;
        persist_task_and_project(
            transaction,
            &mut project,
            &task,
            persistence,
            previous_revision,
            now,
        )
        .await?;
        if let Some(StageTransitionEffect::EnteredStage { executor_kind, .. }) = effect {
            enqueue_if_employee(
                transaction,
                &project,
                &task,
                &persistence,
                executor_kind,
                self.clock.now(),
            )
            .await?;
        }
        if let Some(effect) = effect.as_ref() {
            events.push(task_event(
                &project,
                &task,
                outcome_event_kind(effect.clone()),
                self.actors.human,
                command_id,
                now,
                stage_payload(&task),
            )?);
        }
        match &effect {
            Some(StageTransitionEffect::Completed) => {
                events.extend(
                    self.resolve_dependency_waits_after_completion(
                        transaction,
                        &mut project,
                        task.id(),
                        command_id,
                        now,
                    )
                    .await?,
                );
            }
            Some(StageTransitionEffect::Cancelled) => {
                events.extend(
                    self.wait_dependents_for_unsatisfied_blocker(
                        transaction,
                        &mut project,
                        task.id(),
                        command_id,
                        now,
                    )
                    .await?,
                );
            }
            Some(StageTransitionEffect::EnteredStage { .. }) | None => {}
        }
        let receipt = finish_command(
            transaction,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            events,
            Some(resource("task", task.id().as_uuid())),
        )
        .await?;
        if effect.is_some() {
            super::external_handoff::persist(
                transaction,
                &task,
                self.actors.human,
                command_id,
                expected_stage_id,
                outcome,
                outcome_artifact_ids,
                now,
            )
            .await?;
        }
        Ok(receipt)
    }
}

fn next_stage_wait(
    version: &forge_domain::PipelineVersion,
    task: &forge_domain::Task,
    outcome: &forge_domain::OutcomeKey,
    actor: forge_domain::Actor,
    now: Timestamp,
) -> Result<Option<TaskWaitCondition>, CommandError> {
    let current_stage_id = task
        .current_stage_id()
        .ok_or(CommandError::InvalidTransport {
            field: "task.current_stage_id",
            reason: "is absent for the submitted outcome".to_owned(),
        })?;
    let current_stage = version
        .stage(current_stage_id)
        .ok_or(CommandError::InvalidTransport {
            field: "task.current_stage_id",
            reason: "is absent from the pinned pipeline version".to_owned(),
        })?;
    let transition = current_stage
        .transition(outcome)
        .ok_or(CommandError::InvalidTransport {
            field: "outcome",
            reason: "is not declared by the current pipeline stage".to_owned(),
        })?;
    let PipelineTransitionTarget::Stage(next_stage_id) = transition.target() else {
        return Ok(None);
    };
    let next_stage = version
        .stage(next_stage_id)
        .ok_or(CommandError::InvalidTransport {
            field: "pipeline.transition.target",
            reason: "is absent from the pinned pipeline version".to_owned(),
        })?;
    super::task_support::reject_unimplemented_system_stage(next_stage, task)?;
    if !next_stage.executor_kind().requires_wait() {
        return Ok(None);
    }
    let next_visit = task
        .current_stage_visit()
        .ok_or(CommandError::InvalidTransport {
            field: "task.current_stage_visit",
            reason: "is absent for the submitted outcome".to_owned(),
        })?
        .next()?;
    Ok(Some(TaskWaitCondition::for_stage(
        forge_domain::WaitConditionId::new(),
        next_stage_id.clone(),
        next_visit,
        None,
        actor,
        now,
    )?))
}

async fn verify_existing_artifacts(
    transaction: &mut impl CommandTransaction,
    project: &Project,
    task: &forge_domain::Task,
    artifact_ids: &BTreeSet<ArtifactId>,
) -> Result<(), CommandError> {
    for artifact_id in artifact_ids {
        let stored =
            transaction
                .lock_artifact(*artifact_id)
                .await?
                .ok_or(CommandError::NotFound {
                    aggregate: "outcome artifact",
                })?;
        if stored.artifact.project_id() != project.id()
            || stored.location.task_id != Some(task.id())
        {
            return Err(CommandError::InvalidTransport {
                field: "outcome_artifact_ids",
                reason: "must reference an artifact already linked to this task".to_owned(),
            });
        }
    }
    Ok(())
}

fn outcome_event_kind(effect: StageTransitionEffect) -> DomainEventKind {
    match effect {
        StageTransitionEffect::EnteredStage { .. } => DomainEventKind::TaskStageAdvanced,
        StageTransitionEffect::Completed => DomainEventKind::TaskCompleted,
        StageTransitionEffect::Cancelled => DomainEventKind::TaskCancelled,
    }
}
