//! Explicit browser mutation contracts, independent of the wider Core command catalog.

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use forge_protocol::wire::{ApiErrorCode, CommandReceipt, ErrorResponse};
use http_body_util::Full;
use serde::{Deserialize, Deserializer};

use crate::http::{ApiError, HttpState, single_header};

pub(crate) const BODY_LIMIT: usize = 512 * 1024;
pub(crate) const RECEIPT_LIMIT: usize = 64 * 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy)]
pub(crate) enum CommandTarget {
    ConfigureTaskPropertySchema,
    CreateEmployee,
    AmendEmployee,
    EnableEmployee,
    DisableEmployee,
    RetireEmployee,
    CreateTask,
    ApproveTask,
    CancelTask,
    AmendDraft,
    SetTaskPriority,
    CreateDependency,
    RemoveDependency,
    AuthorKnowledgePage,
    PublishKnowledgePage,
    SupersedeKnowledgePage,
    WithdrawKnowledgePage,
    PublishPipelineVersion,
    SetPipelineDefaultVersion,
    DeletePipeline,
    CreateProject,
    CreatePipeline,
    ConfigureProjectHook,
    ConfigureEmployeeRuntime,
    ConfigureSystemJobs,
    RequestTaskSummary,
    RequestEmployeeOnboarding,
    RetrySystemJob,
    SkipEmployeeOnboarding,
    OpenEmployeeThread,
    SendEmployeeMessage,
    WaiveMessageRequirement,
    RetryCommunication,
    RetryGitIntegration,
    AcceptGitIntegrationResult,
    StartProjectExecution,
    StopProjectExecution,
    PauseTask,
    ResumeTask,
    StopEmployee,
    SubmitHumanResolution,
    RerouteEscalation,
    AcceptRunRecoveryAssessment,
    SetNextRunEmployee,
    ClearNextRunEmployee,
    ScheduleTaskResume,
    CancelTaskResume,
    ConfigureResolverRoute,
    RaiseEscalation,
}

