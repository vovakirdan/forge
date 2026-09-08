//! Preserve physical facts even when authority for the Task transition has changed.
use super::*;
use crate::supervisor::{SupervisorIdentity, bridge::InboundResult};
use forge_domain::{
    Artifact, ArtifactBody, ArtifactId, ArtifactKind, ArtifactProducer, NewArtifact,
    StageOutcomeSubmission, StageTransitionEffect, git_integration::IntegrationResult,
};
use forge_protocol::supervisor::v1::GitIntegrationReply;

impl CoreService {
    pub(crate) async fn record_git_integration_result(
        &self,
        identity: &SupervisorIdentity,
        wire: GitIntegrationReply,
    ) -> Result<InboundResult, CoreError> {
        if wire.result_json.len() > 32768 {
            return Err(invalid("reply is too large"));
        }
        let result: IntegrationResult =
            serde_json::from_str(&wire.result_json).map_err(|_| invalid("invalid result"))?;
        if result.message_id.to_string() != wire.message_id
            || result.message_id.get_version_num() != 7
            || result.command_id.get_version_num() != 7
            || result.host_id != identity.host_id
            || result.boot_id != identity.boot_id
        {
            return Err(invalid("result identity mismatch"));
        }
        let project_id = self
            .store
            .integration_project(result.operation_id)
            .await?
            .ok_or_else(|| invalid("unknown integration"))?;
        let mut tx = self.store.begin().await?;
        let mut project = tx
            .lock_project(project_id)
            .await?
            .ok_or_else(|| invalid("Project absent"))?;
        let mut stored = tx
            .load_git_integration(project_id, result.operation_id)
            .await?
            .ok_or_else(|| invalid("integration absent"))?;
        let (request, previous) = tx
            .integration_receipt(result.command_id)
            .await?
            .ok_or_else(|| invalid("unrequested result"))?;
        if request.operation != stored.operation
            || result.fence != stored.operation.fence
            || request.host_id != result.host_id
            || request.boot_id != result.boot_id
        {
            return Err(invalid("result scope mismatch"));
        }
        if let Some(previous) = previous {
            return if previous == result {
                Ok(InboundResult::Accepted(
                    "Integration receipt already recorded",
                ))
            } else {
                Err(CoreError::IdempotencyConflict)
            };
        }
        if let Some(intent) = &result.intent {
            stored.operation.validate_intent(intent)?;
            if stored
                .intent
                .as_ref()
                .is_some_and(|existing| existing != intent)
                || request
                    .intent
                    .as_ref()
                    .is_some_and(|expected| expected != intent)
            {
                return Err(invalid("prepared intent changed"));
            }
        }
        if matches!(
            result.code,
            IntegrationCode::Prepared | IntegrationCode::Applied | IntegrationCode::Retryable
        ) && result.intent.is_none()
            || (matches!(
                result.code,
                IntegrationCode::NoChanges | IntegrationCode::StaleBase
            ) && request.phase != IntegrationPhase::Prepare)
            || (result.code == IntegrationCode::NoPreparation
                && (request.phase != IntegrationPhase::Reconcile
                    || request.intent.is_some()
                    || result.intent.is_some()))
            || (result.code == IntegrationCode::Applied
                && request.phase == IntegrationPhase::Prepare)
        {
            return Err(invalid("result phase or intent mismatch"));
        }
        tx.save_integration_result(&result).await?;
        if matches!(
            stored.state,
            IntegrationState::Completed | IntegrationState::Retired
        ) || stored
            .command
            .as_ref()
            .is_none_or(|current| current.command_id != result.command_id)
        {
            // Old observations remain facts, but cannot authorize a replacement command.
            integration_audit(
                &mut tx,
                &mut project,
                self.actors.core,
                stored.operation.id,
                "historical_result",
                json!(result),
            )
            .await?;
            tx.commit().await?;
            return Ok(InboundResult::Accepted(
                "Historical integration result retained",
            ));
        }
        if stored.intent.is_none() {
            stored.intent = result.intent.clone();
        }
        stored.result = Some(result.clone());
        let task = load_scoped_task(&mut tx, &project, stored.operation.task_id)
            .await?
            .task;
        let authorized = stored.operation.matches_task(&task)
            && stored.expected_task_revision == task.revision().get()
            && task.lifecycle() == LifecycleStatus::InProgress
            && stored.core_instance == self.instance_id
            && tx.integration_dispatch_open(project_id, task.id()).await?
            && self
                .integration_gates(&mut tx, &project, &task, &stored)
                .await?;
        let resolution = stored.resolution.take();
        match result.code {
            IntegrationCode::NoPreparation
                if authorized
                    && resolution.as_deref() == Some("retry")
                    && request.phase == IntegrationPhase::Reconcile
                    && stored.intent.is_none()
                    && result.intent.is_none()
                    && !tx
                        .integration_ever_authorized_apply(stored.operation.id)
                        .await? =>
            {
                stored.state = IntegrationState::Retired;
                tx.save_git_integration(&stored).await?;
            }
            IntegrationCode::Prepared
                if authorized
                    && stored.state != IntegrationState::Held
                    && request.phase == IntegrationPhase::Prepare =>
            {
                stored.state = IntegrationState::Prepared;
                tx.save_git_integration(&stored).await?;
            }
            IntegrationCode::Retryable
                if authorized
                    && resolution.as_deref() == Some("retry")
                    && request.phase == IntegrationPhase::Reconcile =>
            {
                stored.state = IntegrationState::Prepared;
                tx.save_git_integration(&stored).await?;
            }
            IntegrationCode::Applied | IntegrationCode::NoChanges | IntegrationCode::StaleBase
                if authorized
                    && ((stored.state != IntegrationState::Held
                        && request.phase != IntegrationPhase::Reconcile)
                        || (resolution.as_deref() == Some("accept")
                            && request.phase == IntegrationPhase::Reconcile
                            && result.code == IntegrationCode::Applied)) =>
            {
                self.apply_integration_outcome(&mut tx, &mut project, &mut stored)
                    .await?;
            }
            _ => {
                self.hold_integration(
                    &mut tx,
                    &mut project,
                    &mut stored,
                    "Integration result retained; explicit management resolution required",
                )
                .await?
            }
        }
        integration_audit(
            &mut tx,
            &mut project,
            self.actors.core,
            stored.operation.id,
            "observed",
            json!(result),
        )
        .await?;
        tx.commit().await?;
        Ok(InboundResult::Accepted("Integration result recorded"))
    }

