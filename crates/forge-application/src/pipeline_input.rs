use std::collections::{BTreeMap, BTreeSet};

use forge_domain::{
    Actor, ArtifactRequirement, ArtifactRequirementScope, ExecutorKind, Pipeline, PipelineId,
    PipelineStage, PipelineTransition, PipelineTransitionTarget, PipelineVersion,
    PipelineVersionId, PipelineVersionInput, ProjectId, StageId, TaskKind, Timestamp,
};
use forge_protocol::wire::CommandName;
use serde::Deserialize;

use crate::{
    ApplicationError,
    payload::{artifact_kind, bounded_len, domain_error, outcome_key, stage_id},
};

const MAX_PIPELINE_STAGES: usize = 128;
const MAX_PIPELINE_TRANSITIONS: usize = 1_024;
const MAX_STAGE_OUTCOMES: usize = 64;
const MAX_ARTIFACT_REQUIREMENTS: usize = 32;
const MAX_STAGE_VISITS: u32 = 1_000_000;

/// Input graph used to create a named Pipeline and its first immutable version.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CreatePipelineCommand {
    /// Human-readable Pipeline name.
    pub name: String,
    /// Supported Task categories.
    pub task_kinds: Vec<TaskKind>,
    /// Stable entry-stage identity.
    pub entry_stage_id: String,
    /// Finite total stage-entry cap required for a graph with retry cycles.
    #[serde(default)]
    pub max_stage_visits: Option<u32>,
    /// Stage definitions, with outcome declarations.
    pub stages: Vec<PipelineStageInput>,
    /// Complete directed outcome graph.
    pub transitions: Vec<PipelineTransitionInput>,
}

impl CreatePipelineCommand {
    /// Builds the initial immutable graph and its named catalog entry.
    pub fn build(
        &self,
        pipeline_id: PipelineId,
        version_id: PipelineVersionId,
        project_id: ProjectId,
        created_by: Actor,
        created_at: Timestamp,
    ) -> Result<(Pipeline, PipelineVersion), ApplicationError> {
        self.validate_public_contract()?;
        let mut transitions_by_stage = BTreeMap::<StageId, Vec<PipelineTransition>>::new();
        for input in &self.transitions {
            transitions_by_stage
                .entry(input.source_stage_id()?)
                .or_default()
                .push(input.to_domain()?);
        }
        let mut stages = Vec::with_capacity(self.stages.len());
        for input in &self.stages {
            let stage_id = input.stage_id()?;
            let transitions = transitions_by_stage.remove(&stage_id).unwrap_or_default();
            stages.push(input.to_domain(transitions)?);
        }
        if !transitions_by_stage.is_empty() {
            return Err(ApplicationError::InvalidIdentifier {
                field: "transitions.from_stage_id",
                reason: "references a stage not listed in stages".to_owned(),
            });
        }
        let version = PipelineVersion::new(PipelineVersionInput {
            id: version_id,
            pipeline_id,
            project_id,
            version: 1,
            task_kinds: unique_set(CommandName::CreatePipeline, "task_kinds", &self.task_kinds)?,
            entry_stage_id: stage_id("entry_stage_id", &self.entry_stage_id)?,
            max_stage_visits: self.max_stage_visits,
            stages,
            created_by,
            created_at,
        })
        .map_err(domain_error)?;
        let pipeline = Pipeline::new(pipeline_id, project_id, self.name.clone(), version_id)
            .map_err(domain_error)?;
        Ok((pipeline, version))
    }

    fn validate_public_contract(&self) -> Result<(), ApplicationError> {
        bounded_len(
            CommandName::CreatePipeline,
            "task_kinds",
            self.task_kinds.len(),
            1,
            2,
        )?;
        let _ = unique_set(CommandName::CreatePipeline, "task_kinds", &self.task_kinds)?;
        bounded_len(
            CommandName::CreatePipeline,
            "stages",
            self.stages.len(),
            1,
            MAX_PIPELINE_STAGES,
        )?;
        bounded_len(
            CommandName::CreatePipeline,
            "transitions",
            self.transitions.len(),
            0,
            MAX_PIPELINE_TRANSITIONS,
        )?;
        if let Some(maximum) = self.max_stage_visits
            && !(1..=MAX_STAGE_VISITS).contains(&maximum)
        {
            return Err(ApplicationError::InvalidPayload {
                command: CommandName::CreatePipeline,
                reason: format!(
                    "max_stage_visits must be between 1 and {MAX_STAGE_VISITS} when present"
                ),
            });
        }
        for stage in &self.stages {
            stage.validate_public_contract()?;
        }
        for transition in &self.transitions {
            transition.validate_public_contract()?;
        }
        Ok(())
    }
}

