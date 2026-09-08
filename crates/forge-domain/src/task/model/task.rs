use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    Actor, ArtifactLink, DomainError, LifecycleStatus, PriorityLevelId, PriorityScheme, ProjectId,
    StageId, StageVisit, TaskId, TaskKey, TaskRevision, TaskWaitCondition, Timestamp,
    WaitConditionId,
};

use super::{
    NewTask, TaskKind, TaskPipelineBinding, TaskSource, TaskSpec, TaskWorkSurface, TerminalData,
};

/// Project-owned durable contract for one bounded unit of engineering work.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub(super) id: TaskId,
    pub(super) project_id: ProjectId,
    pub(super) key: TaskKey,
    pub(super) kind: TaskKind,
    pub(super) source: TaskSource,
    pub(super) spec: TaskSpec,
    pub(super) priority_level_id: PriorityLevelId,
    pub(super) pipeline: TaskPipelineBinding,
    pub(super) lifecycle: LifecycleStatus,
    pub(super) current_stage_id: Option<StageId>,
    pub(super) current_stage_visit: Option<StageVisit>,
    pub(super) wait_conditions: BTreeMap<WaitConditionId, TaskWaitCondition>,
    pub(super) resume_to: Option<crate::ResumeLifecycleStatus>,
    pub(super) work_surface: TaskWorkSurface,
    pub(super) artifact_links: Vec<ArtifactLink>,
    pub(super) terminal_data: Option<TerminalData>,
    pub(super) created_by: Actor,
    pub(super) created_at: Timestamp,
    pub(super) updated_at: Timestamp,
    pub(super) revision: TaskRevision,
}

impl Task {
    /// Creates a draft Task after validating its selected priority level.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidValue`] when the selected priority is
    /// unknown or retired in the Project scheme.
    pub fn new_draft(
        input: NewTask,
        priority_scheme: &PriorityScheme,
    ) -> Result<Self, DomainError> {
        priority_scheme.validate_active_level(&input.priority_level_id)?;
        Ok(Self {
            id: input.id,
            project_id: input.project_id,
            key: input.key,
            kind: input.kind,
            source: input.source,
            spec: input.spec,
            priority_level_id: input.priority_level_id,
            pipeline: input.pipeline,
            lifecycle: LifecycleStatus::Draft,
            current_stage_id: None,
            current_stage_visit: None,
            wait_conditions: BTreeMap::new(),
            resume_to: None,
            work_surface: TaskWorkSurface::None,
            artifact_links: Vec::new(),
            terminal_data: None,
            created_by: input.created_by,
            created_at: input.created_at,
            updated_at: input.created_at,
            revision: TaskRevision::INITIAL,
        })
    }

    /// Returns the immutable global identity.
    #[must_use]
    pub const fn id(&self) -> TaskId {
        self.id
    }

    /// Returns the Project owner.
    #[must_use]
    pub const fn project_id(&self) -> ProjectId {
        self.project_id
    }

    /// Returns the Project-local human key.
    #[must_use]
    pub const fn key(&self) -> TaskKey {
        self.key
    }

    /// Returns the bounded work category.
    #[must_use]
    pub const fn kind(&self) -> TaskKind {
        self.kind
    }

    /// Returns the intent source.
    #[must_use]
    pub const fn source(&self) -> TaskSource {
        self.source
    }

    /// Returns the current immutable/mutable Task intent.
    #[must_use]
    pub fn spec(&self) -> &TaskSpec {
        &self.spec
    }

    /// Returns the Project scheduler level selected for this Task.
    #[must_use]
    pub fn priority_level_id(&self) -> &PriorityLevelId {
        &self.priority_level_id
    }

    /// Returns the selected Pipeline version binding.
    #[must_use]
    pub fn pipeline(&self) -> &TaskPipelineBinding {
        &self.pipeline
    }

    /// Returns lifecycle independently from queue, lease, and Run activity.
    #[must_use]
    pub const fn lifecycle(&self) -> LifecycleStatus {
        self.lifecycle
    }

    /// Returns the Pipeline stage active after approval, if any.
    #[must_use]
    pub fn current_stage_id(&self) -> Option<&StageId> {
        self.current_stage_id.as_ref()
    }

    /// Returns the Task-local visit identity of its current Pipeline stage.
    #[must_use]
    pub const fn current_stage_visit(&self) -> Option<StageVisit> {
        self.current_stage_visit
    }

    /// Returns active wait conditions in deterministic identity order.
    pub fn wait_conditions(&self) -> impl Iterator<Item = &TaskWaitCondition> {
        self.wait_conditions.values()
    }

    /// Returns the lifecycle restored after the final active wait resolves.
    #[must_use]
    pub const fn resume_to_lifecycle(&self) -> Option<crate::ResumeLifecycleStatus> {
        self.resume_to
    }

    /// Returns the immutable Task-owned source binding, if configured.
    #[must_use]
    pub const fn work_surface(&self) -> &TaskWorkSurface {
        &self.work_surface
    }

    /// Returns links to immutable Task evidence in attachment order.
    #[must_use]
    pub fn artifact_links(&self) -> &[ArtifactLink] {
        &self.artifact_links
    }

