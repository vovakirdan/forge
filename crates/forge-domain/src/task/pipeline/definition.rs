use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{ArtifactKind, DomainError, StageId};

/// Stable key for one outcome declared by a Pipeline stage.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OutcomeKey(String);

impl OutcomeKey {
    /// Validates one lower-snake-case outcome key.
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        crate::ids::validate_stable_key("pipeline.outcome", &value)?;
        Ok(Self(value))
    }

    /// Returns the stable outcome key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn validate(&self) -> Result<(), DomainError> {
        Self::new(self.0.clone()).map(|_| ())
    }
}

/// Who performs a Pipeline stage.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorKind {
    /// A Forge Employee executes the stage through a Run.
    Employee,
    /// A human supplies the next outcome.
    Human,
    /// Core executes a deterministic action.
    System,
    /// An external system supplies the next outcome.
    External,
}

impl ExecutorKind {
    /// Returns whether entering this stage creates a typed Task wait.
    #[must_use]
    pub const fn requires_wait(self) -> bool {
        matches!(self, Self::Human | Self::External)
    }
}

/// Mechanically required evidence for one declared outcome.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactRequirement {
    kind: ArtifactKind,
    minimum_count: u16,
    #[serde(default)]
    scope: ArtifactRequirementScope,
}

/// Task evidence visibility used by an outcome contract.
///
/// A Pipeline must opt in to prior-stage evidence. This prevents a stale
/// artifact from a previous iteration from satisfying a requirement generated
/// by the current stage.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactRequirementScope {
    /// Only artifacts attached while the current stage was active qualify.
    #[default]
    CurrentStage,
    /// Any Artifact already attached to this Task may qualify.
    ///
    /// Use this deliberately for evidence such as an analysis result that a
    /// later human verdict must cite before terminal completion.
    TaskHistory,
}

impl ArtifactRequirement {
    /// Creates a positive-count current-stage evidence requirement.
    pub fn new(kind: ArtifactKind, minimum_count: u16) -> Result<Self, DomainError> {
        Self::new_with_scope(kind, minimum_count, ArtifactRequirementScope::CurrentStage)
    }

    /// Creates a positive-count evidence requirement with an explicit scope.
    pub fn new_with_scope(
        kind: ArtifactKind,
        minimum_count: u16,
        scope: ArtifactRequirementScope,
    ) -> Result<Self, DomainError> {
        if minimum_count == 0 {
            return Err(DomainError::InvalidPipeline {
                reason: "artifact requirement minimum_count must be greater than zero".to_owned(),
            });
        }
        Ok(Self {
            kind,
            minimum_count,
            scope,
        })
    }

    /// Returns the required Artifact kind.
    #[must_use]
    pub fn kind(&self) -> &ArtifactKind {
        &self.kind
    }

    /// Returns the number of matching evidence links required.
    #[must_use]
    pub const fn minimum_count(&self) -> u16 {
        self.minimum_count
    }

    /// Returns the artifact history scope selected by this contract.
    #[must_use]
    pub const fn scope(&self) -> ArtifactRequirementScope {
        self.scope
    }

    pub(crate) fn validate_snapshot(&self) -> Result<(), DomainError> {
        self.kind.validate()?;
        Self::new_with_scope(self.kind.clone(), self.minimum_count, self.scope).map(|_| ())
    }
}

/// Destination selected by one declared outcome.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineTransitionTarget {
    /// Continue at another named stage in this immutable version.
    Stage(StageId),
    /// Satisfy the Task's terminal-success contract.
    Done,
    /// Cancel the Task through its Project reason catalog.
    Cancelled,
}

/// One allowable outcome from a Pipeline stage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PipelineTransition {
    outcome: OutcomeKey,
    target: PipelineTransitionTarget,
    artifact_requirements: Vec<ArtifactRequirement>,
}

impl PipelineTransition {
    /// Creates one stage transition and its mechanical evidence contract.
    pub fn new(
        outcome: OutcomeKey,
        target: PipelineTransitionTarget,
        artifact_requirements: Vec<ArtifactRequirement>,
    ) -> Result<Self, DomainError> {
        let mut seen = BTreeSet::new();
        for requirement in &artifact_requirements {
            if !seen.insert((requirement.kind().clone(), requirement.scope())) {
                return Err(DomainError::InvalidPipeline {
                    reason: format!(
                        "outcome {} repeats {} artifact requirement {}",
                        outcome.as_str(),
                        requirement_scope_name(requirement.scope()),
                        requirement.kind().as_str()
                    ),
                });
            }
        }
        Ok(Self {
            outcome,
            target,
            artifact_requirements,
        })
    }

    /// Returns the declared outcome key.
    #[must_use]
    pub fn outcome(&self) -> &OutcomeKey {
        &self.outcome
    }

    /// Returns the destination selected by this outcome.
    #[must_use]
    pub fn target(&self) -> &PipelineTransitionTarget {
        &self.target
    }

    /// Returns required evidence in declaration order.
    #[must_use]
    pub fn artifact_requirements(&self) -> &[ArtifactRequirement] {
        &self.artifact_requirements
    }

    pub(crate) fn validate_snapshot(&self) -> Result<(), DomainError> {
        self.outcome.validate()?;
        if let PipelineTransitionTarget::Stage(stage_id) = &self.target {
            stage_id.validate()?;
        }
        for requirement in &self.artifact_requirements {
            requirement.validate_snapshot()?;
        }
        Self::new(
            self.outcome.clone(),
            self.target.clone(),
            self.artifact_requirements.clone(),
        )
        .map(|_| ())
    }
}