/// One stage's public graph input.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PipelineStageInput {
    /// Stable stage key.
    pub id: String,
    /// Human-readable stage name.
    pub name: String,
    /// Actor category responsible for the stage.
    pub executor_kind: ExecutorKind,
    /// Complete declared outcome set for cross-checking edge input.
    pub outcomes: Vec<String>,
}

impl PipelineStageInput {
    fn validate_public_contract(&self) -> Result<(), ApplicationError> {
        bounded_len(
            CommandName::CreatePipeline,
            "stages.outcomes",
            self.outcomes.len(),
            1,
            MAX_STAGE_OUTCOMES,
        )?;
        let _ = unique_set(
            CommandName::CreatePipeline,
            "stages.outcomes",
            &self.outcomes,
        )?;
        Ok(())
    }

    fn stage_id(&self) -> Result<StageId, ApplicationError> {
        stage_id("stages.id", &self.id)
    }

    fn to_domain(
        &self,
        transitions: Vec<PipelineTransition>,
    ) -> Result<PipelineStage, ApplicationError> {
        let actual = transitions
            .iter()
            .map(|transition| transition.outcome().as_str().to_owned())
            .collect::<BTreeSet<_>>();
        if actual
            != unique_set(
                CommandName::CreatePipeline,
                "stages.outcomes",
                &self.outcomes,
            )?
        {
            return Err(ApplicationError::InvalidPayload {
                command: CommandName::CreatePipeline,
                reason: format!(
                    "stage {} outcomes must exactly match transition outcomes",
                    self.id
                ),
            });
        }
        PipelineStage::new(
            self.stage_id()?,
            self.name.clone(),
            self.executor_kind,
            transitions,
        )
        .map_err(domain_error)
    }
}

/// Directed edge input for a Pipeline stage outcome.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PipelineTransitionInput {
    /// Source stage.
    pub from_stage_id: String,
    /// Declared source outcome.
    pub outcome: String,
    /// Target stage or terminal effect.
    pub target: PipelineTargetInput,
    /// Required evidence for this outcome; empty by default in M0.
    #[serde(default)]
    pub artifact_requirements: Vec<ArtifactRequirementInput>,
}

impl PipelineTransitionInput {
    fn validate_public_contract(&self) -> Result<(), ApplicationError> {
        bounded_len(
            CommandName::CreatePipeline,
            "transitions.artifact_requirements",
            self.artifact_requirements.len(),
            0,
            MAX_ARTIFACT_REQUIREMENTS,
        )
    }

    fn source_stage_id(&self) -> Result<StageId, ApplicationError> {
        stage_id("transitions.from_stage_id", &self.from_stage_id)
    }

    fn to_domain(&self) -> Result<PipelineTransition, ApplicationError> {
        let requirements = self
            .artifact_requirements
            .iter()
            .map(ArtifactRequirementInput::to_domain)
            .collect::<Result<Vec<_>, _>>()?;
        PipelineTransition::new(
            outcome_key("transitions.outcome", &self.outcome)?,
            self.target.to_domain()?,
            requirements,
        )
        .map_err(domain_error)
    }
}

/// Wire representation of an explicit Pipeline transition destination.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PipelineTargetInput {
    /// Continue at another stage.
    Stage { stage_id: String },
    /// Complete the Task.
    Done,
    /// Cancel the Task.
    Cancelled,
}

impl PipelineTargetInput {
    fn to_domain(&self) -> Result<PipelineTransitionTarget, ApplicationError> {
        match self {
            Self::Stage { stage_id: value } => Ok(PipelineTransitionTarget::Stage(stage_id(
                "transitions.target.stage_id",
                value,
            )?)),
            Self::Done => Ok(PipelineTransitionTarget::Done),
            Self::Cancelled => Ok(PipelineTransitionTarget::Cancelled),
        }
    }
}

/// Structural evidence requirement declared by one Pipeline transition.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRequirementInput {
    /// Required Artifact kind.
    pub kind: String,
    /// Positive number of matching artifacts required.
    pub minimum_count: u16,
    /// Explicit visibility of evidence required by the outcome.
    #[serde(default)]
    pub scope: ArtifactRequirementScope,
}

impl ArtifactRequirementInput {
    fn to_domain(&self) -> Result<ArtifactRequirement, ApplicationError> {
        ArtifactRequirement::new_with_scope(
            artifact_kind("artifact_requirements.kind", &self.kind)?,
            self.minimum_count,
            self.scope,
        )
        .map_err(domain_error)
    }
}

fn unique_set<T>(
    command: CommandName,
    field: &'static str,
    values: &[T],
) -> Result<BTreeSet<T>, ApplicationError>
where
    T: Clone + Ord,
{
    let unique = values.iter().cloned().collect::<BTreeSet<_>>();
    if unique.len() == values.len() {
        Ok(unique)
    } else {
        Err(ApplicationError::InvalidPayload {
            command,
            reason: format!("{field} must not contain duplicates"),
        })
    }
}
