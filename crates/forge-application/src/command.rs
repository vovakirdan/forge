use forge_domain::{
    ArtifactId, CancellationReasonId, CommandId, EmployeeId, EventId, PipelineId,
    PipelineVersionId, PriorityLevelId, ProjectId, TaskId, WaitConditionId,
};
use forge_protocol::wire::{CommandName, CommandRequest, MAX_COMMAND_AUDIT_DETAIL_CHARACTERS};
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
#[derive(Clone, Debug)]
pub enum CommandPayload {
    /// Explicit Project reboot behavior.
    ConfigureBootRecoveryPolicy {
        policy: forge_domain::runtime::BootRecoveryPolicy,
    },
    /// Human acceptance of a Run-local candidate assessment.
    AcceptRunRecoveryAssessment {
        run_id: Uuid,
        assessment: forge_domain::runtime::RecoveryAssessment,
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
    /// Create a Pipeline and initial immutable graph version.
    CreatePipeline(CreatePipelineCommand),
    /// Create an enabled Employee identity.
    CreateEmployee(CreateEmployeeCommand),
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
                    || !matches!(input.kind.as_str(), "codex_chatgpt" | "api_key")
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
            CommandName::CreateProject => parse_typed(name, value).map(Self::CreateProject),
            CommandName::CreatePipeline => parse_typed(name, value).map(Self::CreatePipeline),
            CommandName::CreateEmployee => parse_typed(name, value).map(Self::CreateEmployee),
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

#[cfg(test)]
#[path = "command/tests.rs"]
mod tests;
