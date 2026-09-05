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
        let now = Timestamp::now_utc();
        let command_id = CommandId::new();
        let mut transaction = self.store.begin().await?;
        transaction
            .lock_project_creation(envelope.project_id)
            .await?;
        let project = transaction.lock_project(envelope.project_id).await?;

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

async fn replay_if_present(
    transaction: &mut StorageTransaction<'_>,
    envelope: &CommandEnvelope,
    request_hash: &str,
) -> Result<Option<CommandReceipt>, CoreError> {
    let Some(record) = transaction
        .lookup_idempotency(envelope.project_id, envelope.idempotency_key.as_str())
        .await?
    else {
        return Ok(None);
    };
    if record.request_hash != request_hash || record.command_name != command_name(envelope) {
        return Err(CoreError::IdempotencyConflict);
    }
    let mut receipt: CommandReceipt =
        serde_json::from_value(record.receipt).map_err(|error| CoreError::InvalidTransport {
            field: "idempotency.receipt",
            reason: error.to_string(),
        })?;
    receipt.status = CommandStatus::Replayed;
    Ok(Some(receipt))
}

#[expect(
    clippy::too_many_arguments,
    reason = "the canonical transaction boundary keeps command audit inputs explicit"
)]
pub(crate) async fn finish_command(
    transaction: &mut StorageTransaction<'_>,
    project: &Project,
    envelope: &CommandEnvelope,
    request_hash: &str,
    command_id: CommandId,
    actor: forge_domain::Actor,
    events: Vec<DomainEvent>,
    resource: Option<ResourceReference>,
) -> Result<CommandReceipt, CoreError> {
    let mut event_ids = Vec::with_capacity(events.len());
    for audit in &events {
        event_ids.push(transaction.append_event_and_outbox(audit).await?.id);
    }
    let receipt = CommandReceipt {
        command_id: command_id.as_uuid().to_string(),
        status: CommandStatus::Applied,
        project_revision: project.revision(),
        event_ids: event_ids.iter().map(ToString::to_string).collect(),
        resource,
    };
    let record = IdempotencyRecord {
        project_id: project.id(),
        key: envelope.idempotency_key.as_str().to_owned(),
        command_id,
        command_name: command_name(envelope),
        expected_revision: Some(envelope.expected_project_revision),
        request_hash: request_hash.to_owned(),
        actor: serde_json::to_value(actor).map_err(|error| CoreError::InvalidTransport {
            field: "idempotency.actor",
            reason: error.to_string(),
        })?,
        receipt: serde_json::to_value(&receipt).map_err(|error| CoreError::InvalidTransport {
            field: "idempotency.receipt",
            reason: error.to_string(),
        })?,
        event_id: event_ids.first().copied(),
        response_revision: Some(project.revision()),
    };
    transaction.insert_idempotency(&record).await?;
    Ok(receipt)
}

pub(crate) fn command_name(envelope: &CommandEnvelope) -> String {
    match envelope.name {
        forge_protocol::wire::CommandName::CreateProject => "create_project",
        forge_protocol::wire::CommandName::CreatePipeline => "create_pipeline",
        forge_protocol::wire::CommandName::CreateEmployee => "create_employee",
        forge_protocol::wire::CommandName::CreateTask => "create_task",
        forge_protocol::wire::CommandName::AmendDraft => "amend_draft",
        forge_protocol::wire::CommandName::ApproveTask => "approve_task",
        forge_protocol::wire::CommandName::CancelTask => "cancel_task",
        forge_protocol::wire::CommandName::ResumeTask => "resume_task",
        forge_protocol::wire::CommandName::SetTaskPriority => "set_task_priority",
        forge_protocol::wire::CommandName::CreateDependency => "create_dependency",
        forge_protocol::wire::CommandName::RemoveDependency => "remove_dependency",
        forge_protocol::wire::CommandName::StartProjectExecution => "start_project_execution",
        forge_protocol::wire::CommandName::StopProjectExecution => "stop_project_execution",
        forge_protocol::wire::CommandName::SubmitExternalStageOutcome => {
            "submit_external_stage_outcome"
        }
    }
    .to_owned()
}

pub(crate) fn resource(kind: &str, id: uuid::Uuid) -> ResourceReference {
    ResourceReference {
        kind: kind.to_owned(),
        id: id.to_string(),
    }
}

fn command_fingerprint(
    envelope: &CommandEnvelope,
    actor: forge_domain::Actor,
) -> Result<String, CoreError> {
    let canonical = json!({
        "command_name": command_name(envelope),
        "project_id": envelope.project_id.as_uuid(),
        "expected_project_revision": envelope.expected_project_revision,
        "payload": envelope.canonical_payload(),
        "actor": actor,
    });
    let bytes = serde_json::to_vec(&canonical).map_err(|error| CoreError::InvalidTransport {
        field: "idempotency.request",
        reason: error.to_string(),
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