impl CommandTarget {
    pub(crate) fn from_browser_path(path: &str) -> Option<Self> {
        match path {
            "/api/commands/configure_task_property_schema" => {
                Some(Self::ConfigureTaskPropertySchema)
            }
            "/api/commands/create_employee" => Some(Self::CreateEmployee),
            "/api/commands/amend_employee" => Some(Self::AmendEmployee),
            "/api/commands/enable_employee" => Some(Self::EnableEmployee),
            "/api/commands/disable_employee" => Some(Self::DisableEmployee),
            "/api/commands/retire_employee" => Some(Self::RetireEmployee),
            "/api/commands/create_task" => Some(Self::CreateTask),
            "/api/commands/approve_task" => Some(Self::ApproveTask),
            "/api/commands/cancel_task" => Some(Self::CancelTask),
            "/api/commands/amend_draft" => Some(Self::AmendDraft),
            "/api/commands/set_task_priority" => Some(Self::SetTaskPriority),
            "/api/commands/create_dependency" => Some(Self::CreateDependency),
            "/api/commands/remove_dependency" => Some(Self::RemoveDependency),
            "/api/commands/author_knowledge_page" => Some(Self::AuthorKnowledgePage),
            "/api/commands/publish_knowledge_page" => Some(Self::PublishKnowledgePage),
            "/api/commands/supersede_knowledge_page" => Some(Self::SupersedeKnowledgePage),
            "/api/commands/withdraw_knowledge_page" => Some(Self::WithdrawKnowledgePage),
            "/api/commands/publish_pipeline_version" => Some(Self::PublishPipelineVersion),
            "/api/commands/set_pipeline_default_version" => Some(Self::SetPipelineDefaultVersion),
            "/api/commands/delete_pipeline" => Some(Self::DeletePipeline),
            "/api/commands/create_project" => Some(Self::CreateProject),
            "/api/commands/create_pipeline" => Some(Self::CreatePipeline),
            "/api/commands/configure_project_hook" => Some(Self::ConfigureProjectHook),
            "/api/commands/configure_employee_runtime" => Some(Self::ConfigureEmployeeRuntime),
            "/api/commands/configure_system_jobs" => Some(Self::ConfigureSystemJobs),
            "/api/commands/request_task_summary" => Some(Self::RequestTaskSummary),
            "/api/commands/request_employee_onboarding" => Some(Self::RequestEmployeeOnboarding),
            "/api/commands/retry_system_job" => Some(Self::RetrySystemJob),
            "/api/commands/skip_employee_onboarding" => Some(Self::SkipEmployeeOnboarding),
            "/api/commands/open_employee_thread" => Some(Self::OpenEmployeeThread),
            "/api/commands/send_employee_message" => Some(Self::SendEmployeeMessage),
            "/api/commands/waive_message_requirement" => Some(Self::WaiveMessageRequirement),
            "/api/commands/retry_communication" => Some(Self::RetryCommunication),
            "/api/commands/retry_git_integration" => Some(Self::RetryGitIntegration),
            "/api/commands/accept_git_integration_result" => Some(Self::AcceptGitIntegrationResult),
            "/api/commands/start_project_execution" => Some(Self::StartProjectExecution),
            "/api/commands/stop_project_execution" => Some(Self::StopProjectExecution),
            "/api/commands/pause_task" => Some(Self::PauseTask),
            "/api/commands/resume_task" => Some(Self::ResumeTask),
            "/api/commands/stop_employee" => Some(Self::StopEmployee),
            "/api/commands/submit_human_resolution" => Some(Self::SubmitHumanResolution),
            "/api/commands/reroute_escalation" => Some(Self::RerouteEscalation),
            "/api/commands/accept_run_recovery_assessment" => {
                Some(Self::AcceptRunRecoveryAssessment)
            }
            "/api/commands/set_next_run_employee" => Some(Self::SetNextRunEmployee),
            "/api/commands/clear_next_run_employee" => Some(Self::ClearNextRunEmployee),
            "/api/commands/schedule_task_resume" => Some(Self::ScheduleTaskResume),
            "/api/commands/cancel_task_resume" => Some(Self::CancelTaskResume),
            "/api/commands/configure_resolver_route" => Some(Self::ConfigureResolverRoute),
            "/api/commands/raise_escalation" => Some(Self::RaiseEscalation),
            _ => None,
        }
    }

