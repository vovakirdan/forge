//! Physical results survive revoked authority; only a current source can advance.
use super::*;
use forge_domain::{
    Artifact, ArtifactBody, ArtifactId, ArtifactKind, ArtifactProducer, HookExecutionResult,
    HookVerdict, NewArtifact, StageOutcomeSubmission, StageTransitionEffect, Timestamp,
};
use forge_storage::{RunDesiredState, RunObservedState, RunProjection};
use serde_json::Value;

impl CoreService {
    pub(crate) async fn record_hook_terminal(
        &self,
        tx: &mut StorageTransaction<'_>,
        project: &mut Project,
        run: &RunProjection,
        state: RunObservedState,
        details: &Value,
    ) -> Result<(), CoreError> {
        if run.assignment.hook().is_none() {
            return Ok(());
        }
        let Some(stored) = tx.hook_for_run(run.id).await? else {
            return Err(invalid("Hook invocation absent"));
        };
        if stored.state != "running" {
            return Ok(());
        }
        let task = load_scoped_task(tx, project, stored.spec.assignment.task_id)
            .await?
            .task;
        let candidate = tx
            .latest_accepted_git_candidate(project.id(), task.id())
            .await?;
        let physical = state == RunObservedState::Stopped;
        let current = physical
            && stored.core_instance == self.instance_id
            && matches!(
                run.desired_state,
                RunDesiredState::ProvisionRequested | RunDesiredState::Running
            )
            && task.lifecycle() == LifecycleStatus::InProgress
            && task.revision().get() == stored.expected_task_revision
            && task.current_stage_id() == Some(&stored.spec.assignment.stage_id)
            && task
                .current_stage_visit()
                .is_some_and(|visit| visit.get() == stored.spec.assignment.stage_visit)
            && task.pipeline().pipeline_version_id() == stored.spec.assignment.pipeline_version_id
            && candidate.is_some_and(|candidate| {
                candidate.proposal_id == stored.spec.assignment.candidate_proposal_id
                    && candidate.candidate == stored.spec.candidate
            })
            && tx
                .hook_dispatch_open(project.id(), task.id(), Some(run.id))
                .await?;
        let result = details.get("hook_result").cloned();
        let parsed = result
            .clone()
            .and_then(|value| serde_json::from_value::<HookExecutionResult>(value).ok());
        let valid = parsed.as_ref().is_some_and(|result| {
            result.validate().is_ok()
                && result.invocation_id == stored.spec.assignment.invocation_id
                && result.candidate_proposal_id == stored.spec.assignment.candidate_proposal_id
                && result.candidate == stored.spec.candidate
                && result.verdict != HookVerdict::Interrupted
        });
        if current && valid {
            return self
                .apply_hook_result(
                    tx,
                    project,
                    &HookCompletion::from(&stored),
                    result.ok_or_else(|| invalid("result absent"))?,
                )
                .await;
        }
        // Untrusted/missing/late results remain evidence, never a fabricated pass.
        let evidence = json!({"reported":result,"observed_state":format!("{state:?}"),"accepted_for_transition":false});
        let artifact = self
            .hook_artifact(tx, project, &HookCompletion::from(&stored), &evidence)
            .await?;
        tx.finish_hook(
            stored.spec.assignment.invocation_id,
            "held",
            Some(&evidence),
            Some(artifact.id()),
        )
        .await?;
        if task.current_stage_id() == Some(&stored.spec.assignment.stage_id)
            && task
                .current_stage_visit()
                .is_some_and(|visit| visit.get() == stored.spec.assignment.stage_visit)
        {
            self.hold_hook_task(tx,project,&task,"Hook interrupted, result invalid, or execution authority changed; explicit management decision required").await?;
        }
        Ok(())
    }

