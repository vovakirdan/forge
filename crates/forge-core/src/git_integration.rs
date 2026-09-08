//! Deterministic System action scheduling, separate from Employee admission.
mod acceptance;
mod handoff;
mod management;
mod preparation;
use crate::{
    CoreError, CoreService,
    event::{event, event_payload},
    task_support::{
        load_scoped_task, persist_task_and_project, pinned_pipeline_version, retained_persistence,
    },
};
use forge_domain::{
    AggregateRef, CommandId, DomainEventKind, LifecycleStatus, Project, Task, TaskWaitCondition,
    TaskWaitKind, TaskWorkSurface, Timestamp, WaitConditionId,
    git_integration::{
        GitIntegrationOperation, IntegrationCode, IntegrationPhase, IntegrationRequest,
        SystemStageAction,
    },
};
use forge_protocol::supervisor::v1::{CoreToSupervisor, GitIntegrationCommand, core_to_supervisor};
use forge_storage::{IntegrationState, StorageTransaction, StoredIntegration};
use serde_json::json;
use uuid::Uuid;

impl CoreService {
    pub(crate) async fn reconcile_git_integrations(&self) -> Result<(), CoreError> {
        if self.fake_runtime_enabled {
            return Ok(());
        }
        let Some(identity) = self.supervisor.reconciled_identity().await else {
            return Ok(());
        };
        for (project_id, task_id) in self.store.integration_work().await? {
            let mut tx = self.store.begin().await?;
            let Some(mut project) = tx.lock_project(project_id).await? else {
                continue;
            };
            let task = load_scoped_task(&mut tx, &project, task_id).await?.task;
            let Some(mut stored) = self
                .select_git_integration(&mut tx, &mut project, &task)
                .await?
            else {
                tx.commit().await?;
                continue;
            };
            let owns_current = stored.operation.matches_task(&task)
                && stored.expected_task_revision == task.revision().get()
                && task.lifecycle() == LifecycleStatus::InProgress;
            if stored.state != IntegrationState::Held
                && let Some(command) = &stored.command
                && tx.integration_request_expired(command.command_id).await?
            {
                self.hold_integration(
                    &mut tx,
                    &mut project,
                    &mut stored,
                    "Integration response deadline exceeded; target effect remains unknown",
                )
                .await?;
            }
            let can_execute = owns_current
                && stored.core_instance == self.instance_id
                && tx.integration_dispatch_open(project_id, task_id).await?
                && self
                    .integration_gates(&mut tx, &project, &task, &stored)
                    .await?;
            if !can_execute && stored.state != IntegrationState::Held {
                self.hold_integration(&mut tx,&mut project,&mut stored,"integration requires current Task, open Project, physical quiescence and configured evidence").await?;
            }
            let completed_reply = stored.command.as_ref().is_some_and(|request| {
                stored
                    .result
                    .as_ref()
                    .is_some_and(|result| result.command_id == request.command_id)
            });
            let phase = if stored.resolution.is_some() {
                IntegrationPhase::Reconcile
            } else if stored.state == IntegrationState::Held {
                if stored.result.as_ref().is_some_and(|result| {
                    matches!(
                        result.code,
                        IntegrationCode::Applied | IntegrationCode::Retryable
                    )
                }) || (completed_reply
                    && stored
                        .command
                        .as_ref()
                        .is_some_and(|cmd| cmd.phase == IntegrationPhase::Reconcile))
                {
                    tx.commit().await?;
                    continue;
                }
                // No prepare was ever authorized: a Manager may repair the gates.
                if stored.command.is_none() {
                    tx.commit().await?;
                    continue;
                }
                IntegrationPhase::Reconcile
            } else if stored.intent.is_some() {
                IntegrationPhase::Apply
            } else {
                IntegrationPhase::Prepare
            };
            let reuse = stored.command.as_ref().is_some_and(|request| {
                request.phase == phase
                    && request.host_id == identity.host_id
                    && request.boot_id == identity.boot_id
                    && !completed_reply
            });
            if !reuse {
                let request = IntegrationRequest {
                    command_id: Uuid::now_v7(),
                    operation: stored.operation.clone(),
                    phase,
                    intent: stored.intent.clone(),
                    host_id: identity.host_id.clone(),
                    boot_id: identity.boot_id.clone(),
                };
                tx.save_integration_request(&request).await?;
                stored.command = Some(request);
                if stored.state != IntegrationState::Held {
                    stored.state = if phase == IntegrationPhase::Apply {
                        IntegrationState::Applying
                    } else {
                        IntegrationState::Preparing
                    };
                }
                tx.save_git_integration(&stored).await?;
                integration_audit(
                    &mut tx,
                    &mut project,
                    self.actors.core,
                    stored.operation.id,
                    "requested",
                    json!({"phase":phase}),
                )
                .await?;
            }
            if reuse {
                // Rotate unanswered commands behind other work in the bounded scan.
                // A connected but non-responsive peer must not pin the oldest 64 rows.
                tx.save_git_integration(&stored).await?;
            }
            let request = stored
                .command
                .clone()
                .ok_or_else(|| invalid("request absent"))?;
            tx.commit().await?;
            self.deliver_integration_request(&request).await?;
        }
        Ok(())
    }

