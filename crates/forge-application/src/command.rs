use forge_domain::{
    ArtifactId, CancellationReasonId, CommandId, EmployeeId, EventId, PipelineId,
    PipelineVersionId, PriorityLevelId, ProjectId, TaskId, WaitConditionId,
};
use forge_protocol::wire::{CommandName, CommandRequest, MAX_COMMAND_AUDIT_DETAIL_CHARACTERS};
pub use resolver_input::RaiseEscalationInput;
use serde_json::Value;
use uuid::Uuid;

use crate::{
    ApplicationError, CreateEmployeeCommand, CreatePipelineCommand, CreateProjectCommand,
    CreateTaskCommand, DraftTaskPatch, ExternalStageOutcomeCommand,
};

/// Bounded caller-provided identity of one logical command invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Validates an idempotency key without interpreting it as a domain ID.
    pub fn new(value: impl Into<String>) -> Result<Self, ApplicationError> {
        let value = value.into();
        if value.trim().is_empty() || value.len() > 128 {
            return Err(ApplicationError::InvalidIdempotencyKey {
                reason: "must be non-blank and at most 128 bytes".to_owned(),
            });
        }
        Ok(Self(value))
    }

    /// Returns the stable caller value exactly as provided after validation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Typed intent selected by one named HTTP command path.