    fn core_path(self) -> &'static str {
        match self {
            Self::ConfigureTaskPropertySchema => "/v1/commands/configure_task_property_schema",
            Self::CreateEmployee => "/v1/commands/create_employee",
            Self::AmendEmployee => "/v1/commands/amend_employee",
            Self::EnableEmployee => "/v1/commands/enable_employee",
            Self::DisableEmployee => "/v1/commands/disable_employee",
            Self::RetireEmployee => "/v1/commands/retire_employee",
            Self::CreateTask => "/v1/commands/create_task",
            Self::ApproveTask => "/v1/commands/approve_task",
            Self::CancelTask => "/v1/commands/cancel_task",
            Self::AmendDraft => "/v1/commands/amend_draft",
            Self::SetTaskPriority => "/v1/commands/set_task_priority",
            Self::CreateDependency => "/v1/commands/create_dependency",
            Self::RemoveDependency => "/v1/commands/remove_dependency",
            Self::AuthorKnowledgePage => "/v1/commands/author_knowledge_page",
            Self::PublishKnowledgePage => "/v1/commands/publish_knowledge_page",
            Self::SupersedeKnowledgePage => "/v1/commands/supersede_knowledge_page",
            Self::WithdrawKnowledgePage => "/v1/commands/withdraw_knowledge_page",
            Self::PublishPipelineVersion => "/v1/commands/publish_pipeline_version",
            Self::SetPipelineDefaultVersion => "/v1/commands/set_pipeline_default_version",
            Self::DeletePipeline => "/v1/commands/delete_pipeline",
            Self::CreateProject => "/v1/commands/create_project",
            Self::CreatePipeline => "/v1/commands/create_pipeline",
            Self::ConfigureProjectHook => "/v1/commands/configure_project_hook",
            Self::ConfigureEmployeeRuntime => "/v1/commands/configure_employee_runtime",
            Self::ConfigureSystemJobs => "/v1/commands/configure_system_jobs",
            Self::RequestTaskSummary => "/v1/commands/request_task_summary",
            Self::RequestEmployeeOnboarding => "/v1/commands/request_employee_onboarding",
            Self::RetrySystemJob => "/v1/commands/retry_system_job",
            Self::SkipEmployeeOnboarding => "/v1/commands/skip_employee_onboarding",
            Self::OpenEmployeeThread => "/v1/commands/open_employee_thread",
            Self::SendEmployeeMessage => "/v1/commands/send_employee_message",
            Self::WaiveMessageRequirement => "/v1/commands/waive_message_requirement",
            Self::RetryCommunication => "/v1/commands/retry_communication",
            Self::RetryGitIntegration => "/v1/commands/retry_git_integration",
            Self::AcceptGitIntegrationResult => "/v1/commands/accept_git_integration_result",
            Self::StartProjectExecution => "/v1/commands/start_project_execution",
            Self::StopProjectExecution => "/v1/commands/stop_project_execution",
            Self::PauseTask => "/v1/commands/pause_task",
            Self::ResumeTask => "/v1/commands/resume_task",
            Self::StopEmployee => "/v1/commands/stop_employee",
            Self::SubmitHumanResolution => "/v1/commands/submit_human_resolution",
            Self::RerouteEscalation => "/v1/commands/reroute_escalation",
            Self::AcceptRunRecoveryAssessment => "/v1/commands/accept_run_recovery_assessment",
            Self::SetNextRunEmployee => "/v1/commands/set_next_run_employee",
            Self::ClearNextRunEmployee => "/v1/commands/clear_next_run_employee",
            Self::ScheduleTaskResume => "/v1/commands/schedule_task_resume",
            Self::CancelTaskResume => "/v1/commands/cancel_task_resume",
            Self::ConfigureResolverRoute => "/v1/commands/configure_resolver_route",
            Self::RaiseEscalation => "/v1/commands/raise_escalation",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetTaskPriority {
    project_id: String,
    expected_revision: u64,
    payload: PriorityPayload,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PriorityPayload {
    task_id: String,
    expected_task_revision: u64,
    priority: String,
}

impl SetTaskPriority {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, ApiError> {
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !uuid_v7(&value.project_id)
            || !uuid_v7(&value.payload.task_id)
            || !(1..MAX_SAFE_INTEGER).contains(&value.expected_revision)
            || !(1..MAX_SAFE_INTEGER).contains(&value.payload.expected_task_revision)
            || !(1..=64).contains(&value.payload.priority.len())
            || !value
                .payload
                .priority
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_lowercase)
            || !value
                .payload
                .priority
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(ApiError::BadRequest);
        }
        Ok(value)
    }

    pub(crate) fn validate_receipt(&self, bytes: &[u8]) -> Result<(), ApiError> {
        validate_receipt(bytes, self.expected_revision, Some(&self.payload.task_id))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AmendDraft {
    project_id: String,
    expected_revision: u64,
    payload: DraftPayload,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftPayload {
    task_id: String,
    expected_task_revision: u64,
    patch: TextPatch,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextPatch {
    #[serde(default, deserialize_with = "present_nullable_string")]
    definition_of_done: Option<Option<String>>,
    #[serde(default, deserialize_with = "present_string")]
    title: Option<String>,
    #[serde(default, deserialize_with = "present_string")]
    description: Option<String>,
}

fn present_nullable_string<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}

fn present_string<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    // Missing means unchanged; JSON null is not an instruction to erase a text field.
    String::deserialize(deserializer).map(Some)
}

impl AmendDraft {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, ApiError> {
        let shape: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !shape.is_object()
            || !shape
                .get("payload")
                .is_some_and(serde_json::Value::is_object)
            || !shape
                .pointer("/payload/patch")
                .is_some_and(serde_json::Value::is_object)
        {
            return Err(ApiError::BadRequest);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !uuid_v7(&value.project_id)
            || !uuid_v7(&value.payload.task_id)
            || !(1..MAX_SAFE_INTEGER).contains(&value.expected_revision)
            || !(1..MAX_SAFE_INTEGER).contains(&value.payload.expected_task_revision)
            || (value.payload.patch.title.is_none()
                && value.payload.patch.description.is_none()
                && value.payload.patch.definition_of_done.is_none())
            || value
                .payload
                .patch
                .definition_of_done
                .as_ref()
                .and_then(Option::as_ref)
                .is_some_and(|text| text.trim().is_empty() || text.chars().count() > 20_000)
        {
            return Err(ApiError::BadRequest);
        }
        Ok(value)
    }

    pub(crate) fn validate_receipt(&self, bytes: &[u8]) -> Result<(), ApiError> {
        validate_receipt(bytes, self.expected_revision, Some(&self.payload.task_id))
    }
}

pub(crate) fn validate_receipt(
    bytes: &[u8],
    expected_revision: u64,
    task_id: Option<&str>,
) -> Result<(), ApiError> {
    validate_receipt_revision(bytes, expected_revision, task_id, false)
}

pub(crate) fn validate_receipt_revision(
    bytes: &[u8],
    expected_revision: u64,
    task_id: Option<&str>,
    dependent_mutations: bool,
) -> Result<(), ApiError> {
    let receipt: CommandReceipt =
        serde_json::from_slice(bytes).map_err(|_| ApiError::BadGateway)?;
    if !uuid_v7(&receipt.command_id)
        || if dependent_mutations {
            receipt.project_revision <= expected_revision
                || receipt.project_revision > MAX_SAFE_INTEGER
        } else {
            receipt.project_revision != expected_revision + 1
        }
        || receipt.event_ids.is_empty()
        || !receipt.event_ids.iter().all(|id| uuid_v7(id))
        || !receipt.resource.as_ref().is_some_and(|resource| {
            resource.kind == "task"
                && uuid_v7(&resource.id)
                && task_id.is_none_or(|task_id| resource.id.eq_ignore_ascii_case(task_id))
        })
    {
        return Err(ApiError::BadGateway);
    }
    Ok(())
}

pub(crate) fn uuid_v7(value: &str) -> bool {
    value.len() == 36
        && uuid::Uuid::parse_str(value)
            .is_ok_and(|id| id.get_version_num() == 7 && id.get_variant() == uuid::Variant::RFC4122)
}

pub(crate) async fn execute(
    state: &HttpState,
    request: Request<Body>,
    target: CommandTarget,
) -> Result<Response, ApiError> {
    if !single_header(request.headers(), header::CONTENT_TYPE).is_some_and(|value| {
        value.eq_ignore_ascii_case("application/json")
            || value.eq_ignore_ascii_case("application/json; charset=utf-8")
    }) {
        return Err(ApiError::BadRequest);
    }
    let key = single_header(
        request.headers(),
        header::HeaderName::from_static("idempotency-key"),
    )
    .filter(|key| !key.trim().is_empty() && key.len() <= 128)
    .ok_or(ApiError::BadRequest)?
    .to_owned();
    if single_header(request.headers(), header::CONTENT_LENGTH)
        .and_then(|length| length.parse::<u64>().ok())
        .is_some_and(|length| length > BODY_LIMIT as u64)
    {
        return Err(ApiError::TooLarge);
    }
    let body = to_bytes(request.into_body(), BODY_LIMIT)
        .await
        .map_err(|_| ApiError::TooLarge)?;
    let response = match target {
        CommandTarget::ConfigureTaskPropertySchema => {
            let command = crate::property_schema_command::PropertySchemaCommand::parse(&body)?;
            let response = state.core.command(target, &key, body).await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::CreateEmployee => {
            let command = crate::create_employee::CreateEmployee::parse(&body)?;
            let response = state.core.command(target, &key, body).await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::AmendEmployee => {
            let command = crate::amend_employee::AmendEmployee::parse(&body)?;
            let response = state.core.command(target, &key, body).await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::EnableEmployee
        | CommandTarget::DisableEmployee
        | CommandTarget::RetireEmployee => {
            let command = crate::employee_lifecycle::EmployeeLifecycle::parse(&body)?;
            let response = state.core.command(target, &key, body).await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::CancelTask => {
            let command = crate::cancel_task::CancelTask::parse(&body)?;
            let response = state
                .core
                .command(CommandTarget::CancelTask, &key, body)
                .await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::ApproveTask => {
            let command = crate::approve_task::ApproveTask::parse(&body)?;
            let response = state
                .core
                .command(CommandTarget::ApproveTask, &key, body)
                .await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::CreateTask => {
            let command = crate::create_task::CreateTask::parse(&body)?;
            let response = state
                .core
                .command(CommandTarget::CreateTask, &key, body)
                .await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::AmendDraft => {
            let command = AmendDraft::parse(&body)?;
            state.core.amend_draft(&command, &key, body).await?
        }
        CommandTarget::SetTaskPriority => {
            let command = SetTaskPriority::parse(&body)?;
            state.core.set_task_priority(&command, &key, body).await?
        }
        CommandTarget::CreateDependency | CommandTarget::RemoveDependency => {
            let command = crate::dependency_command::DependencyCommand::parse(&body, target)?;
            let response = state.core.command(target, &key, body).await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::AuthorKnowledgePage
        | CommandTarget::PublishKnowledgePage
        | CommandTarget::SupersedeKnowledgePage
        | CommandTarget::WithdrawKnowledgePage => {
            let command = crate::knowledge_command::KnowledgeCommand::parse(&body, target)?;
            let response = state.core.command(target, &key, body).await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::PublishPipelineVersion
        | CommandTarget::SetPipelineDefaultVersion
        | CommandTarget::DeletePipeline => {
            let command = crate::pipeline_management_command::PipelineManagementCommand::parse(
                &body, target,
            )?;
            let response = state.core.command(target, &key, body).await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::CreateProject => {
            let command = crate::create_project::CreateProject::parse(&body)?;
            let response = state.core.command(target, &key, body).await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::CreatePipeline => {
            let command = crate::create_pipeline::CreatePipeline::parse(&body)?;
            let response = state.core.command(target, &key, body).await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::ConfigureProjectHook => {
            let command = crate::project_hook_command::ConfigureProjectHook::parse(&body)?;
            let response = state.core.command(target, &key, body).await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::ConfigureEmployeeRuntime => {
            let command = crate::employee_runtime_command::ConfigureEmployeeRuntime::parse(&body)?;
            let response = state.core.command(target, &key, body).await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::ConfigureSystemJobs
        | CommandTarget::RequestTaskSummary
        | CommandTarget::RequestEmployeeOnboarding
        | CommandTarget::RetrySystemJob
        | CommandTarget::SkipEmployeeOnboarding => {
            let command = crate::system_job_command::SystemJobCommand::parse(&body, target)?;
            let response = state.core.command(target, &key, body).await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::OpenEmployeeThread
        | CommandTarget::SendEmployeeMessage
        | CommandTarget::WaiveMessageRequirement
        | CommandTarget::RetryCommunication => {
            let command = crate::inbox_command::InboxCommand::parse(&body, target)?;
            let response = state.core.command(target, &key, body).await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::RetryGitIntegration | CommandTarget::AcceptGitIntegrationResult => {
            let command = crate::git_recovery_command::GitRecoveryCommand::parse(&body, target)?;
            let response = state.core.command(target, &key, body).await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::StartProjectExecution
        | CommandTarget::StopProjectExecution
        | CommandTarget::PauseTask
        | CommandTarget::ResumeTask
        | CommandTarget::StopEmployee
        | CommandTarget::SubmitHumanResolution
        | CommandTarget::RerouteEscalation
        | CommandTarget::AcceptRunRecoveryAssessment => {
            let command = crate::management_command::ManagementCommand::parse(&body, target)?;
            let response = state.core.command(target, &key, body).await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::SetNextRunEmployee
        | CommandTarget::ClearNextRunEmployee
        | CommandTarget::ScheduleTaskResume
        | CommandTarget::CancelTaskResume => {
            let command =
                crate::manager_planning_command::ManagerPlanningCommand::parse(&body, target)?;
            let response = state.core.command(target, &key, body).await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::ConfigureResolverRoute | CommandTarget::RaiseEscalation => {
            let command = crate::resolver_command::ResolverCommand::parse(&body, target)?;
            let response = state.core.command(target, &key, body).await?;
            command.validate_receipt(&response)?;
            response
        }
    };
    Ok(([(header::CONTENT_TYPE, "application/json")], response).into_response())
}

impl crate::core_client::CoreClient {
    pub(crate) async fn amend_draft(
        &self,
        command: &AmendDraft,
        key: &str,
        body: Bytes,
    ) -> Result<Bytes, ApiError> {
        let bytes = self.command(CommandTarget::AmendDraft, key, body).await?;
        command.validate_receipt(&bytes)?;
        Ok(bytes)
    }

    pub(crate) async fn set_task_priority(
        &self,
        command: &SetTaskPriority,
        key: &str,
        body: Bytes,
    ) -> Result<Bytes, ApiError> {
        let bytes = self
            .command(CommandTarget::SetTaskPriority, key, body)
            .await?;
        command.validate_receipt(&bytes)?;
        Ok(bytes)
    }

    async fn command(
        &self,
        target: CommandTarget,
        key: &str,
        body: Bytes,
    ) -> Result<Bytes, ApiError> {
        let request = Request::builder()
            .method("POST")
            .uri(target.core_path())
            .header(header::HOST, "localhost")
            .header(header::ACCEPT, "application/json")
            .header(header::CONTENT_TYPE, "application/json")
            .header("idempotency-key", key)
            .body(Full::new(body))
            .map_err(|_| ApiError::BadRequest)?;
        let (status, json, _, bytes) = self.exchange(request, RECEIPT_LIMIT).await?;
        if !json {
            return Err(ApiError::BadGateway);
        }
        if status == StatusCode::OK {
            return Ok(bytes);
        }
        let error: ErrorResponse =
            serde_json::from_slice(&bytes).map_err(|_| ApiError::BadGateway)?;
        // Only explicit, typed Core refusals prove that the command was not applied.
        // Unknown statuses, malformed bodies and 5xx never become a definite refusal.
        Err(match (status, error.error.code) {
            (StatusCode::BAD_REQUEST, ApiErrorCode::InvalidRequest) => ApiError::BadRequest,
            (StatusCode::CONFLICT, ApiErrorCode::StaleRevision) => ApiError::StaleRevision,
            (StatusCode::CONFLICT, ApiErrorCode::Conflict) => ApiError::Conflict,
            (StatusCode::CONFLICT, ApiErrorCode::IdempotencyConflict) => {
                ApiError::IdempotencyConflict
            }
            (StatusCode::UNPROCESSABLE_ENTITY, ApiErrorCode::ValidationFailed) => {
                ApiError::ValidationFailed
            }
            (StatusCode::NOT_FOUND, ApiErrorCode::NotFound) => ApiError::NotFound,
            (StatusCode::FORBIDDEN, ApiErrorCode::Forbidden) => ApiError::CommandForbidden,
            (StatusCode::SERVICE_UNAVAILABLE, ApiErrorCode::Unavailable) => ApiError::Unavailable,
            _ => ApiError::BadGateway,
        })
    }
}
