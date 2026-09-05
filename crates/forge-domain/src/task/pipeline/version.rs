use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    Actor, DomainError, PipelineId, PipelineVersionId, ProjectId, StageId, Task, TaskKind,
    Timestamp,
};

use super::{PipelineStage, validation};

/// Immutable graph input selected by Tasks pinned to this exact version.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PipelineVersionInput {
    /// Global immutable version identity.
    pub id: PipelineVersionId,
    /// Owning named Pipeline identity.
    pub pipeline_id: PipelineId,
    /// Project owner shared with every Task that may pin this version.
    pub project_id: ProjectId,
    /// Positive human-visible revision number within one Pipeline.
    pub version: u32,
    /// Supported Task work categories.
    pub task_kinds: BTreeSet<TaskKind>,
    /// Entry stage selected at Task approval.
    pub entry_stage_id: StageId,
    /// Optional finite cap on total stage entries for a single Task.
    ///
    /// It is required when the graph contains a cycle. The Task's monotonic
    /// `StageVisit` sequence enforces it at every non-terminal transition.
    pub max_stage_visits: Option<u32>,
    /// Complete stage graph.
    pub stages: Vec<PipelineStage>,
    /// Actor that published this immutable graph.
    pub created_by: Actor,
    /// Publication time.
    pub created_at: Timestamp,
}

/// Immutable, validated graph which owns stage/outcome semantics.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PipelineVersion {
    id: PipelineVersionId,
    pipeline_id: PipelineId,
    project_id: ProjectId,
    version: u32,
    task_kinds: BTreeSet<TaskKind>,
    entry_stage_id: StageId,
    #[serde(default)]
    max_stage_visits: Option<u32>,
    stages: BTreeMap<StageId, PipelineStage>,
    created_by: Actor,
    created_at: Timestamp,
}

impl PipelineVersion {
    /// Validates and publishes one immutable Pipeline graph.
    pub fn new(input: PipelineVersionInput) -> Result<Self, DomainError> {
        if input.version == 0 {
            return Err(DomainError::InvalidPipeline {
                reason: "version must be greater than zero".to_owned(),
            });
        }
        if input.task_kinds.is_empty() {
            return Err(DomainError::InvalidPipeline {
                reason: "task_kinds must not be empty".to_owned(),
            });
        }
        if input.max_stage_visits == Some(0) {
            return Err(DomainError::InvalidPipeline {
                reason: "max_stage_visits must be greater than zero when supplied".to_owned(),
            });
        }
        let mut stages = BTreeMap::new();
        for stage in input.stages {
            let id = stage.id().clone();
            if stages.insert(id.clone(), stage).is_some() {
                return Err(DomainError::InvalidPipeline {
                    reason: format!("duplicate stage {id}"),
                });
            }
        }
        if !stages.contains_key(&input.entry_stage_id) {
            return Err(DomainError::InvalidPipeline {
                reason: "entry_stage_id must reference a declared stage".to_owned(),
            });
        }
        validation::validate_targets(&stages)?;
        validation::validate_cycle_limit(&stages, input.max_stage_visits)?;
        validation::validate_reachability(&stages, &input.entry_stage_id)?;
        validation::validate_terminal_route(&stages, &input.entry_stage_id)?;
        Ok(Self {
            id: input.id,
            pipeline_id: input.pipeline_id,
            project_id: input.project_id,
            version: input.version,
            task_kinds: input.task_kinds,
            entry_stage_id: input.entry_stage_id,
            max_stage_visits: input.max_stage_visits,
            stages,
            created_by: input.created_by,
            created_at: input.created_at,
        })
    }

    /// Returns immutable version identity.
    #[must_use]
    pub const fn id(&self) -> PipelineVersionId {
        self.id
    }

    /// Returns owning logical Pipeline identity.
    #[must_use]
    pub const fn pipeline_id(&self) -> PipelineId {
        self.pipeline_id
    }

    /// Returns owning Project identity.
    #[must_use]
    pub const fn project_id(&self) -> ProjectId {
        self.project_id
    }

    /// Returns the human-visible graph version.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// Returns the fixed entry stage.
    #[must_use]
    pub fn entry_stage_id(&self) -> &StageId {
        &self.entry_stage_id
    }

    /// Returns the optional finite Task-local cap on all stage entries.
    #[must_use]
    pub const fn max_stage_visits(&self) -> Option<u32> {
        self.max_stage_visits
    }

    /// Returns the declared stage graph in stable identity order.
    pub fn stages(&self) -> impl Iterator<Item = &PipelineStage> {
        self.stages.values()
    }

    /// Returns one stage by its stable identity.
    #[must_use]
    pub fn stage(&self, stage_id: &StageId) -> Option<&PipelineStage> {
        self.stages.get(stage_id)
    }

    /// Returns the actor that published this immutable graph.
    #[must_use]
    pub const fn created_by(&self) -> Actor {
        self.created_by
    }

    /// Returns when this immutable graph was published.
    #[must_use]
    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    /// Returns whether this graph can serve a Task kind.
    #[must_use]
    pub fn supports_task_kind(&self, task_kind: TaskKind) -> bool {
        self.task_kinds.contains(&task_kind)
    }

    /// Revalidates a deserialized immutable Pipeline-version snapshot before
    /// a storage adapter exposes it to Core.
    ///
    /// # Errors
    ///
    /// Returns a domain error if the persisted graph cannot be reconstructed
    /// through the same constructor used at publication time.
    pub fn validate_snapshot(&self) -> Result<(), DomainError> {
        self.id.validate_v7("pipeline_version.id")?;
        self.pipeline_id
            .validate_v7("pipeline_version.pipeline_id")?;
        self.project_id.validate_v7("pipeline_version.project_id")?;
        self.entry_stage_id.validate()?;
        for stage in self.stages.values() {
            stage.validate_snapshot()?;
        }
        Self::new(PipelineVersionInput {
            id: self.id,
            pipeline_id: self.pipeline_id,
            project_id: self.project_id,
            version: self.version,
            task_kinds: self.task_kinds.clone(),
            entry_stage_id: self.entry_stage_id.clone(),
            max_stage_visits: self.max_stage_visits,
            stages: self.stages.values().cloned().collect(),
            created_by: self.created_by,
            created_at: self.created_at,
        })
        .map(|_| ())
    }

    pub(super) fn ensure_pinned_task(&self, task: &Task) -> Result<(), DomainError> {
        if task.project_id() != self.project_id
            || task.pipeline().pipeline_id() != self.pipeline_id
            || task.pipeline().pipeline_version_id() != self.id
            || task.pipeline().entry_stage_id() != &self.entry_stage_id
        {
            return Err(DomainError::InvalidStageOutcome {
                reason: "task is not pinned to this pipeline version".to_owned(),
            });
        }
        if !self.supports_task_kind(task.kind()) {
            return Err(DomainError::InvalidStageOutcome {
                reason: "pipeline version does not support this task kind".to_owned(),
            });
        }
        Ok(())
    }

    pub(super) fn ensure_stage_lifecycle(
        &self,
        task: &Task,
        executor_kind: super::ExecutorKind,
    ) -> Result<(), DomainError> {
        let expected = if executor_kind.requires_wait() {
            crate::LifecycleStatus::Waiting
        } else {
            crate::LifecycleStatus::InProgress
        };
        if task.lifecycle() != expected {
            return Err(DomainError::InvalidStageOutcome {
                reason: format!(
                    "stage executor requires task lifecycle {expected}, found {}",
                    task.lifecycle()
                ),
            });
        }
        Ok(())
    }
}
