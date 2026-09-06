//! Idempotent named-command transaction boundary owned exclusively by Core.

use forge_application::{CommandEnvelope, CommandPayload};
use forge_domain::{
    AggregateRef, CommandId, DomainEvent, DomainEventKind, PipelineId, PipelineVersionId, Project,
    TaskPipelineBinding, Timestamp,
};
use forge_protocol::wire::{CommandReceipt, CommandStatus, ResourceReference};
use forge_storage::{IdempotencyRecord, StorageTransaction};
use serde_json::json;
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::{
    CoreError, CoreService,
    event::{event, event_payload},
    pipeline_access::{ensure_assignable_pipeline, validate_employee_stage_eligibility},
    scheduler::task_persistence,
};

impl CoreService {
    /// Applies one parsed HTTP named command as state + event + outbox +
    /// idempotency receipt in a single PostgreSQL transaction.
    pub async fn execute_command(
        &self,
        envelope: CommandEnvelope,
    ) -> Result<CommandReceipt, CoreError> {
        let request_hash = command_fingerprint(&envelope, self.actors.human)?;
        let command_id = CommandId::new();
        let mut transaction = self.store.begin().await?;
        transaction
            .lock_project_creation(envelope.project_id)
            .await?;
        let project = transaction.lock_project(envelope.project_id).await?;
        // Canonical mutation time is taken after lock acquisition; waiting for
        // another writer cannot leave this command with an older timestamp.
        let now = project.as_ref().map_or_else(
            Timestamp::now_utc,
            crate::canonical_clock::project_mutation_time,
        );

        let result = match project {
            Some(project) => {
                if let Some(receipt) =
                    replay_if_present(&mut transaction, &envelope, &request_hash).await?
                {
                    transaction.commit().await?;
                    self.dispatch_after_command(envelope.project_id).await;
                    return Ok(receipt);
                }
                project.require_revision(envelope.expected_project_revision)?;
                self.apply_existing_command(
                    &mut transaction,
                    project,
                    &envelope,
                    &request_hash,
                    command_id,
                    now,
                )
                .await
            }
            None => {
                self.create_project_command(
                    &mut transaction,
                    &envelope,
                    &request_hash,
                    command_id,
                    now,
                )
                .await
            }
        };
        match result {
            Ok(receipt) => {
                transaction.commit().await?;
                self.dispatch_after_command(envelope.project_id).await;
                Ok(receipt)
            }
            Err(error) => Err(error),
        }
    }

    async fn create_project_command(
        &self,
        transaction: &mut StorageTransaction<'_>,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<CommandReceipt, CoreError> {
        let CommandPayload::CreateProject(input) = &envelope.payload else {
            return Err(CoreError::NotFound {
                aggregate: "project",
            });
        };
        let project = Project::new(envelope.project_id, input.name.clone(), now)?;
        transaction.insert_project(&project).await?;
        let audit = event(
            project.id(),
            AggregateRef::Project(project.id()),
            project.revision(),
            DomainEventKind::ProjectCreated,
            self.actors.human,
            command_id,
            None,
            event_payload([("name", json!(project.name()))]),
            now,
        )?;
        finish_command(
            transaction,
            &project,
            envelope,
            request_hash,
            command_id,
            self.actors.human,
            vec![audit],
            Some(resource("project", project.id().as_uuid())),
        )
        .await
    }

