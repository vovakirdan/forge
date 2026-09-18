use std::collections::BTreeSet;

use forge_domain::{
    Actor, ArtifactKind, DomainError, Employee, EmployeeId, EmployeeRole, NewTask, OutcomeKey,
    PipelineVersionId, PriorityLevelId, ProjectId, PropertyKey, PropertyValue, StageEligibility,
    StageEligibilityTarget, StageId, Task, TaskKind, TaskPipelineBinding, TaskProperties,
    TaskScope, TaskSource, TaskSpec, TaskSpecInput, Timestamp,
};
use forge_protocol::wire::CommandName;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{ApplicationError, command::parse_uuid_v7};

const MAX_STAGE_ELIGIBILITY_TARGETS: usize = 128;

/// Intent for a new Project using the envelope's caller-reserved identity.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CreateProjectCommand {
    /// Human-readable Project name.
    pub name: String,
}

/// Intent for a new enabled Employee identity.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CreateEmployeeCommand {
    /// Employee display name.
    pub name: String,
    /// Stable role label.
    pub role: String,
    /// Dispatch-stage allow list.
    pub stage_eligibility: StageEligibilityInput,
}

impl CreateEmployeeCommand {
    /// Builds one Employee with a generated identity supplied by Core.
    pub fn build(
        &self,
        id: EmployeeId,
        project_id: ProjectId,
        created_by: Actor,
        created_at: Timestamp,
    ) -> Result<Employee, ApplicationError> {
        Employee::new(
            id,
            project_id,
            self.name.clone(),
            EmployeeRole::new(self.role.clone()).map_err(domain_error)?,
            self.stage_eligibility.to_domain()?,
            created_by,
            created_at,
        )
        .map_err(domain_error)
    }
}

/// Public input for Employee stage eligibility.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum StageEligibilityInput {
    /// Eligible for every stage subject to later policy.
    Any,
    /// Eligible only for the listed Pipeline-version-scoped stages.
    Only {
        stages: Vec<StageEligibilityTargetInput>,
    },
}

impl StageEligibilityInput {
    fn to_domain(&self) -> Result<StageEligibility, ApplicationError> {
        match self {
            Self::Any => Ok(StageEligibility::Any),
            Self::Only { stages } => {
                bounded_len(
                    CommandName::CreateEmployee,
                    "stage_eligibility.stages",
                    stages.len(),
                    1,
                    MAX_STAGE_ELIGIBILITY_TARGETS,
                )?;
                let targets = stages
                    .iter()
                    .map(StageEligibilityTargetInput::to_domain)
                    .collect::<Result<Vec<_>, _>>()?;
                let unique = targets.iter().collect::<BTreeSet<_>>();
                if unique.len() != targets.len() {
                    return Err(ApplicationError::InvalidPayload {
                        command: CommandName::CreateEmployee,
                        reason: "stage_eligibility.stages must not contain duplicates".to_owned(),
                    });
                }
                StageEligibility::only(targets).map_err(domain_error)
            }
        }
    }
}

/// One public selector for a Pipeline-version-scoped Employee stage allow-list.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StageEligibilityTargetInput {
    /// Immutable Pipeline version that scopes the stage key.
    pub pipeline_version_id: String,
    /// Stable stage key inside that version.
    pub stage_id: String,
}

impl StageEligibilityTargetInput {
    fn to_domain(&self) -> Result<StageEligibilityTarget, ApplicationError> {
        Ok(StageEligibilityTarget::new(
            parse_uuid_v7(
                "stage_eligibility.pipeline_version_id",
                &self.pipeline_version_id,
            )?,
            stage_id("stage_eligibility.stage_id", &self.stage_id)?,
        ))
    }
}

/// Intent for one draft Task.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CreateTaskCommand {
    /// Concise title.
    pub title: String,
    /// Long description, blank by default.
    #[serde(default)]
    pub description: String,
    /// Optional Definition of Done required for later approval.
    #[serde(default)]
    pub definition_of_done: Option<String>,
    /// Bounded work category.
    pub kind: TaskKind,
    /// Immutable version to pin; mutually exclusive with `pipeline_id`.
    #[serde(default)]
    pub pipeline_version_id: Option<String>,
    /// Named Pipeline whose current default is resolved atomically at creation.
    #[serde(default)]
    pub pipeline_id: Option<String>,
    /// Project priority key.
    pub priority: String,
    /// Typed property values serialized in the domain's tagged representation.
    #[serde(default)]
    pub properties: Map<String, Value>,
}