    /// Returns terminal data exactly for `done` or `cancelled` Tasks.
    #[must_use]
    pub fn terminal_data(&self) -> Option<&TerminalData> {
        self.terminal_data.as_ref()
    }

    /// Returns the actor that created the Task.
    #[must_use]
    pub const fn created_by(&self) -> Actor {
        self.created_by
    }

    /// Returns the creation time.
    #[must_use]
    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    /// Returns the time of the current revision.
    #[must_use]
    pub const fn updated_at(&self) -> Timestamp {
        self.updated_at
    }

    /// Returns the optimistic-concurrency revision.
    #[must_use]
    pub const fn revision(&self) -> TaskRevision {
        self.revision
    }

    /// Revalidates a deserialized Task snapshot before storage exposes it to
    /// Core. This is deliberately structural: cross-aggregate checks such as
    /// Project policy and Pipeline membership remain Core transaction work.
    pub fn validate_snapshot(&self) -> Result<(), DomainError> {
        self.id.validate_v7("task.id")?;
        self.project_id.validate_v7("task.project_id")?;
        self.priority_level_id.validate()?;
        self.pipeline.validate_snapshot()?;
        self.spec.validate_snapshot()?;
        self.created_by.validate_snapshot()?;
        if let TaskWorkSurface::Git(binding) = &self.work_surface {
            binding.validate_snapshot()?;
        }
        if self.updated_at < self.created_at {
            return Err(DomainError::NonMonotonicTimestamp);
        }
        for condition in self.wait_conditions.values() {
            condition.validate_snapshot()?;
        }
        let mut linked_artifacts = BTreeSet::new();
        for link in &self.artifact_links {
            link.validate_snapshot()?;
            if !linked_artifacts.insert(link.artifact_id()) {
                return Err(DomainError::DuplicateArtifactLink);
            }
        }
        if let Some(stage_id) = &self.current_stage_id {
            stage_id.validate()?;
        }
        if self.current_stage_id.is_some() != self.current_stage_visit.is_some() {
            return Err(DomainError::InvalidStageOutcome {
                reason: "current stage and visit must either both be present or both be absent"
                    .to_owned(),
            });
        }
        if let Some(terminal_data) = &self.terminal_data {
            terminal_data.validate_snapshot()?;
        }
        self.validate_lifecycle_snapshot()
    }

    fn validate_lifecycle_snapshot(&self) -> Result<(), DomainError> {
        let has_stage = self.current_stage_id.is_some();
        let has_waits = !self.wait_conditions.is_empty();
        match self.lifecycle {
            LifecycleStatus::Draft => {
                if has_stage
                    || has_waits
                    || self.resume_to.is_some()
                    || self.terminal_data.is_some()
                {
                    return Err(invalid_task_snapshot("draft task carries execution state"));
                }
            }
            LifecycleStatus::Ready | LifecycleStatus::InProgress => {
                if !has_stage
                    || has_waits
                    || self.resume_to.is_some()
                    || self.terminal_data.is_some()
                {
                    return Err(invalid_task_snapshot(
                        "active task lifecycle has inconsistent wait or terminal state",
                    ));
                }
            }
            LifecycleStatus::Waiting => {
                if !has_stage
                    || !has_waits
                    || self.resume_to.is_none()
                    || self.terminal_data.is_some()
                {
                    return Err(invalid_task_snapshot(
                        "waiting task requires stage, wait conditions, and resume lifecycle",
                    ));
                }
                let stage_id = self.current_stage_id.as_ref().ok_or_else(|| {
                    invalid_task_snapshot("waiting task is missing its current stage")
                })?;
                let stage_visit = self.current_stage_visit.ok_or_else(|| {
                    invalid_task_snapshot("waiting task is missing its current stage visit")
                })?;
                for condition in self.wait_conditions.values() {
                    if condition.source_stage_id().is_some()
                        && (condition.source_stage_id() != Some(stage_id)
                            || condition.source_stage_visit() != Some(stage_visit))
                    {
                        return Err(invalid_task_snapshot(
                            "stage-owned wait does not belong to current stage visit",
                        ));
                    }
                }
            }
            LifecycleStatus::Done => {
                let Some(TerminalData::Completed(completion)) = &self.terminal_data else {
                    return Err(invalid_task_snapshot("done task requires completion data"));
                };
                if !has_stage || has_waits || self.resume_to.is_some() {
                    return Err(invalid_task_snapshot(
                        "done task carries inconsistent active state",
                    ));
                }
                self.ensure_outcome_artifacts_are_linked(completion)?;
                if self.kind == super::TaskKind::Analysis && !self.has_analysis_result(completion) {
                    return Err(DomainError::MissingAnalysisResult);
                }
            }
            LifecycleStatus::Cancelled => {
                if !matches!(self.terminal_data, Some(TerminalData::Cancelled(_)))
                    || has_waits
                    || self.resume_to.is_some()
                {
                    return Err(invalid_task_snapshot(
                        "cancelled task requires cancellation data and no active waits",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn invalid_task_snapshot(reason: &str) -> DomainError {
    DomainError::InvalidValue {
        field: "task.snapshot",
        reason: reason.to_owned(),
    }
}