    async fn apply_existing_command(
        &self,
        transaction: &mut StorageTransaction<'_>,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<CommandReceipt, CoreError> {
        match &envelope.payload {
            CommandPayload::ConfigureBootRecoveryPolicy { .. }
            | CommandPayload::AcceptRunRecoveryAssessment { .. } => {
                self.apply_recovery_command(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                )
                .await
            }
            CommandPayload::EnrollCredential {
                secret_id,
                binding_id,
                kind,
                source_file,
            } => {
                self.enroll_credential(
                    transaction,
                    project.id(),
                    *secret_id,
                    *binding_id,
                    kind,
                    source_file,
                )
                .await?;
                let previous_revision = project.revision();
                project.record_child_mutation(now)?;
                transaction
                    .update_project(&project, previous_revision)
                    .await?;
                let audit = event(
                    project.id(),
                    AggregateRef::Project(project.id()),
                    project.revision(),
                    DomainEventKind::CredentialUpdated,
                    self.actors.human,
                    command_id,
                    None,
                    event_payload([
                        ("secret_id", json!(secret_id)),
                        ("version", json!(1)),
                        ("kind", json!(kind)),
                    ]),
                    now,
                )?;
                finish_command(
                    transaction,
                    &project,
                    envelope,
                    request_hash,
                    command_id,
                    self.actors.human,
                    vec![audit],
                    Some(resource("credential", *secret_id)),
                )
                .await
            }
            CommandPayload::ConfigureEmployeeRuntime {
                employee_id,
                binding,
            } => {
                transaction
                    .bind_employee_runtime(project.id(), *employee_id, binding)
                    .await?;
                let previous_revision = project.revision();
                project.record_child_mutation(now)?;
                transaction
                    .update_project(&project, previous_revision)
                    .await?;
                let audit = event(
                    project.id(),
                    AggregateRef::Project(project.id()),
                    project.revision(),
                    DomainEventKind::EmployeeRuntimeConfigured,
                    self.actors.human,
                    command_id,
                    None,
                    event_payload([
                        ("employee_id", json!(employee_id)),
                        ("profile_id", json!(binding.execution_profile.id())),
                        (
                            "profile_revision",
                            json!(binding.execution_profile.revision()),
                        ),
                    ]),
                    now,
                )?;
                finish_command(
                    transaction,
                    &project,
                    envelope,
                    request_hash,
                    command_id,
                    self.actors.human,
                    vec![audit],
                    Some(resource("employee", employee_id.as_uuid())),
                )
                .await
            }
            CommandPayload::CreateProject(_) => Err(CoreError::InvalidTransport {
                field: "create_project.project_id",
                reason: "already exists".to_owned(),
            }),
            CommandPayload::CreatePipeline(input) => {
                let pipeline_id = PipelineId::new();
                let version_id = PipelineVersionId::new();
                let (pipeline, version) = input.build(
                    pipeline_id,
                    version_id,
                    project.id(),
                    self.actors.human,
                    now,
                )?;
                transaction.insert_pipeline(&pipeline).await?;
                transaction.insert_pipeline_version(&version).await?;
                let original_revision = project.revision();
                project.record_child_mutation(now)?;
                transaction
                    .update_project(&project, original_revision)
                    .await?;
                let events = vec![
                    event(
                        project.id(),
                        AggregateRef::Pipeline(pipeline.id()),
                        1,
                        DomainEventKind::PipelineCreated,
                        self.actors.human,
                        command_id,
                        None,
                        event_payload([
                            ("pipeline_id", json!(pipeline.id().as_uuid())),
                            ("name", json!(pipeline.name())),
                        ]),
                        now,
                    )?,
                    event(
                        project.id(),
                        AggregateRef::PipelineVersion(version.id()),
                        u64::from(version.version()),
                        DomainEventKind::PipelineVersionPublished,
                        self.actors.human,
                        command_id,
                        None,
                        event_payload([
                            ("pipeline_id", json!(pipeline.id().as_uuid())),
                            ("pipeline_version_id", json!(version.id().as_uuid())),
                            ("max_stage_visits", json!(version.max_stage_visits())),
                        ]),
                        now,
                    )?,
                ];
                finish_command(
                    transaction,
                    &project,
                    envelope,
                    request_hash,
                    command_id,
                    self.actors.human,
                    events,
                    Some(resource("pipeline", pipeline.id().as_uuid())),
                )
                .await
            }
            CommandPayload::CreateEmployee(input) => {
                let employee = input.build(
                    forge_domain::EmployeeId::new(),
                    project.id(),
                    self.actors.human,
                    now,
                )?;
                validate_employee_stage_eligibility(transaction, &project, &employee).await?;
                transaction.insert_employee(&employee).await?;
                let original_revision = project.revision();
                project.record_child_mutation(now)?;
                transaction
                    .update_project(&project, original_revision)
                    .await?;
                let audit = event(
                    project.id(),
                    AggregateRef::Employee(employee.id()),
                    forge_domain::Employee::INITIAL_REVISION,
                    DomainEventKind::EmployeeCreated,
                    self.actors.human,
                    command_id,
                    None,
                    event_payload([("name", json!(employee.name()))]),
                    now,
                )?;
                finish_command(
                    transaction,
                    &project,
                    envelope,
                    request_hash,
                    command_id,
                    self.actors.human,
                    vec![audit],
                    Some(resource("employee", employee.id().as_uuid())),
                )
                .await
            }
            CommandPayload::CreateTask(input) => {
                let version_id = input.pipeline_version_id()?;
                let version = transaction.lock_pipeline_version(version_id).await?.ok_or(
                    CoreError::NotFound {
                        aggregate: "pipeline version",
                    },
                )?;
                let pipeline = transaction
                    .lock_pipeline(version.pipeline_id())
                    .await?
                    .ok_or(CoreError::NotFound {
                        aggregate: "pipeline",
                    })?;
                ensure_assignable_pipeline(&project, &pipeline, &version)?;
                if !version.supports_task_kind(input.kind) {
                    return Err(CoreError::InvalidTransport {
                        field: "kind",
                        reason: "is not supported by the selected pipeline version".to_owned(),
                    });
                }
                let original_project_revision = project.revision();
                let key = project.allocate_task_key(now)?;
                let task = input.build(forge_application::TaskDraftContext {
                    id: forge_domain::TaskId::new(),
                    project_id: project.id(),
                    key,
                    pipeline: TaskPipelineBinding::new(
                        pipeline.id(),
                        version.id(),
                        version.entry_stage_id().clone(),
                    ),
                    created_by: self.actors.human,
                    created_at: now,
                    priority_scheme: project.priority_scheme(),
                })?;
                let persistence = task_persistence(&project, &task, 0)?;
                transaction.insert_task(&task, persistence).await?;
                transaction
                    .update_project(&project, original_project_revision)
                    .await?;
                let audit = event(
                    project.id(),
                    AggregateRef::Task(task.id()),
                    task.revision().get(),
                    DomainEventKind::TaskCreated,
                    self.actors.human,
                    command_id,
                    None,
                    event_payload([
                        ("task_key", json!(task.key().to_string())),
                        ("title", json!(task.spec().title())),
                    ]),
                    now,
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
            CommandPayload::StartProjectExecution { reason } => {
                transaction.set_recovery_hold(project.id(), false).await?;
                let original_revision = project.revision();
                project.start_execution(now)?;
                transaction
                    .update_project(&project, original_revision)
                    .await?;
                let audit = event(
                    project.id(),
                    AggregateRef::Project(project.id()),
                    project.revision(),
                    DomainEventKind::ProjectExecutionStarted,
                    self.actors.human,
                    command_id,
                    reason.clone(),
                    event_payload([]),
                    now,
                )?;
                finish_command(
                    transaction,
                    &project,
                    envelope,
                    request_hash,
                    command_id,
                    self.actors.human,
                    vec![audit],
                    Some(resource("project", project.id().as_uuid())),
                )
                .await
            }
            CommandPayload::StopProjectExecution { reason } => {
                self.stop_project_execution(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                    reason.clone(),
                )
                .await
            }
            CommandPayload::AmendDraft { .. }
            | CommandPayload::ApproveTask { .. }
            | CommandPayload::CancelTask { .. }
            | CommandPayload::ResumeTask { .. }
            | CommandPayload::SetTaskPriority { .. }
            | CommandPayload::CreateDependency { .. }
            | CommandPayload::RemoveDependency { .. }
            | CommandPayload::SubmitExternalStageOutcome(_) => {
                self.apply_task_or_dependency_command(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                )
                .await
            }
        }
    }
}

impl CoreService {
    async fn dispatch_after_command(&self, project_id: forge_domain::ProjectId) {
        if let Err(error) = self.deliver_pending_stop_requests(project_id).await {
            warn!(project_id = %project_id, error = %error, "deferred Supervisor stop delivery failed");
        }
        if let Err(error) = self.dispatch_available(project_id).await {
            // Command state, audit Event, and outbox have already committed.
            // Dispatch is recoverable desired-state delivery, so returning this
            // error here would falsely report that the accepted command failed.
            warn!(project_id = %project_id, error = %error, "deferred scheduler dispatch failed");
        }
    }
}

#[path = "command/receipt.rs"]
mod receipt;
use receipt::{command_fingerprint, replay_if_present};
pub(crate) use receipt::{finish_command, resource};