    pub(super) async fn apply_integration_outcome(
        &self,
        tx: &mut StorageTransaction<'_>,
        project: &mut Project,
        stored: &mut StoredIntegration,
    ) -> Result<(), CoreError> {
        let scoped = load_scoped_task(tx, project, stored.operation.task_id).await?;
        let mut task = scoped.task;
        let version = pinned_pipeline_version(tx, project, &task).await?;
        let stage = version
            .stage(&stored.operation.stage_id)
            .ok_or_else(|| invalid("stage absent"))?;
        let Some(SystemStageAction::GitIntegration { outcomes, .. }) = stage.system_action() else {
            return Err(invalid("system action absent"));
        };
        let result = stored
            .result
            .as_ref()
            .ok_or_else(|| invalid("result absent"))?;
        let outcome = match result.code {
            IntegrationCode::Applied => outcomes.applied.clone(),
            IntegrationCode::NoChanges => outcomes.no_changes.clone(),
            IntegrationCode::StaleBase => outcomes.stale_base.clone(),
            _ => return Err(invalid("no mapped outcome for this observation")),
        };
        let previous = task.revision().get();
        let now = crate::canonical_clock::project_mutation_time(project);
        let artifact = Artifact::new(
            ArtifactId::new(),
            NewArtifact {
                project_id: project.id(),
                kind: ArtifactKind::new("integration_result")?,
                title: "Local Git integration result".into(),
                body: ArtifactBody::inline_json(
                    json!({"operation_id":stored.operation.id,"candidate":stored.operation.candidate,"result":result}),
                ),
                metadata: json!({}),
                created_by: self.actors.core,
                created_at: now,
            },
        )?;
        task.attach_artifact(
            &artifact,
            ArtifactProducer::System,
            self.actors.core,
            None,
            now,
        )?;
        let wait = match crate::supervisor::outcome::next_stage_wait(
            &version,
            &task,
            &outcome,
            self.actors.core,
            now,
        ) {
            Ok(wait) => wait,
            Err(_) => {
                self.hold_integration(
                    tx,
                    project,
                    stored,
                    "Physical result retained; configured next stage cannot be entered",
                )
                .await?;
                return Ok(());
            }
        };
        let effect = match version.apply_outcome(
            &mut task,
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
        ) {
            Ok(effect) => effect,
            Err(_) => {
                self.hold_integration(tx,project,stored,"Physical result retained; Pipeline outcome/evidence/visit contract requires management").await?;
                return Ok(());
            }
        };
        tx.insert_artifact(
            &artifact,
            &forge_storage::ArtifactLocation {
                task_id: Some(task.id()),
                run_id: None,
                stage_id: Some(stage.id().clone()),
                producer: ArtifactProducer::System,
                producer_id: None,
                producer_data: json!({"integration_id":stored.operation.id}),
            },
        )
        .await?;
        let persistence = retained_persistence(project, &task, scoped.persistence)?;
        persist_task_and_project(tx, project, &task, persistence, previous, now).await?;
        super::handoff::persist_system_handoff(
            tx,
            stored,
            &task,
            self.actors.core,
            outcome.clone(),
            artifact.id(),
            now,
        )
        .await?;
        stored.state = IntegrationState::Completed;
        tx.save_git_integration(stored).await?;
        tx.append_event_and_outbox(&event(
            project.id(),
            AggregateRef::Artifact(artifact.id()),
            1,
            DomainEventKind::ArtifactCreated,
            self.actors.core,
            CommandId::from(stored.operation.id),
            None,
            event_payload([("kind", json!("integration_result"))]),
            now,
        )?)
        .await?;
        tx.append_event_and_outbox(&crate::task_support::task_event(
            project,
            &task,
            DomainEventKind::TaskArtifactAttached,
            self.actors.core,
            CommandId::from(stored.operation.id),
            now,
            event_payload([
                ("artifact_id", json!(artifact.id())),
                ("integration_id", json!(stored.operation.id)),
            ]),
        )?)
        .await?;
        let kind = match &effect {
            StageTransitionEffect::Completed => DomainEventKind::TaskCompleted,
            StageTransitionEffect::Cancelled => DomainEventKind::TaskCancelled,
            StageTransitionEffect::EnteredStage { .. } => DomainEventKind::TaskStageAdvanced,
        };
        tx.append_event_and_outbox(&crate::task_support::task_event(
            project,
            &task,
            kind,
            self.actors.core,
            CommandId::from(stored.operation.id),
            now,
            event_payload([
                ("outcome", json!(outcome)),
                ("artifact_id", json!(artifact.id())),
            ]),
        )?)
        .await?;
        if let StageTransitionEffect::EnteredStage { executor_kind, .. } = effect {
            crate::scheduler::enqueue_if_employee(
                tx,
                project,
                &task,
                &persistence,
                executor_kind,
                Timestamp::now_utc(),
            )
            .await?;
        } else if effect == StageTransitionEffect::Completed {
            for event in self
                .resolve_dependency_waits_after_completion(
                    tx,
                    project,
                    task.id(),
                    CommandId::from(stored.operation.id),
                    now,
                )
                .await?
            {
                tx.append_event_and_outbox(&event).await?;
            }
        }
        Ok(())
    }
}