#[derive(Clone, Debug, PartialEq)]
pub enum CommandPayload {
    ConfigureSystemJobs(forge_domain::system_job::SystemJobManagement),
    RequestTaskSummary(forge_domain::system_job::SystemJobManagement),
    RequestEmployeeOnboarding(forge_domain::system_job::SystemJobManagement),
    RetrySystemJob(forge_domain::system_job::SystemJobManagement),
    SkipEmployeeOnboarding(forge_domain::system_job::SystemJobManagement),
    AuthorKnowledgePage(forge_domain::knowledge::KnowledgePageMutation),
    PublishKnowledgePage(forge_domain::knowledge::KnowledgePageMutation),
    SupersedeKnowledgePage(forge_domain::knowledge::KnowledgePageMutation),
    WithdrawKnowledgePage(forge_domain::knowledge::KnowledgePageMutation),
    ImportTaskFileSnapshot(file_snapshot_input::ImportFileSnapshotInput),
    CaptureTaskFileSnapshot(file_snapshot_input::CaptureFileSnapshotInput),
    AttachTaskFileInput(file_snapshot_input::AttachFileInput),
    ConfigureProjectHook(Box<crate::ConfigureProjectHookInput>),
    ConfigureResolverRoute(resolver_input::ConfigureResolverRouteInput),
    RaiseEscalation(resolver_input::RaiseEscalationInput),
    SubmitHumanResolution(resolver_input::SubmitHumanResolutionInput),
    RerouteEscalation(resolver_input::RerouteEscalationInput),
    ReportFinding(crate::ReportFindingCommand),
    TriageFinding(crate::TriageFindingCommand),
    PromoteFinding(crate::PromoteFindingCommand),
    /// Request one future resolution of a specific manual pause.
    ScheduleTaskResume {
        task_id: TaskId,
        expected_task_revision: u64,
        wait_condition_id: WaitConditionId,
        not_before: forge_domain::Timestamp,
        reason: String,
    },
    /// Retire pending alarm intent; this does not pause/resume its Task.
    CancelTaskResume {
        schedule_id: Uuid,
        reason: Option<String>,
    },
    /// Pin the next Lease to one eligible Employee, not the Task owner.
    SetNextRunEmployee {
        task_id: TaskId,
        expected_task_revision: u64,
        employee_id: EmployeeId,
        reason: Option<String>,
    },
    /// Remove a pending or held constraint without changing current execution.
    ClearNextRunEmployee {
        task_id: TaskId,
        expected_task_revision: u64,
        reason: Option<String>,
    },
    /// Disable future Employee admission and request explicit execution stops.
    StopEmployee {
        employee_id: EmployeeId,
        expected_employee_revision: u64,
        mode: forge_domain::ExecutionStopMode,
        reason: Option<String>,
    },
    /// Pause the named Task independently of its processes' observed state.
    PauseTask {
        task_id: TaskId,
        expected_task_revision: u64,
        mode: forge_domain::ExecutionStopMode,
        reason: Option<String>,
    },
    /// Operator-only enrollment of a local source, never a worker-supplied path.
    RegisterProjectRepository {
        name: String,
        source: forge_domain::git::LocalGitPath,
        target_ref: forge_domain::git::GitBranchRef,
    },
    /// Bind once before Task approval; source existence is verified at provisioning.
    BindTaskGitRepository {
        task_id: TaskId,
        expected_task_revision: u64,
        repository_id: Uuid,
        initial_base: forge_domain::git::GitInitialRevision,
    },
    /// Future-only Git selection; does not alter the current Task/Run revision.
    SetTaskGitSourcePolicy {
        task_id: TaskId,
        expected_policy_revision: u64,
        policy: forge_domain::git::TaskGitSourcePolicy,
        reason: String,
    },
    /// Open an independent durable Employee conversation.
    OpenEmployeeThread(crate::OpenEmployeeThreadCommand),
    /// Append a message using the authenticated command actor as sender.
    SendEmployeeMessage(crate::SendEmployeeMessageCommand),
    /// Explicit management resolution, never fabricated Employee acknowledgement.
    WaiveMessageRequirement(crate::WaiveMessageRequirementCommand),
    /// Explicit Project reboot behavior.
    ConfigureBootRecoveryPolicy {
        policy: forge_domain::runtime::BootRecoveryPolicy,
    },
    /// Human acceptance of a Run-local candidate assessment.
    AcceptRunRecoveryAssessment {
        run_id: Uuid,
        assessment: forge_domain::runtime::RecoveryAssessment,
    },
    /// Explicitly retry a held conversation after physical quiescence.
    RetryCommunication {
        run_id: Uuid,
        reason: String,
    },
    RetryGitIntegration {
        operation_id: Uuid,
        expected_task_revision: u64,
        reason: String,
    },
    AcceptGitIntegrationResult {
        operation_id: Uuid,
        expected_task_revision: u64,
        reason: String,
    },
    /// Local management enrollment: payload carries a filename, never key bytes.
    EnrollCredential {
        secret_id: Uuid,
        binding_id: Uuid,
        kind: String,
        source_file: String,
    },
    /// Create the Project represented by the envelope's reserved identity.
    CreateProject(CreateProjectCommand),
    /// Replace the Project's complete Task-property schema before Task creation.
    ConfigureTaskPropertySchema(forge_domain::TaskPropertySchema),
    /// Create a Pipeline and initial immutable graph version.
    CreatePipeline(CreatePipelineCommand),
    /// Publish a complete immutable graph under an exact catalog revision.
    PublishPipelineVersion {
        pipeline_id: PipelineId,
        expected_pipeline_revision: u64,
        definition: crate::PipelineDefinitionInput,
        make_default: bool,
    },
    /// Change the default for future Tasks only.
    SetPipelineDefaultVersion {
        pipeline_id: PipelineId,
        expected_pipeline_revision: u64,
        pipeline_version_id: PipelineVersionId,
    },
    /// Preserve existing Tasks and versions while disallowing new bindings.
    DeletePipeline {
        pipeline_id: PipelineId,
        expected_pipeline_revision: u64,
    },
    /// Create an enabled Employee identity.
    CreateEmployee(CreateEmployeeCommand),
    /// Replace selected Employee catalog fields at an exact revision.
    AmendEmployee {
        employee_id: EmployeeId,
        expected_employee_revision: u64,
        patch: forge_domain::EmployeeAmendment,
    },
    /// Enable future assignment without changing active Runs.
    EnableEmployee {
        employee_id: EmployeeId,
        expected_employee_revision: u64,
        reason: Option<String>,
    },
    /// Disable future assignment without changing active Runs.
    DisableEmployee {
        employee_id: EmployeeId,
        expected_employee_revision: u64,
        reason: Option<String>,
    },
    /// Permanently exclude this Employee from new assignments.
    RetireEmployee {
        employee_id: EmployeeId,
        expected_employee_revision: u64,
        reason: Option<String>,
    },
    /// Create a Task in draft state.
    CreateTask(CreateTaskCommand),
    /// Replace selected draft Task intent fields.
    AmendDraft {
        /// Target Task.
        task_id: TaskId,
        /// Revision expected by the Task-level command.
        expected_task_revision: u64,
        /// Fields to replace.
        patch: DraftTaskPatch,
    },
    /// Approve a draft Task for its entry stage.
    ApproveTask {
        /// Target Task.
        task_id: TaskId,
        /// Revision expected by the Task-level command.
        expected_task_revision: u64,
    },
    /// Cancel a mutable Task through a stable catalog reason.
    CancelTask {
        /// Target Task.
        task_id: TaskId,
        /// Revision expected by the Task-level command.
        expected_task_revision: u64,
        /// Stable cancellation catalog key.
        reason_id: CancellationReasonId,
        /// Optional explanatory note.
        note: Option<String>,
    },
    /// Resolve one named typed wait condition.
    ResumeTask {
        /// Target Task.
        task_id: TaskId,
        /// Revision expected by the Task-level command.
        expected_task_revision: u64,
        /// Exact active condition to resolve.
        wait_condition_id: WaitConditionId,
    },
    /// Change a mutable Task's Project-defined priority level.
    SetTaskPriority {
        /// Target Task.
        task_id: TaskId,
        /// Revision expected by the Task-level command.
        expected_task_revision: u64,
        /// New active priority key.
        priority_level_id: PriorityLevelId,
    },
    /// Add a `done` dependency from blocker to blocked Task.
    CreateDependency {
        /// Task that must complete first.
        blocker_task_id: TaskId,
        /// Task that cannot start until the blocker completes.
        blocked_task_id: TaskId,
    },
    /// Remove one directed dependency by its stable endpoint pair.
    RemoveDependency {
        /// Task that previously had to complete first.
        blocker_task_id: TaskId,
        /// Task that was previously gated by the blocker.
        blocked_task_id: TaskId,
    },
    /// Open Project dispatch for eligible queued work.
    StartProjectExecution {
        /// Optional audit context.
        reason: Option<String>,
    },
    /// Stop new Project dispatch while retaining durable history.
    StopProjectExecution {
        /// Optional audit context.
        reason: Option<String>,
    },
    /// Submit a human/external outcome to the current waiting Pipeline stage.
    SubmitExternalStageOutcome(ExternalStageOutcomeCommand),
    /// Explicit runtime selection; no secrets are accepted on this endpoint.
    ConfigureEmployeeRuntime {
        /// Project-owned employee to configure.
        employee_id: EmployeeId,
        /// Profile and sandbox defaults pinned by future Runs.
        binding: Box<forge_domain::runtime::RuntimeBinding>,
    },
}

