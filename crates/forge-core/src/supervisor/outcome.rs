//! Canonical executor StageOutcomeSubmission handling after a fenced Run claim.

use std::collections::BTreeSet;

use forge_domain::{
    DomainError, DomainEvent, DomainEventKind, ExecutorKind, PipelineTransitionTarget,
    PipelineVersion, Project, StageOutcomeSubmission, StageTransitionEffect, Task,
    TaskWaitCondition, TaskWaitKind, Timestamp, WaitConditionId,
};
use forge_storage::{ExecutorArtifactRunScope, FencedWrite, StorageTransaction};
use serde_json::json;
use tracing::warn;

use crate::{
    CoreError, CoreService,
    event::event_payload,
    scheduler::enqueue_if_employee,
    task_support::{persist_task_and_project, retained_persistence, stage_payload, task_event},
};

use super::{
    SupervisorIdentity,
    bridge::InboundResult,
    executor::{employee_actor, reserve_submission},
    validation::{ParsedStageOutcomeSubmission, SubmissionScope},
};

impl CoreService {
    pub(super) async fn record_executor_stage_outcome(
        &self,
        _identity: Option<&SupervisorIdentity>,
        submission: ParsedStageOutcomeSubmission,
    ) -> Result<InboundResult, CoreError> {
        let mut transaction = self.store.begin().await?;
        if submission.scope.is_gateway()
            && let Some(refused) = reserve_submission(
                &mut transaction,
                &submission.scope,
                "stage_outcome_submission",
            )
            .await?
        {
            return Ok(refused);
        }
        let mut context = self
            .load_employee_submission_context(&mut transaction, &submission.scope)
            .await?;
        let now = crate::canonical_clock::project_mutation_time(&context.project);
        let current_stage_id =
            context
                .stored_task
                .task
                .current_stage_id()
                .ok_or(CoreError::InvalidTransport {
                    field: "task.current_stage_id",
                    reason: "is absent for an employee stage outcome".to_owned(),
                })?;
        if current_stage_id != &submission.stage_id
            || context.run.stage_id != submission.stage_id.as_str()
        {
            return Err(CoreError::InvalidTransport {
                field: "stage_outcome.stage_id",
                reason: "does not match the active Run stage".to_owned(),
            });
        }
        if !submission.scope.is_gateway()
            && let Some(refused) = reserve_submission(
                &mut transaction,
                &submission.scope,
                "stage_outcome_submission",
            )
            .await?
        {
            return Ok(refused);
        }

        let actor = employee_actor(&context.run);
        let previous_revision = context.stored_task.task.revision().get();
        let task = &mut context.stored_task.task;
        let mut outcome_artifact_ids = submission.artifact_ids.clone();
        verify_existing_outcome_artifacts(
            &mut transaction,
            &context.project,
            task,
            &outcome_artifact_ids,
        )
        .await?;
        resolve_current_run_artifacts(
            &mut transaction,
            task,
            &submission.scope,
            &submission.artifact_submission_message_ids,
            &mut outcome_artifact_ids,
        )
        .await?;
        let next_wait = next_stage_wait(
            &context.version,
            task,
            &submission.outcome,
            self.actors.core,
            now,
        )?;
        let command_id = forge_domain::CommandId::from(submission.scope.message_id);
        let mut events = Vec::<DomainEvent>::new();
        let effect = match context.version.apply_outcome(
            task,
            StageOutcomeSubmission {
                outcome: submission.outcome.clone(),
                outcome_artifact_ids,
                submitted_by: actor,
                submitted_at: now,
                cancellation_reason_id: submission.cancellation_reason_id,
                cancellation_note: submission.note.clone(),
            },
            context.project.cancellation_reasons(),
            None,
            next_wait,
        ) {
            Ok(effect) => Some(effect),
            Err(DomainError::StageVisitLimitExceeded { maximum }) => {
                task.add_wait_condition(
                    TaskWaitCondition::new(
                        WaitConditionId::new(),
                        TaskWaitKind::RetryExhausted,
                        Some(format!("pipeline max_stage_visits={maximum} exhausted")),
                        self.actors.core,
                        now,
                    )?,
                    now,
                )?;
                events.push(task_event(
                    &context.project,
                    task,
                    DomainEventKind::TaskRetryExhausted,
                    self.actors.core,
                    command_id,
                    now,
                    event_payload([("max_stage_visits", json!(maximum))]),
                )?);
                // The retry limit is not merely a diagnostic. It leaves the
                // Task in `waiting`, so its durable history must expose the
                // same lifecycle fact for SSE/audit consumers.
                events.push(task_event(
                    &context.project,
                    task,
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
        let persistence =
            retained_persistence(&context.project, task, context.stored_task.persistence)?;
        persist_task_and_project(
            &mut transaction,
            &mut context.project,
            task,
            persistence,
            previous_revision,
            now,
        )
        .await?;
        if transaction
            .complete_queue_after_accepted_stage_outcome(
                submission.scope.run_id,
                submission.scope.lease_fencing_token,
                submission.scope.environment_epoch,
            )
            .await?
            != FencedWrite::Applied
        {
            return Err(CoreError::InvalidTransport {
                field: "executor_submission",
                reason: "Run queue lease was no longer current".to_owned(),
            });
        }
        let next_employee_stage = matches!(
            &effect,
            Some(StageTransitionEffect::EnteredStage {
                executor_kind: ExecutorKind::Employee,
                ..
            })
        );
        if effect.is_some() {
            crate::handoff::persist_handoff(
                &mut transaction,
                &context.run,
                task,
                actor,
                Some(submission.outcome.clone()),
                None,
                now,
            )
            .await?;
        }
        if next_employee_stage {
            enqueue_if_employee(
                &mut transaction,
                &context.project,
                task,
                &persistence,
                ExecutorKind::Employee,
                Timestamp::now_utc(),
            )
            .await?;
        }
        if let Some(effect) = effect {
            let mut payload = stage_payload(task);
            payload.insert("outcome".to_owned(), json!(submission.outcome.as_str()));
            if let Some(note) = submission.note {
                payload.insert("note".to_owned(), json!(note));
            }
            events.push(task_event(
                &context.project,
                task,
                outcome_event_kind(&effect),
                actor,
                command_id,
                now,
                payload,
            )?);
            match effect {
                StageTransitionEffect::Completed => events.extend(
                    self.resolve_dependency_waits_after_completion(
                        &mut transaction,
                        &mut context.project,
                        task.id(),
                        command_id,
                        now,
                    )
                    .await?,
                ),
                StageTransitionEffect::Cancelled => events.extend(
                    self.wait_dependents_for_unsatisfied_blocker(
                        &mut transaction,
                        &mut context.project,
                        task.id(),
                        command_id,
                        now,
                    )
                    .await?,
                ),
                StageTransitionEffect::EnteredStage { .. } => {}
            }
        }
        for event in events {
            transaction.append_event_and_outbox(&event).await?;
        }
        let project_id = context.project.id();
        transaction.commit().await?;
        if let Err(error) = self.dispatch_available(project_id).await {
            warn!(error = %error, "could not dispatch newly eligible executor work");
        }
        Ok(InboundResult::Accepted("Executor stage outcome applied"))
    }
}

async fn verify_existing_outcome_artifacts(
    transaction: &mut StorageTransaction<'_>,
    project: &Project,
    task: &Task,
    artifact_ids: &BTreeSet<forge_domain::ArtifactId>,
) -> Result<(), CoreError> {
    for artifact_id in artifact_ids {
        let stored = transaction
            .lock_artifact(*artifact_id)
            .await?
            .ok_or(CoreError::NotFound {
                aggregate: "outcome artifact",
            })?;
        if stored.artifact.project_id() != project.id()
            || stored.location.task_id != Some(task.id())
        {
            return Err(CoreError::InvalidTransport {
                field: "stage_outcome.artifact_ids",
                reason: "must reference an artifact linked to this task".to_owned(),
            });
        }
    }
    Ok(())
}

async fn resolve_current_run_artifacts(
    transaction: &mut StorageTransaction<'_>,
    task: &Task,
    scope: &SubmissionScope,
    message_ids: &[uuid::Uuid],
    outcome_artifact_ids: &mut BTreeSet<forge_domain::ArtifactId>,
) -> Result<(), CoreError> {
    let run_scope = ExecutorArtifactRunScope::new(
        scope.run_id,
        scope.lease_fencing_token,
        scope.environment_epoch,
    )?;
    let mappings = transaction
        .lookup_executor_artifacts(run_scope, message_ids)
        .await?;
    if mappings.len() != message_ids.len() {
        return Err(CoreError::InvalidTransport {
            field: "stage_outcome.artifact_submission_message_ids",
            reason: "must reference accepted artifact submissions from this Run".to_owned(),
        });
    }
    for mapping in mappings {
        if mapping.artifact.location.task_id != Some(task.id())
            || mapping.artifact.location.run_id != Some(scope.run_id)
            || !task
                .artifact_links()
                .iter()
                .any(|link| link.artifact_id() == mapping.artifact.artifact.id())
        {
            return Err(CoreError::InvalidTransport {
                field: "stage_outcome.artifact_submission_message_ids",
                reason: "does not map to an artifact linked to the active task".to_owned(),
            });
        }
        if !outcome_artifact_ids.insert(mapping.artifact.artifact.id()) {
            return Err(CoreError::InvalidTransport {
                field: "stage_outcome",
                reason: "repeats one outcome artifact through two references".to_owned(),
            });
        }
    }
    Ok(())
}

fn next_stage_wait(
    version: &PipelineVersion,
    task: &Task,
    outcome: &forge_domain::OutcomeKey,
    actor: forge_domain::Actor,
    now: Timestamp,
) -> Result<Option<TaskWaitCondition>, CoreError> {
    let current_stage_id = task.current_stage_id().ok_or(CoreError::InvalidTransport {
        field: "task.current_stage_id",
        reason: "is absent for the submitted outcome".to_owned(),
    })?;
    let current_stage = version
        .stage(current_stage_id)
        .ok_or(CoreError::InvalidTransport {
            field: "task.current_stage_id",
            reason: "is absent from the pinned pipeline version".to_owned(),
        })?;
    let transition = current_stage
        .transition(outcome)
        .ok_or(CoreError::InvalidTransport {
            field: "stage_outcome.outcome",
            reason: "is not declared by the current pipeline stage".to_owned(),
        })?;
    let PipelineTransitionTarget::Stage(next_stage_id) = transition.target() else {
        return Ok(None);
    };
    let next_stage = version
        .stage(next_stage_id)
        .ok_or(CoreError::InvalidTransport {
            field: "pipeline.transition.target",
            reason: "is absent from the pinned pipeline version".to_owned(),
        })?;
    if next_stage.executor_kind() == ExecutorKind::System {
        return Err(CoreError::InvalidTransport {
            field: "pipeline.transition.target",
            reason: "system stages are not implemented in M0".to_owned(),
        });
    }
    if !next_stage.executor_kind().requires_wait() {
        return Ok(None);
    }
    let next_visit = task
        .current_stage_visit()
        .ok_or(CoreError::InvalidTransport {
            field: "task.current_stage_visit",
            reason: "is absent for the submitted outcome".to_owned(),
        })?
        .next()?;
    Ok(Some(TaskWaitCondition::for_stage(
        WaitConditionId::new(),
        next_stage_id.clone(),
        next_visit,
        None,
        actor,
        now,
    )?))
}

const fn outcome_event_kind(effect: &StageTransitionEffect) -> DomainEventKind {
    match effect {
        StageTransitionEffect::EnteredStage { .. } => DomainEventKind::TaskStageAdvanced,
        StageTransitionEffect::Completed => DomainEventKind::TaskCompleted,
        StageTransitionEffect::Cancelled => DomainEventKind::TaskCancelled,
    }
}