    pub(super) async fn apply_hook_result(
        &self,
        tx: &mut StorageTransaction<'_>,
        project: &mut Project,
        source: &HookCompletion,
        result: Value,
    ) -> Result<(), CoreError> {
        let scoped = load_scoped_task(tx, project, source.task_id).await?;
        let mut task = scoped.task;
        let version = pinned_pipeline_version(tx, project, &task).await?;
        if source.project_id != project.id()
            || source.pipeline_version_id != version.id()
            || task.current_stage_id() != Some(&source.stage_id)
            || task
                .current_stage_visit()
                .is_none_or(|visit| visit.get() != source.stage_visit)
        {
            return Err(invalid("Hook result no longer owns this Task stage"));
        }
        let stage = version
            .stage(&source.stage_id)
            .ok_or_else(|| invalid("stage absent"))?;
        let Some(action @ SystemStageAction::ProjectHook { outcomes, .. }) = stage.system_action()
        else {
            return Err(invalid("Hook action absent"));
        };
        action.validate_hook_version(stage, &version, &source.hook)?;
        let outcome = match result["verdict"].as_str() {
            Some("passed") => outcomes.passed.clone(),
            Some("failed") => outcomes.failed.clone(),
            Some("timed_out") => outcomes.timed_out.clone(),
            Some("skipped") if !source.hook.applies_to(task.kind()) => outcomes.skipped.clone(),
            _ => return Err(invalid("no permitted Hook outcome")),
        };
        let artifact = self.hook_artifact(tx, project, source, &result).await?;
        // Completion describes the command's evidence, not Task completion.
        // Other required hooks can still hold the Task below.
        tx.finish_hook(
            source.invocation_id,
            "completed",
            Some(&result),
            Some(artifact.id()),
        )
        .await?;
        let previous = task.revision().get();
        let now = crate::canonical_clock::project_mutation_time(project);
        task.attach_artifact(
            &artifact,
            ArtifactProducer::System,
            self.actors.core,
            None,
            now,
        )?;
        let mut proposed = task.clone();
        let pending = tx
            .pending_task_messages(
                project.id(),
                &forge_domain::communication::TaskMessageContext {
                    task_id: task.id(),
                    pipeline_version_id: version.id(),
                    stage_id: source.stage_id.clone(),
                    stage_visit: source.stage_visit,
                },
            )
            .await?;
        let wait = crate::supervisor::outcome::next_stage_wait(
            &version,
            &proposed,
            &outcome,
            self.actors.core,
            now,
        );
        let mut effect = None;
        if pending.is_empty()
            && let Ok(wait) = wait
        {
            effect = version
                .apply_outcome(
                    &mut proposed,
                    StageOutcomeSubmission {
                        outcome: outcome.clone(),
                        outcome_artifact_ids: std::collections::BTreeSet::from([artifact.id()]),
                        submitted_by: self.actors.core,
                        submitted_at: now,
                        cancellation_reason_id: None,
                        cancellation_note: None,
                    },
                    project.cancellation_reasons(),
                    None,
                    wait,
                )
                .ok();
            if proposed.lifecycle() == LifecycleStatus::Done
                && !tx
                    .required_hooks_satisfied(&proposed, &version, source.candidate_proposal_id)
                    .await?
            {
                effect = None
            }
        }
        if effect.is_some() {
            task = proposed
        }
        let persistence = retained_persistence(project, &task, scoped.persistence)?;
        persist_task_and_project(tx, project, &task, persistence, previous, now).await?;
        tx.append_event_and_outbox(&task_event(
            project,
            &task,
            DomainEventKind::TaskArtifactAttached,
            self.actors.core,
            CommandId::from(source.invocation_id),
            now,
            event_payload([
                ("artifact_id", json!(artifact.id())),
                ("hook_invocation_id", json!(source.invocation_id)),
            ]),
        )?)
        .await?;
        let Some(effect) = effect else {
            self.hold_hook_task(tx,project,&task,"Hook evidence retained; Pipeline, required hooks or addressed messages need management").await?;
            return Ok(());
        };
        super::handoff::persist(
            tx,
            source,
            &task,
            self.actors.core,
            outcome.clone(),
            artifact.id(),
            now,
        )
        .await?;
        let kind = match &effect {
            StageTransitionEffect::Completed => DomainEventKind::TaskCompleted,
            StageTransitionEffect::Cancelled => DomainEventKind::TaskCancelled,
            StageTransitionEffect::EnteredStage { .. } => DomainEventKind::TaskStageAdvanced,
        };
        tx.append_event_and_outbox(&task_event(
            project,
            &task,
            kind,
            self.actors.core,
            CommandId::from(source.invocation_id),
            now,
            event_payload([
                ("outcome", json!(outcome)),
                ("artifact_id", json!(artifact.id())),
            ]),
        )?)
        .await?;
        match effect {
            StageTransitionEffect::EnteredStage { executor_kind, .. } => {
                crate::scheduler::enqueue_if_employee(
                    tx,
                    project,
                    &task,
                    &persistence,
                    executor_kind,
                    Timestamp::now_utc(),
                )
                .await?
            }
            StageTransitionEffect::Completed => {
                for event in self
                    .resolve_dependency_waits_after_completion(
                        tx,
                        project,
                        task.id(),
                        CommandId::from(source.invocation_id),
                        now,
                    )
                    .await?
                {
                    tx.append_event_and_outbox(&event).await?;
                }
            }
            StageTransitionEffect::Cancelled => {}
        }
        Ok(())
    }
    async fn hook_artifact(
        &self,
        tx: &mut StorageTransaction<'_>,
        project: &Project,
        source: &HookCompletion,
        result: &Value,
    ) -> Result<Artifact, CoreError> {
        let now = crate::canonical_clock::project_mutation_time(project);
        let artifact = Artifact::new(
            ArtifactId::new(),
            NewArtifact {
                project_id: project.id(),
                kind: ArtifactKind::new("hook_result")?,
                title: format!("Project Hook: {}", source.hook.name),
                body: ArtifactBody::inline_json(
                    json!({"hook_version_id":source.hook.id,"invocation_id":source.invocation_id,"run_id":source.run_id,"candidate":source.candidate,"result":result}),
                ),
                metadata: json!({}),
                created_by: self.actors.core,
                created_at: now,
            },
        )?;
        tx.insert_artifact(
            &artifact,
            &forge_storage::ArtifactLocation {
                task_id: Some(source.task_id),
                run_id: source.run_id,
                stage_id: Some(source.stage_id.clone()),
                producer: ArtifactProducer::System,
                producer_id: Some(source.invocation_id),
                producer_data: json!({"hook_invocation_id":source.invocation_id}),
            },
        )
        .await?;
        tx.append_event_and_outbox(&event(
            project.id(),
            AggregateRef::Artifact(artifact.id()),
            1,
            DomainEventKind::ArtifactCreated,
            self.actors.core,
            CommandId::from(source.invocation_id),
            None,
            event_payload([("kind", json!("hook_result"))]),
            now,
        )?)
        .await?;
        Ok(artifact)
    }
}