/// A parsed named command ready for Core authorization and transactional apply.
#[derive(Clone, Debug)]
pub struct CommandEnvelope {
    /// Request-path command name.
    pub name: CommandName,
    /// Project command scope.
    pub project_id: ProjectId,
    /// Project revision expected by the caller.
    pub expected_project_revision: u64,
    /// Caller retry identity.
    pub idempotency_key: IdempotencyKey,
    /// Typed command-specific intent.
    pub payload: CommandPayload,
    /// Original syntactically validated JSON object retained so Core can derive
    /// its own idempotency fingerprint. Callers never provide a digest.
    canonical_payload: Value,
}

impl CommandEnvelope {
    /// Rejects mutations to the public typed intent that would evade its original JSON hash.
    pub(crate) fn validate_integrity(&self) -> Result<(), ApplicationError> {
        let expected = CommandPayload::parse(self.name, self.canonical_payload.clone())?;
        if self.payload != expected {
            return Err(ApplicationError::InvalidPayload {
                command: self.name,
                reason: "typed payload does not match the parsed request".to_owned(),
            });
        }
        if self.name == CommandName::CreateProject && self.expected_project_revision != 0 {
            return Err(ApplicationError::InvalidPayload {
                command: self.name,
                reason: "create_project requires expected_revision to be zero".to_owned(),
            });
        }
        Ok(())
    }

