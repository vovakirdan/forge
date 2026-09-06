//! Canonical M0 Project, Pipeline, Employee and Task creation commands.
use super::{
    CommandError, CommandTransaction, Engine,
    event::{event, event_payload},
    pipeline_access::{ensure_assignable_pipeline, validate_employee_stage_eligibility},
    receipt::{finish_command, resource},
    scheduler::task_persistence,
};
use crate::{CommandEnvelope, CommandPayload};
use forge_domain::{
    AggregateRef, CommandId, DomainEventKind, PipelineId, PipelineVersionId, Project,
    TaskPipelineBinding, Timestamp,
};
use forge_protocol::wire::CommandReceipt;
use serde_json::json;
impl Engine<'_> {
    pub(super) async fn create_project_command(
        &self,
        transaction: &mut impl CommandTransaction,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<CommandReceipt, CommandError> {
        let CommandPayload::CreateProject(input) = &envelope.payload else {
            return Err(CommandError::NotFound {
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

    pub(super) async fn apply_existing_command(
        &self,
        transaction: &mut impl CommandTransaction,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<CommandReceipt, CommandError> {
        match &envelope.payload {
            CommandPayload::ConfigureBootRecoveryPolicy { .. }
            | CommandPayload::AcceptRunRecoveryAssessment { .. }
            | CommandPayload::EnrollCredential { .. }
            | CommandPayload::ConfigureEmployeeRuntime { .. } => {
                Err(CommandError::UnsupportedCommand)
            }
            CommandPayload::CreateProject(_) => Err(CommandError::InvalidTransport {
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
                    CommandError::NotFound {
                        aggregate: "pipeline version",
                    },
                )?;
                let pipeline = transaction
                    .lock_pipeline(version.pipeline_id())
                    .await?
                    .ok_or(CommandError::NotFound {
                        aggregate: "pipeline",
                    })?;
                ensure_assignable_pipeline(&project, &pipeline, &version)?;
                if !version.supports_task_kind(input.kind) {
                    return Err(CommandError::InvalidTransport {
                        field: "kind",
                        reason: "is not supported by the selected pipeline version".to_owned(),
                    });
                }
                let original_project_revision = project.revision();
                let key = project.allocate_task_key(now)?;
                let task = input.build(crate::TaskDraftContext {
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