    async fn deliver_integration_request(
        &self,
        request: &IntegrationRequest,
    ) -> Result<(), CoreError> {
        // The short control send is serialized with the Project stop command.
        // Git work is asynchronous and never runs while this transaction is open.
        let mut tx = self.store.begin().await?;
        let Some(project) = tx.lock_project(request.operation.project_id).await? else {
            return Ok(());
        };
        let Some(stored) = tx
            .load_git_integration(project.id(), request.operation.id)
            .await?
        else {
            return Ok(());
        };
        if stored.command.as_ref() != Some(request) {
            return Ok(());
        }
        if request.phase != IntegrationPhase::Reconcile {
            let task = load_scoped_task(&mut tx, &project, stored.operation.task_id)
                .await?
                .task;
            if stored.state == IntegrationState::Held
                || stored.core_instance != self.instance_id
                || stored.expected_task_revision != task.revision().get()
                || !stored.operation.matches_task(&task)
                || !tx
                    .integration_dispatch_open(project.id(), task.id())
                    .await?
                || !self
                    .integration_gates(&mut tx, &project, &task, &stored)
                    .await?
            {
                return Ok(());
            }
        }
        let wire = CoreToSupervisor {
            message: Some(core_to_supervisor::Message::GitIntegration(
                GitIntegrationCommand {
                    command_id: request.command_id.to_string(),
                    request_json: serde_json::to_string(request)
                        .map_err(|_| invalid("cannot encode request"))?,
                },
            )),
        };
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            self.supervisor.send(wire),
        )
        .await;
        tx.commit().await?;
        Ok(())
    }

    async fn integration_gates(
        &self,
        tx: &mut StorageTransaction<'_>,
        project: &Project,
        task: &Task,
        stored: &StoredIntegration,
    ) -> Result<bool, CoreError> {
        let Some(candidate) = tx
            .latest_accepted_git_candidate(project.id(), task.id())
            .await?
        else {
            return Ok(false);
        };
        if candidate.proposal_id != stored.operation.candidate_proposal_id
            || candidate.candidate != stored.operation.candidate
        {
            return Ok(false);
        }
        let version = pinned_pipeline_version(tx, project, task).await?;
        if !tx
            .required_hooks_satisfied(task, &version, Some(candidate.proposal_id))
            .await?
        {
            return Ok(false);
        }
        let Some(stage) = task.current_stage_id().and_then(|id| version.stage(id)) else {
            return Ok(false);
        };
        let Some(SystemStageAction::GitIntegration {
            required_review_stages,
            ..
        }) = stage.system_action()
        else {
            return Ok(false);
        };
        let context = forge_domain::communication::TaskMessageContext {
            task_id: task.id(),
            pipeline_version_id: version.id(),
            stage_id: stage.id().clone(),
            stage_visit: stored.operation.stage_visit,
        };
        Ok(tx
            .pending_task_messages(project.id(), &context)
            .await?
            .is_empty()
            && tx
                .integration_reviews_accepted(task, candidate.proposal_id, required_review_stages)
                .await?)
    }

    async fn hold_integration(
        &self,
        tx: &mut StorageTransaction<'_>,
        project: &mut Project,
        stored: &mut StoredIntegration,
        reason: &str,
    ) -> Result<(), CoreError> {
        let scoped = load_scoped_task(tx, project, stored.operation.task_id).await?;
        let mut task = scoped.task;
        if stored.operation.matches_task(&task) && task.lifecycle() == LifecycleStatus::InProgress {
            let previous = task.revision().get();
            let now = crate::canonical_clock::project_mutation_time(project);
            let id = WaitConditionId::new();
            task.add_wait_condition(
                TaskWaitCondition::new(
                    id,
                    TaskWaitKind::ManualPause,
                    Some(format!("integration={} {reason}", stored.operation.id)),
                    self.actors.core,
                    now,
                )?,
                now,
            )?;
            let persistence = retained_persistence(project, &task, scoped.persistence)?;
            persist_task_and_project(tx, project, &task, persistence, previous, now).await?;
            tx.append_event_and_outbox(&crate::task_support::task_event(
                project,
                &task,
                DomainEventKind::TaskWaiting,
                self.actors.core,
                CommandId::from(stored.operation.id),
                now,
                event_payload([
                    ("integration_id", json!(stored.operation.id)),
                    ("wait_condition_id", json!(id)),
                    ("reason", json!(reason)),
                ]),
            )?)
            .await?;
            stored.wait_condition_id = Some(id.as_uuid());
        }
        stored.state = IntegrationState::Held;
        tx.save_git_integration(stored).await?;
        integration_audit(
            tx,
            project,
            self.actors.core,
            stored.operation.id,
            "held",
            json!({"reason":reason}),
        )
        .await
    }
    async fn integration_missing_candidate_wait(
        &self,
        tx: &mut StorageTransaction<'_>,
        project: &mut Project,
        task: &Task,
    ) -> Result<(), CoreError> {
        let scoped = load_scoped_task(tx, project, task.id()).await?;
        let mut task = scoped.task;
        let previous = task.revision().get();
        let now = crate::canonical_clock::project_mutation_time(project);
        let wait_id = WaitConditionId::new();
        task.add_wait_condition(
            TaskWaitCondition::new(
                wait_id,
                TaskWaitKind::ManualPause,
                Some("Git integration requires an accepted candidate".into()),
                self.actors.core,
                now,
            )?,
            now,
        )?;
        let persistence = retained_persistence(project, &task, scoped.persistence)?;
        persist_task_and_project(tx, project, &task, persistence, previous, now).await?;
        tx.append_event_and_outbox(&crate::task_support::task_event(
            project,
            &task,
            DomainEventKind::TaskWaiting,
            self.actors.core,
            CommandId::new(),
            now,
            event_payload([
                ("wait_condition_id", json!(wait_id)),
                (
                    "reason",
                    json!("Git integration requires an accepted candidate"),
                ),
            ]),
        )?)
        .await?;
        Ok(())
    }
}
async fn integration_audit(
    tx: &mut StorageTransaction<'_>,
    project: &mut Project,
    actor: forge_domain::Actor,
    id: Uuid,
    state: &str,
    detail: serde_json::Value,
) -> Result<(), CoreError> {
    let previous = project.revision();
    let now = crate::canonical_clock::project_mutation_time(project);
    project.record_child_mutation(now)?;
    tx.update_project(project, previous).await?;
    tx.append_event_and_outbox(&event(
        project.id(),
        AggregateRef::Project(project.id()),
        project.revision(),
        DomainEventKind::GitIntegrationChanged,
        actor,
        CommandId::from(id),
        None,
        event_payload([
            ("operation_id", json!(id)),
            ("state", json!(state)),
            ("detail", detail),
        ]),
        now,
    )?)
    .await?;
    Ok(())
}
fn invalid(reason: &str) -> CoreError {
    CoreError::InvalidTransport {
        field: "git_integration",
        reason: reason.into(),
    }
}