    /// Parses a public wire request using the command selected by the path.
    ///
    /// # Errors
    ///
    /// Rejects a malformed Project id, idempotency key, or payload that does
    /// not exactly match the selected named command.
    pub fn parse(
        name: CommandName,
        request: CommandRequest,
        idempotency_key: impl Into<String>,
    ) -> Result<Self, ApplicationError> {
        if name == CommandName::CreateProject && request.expected_revision != 0 {
            return Err(ApplicationError::InvalidPayload {
                command: name,
                reason: "create_project requires expected_revision to be zero".to_owned(),
            });
        }
        let project_id = parse_uuid_v7("project_id", &request.project_id)?;
        let idempotency_key = IdempotencyKey::new(idempotency_key)?;
        let canonical_payload = Value::Object(request.payload);
        let payload = CommandPayload::parse(name, canonical_payload.clone())?;
        if let CommandPayload::ConfigureSystemJobs(input) = &payload {
            input.validate_project(project_id).map_err(|error| {
                ApplicationError::InvalidPayload {
                    command: name,
                    reason: error.to_string(),
                }
            })?;
        }
        Ok(Self {
            name,
            project_id,
            expected_project_revision: request.expected_revision,
            idempotency_key,
            payload,
            canonical_payload,
        })
    }

    /// Returns the exact typed-request JSON that Core fingerprints together
    /// with the command route, project revision, and authenticated actor.
    #[must_use]
    pub fn canonical_payload(&self) -> &Value {
        &self.canonical_payload
    }

    /// Returns a referenced Pipeline version when this command creates a Task.
    #[must_use]
    pub fn task_pipeline_version_id(&self) -> Option<PipelineVersionId> {
        match &self.payload {
            CommandPayload::CreateTask(command) => command.pipeline_version_id().ok(),
            _ => None,
        }
    }
}