fn requirement_scope_name(scope: ArtifactRequirementScope) -> &'static str {
    match scope {
        ArtifactRequirementScope::CurrentStage => "current-stage",
        ArtifactRequirementScope::TaskHistory => "task-history",
    }
}

/// Immutable definition of one named Pipeline stage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PipelineStage {
    id: StageId,
    display_name: String,
    executor_kind: ExecutorKind,
    transitions: BTreeMap<OutcomeKey, PipelineTransition>,
    #[serde(default)]
    instructions: String,
    #[serde(default)]
    workspace: Option<super::StageWorkspaceRequirements>,
    #[serde(default)]
    acceptance_policy: Option<crate::candidate_review::StageAcceptancePolicy>,
    #[serde(default)]
    system_action: Option<crate::git_integration::SystemStageAction>,
}

impl PipelineStage {
    /// Creates a stage with a non-empty outcome matrix.
    pub fn new(
        id: StageId,
        display_name: impl Into<String>,
        executor_kind: ExecutorKind,
        transitions: impl IntoIterator<Item = PipelineTransition>,
    ) -> Result<Self, DomainError> {
        let display_name = display_name.into();
        if display_name.trim().is_empty() || display_name.chars().count() > 128 {
            return Err(DomainError::InvalidPipeline {
                reason: format!(
                    "stage {} display_name must be non-blank and at most 128 characters",
                    id
                ),
            });
        }
        let mut values = BTreeMap::new();
        for transition in transitions {
            let outcome = transition.outcome().clone();
            if values.insert(outcome.clone(), transition).is_some() {
                return Err(DomainError::InvalidPipeline {
                    reason: format!("stage {} repeats outcome {}", id, outcome.as_str()),
                });
            }
        }
        if values.is_empty() {
            return Err(DomainError::InvalidPipeline {
                reason: format!("stage {id} must declare at least one outcome"),
            });
        }
        Ok(Self {
            id,
            display_name,
            executor_kind,
            transitions: values,
            instructions: String::new(),
            workspace: None,
            acceptance_policy: None,
            system_action: None,
        })
    }

    /// Adds bounded instructions and optional restrictions before publication.
    pub fn with_requirements(
        mut self,
        instructions: String,
        workspace: Option<super::StageWorkspaceRequirements>,
    ) -> Result<Self, DomainError> {
        if instructions.len() > 64 * 1024 || instructions.contains('\0') {
            return Err(DomainError::InvalidPipeline {
                reason: "stage instructions must fit 64 KiB and contain no NUL".into(),
            });
        }
        self.instructions = instructions;
        self.workspace = workspace;
        Ok(self)
    }

    /// Instructions owned by this exact immutable stage definition.
    pub fn with_acceptance_policy(
        mut self,
        policy: Option<crate::candidate_review::StageAcceptancePolicy>,
    ) -> Result<Self, DomainError> {
        if let Some(policy) = &policy {
            policy.validate_for(&self)?;
        }
        self.acceptance_policy = policy;
        Ok(self)
    }
    /// No assessment policy is inferred from stage names or Employee role text.
    pub fn acceptance_policy(&self) -> Option<&crate::candidate_review::StageAcceptancePolicy> {
        self.acceptance_policy.as_ref()
    }
    /// Explicit registered deterministic action; no stage-name inference.
    pub fn with_system_action(
        mut self,
        action: Option<crate::git_integration::SystemStageAction>,
    ) -> Result<Self, DomainError> {
        if let Some(action) = &action {
            action.validate_for(&self)?;
        }
        self.system_action = action;
        Ok(self)
    }
    pub fn system_action(&self) -> Option<&crate::git_integration::SystemStageAction> {
        self.system_action.as_ref()
    }
    /// Instructions owned by this exact immutable stage definition.
    #[must_use]
    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    /// Absent requirements preserve the explicit M1 runtime binding semantics.
    #[must_use]
    pub const fn workspace(&self) -> Option<&super::StageWorkspaceRequirements> {
        self.workspace.as_ref()
    }

    /// Returns the stable stage identity.
    #[must_use]
    pub fn id(&self) -> &StageId {
        &self.id
    }

    /// Returns the human-facing display label.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Returns the stage executor category.
    #[must_use]
    pub const fn executor_kind(&self) -> ExecutorKind {
        self.executor_kind
    }

    /// Looks up a declared outcome.
    #[must_use]
    pub fn transition(&self, outcome: &OutcomeKey) -> Option<&PipelineTransition> {
        self.transitions.get(outcome)
    }

    /// Returns all transitions in stable outcome-key order.
    pub fn transitions(&self) -> impl Iterator<Item = &PipelineTransition> {
        self.transitions.values()
    }

    pub(crate) fn validate_snapshot(&self) -> Result<(), DomainError> {
        self.id.validate()?;
        for transition in self.transitions.values() {
            transition.validate_snapshot()?;
        }
        Self::new(
            self.id.clone(),
            self.display_name.clone(),
            self.executor_kind,
            self.transitions.values().cloned(),
        )
        .and_then(|stage| stage.with_requirements(self.instructions.clone(), self.workspace))
        .and_then(|stage| stage.with_acceptance_policy(self.acceptance_policy.clone()))
        .and_then(|stage| stage.with_system_action(self.system_action.clone()))
        .map(|_| ())
    }
}
