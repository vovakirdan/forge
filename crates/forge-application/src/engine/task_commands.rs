//! Task, dependency, and waiting-state command handlers.

use super::CommandTransaction;
use crate::{CommandEnvelope, CommandPayload, DraftTaskPatch};
use forge_domain::{
    AggregateRef, CancellationRequest, CommandId, DomainEventKind, Project, TaskId, Timestamp,
};
use serde_json::json;

use super::{
    CommandError, Engine,
    dependency_waits::add_unsatisfied_dependency_waits,
    event::{event, event_payload},
    receipt::{finish_command, resource},
    run_control::run_stop_event,
    scheduler::enqueue_if_employee,
    task_support::{
        load_scoped_task, persist_task_and_project, pinned_pipeline_version,
        reject_unimplemented_system_stage, require_task_revision, retained_persistence,
        stage_payload, stage_wait_for_current_task, task_event,
    },
};

impl Engine<'_> {
    pub async fn apply_task_or_dependency_command(
        &self,
        transaction: &mut impl CommandTransaction,
        project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<forge_protocol::wire::CommandReceipt, CommandError> {
        match &envelope.payload {
            CommandPayload::AmendDraft {
                task_id,
                expected_task_revision,
                patch,
            } => {
                self.amend_draft(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                    *task_id,
                    *expected_task_revision,
                    patch,
                )
                .await
            }
            CommandPayload::ApproveTask {
                task_id,
                expected_task_revision,
            } => {
                self.approve_task(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                    *task_id,
                    *expected_task_revision,
                )
                .await
            }
            CommandPayload::CancelTask {
                task_id,
                expected_task_revision,
                reason_id,
                note,
            } => {
                self.cancel_task(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                    *task_id,
                    *expected_task_revision,
                    reason_id.clone(),
                    note.clone(),
                )
                .await
            }
            CommandPayload::ResumeTask {
                task_id,
                expected_task_revision,
                wait_condition_id,
            } => {
                self.resume_task(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                    *task_id,
                    *expected_task_revision,
                    *wait_condition_id,
                )
                .await
            }
            CommandPayload::SetTaskPriority {
                task_id,
                expected_task_revision,
                priority_level_id,
            } => {
                self.set_task_priority(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                    *task_id,
                    *expected_task_revision,
                    priority_level_id.clone(),
                )
                .await
            }
            CommandPayload::CreateDependency {
                blocker_task_id,
                blocked_task_id,
            } => {
                self.change_dependency(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                    *blocker_task_id,
                    *blocked_task_id,
                    true,
                )
                .await
            }
            CommandPayload::RemoveDependency {
                blocker_task_id,
                blocked_task_id,
            } => {
                self.change_dependency(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                    *blocker_task_id,
                    *blocked_task_id,
                    false,
                )
                .await
            }
            CommandPayload::SubmitExternalStageOutcome(input) => {
                self.apply_external_outcome_command(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                    input,
                )
                .await
            }
            _ => Err(CommandError::InvalidTransport {
                field: "command",
                reason: "is not a Task or dependency command".to_owned(),
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn amend_draft(
        &self,
        transaction: &mut impl CommandTransaction,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
        task_id: TaskId,
        expected_task_revision: u64,
        patch: &DraftTaskPatch,
    ) -> Result<forge_protocol::wire::CommandReceipt, CommandError> {
        let stored = load_scoped_task(transaction, &project, task_id).await?;
        require_task_revision(&stored.task, expected_task_revision)?;
        let previous_revision = stored.task.revision().get();
        let mut task = stored.task;
        task.amend_draft(patch.apply_to(task.spec())?, now)?;
        if let Some(priority) = &patch.priority {
            task.set_priority(
                project.priority_scheme(),
                forge_domain::PriorityLevelId::new(priority.clone())?,
                now,
            )?;
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
        let audit = task_event(
            &project,
            &task,
            DomainEventKind::TaskAmended,
            self.actors.human,
            command_id,
            now,
            event_payload([]),
        )?;
        finish_command(
            transaction,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            vec![audit],
            Some(resource("task", task.id().as_uuid())),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn approve_task(
        &self,
        transaction: &mut impl CommandTransaction,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
        task_id: TaskId,
        expected_task_revision: u64,
    ) -> Result<forge_protocol::wire::CommandReceipt, CommandError> {
        let stored = load_scoped_task(transaction, &project, task_id).await?;
        require_task_revision(&stored.task, expected_task_revision)?;
        let version = pinned_pipeline_version(transaction, &project, &stored.task).await?;
        for stage in version.stages() {
            if let Some(policy) = stage.acceptance_policy() {
                policy.validate_task_surface(stored.task.work_surface())?;
            }
        }
        let entry_executor = version
            .stage(version.entry_stage_id())
            .ok_or(CommandError::InvalidTransport {
                field: "pipeline.entry_stage_id",
                reason: "is absent from the pinned pipeline version".to_owned(),
            })?
            .executor_kind();
        reject_unimplemented_system_stage(
            version
                .stage(version.entry_stage_id())
                .ok_or(CommandError::NotFound {
                    aggregate: "entry stage",
                })?,
            &stored.task,
        )?;
        for stage in version.stages() {
            if let Some(action) = stage.system_action() {
                action.validate_task(&stored.task)?;
            }
        }

        let previous_revision = stored.task.revision().get();
        let mut task = stored.task;
        task.approve(project.property_schema(), now)?;
        let approved_revision = task.revision().get();
        let wait = stage_wait_for_current_task(&task, entry_executor, self.actors.core, now)?;
        let executor_kind = version.enter_entry_stage(&mut task, wait, now)?;
        let has_dependency_wait = add_unsatisfied_dependency_waits(
            transaction,
            &project,
            &mut task,
            self.actors.core,
            now,
        )
        .await?;
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
        enqueue_if_employee(
            transaction,
            &project,
            &task,
            &persistence,
            executor_kind,
            self.clock.now(),
        )
        .await?;
        let approved = event(
            project.id(),
            AggregateRef::Task(task.id()),
            approved_revision,
            DomainEventKind::TaskApproved,
            self.actors.human,
            command_id,
            None,
            event_payload([]),
            now,
        )?;
        let mut events = vec![approved];
        if executor_kind.requires_wait() || has_dependency_wait {
            events.push(task_event(
                &project,
                &task,
                DomainEventKind::TaskWaiting,
                self.actors.core,
                command_id,
                now,
                stage_payload(&task),
            )?);
        }
        finish_command(
            transaction,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            events,
            Some(resource("task", task.id().as_uuid())),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn cancel_task(
        &self,
        transaction: &mut impl CommandTransaction,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
        task_id: TaskId,
        expected_task_revision: u64,
        reason_id: forge_domain::CancellationReasonId,
        note: Option<String>,
    ) -> Result<forge_protocol::wire::CommandReceipt, CommandError> {
        let stored = load_scoped_task(transaction, &project, task_id).await?;
        require_task_revision(&stored.task, expected_task_revision)?;
        // A cancellation is terminal at the task boundary, but an already
        // leased executor may still be running. Fence it in this same
        // transaction so it cannot submit a late outcome after cancellation.
        let stopped_runs = self
            .stop_active_task_runs(transaction, project.id(), task_id)
            .await?;
        let previous_revision = stored.task.revision().get();
        let mut task = stored.task;
        task.cancel(
            project.cancellation_reasons(),
            CancellationRequest {
                reason_id: Some(reason_id.clone()),
                note: note.clone(),
                cancelled_by: self.actors.human,
                cancelled_at: now,
            },
        )?;
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
        let audit = task_event(
            &project,
            &task,
            DomainEventKind::TaskCancelled,
            self.actors.human,
            command_id,
            now,
            event_payload([("cancellation_reason_key", json!(reason_id.as_str()))]),
        )?;
        let dependent_events = self
            .wait_dependents_for_unsatisfied_blocker(
                transaction,
                &mut project,
                task.id(),
                command_id,
                now,
            )
            .await?;
        let mut events = vec![audit];
        for run in &stopped_runs {
            events.push(run_stop_event(
                &project,
                run,
                command_id,
                self.actors.core,
                "task_cancelled",
                now,
            )?);
        }
        events.extend(dependent_events);
        finish_command(
            transaction,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            events,
            Some(resource("task", task.id().as_uuid())),
        )
        .await
    }
}
