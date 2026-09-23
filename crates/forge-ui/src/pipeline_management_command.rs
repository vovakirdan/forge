//! Exact owner commands for an existing Pipeline catalog entry.

use forge_protocol::wire::CommandReceipt;
use serde::Deserialize;

use crate::{
    command::{CommandTarget, uuid_v7},
    http::ApiError,
};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    project_id: String,
    expected_revision: u64,
    payload: serde_json::Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    pipeline_id: String,
    expected_pipeline_revision: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetDefault {
    pipeline_id: String,
    expected_pipeline_revision: u64,
    pipeline_version_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Publish {
    pipeline_id: String,
    expected_pipeline_revision: u64,
    definition: Definition,
    #[serde(default)]
    make_default: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Definition {
    task_kinds: Vec<TaskKind>,
    entry_stage_id: String,
    max_stage_visits: Option<u32>,
    stages: Vec<Stage>,
    transitions: Vec<Transition>,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum TaskKind {
    Delivery,
    Analysis,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ExecutorKind {
    Employee,
    Human,
    System,
    External,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stage {
    id: String,
    name: String,
    executor_kind: ExecutorKind,
    outcomes: Vec<String>,
    #[serde(default)]
    instructions: String,
    workspace: Option<Workspace>,
    acceptance_policy: Option<Acceptance>,
    system_action: Option<SystemAction>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Workspace {
    kind: WorkspaceKind,
    access: Access,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum WorkspaceKind {
    Any,
    Filesystem,
    Git,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Access {
    ReadOnly,
    ReadWrite,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Acceptance {
    CandidateReview {
        #[serde(default)]
        independent: bool,
        verdicts: std::collections::BTreeMap<String, Verdict>,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Verdict {
    Accepted,
    Rejected,
    Inconclusive,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum SystemAction {
    GitIntegration {
        outcomes: IntegrationOutcomes,
        #[serde(default)]
        required_review_stages: Vec<String>,
    },
    ProjectHook {
        hook_version_id: String,
        outcomes: HookOutcomes,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IntegrationOutcomes {
    applied: String,
    no_changes: String,
    stale_base: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HookOutcomes {
    passed: String,
    failed: String,
    timed_out: String,
    skipped: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Transition {
    from_stage_id: String,
    outcome: String,
    target: Target,
    #[serde(default)]
    artifact_requirements: Vec<ArtifactRequirement>,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Target {
    Stage { stage_id: String },
    Done,
    Cancelled,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactRequirement {
    kind: String,
    minimum_count: u16,
    #[serde(default)]
    scope: RequirementScope,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RequirementScope {
    #[default]
    CurrentStage,
    TaskHistory,
}

fn stable_key(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}
pub(crate) fn valid_definition(value: &Definition) -> bool {
    let _ = &value.task_kinds;
    (1..=2).contains(&value.task_kinds.len())
        && (1..=128).contains(&value.stages.len())
        && value.transitions.len() <= 1024
        && value
            .max_stage_visits
            .is_none_or(|count| (1..=1_000_000).contains(&count))
        && stable_key(&value.entry_stage_id)
        && value.stages.iter().all(|stage| {
            let _ = (&stage.executor_kind, &stage.instructions);
            let workspace_valid = stage.workspace.as_ref().is_none_or(|workspace| {
                let _ = (&workspace.kind, &workspace.access);
                true
            });
            workspace_valid
                && stable_key(&stage.id)
                && !stage.name.trim().is_empty()
                && stage.name.len() <= 128
                && (1..=64).contains(&stage.outcomes.len())
                && stage.outcomes.iter().all(|outcome| stable_key(outcome))
                && stage
                    .acceptance_policy
                    .as_ref()
                    .is_none_or(|policy| match policy {
                        Acceptance::CandidateReview {
                            independent,
                            verdicts,
                        } => {
                            let _ = independent;
                            verdicts.len() <= 64
                                && verdicts.iter().all(|(key, value)| {
                                    let _ = value;
                                    stable_key(key)
                                })
                        }
                    })
                && stage
                    .system_action
                    .as_ref()
                    .is_none_or(|action| match action {
                        SystemAction::GitIntegration {
                            outcomes,
                            required_review_stages,
                        } => {
                            stable_key(&outcomes.applied)
                                && stable_key(&outcomes.no_changes)
                                && stable_key(&outcomes.stale_base)
                                && required_review_stages.len() <= 64
                                && required_review_stages.iter().all(|key| stable_key(key))
                        }
                        SystemAction::ProjectHook {
                            hook_version_id,
                            outcomes,
                        } => {
                            uuid_v7(hook_version_id)
                                && stable_key(&outcomes.passed)
                                && stable_key(&outcomes.failed)
                                && stable_key(&outcomes.timed_out)
                                && stable_key(&outcomes.skipped)
                        }
                    })
        })
        && value.transitions.iter().all(|transition| {
            stable_key(&transition.from_stage_id)
                && stable_key(&transition.outcome)
                && match &transition.target {
                    Target::Stage { stage_id } => stable_key(stage_id),
                    Target::Done | Target::Cancelled => true,
                }
                && transition.artifact_requirements.len() <= 32
                && transition.artifact_requirements.iter().all(|requirement| {
                    let _ = &requirement.scope;
                    stable_key(&requirement.kind) && requirement.minimum_count > 0
                })
        })
}

pub(crate) struct PipelineManagementCommand {
    expected_revision: u64,
    pipeline_id: String,
    expected_kind: &'static str,
}
impl PipelineManagementCommand {
    pub(crate) fn parse(bytes: &[u8], target: CommandTarget) -> Result<Self, ApiError> {
        let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !uuid_v7(&envelope.project_id)
            || !(1..MAX_SAFE_INTEGER).contains(&envelope.expected_revision)
        {
            return Err(ApiError::BadRequest);
        }
        let (id, revision, expected_kind) = match target {
            CommandTarget::PublishPipelineVersion => {
                let input: Publish =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                let _ = input.make_default;
                if !valid_definition(&input.definition) {
                    return Err(ApiError::BadRequest);
                }
                (
                    input.pipeline_id,
                    input.expected_pipeline_revision,
                    "pipeline_version",
                )
            }
            CommandTarget::SetPipelineDefaultVersion => {
                let input: SetDefault =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                if !uuid_v7(&input.pipeline_version_id) {
                    return Err(ApiError::BadRequest);
                }
                (
                    input.pipeline_id,
                    input.expected_pipeline_revision,
                    "pipeline",
                )
            }
            CommandTarget::DeletePipeline => {
                let input: Identity =
                    serde_json::from_value(envelope.payload).map_err(|_| ApiError::BadRequest)?;
                (
                    input.pipeline_id,
                    input.expected_pipeline_revision,
                    "pipeline",
                )
            }
            _ => return Err(ApiError::BadRequest),
        };
        if !uuid_v7(&id) || !(1..MAX_SAFE_INTEGER).contains(&revision) {
            return Err(ApiError::BadRequest);
        }
        Ok(Self {
            expected_revision: envelope.expected_revision,
            pipeline_id: id,
            expected_kind,
        })
    }

    pub(crate) fn validate_receipt(&self, bytes: &[u8]) -> Result<(), ApiError> {
        let receipt: CommandReceipt =
            serde_json::from_slice(bytes).map_err(|_| ApiError::BadGateway)?;
        if !uuid_v7(&receipt.command_id)
            || receipt.project_revision != self.expected_revision + 1
            || receipt.event_ids.is_empty()
            || !receipt.event_ids.iter().all(|id| uuid_v7(id))
            || !receipt.resource.as_ref().is_some_and(|resource| {
                resource.kind == self.expected_kind
                    && uuid_v7(&resource.id)
                    && (self.expected_kind == "pipeline_version"
                        || resource.id.eq_ignore_ascii_case(&self.pipeline_id))
            })
        {
            return Err(ApiError::BadGateway);
        }
        Ok(())
    }
}