impl CommandPayload {
    fn parse(name: CommandName, value: Value) -> Result<Self, ApplicationError> {
        match name {
            CommandName::ConfigureSystemJobs
            | CommandName::RequestTaskSummary
            | CommandName::RequestEmployeeOnboarding
            | CommandName::RetrySystemJob
            | CommandName::SkipEmployeeOnboarding => system_job_input::parse(name, value),
            CommandName::AuthorKnowledgePage
            | CommandName::PublishKnowledgePage
            | CommandName::SupersedeKnowledgePage
            | CommandName::WithdrawKnowledgePage => knowledge_input::parse(name, value),
            CommandName::ImportTaskFileSnapshot
            | CommandName::CaptureTaskFileSnapshot
            | CommandName::AttachTaskFileInput => file_snapshot_input::parse(name, value),
            CommandName::ReportFinding => parse_typed(name, value).map(Self::ReportFinding),
            CommandName::TriageFinding => parse_typed(name, value).map(Self::TriageFinding),
            CommandName::PromoteFinding => parse_typed(name, value).map(Self::PromoteFinding),
            CommandName::StopEmployee
            | CommandName::PauseTask
            | CommandName::SetNextRunEmployee
            | CommandName::ClearNextRunEmployee => manager_input::parse(name, value),
            CommandName::ConfigureResolverRoute
            | CommandName::RaiseEscalation
            | CommandName::SubmitHumanResolution
            | CommandName::RerouteEscalation => resolver_input::parse(name, value),
            CommandName::ScheduleTaskResume | CommandName::CancelTaskResume => {
                resume_input::parse(name, value)
            }
            CommandName::RegisterProjectRepository
            | CommandName::BindTaskGitRepository
            | CommandName::SetTaskGitSourcePolicy => repository_input::parse(name, value),
            CommandName::OpenEmployeeThread => {
                parse_typed(name, value).map(Self::OpenEmployeeThread)
            }
            CommandName::SendEmployeeMessage => {
                parse_typed(name, value).map(Self::SendEmployeeMessage)
            }
            CommandName::WaiveMessageRequirement => {
                parse_typed(name, value).map(Self::WaiveMessageRequirement)
            }
            CommandName::ConfigureBootRecoveryPolicy => {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Input {
                    policy: forge_domain::runtime::BootRecoveryPolicy,
                }
                let input: Input = parse_typed(name, value)?;
                Ok(Self::ConfigureBootRecoveryPolicy {
                    policy: input.policy,
                })
            }
            CommandName::RetryGitIntegration | CommandName::AcceptGitIntegrationResult => {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Input {
                    operation_id: Uuid,
                    expected_task_revision: u64,
                    reason: String,
                }
                let input: Input = parse_typed(name, value)?;
                if input.operation_id.get_version_num() != 7
                    || input.expected_task_revision == 0
                    || input.reason.trim().is_empty()
                    || input.reason.len() > 4096
                {
                    return Err(ApplicationError::InvalidPayload {command:name,reason:"requires operation UUIDv7, current Task revision and a bounded nonblank reason".into()});
                }
                if name == CommandName::RetryGitIntegration {
                    Ok(Self::RetryGitIntegration {
                        operation_id: input.operation_id,
                        expected_task_revision: input.expected_task_revision,
                        reason: input.reason,
                    })
                } else {
                    Ok(Self::AcceptGitIntegrationResult {
                        operation_id: input.operation_id,
                        expected_task_revision: input.expected_task_revision,
                        reason: input.reason,
                    })
                }
            }
            CommandName::RetryCommunication => {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Input {
                    run_id: Uuid,
                    reason: String,
                }
                let input: Input = parse_typed(name, value)?;
                if input.run_id.get_version_num() != 7
                    || input.reason.trim().is_empty()
                    || input.reason.len() > 4096
                {
                    return Err(ApplicationError::InvalidPayload {
                        command: name,
                        reason: "requires Run UUIDv7 and bounded nonblank retry reason".into(),
                    });
                }
                Ok(Self::RetryCommunication {
                    run_id: input.run_id,
                    reason: input.reason,
                })
            }
            CommandName::AcceptRunRecoveryAssessment => {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Input {
                    run_id: Uuid,
                    assessment: forge_domain::runtime::RecoveryAssessment,
                }
                let input: Input = parse_typed(name, value)?;
                if input.run_id.get_version_num() != 7 {
                    return Err(ApplicationError::InvalidPayload {
                        command: name,
                        reason: "run_id must be UUIDv7".into(),
                    });
                }
                Ok(Self::AcceptRunRecoveryAssessment {
                    run_id: input.run_id,
                    assessment: input.assessment,
                })
            }
            CommandName::EnrollCredential => {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Input {
                    secret_id: Uuid,
                    binding_id: Uuid,
                    kind: String,
                    source_file: String,
                }
                let input: Input = parse_typed(name, value)?;
                if input.secret_id.is_nil()
                    || input.binding_id.is_nil()
                    || !matches!(
                        input.kind.as_str(),
                        "codex_chatgpt" | "api_key" | "claude_subscription"
                    )
                    || !input.source_file.starts_with('/')
                    || input.source_file.contains('\0')
                    || input.source_file.len() > 4096
                {
                    return Err(ApplicationError::InvalidPayload {
                        command: name,
                        reason: "invalid private-file credential enrollment".into(),
                    });
                }
                Ok(Self::EnrollCredential {
                    secret_id: input.secret_id,
                    binding_id: input.binding_id,
                    kind: input.kind,
                    source_file: input.source_file,
                })
            }
            CommandName::ConfigureEmployeeRuntime => {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Input {
                    employee_id: String,
                    binding: forge_domain::runtime::RuntimeBinding,
                }
                let input: Input = parse_typed(name, value)?;
                input
                    .binding
                    .validate()
                    .map_err(|error| ApplicationError::InvalidPayload {
                        command: name,
                        reason: error.to_string(),
                    })?;
                Ok(Self::ConfigureEmployeeRuntime {
                    employee_id: parse_uuid_v7("employee_id", &input.employee_id)?,
                    binding: Box::new(input.binding),
                })
            }
            CommandName::ConfigureProjectHook => {
                parse_typed(name, value).map(|input| Self::ConfigureProjectHook(Box::new(input)))
            }
            CommandName::CreateProject => parse_typed(name, value).map(Self::CreateProject),
            CommandName::ConfigureTaskPropertySchema => {
                parse_typed::<ConfigureTaskPropertySchemaInput>(name, value).and_then(|input| {
                    input.schema.validate_for_configuration().map_err(|error| {
                        ApplicationError::InvalidPayload {
                            command: name,
                            reason: error.to_string(),
                        }
                    })?;
                    Ok(Self::ConfigureTaskPropertySchema(input.schema))
                })
            }
            CommandName::CreatePipeline => parse_typed(name, value).map(Self::CreatePipeline),
            CommandName::CreateEmployee => parse_typed(name, value).map(Self::CreateEmployee),
            CommandName::AmendEmployee
            | CommandName::EnableEmployee
            | CommandName::DisableEmployee
            | CommandName::RetireEmployee => employee_input::parse(name, value),
            CommandName::PublishPipelineVersion
            | CommandName::SetPipelineDefaultVersion
            | CommandName::DeletePipeline => pipeline_management_input::parse(name, value),
            CommandName::CreateTask => {
                parse_typed::<CreateTaskCommand>(name, value).and_then(|input| {
                    input.validate_public_contract()?;
                    Ok(Self::CreateTask(input))
                })
            }
            CommandName::AmendDraft => {
                parse_typed::<AmendDraftInput>(name, value).and_then(|input| {
                    input.patch.validate_nonempty()?;
                    input.patch.validate_public_contract()?;
                    Ok(Self::AmendDraft {
                        task_id: parse_uuid_v7("task_id", &input.task_id)?,
                        expected_task_revision: positive_revision(
                            name,
                            input.expected_task_revision,
                        )?,
                        patch: input.patch,
                    })
                })
            }
            CommandName::ApproveTask => {
                parse_typed::<TaskRevisionInput>(name, value).and_then(|input| {
                    Ok(Self::ApproveTask {
                        task_id: parse_uuid_v7("task_id", &input.task_id)?,
                        expected_task_revision: positive_revision(
                            name,
                            input.expected_task_revision,
                        )?,
                    })
                })
            }
            CommandName::CancelTask => {
                parse_typed::<CancelTaskInput>(name, value).and_then(|input| {
                    Ok(Self::CancelTask {
                        task_id: parse_uuid_v7("task_id", &input.task_id)?,
                        expected_task_revision: positive_revision(
                            name,
                            input.expected_task_revision,
                        )?,
                        reason_id: CancellationReasonId::new(input.cancellation_reason_key)
                            .map_err(|error| ApplicationError::InvalidIdentifier {
                                field: "cancellation_reason_key",
                                reason: error.to_string(),
                            })?,
                        note: bounded_optional_audit_detail(name, "note", input.note)?,
                    })
                })
            }
            CommandName::ResumeTask => {
                parse_typed::<ResumeTaskInput>(name, value).and_then(|input| {
                    Ok(Self::ResumeTask {
                        task_id: parse_uuid_v7("task_id", &input.task_id)?,
                        expected_task_revision: positive_revision(
                            name,
                            input.expected_task_revision,
                        )?,
                        wait_condition_id: parse_uuid_v7(
                            "wait_condition_id",
                            &input.wait_condition_id,
                        )?,
                    })
                })
            }
            CommandName::SetTaskPriority => {
                parse_typed::<SetPriorityInput>(name, value).and_then(|input| {
                    Ok(Self::SetTaskPriority {
                        task_id: parse_uuid_v7("task_id", &input.task_id)?,
                        expected_task_revision: positive_revision(
                            name,
                            input.expected_task_revision,
                        )?,
                        priority_level_id: PriorityLevelId::new(input.priority).map_err(
                            |error| ApplicationError::InvalidIdentifier {
                                field: "priority",
                                reason: error.to_string(),
                            },
                        )?,
                    })
                })
            }
            CommandName::CreateDependency => parse_typed::<CreateDependencyInput>(name, value)
                .and_then(|input| {
                    if input.required_condition != "task_done" {
                        return Err(ApplicationError::InvalidPayload {
                            command: name,
                            reason: "required_condition must be task_done in M0".to_owned(),
                        });
                    }
                    Ok(Self::CreateDependency {
                        blocker_task_id: parse_uuid_v7("blocker_task_id", &input.blocker_task_id)?,
                        blocked_task_id: parse_uuid_v7("blocked_task_id", &input.blocked_task_id)?,
                    })
                }),
            CommandName::RemoveDependency => parse_typed::<RemoveDependencyInput>(name, value)
                .and_then(|input| {
                    Ok(Self::RemoveDependency {
                        blocker_task_id: parse_uuid_v7("blocker_task_id", &input.blocker_task_id)?,
                        blocked_task_id: parse_uuid_v7("blocked_task_id", &input.blocked_task_id)?,
                    })
                }),
            CommandName::StartProjectExecution => parse_typed::<ProjectExecutionInput>(name, value)
                .and_then(|input| {
                    Ok(Self::StartProjectExecution {
                        reason: bounded_optional_audit_detail(name, "reason", input.reason)?,
                    })
                }),
            CommandName::StopProjectExecution => parse_typed::<ProjectExecutionInput>(name, value)
                .and_then(|input| {
                    Ok(Self::StopProjectExecution {
                        reason: bounded_optional_audit_detail(name, "reason", input.reason)?,
                    })
                }),
            CommandName::SubmitExternalStageOutcome => {
                parse_typed::<ExternalStageOutcomeCommand>(name, value).and_then(|input| {
                    let _ = positive_revision(name, input.expected_task_revision)?;
                    let _ = input.identifiers()?;
                    let _ = input.stage_id()?;
                    let _ = input.outcome()?;
                    let _ = input.cancellation_reason_id()?;
                    let _ = input.outcome_artifact_ids()?;
                    input.validate_public_contract()?;
                    Ok(Self::SubmitExternalStageOutcome(input))
                })
            }
        }
    }
}