impl CreateTaskCommand {
    /// Builds a draft after Core resolves the selected Pipeline catalog identity.
    pub fn build(&self, context: TaskDraftContext<'_>) -> Result<Task, ApplicationError> {
        self.build_with_source(context, TaskSource::Human)
    }

    pub(crate) fn build_with_source(
        &self,
        context: TaskDraftContext<'_>,
        source: TaskSource,
    ) -> Result<Task, ApplicationError> {
        self.validate_public_contract()?;
        if self.pipeline_version_id.is_some()
            && self.pipeline_version_id()? != context.pipeline.pipeline_version_id()
        {
            return Err(ApplicationError::InvalidPayload {
                command: CommandName::CreateTask,
                reason: "pipeline_version_id does not match the Core-resolved task binding"
                    .to_owned(),
            });
        }
        if self
            .selected_pipeline_id()?
            .is_some_and(|id| id != context.pipeline.pipeline_id())
        {
            return Err(ApplicationError::InvalidPayload {
                command: CommandName::CreateTask,
                reason: "pipeline_id does not match the Core-resolved task binding".into(),
            });
        }
        let spec = TaskSpec::new(TaskSpecInput {
            title: self.title.clone(),
            description: self.description.clone(),
            rationale: None,
            definition_of_done: self.definition_of_done.clone(),
            scope: TaskScope::default(),
            properties: task_properties(&self.properties, CommandName::CreateTask)?,
            labels: BTreeSet::new(),
        })
        .map_err(domain_error)?;
        let priority_level_id =
            PriorityLevelId::new(self.priority.clone()).map_err(domain_error)?;
        Task::new_draft(
            NewTask {
                id: context.id,
                project_id: context.project_id,
                key: context.key,
                kind: self.kind,
                spec,
                priority_level_id,
                pipeline: context.pipeline,
                source,
                created_by: context.created_by,
                created_at: context.created_at,
            },
            context.priority_scheme,
        )
        .map_err(domain_error)
    }

    /// Parses the requested immutable Pipeline version identity.
    pub fn pipeline_version_id(&self) -> Result<PipelineVersionId, ApplicationError> {
        let value = self.pipeline_version_id.as_deref().ok_or_else(|| {
            ApplicationError::InvalidPayload {
                command: CommandName::CreateTask,
                reason: "no explicit pipeline_version_id was selected".into(),
            }
        })?;
        parse_uuid_v7("pipeline_version_id", value)
    }

    /// Parses the named Pipeline selector, if supplied instead of an explicit version.
    pub fn selected_pipeline_id(
        &self,
    ) -> Result<Option<forge_domain::PipelineId>, ApplicationError> {
        self.pipeline_id
            .as_deref()
            .map(|value| parse_uuid_v7("pipeline_id", value))
            .transpose()
    }

    /// Validates public fields whose structure does not require Core context.
    ///
    /// # Errors
    ///
    /// Rejects a malformed pinned Pipeline identity or a Task property value
    /// that does not use Forge's tagged property representation.
    pub fn validate_public_contract(&self) -> Result<(), ApplicationError> {
        match (&self.pipeline_id, &self.pipeline_version_id) {
            (Some(_), None) => {
                let _ = self.selected_pipeline_id()?;
            }
            (None, Some(_)) => {
                let _ = self.pipeline_version_id()?;
            }
            _ => {
                return Err(ApplicationError::InvalidPayload {
                    command: CommandName::CreateTask,
                    reason: "exactly one of pipeline_id and pipeline_version_id is required".into(),
                });
            }
        }
        let _ = task_properties(&self.properties, CommandName::CreateTask)?;
        Ok(())
    }
}

/// Core-supplied identity and Project context for creating a draft Task.
pub struct TaskDraftContext<'a> {
    /// Fresh Task identity.
    pub id: forge_domain::TaskId,
    /// Owning Project.
    pub project_id: ProjectId,
    /// Allocated Project-local key.
    pub key: forge_domain::TaskKey,
    /// Resolved pinned Pipeline binding.
    pub pipeline: TaskPipelineBinding,
    /// Server-derived command actor.
    pub created_by: Actor,
    /// Authoritative command time.
    pub created_at: Timestamp,
    /// Active Project priority scheme.
    pub priority_scheme: &'a forge_domain::PriorityScheme,
}

/// Mutable draft fields selected by an amend command.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DraftTaskPatch {
    /// Replacement title.
    #[serde(default)]
    pub title: Option<String>,
    /// Replacement description.
    #[serde(default)]
    pub description: Option<String>,
    /// Replacement Definition of Done; null explicitly clears it.
    #[serde(default, deserialize_with = "present_nullable_text")]
    pub definition_of_done: Option<Option<String>>,
    /// Optional replacement priority applied by Core after the amended intent.
    #[serde(default)]
    pub priority: Option<String>,
    /// Optional replacement typed properties.
    #[serde(default)]
    pub properties: Option<Map<String, Value>>,
}

