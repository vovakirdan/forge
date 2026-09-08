//! Canonical M0 Project, Pipeline, Employee and Task creation commands.
use super::{
    CommandError, CommandTransaction, Engine,
    event::{event, event_payload},
    pipeline_access::validate_employee_stage_eligibility,
    receipt::{finish_command, resource},
};
use crate::{CommandEnvelope, CommandPayload};
use forge_domain::{
    AggregateRef, CommandId, DomainEventKind, PipelineId, PipelineVersionId, Project, Timestamp,
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
            CommandPayload::ConfigureResolverRoute(_)
            | CommandPayload::RaiseEscalation(_)
            | CommandPayload::SubmitHumanResolution(_)
            | CommandPayload::RerouteEscalation(_) => {
                self.manage_resolution(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                )
                .await
            }
            CommandPayload::ReportFinding(_)
            | CommandPayload::TriageFinding(_)
            | CommandPayload::PromoteFinding(_) => {
                self.manage_finding(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                )
                .await
            }
            CommandPayload::ScheduleTaskResume { .. } | CommandPayload::CancelTaskResume { .. } => {
                self.manage_scheduled_resume(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                )
                .await
            }
            CommandPayload::SetNextRunEmployee { .. }
            | CommandPayload::ClearNextRunEmployee { .. } => {
                self.manage_dispatch_constraint(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                )
                .await
            }
            CommandPayload::StopEmployee { .. } | CommandPayload::PauseTask { .. } => {
                self.manage_execution(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                )
                .await
            }
            CommandPayload::RegisterProjectRepository { .. }
            | CommandPayload::BindTaskGitRepository { .. } => {
                self.manage_repository(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                )
                .await
            }
            CommandPayload::OpenEmployeeThread(_)
            | CommandPayload::SendEmployeeMessage(_)
            | CommandPayload::WaiveMessageRequirement(_) => {
                self.apply_communication_command(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                )
                .await
            }
            CommandPayload::AmendEmployee { .. }
            | CommandPayload::EnableEmployee { .. }
            | CommandPayload::DisableEmployee { .. }
            | CommandPayload::RetireEmployee { .. } => {
                self.manage_employee(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                )
                .await
            }
            CommandPayload::ConfigureBootRecoveryPolicy { .. }
            | CommandPayload::AcceptRunRecoveryAssessment { .. }
            | CommandPayload::RetryCommunication { .. }
            | CommandPayload::RetryGitIntegration { .. }
            | CommandPayload::AcceptGitIntegrationResult { .. }
            | CommandPayload::EnrollCredential { .. }
            | CommandPayload::ConfigureProjectHook(_)
            | CommandPayload::ConfigureEmployeeRuntime { .. } => {
                Err(CommandError::UnsupportedCommand)
            }
            CommandPayload::CreateProject(_) => Err(CommandError::InvalidTransport {
                field: "create_project.project_id",
                reason: "already exists".to_owned(),
            }),
            CommandPayload::PublishPipelineVersion { .. }
            | CommandPayload::SetPipelineDefaultVersion { .. }
            | CommandPayload::DeletePipeline { .. } => {
                self.manage_pipeline(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                )
                .await
            }
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
                        pipeline.revision(),
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
                    employee.revision(),
                    DomainEventKind::EmployeeCreated,
                    self.actors.human,
                    command_id,
                    None,
                    event_payload([
                        ("employee_id", json!(employee.id())),
                        ("name", json!(employee.name())),
                        ("role", json!(employee.role())),
                        ("stage_eligibility", json!(employee.stage_eligibility())),
                        ("state", json!(employee.state())),
                        ("max_concurrent_runs", json!(employee.max_concurrent_runs())),
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
                    Some(resource("employee", employee.id().as_uuid())),
                )
                .await
            }
            CommandPayload::CreateTask(input) => {
                let (task, audit) = self
                    .create_draft(
                        transaction,
                        &mut project,
                        input,
                        forge_domain::TaskSource::Human,
                        command_id,
                        now,
                    )
                    .await?;
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