fn parse_typed<T>(name: CommandName, value: Value) -> Result<T, ApplicationError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(value).map_err(|error| ApplicationError::InvalidPayload {
        command: name,
        reason: error.to_string(),
    })
}

pub(crate) trait UuidV7Identifier: From<Uuid> {}
impl UuidV7Identifier for Uuid {}

impl UuidV7Identifier for ArtifactId {}
impl UuidV7Identifier for CommandId {}
impl UuidV7Identifier for EmployeeId {}
impl UuidV7Identifier for EventId {}
impl UuidV7Identifier for PipelineId {}
impl UuidV7Identifier for PipelineVersionId {}
impl UuidV7Identifier for ProjectId {}
impl UuidV7Identifier for TaskId {}
impl UuidV7Identifier for WaitConditionId {}

/// Parses a public UUID identity and rejects any non-v7 value.
pub(crate) fn parse_uuid_v7<T>(field: &'static str, value: &str) -> Result<T, ApplicationError>
where
    T: UuidV7Identifier,
{
    let identifier =
        Uuid::parse_str(value).map_err(|error| ApplicationError::InvalidIdentifier {
            field,
            reason: error.to_string(),
        })?;
    if identifier.get_version_num() != 7 {
        return Err(ApplicationError::InvalidIdentifier {
            field,
            reason: "must be a UUIDv7".to_owned(),
        });
    }
    Ok(T::from(identifier))
}

fn positive_revision(name: CommandName, revision: u64) -> Result<u64, ApplicationError> {
    if revision == 0 {
        return Err(ApplicationError::InvalidPayload {
            command: name,
            reason: "expected_task_revision must be greater than zero".to_owned(),
        });
    }
    Ok(revision)
}

fn bounded_optional_audit_detail(
    command: CommandName,
    field: &'static str,
    value: Option<String>,
) -> Result<Option<String>, ApplicationError> {
    if value
        .as_ref()
        .is_some_and(|detail| detail.chars().count() > MAX_COMMAND_AUDIT_DETAIL_CHARACTERS)
    {
        return Err(ApplicationError::InvalidPayload {
            command,
            reason: format!(
                "{field} must contain at most {MAX_COMMAND_AUDIT_DETAIL_CHARACTERS} characters"
            ),
        });
    }
    Ok(value)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AmendDraftInput {
    task_id: String,
    expected_task_revision: u64,
    patch: DraftTaskPatch,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskRevisionInput {
    task_id: String,
    expected_task_revision: u64,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CancelTaskInput {
    task_id: String,
    expected_task_revision: u64,
    cancellation_reason_key: String,
    #[serde(default)]
    note: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ResumeTaskInput {
    task_id: String,
    expected_task_revision: u64,
    wait_condition_id: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SetPriorityInput {
    task_id: String,
    expected_task_revision: u64,
    priority: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateDependencyInput {
    blocker_task_id: String,
    blocked_task_id: String,
    required_condition: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveDependencyInput {
    blocker_task_id: String,
    blocked_task_id: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectExecutionInput {
    #[serde(default)]
    reason: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigureTaskPropertySchemaInput {
    schema: forge_domain::TaskPropertySchema,
}

#[cfg(test)]
#[path = "command/tests.rs"]
mod tests;

#[path = "command/employee_input.rs"]
mod employee_input;
mod file_snapshot_input;
mod knowledge_input;
mod manager_input;
mod pipeline_management_input;
mod repository_input;
pub(crate) mod resolver_input;
mod resume_input;
mod system_job_input;