fn present_nullable_text<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error> {
    // Missing uses the field default; a present null is an explicit clearing edit.
    Option::<String>::deserialize(deserializer).map(Some)
}

impl DraftTaskPatch {
    /// Ensures an amend command includes at least one change.
    pub fn validate_nonempty(&self) -> Result<(), ApplicationError> {
        if self.title.is_none()
            && self.description.is_none()
            && self.definition_of_done.is_none()
            && self.priority.is_none()
            && self.properties.is_none()
        {
            return Err(ApplicationError::InvalidPayload {
                command: CommandName::AmendDraft,
                reason: "patch must contain at least one mutable field".to_owned(),
            });
        }
        Ok(())
    }

    /// Validates replacement fields that do not need the current Task.
    ///
    /// # Errors
    ///
    /// Rejects property values that do not use Forge's tagged property
    /// representation.
    pub fn validate_public_contract(&self) -> Result<(), ApplicationError> {
        if let Some(properties) = &self.properties {
            let _ = task_properties(properties, CommandName::AmendDraft)?;
        }
        Ok(())
    }

    /// Builds a replacement specification from current draft intent.
    pub fn apply_to(&self, current: &TaskSpec) -> Result<TaskSpec, ApplicationError> {
        let labels = current
            .labels()
            .map(ToOwned::to_owned)
            .collect::<BTreeSet<_>>();
        let properties = match &self.properties {
            Some(values) => task_properties(values, CommandName::AmendDraft)?,
            None => current.properties().clone(),
        };
        TaskSpec::new(TaskSpecInput {
            title: self
                .title
                .clone()
                .unwrap_or_else(|| current.title().to_owned()),
            description: self
                .description
                .clone()
                .unwrap_or_else(|| current.description().to_owned()),
            rationale: current.rationale().map(ToOwned::to_owned),
            definition_of_done: self
                .definition_of_done
                .clone()
                .unwrap_or_else(|| current.definition_of_done().map(ToOwned::to_owned)),
            scope: current.scope().clone(),
            properties,
            labels,
        })
        .map_err(domain_error)
    }
}

pub(crate) fn bounded_len(
    command: CommandName,
    field: &'static str,
    length: usize,
    minimum: usize,
    maximum: usize,
) -> Result<(), ApplicationError> {
    if (minimum..=maximum).contains(&length) {
        return Ok(());
    }
    Err(ApplicationError::InvalidPayload {
        command,
        reason: format!("{field} must contain between {minimum} and {maximum} item(s)"),
    })
}

fn task_properties(
    values: &Map<String, Value>,
    command: CommandName,
) -> Result<TaskProperties, ApplicationError> {
    let properties = values
        .iter()
        .map(|(key, value)| {
            let key = PropertyKey::new(key.clone()).map_err(|error| {
                ApplicationError::InvalidIdentifier {
                    field: "properties key",
                    reason: error.to_string(),
                }
            })?;
            let value =
                serde_json::from_value::<PropertyValue>(value.clone()).map_err(|error| {
                    ApplicationError::InvalidPayload {
                        command,
                        reason: format!("properties must use Forge typed property values: {error}"),
                    }
                })?;
            Ok((key, value))
        })
        .collect::<Result<Vec<_>, ApplicationError>>()?;
    TaskProperties::new(properties).map_err(|error| ApplicationError::InvalidPayload {
        command,
        reason: error.to_string(),
    })
}

pub(crate) fn stage_id(field: &'static str, value: &str) -> Result<StageId, ApplicationError> {
    StageId::new(value.to_owned()).map_err(|error| ApplicationError::InvalidIdentifier {
        field,
        reason: error.to_string(),
    })
}

pub(crate) fn outcome_key(
    field: &'static str,
    value: &str,
) -> Result<OutcomeKey, ApplicationError> {
    OutcomeKey::new(value.to_owned()).map_err(|error| ApplicationError::InvalidIdentifier {
        field,
        reason: error.to_string(),
    })
}

pub(crate) fn artifact_kind(
    field: &'static str,
    value: &str,
) -> Result<ArtifactKind, ApplicationError> {
    ArtifactKind::new(value.to_owned()).map_err(|error| ApplicationError::InvalidIdentifier {
        field,
        reason: error.to_string(),
    })
}

pub(crate) fn domain_error(error: DomainError) -> ApplicationError {
    ApplicationError::InvalidPayload {
        command: CommandName::CreatePipeline,
        reason: error.to_string(),
    }
}
