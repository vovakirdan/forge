use crate::{
    Actor, Artifact, ArtifactLink, ArtifactProducer, CancellationReasonCatalog, DomainError,
    EmployeeId, LifecycleStatus, PriorityLevelId, PriorityScheme, ResumeLifecycleStatus, StageId,
    TaskPropertySchema, TaskWaitCondition, TaskWaitKind, Timestamp, WaitConditionId,
};

use super::{
    CancellationData, CancellationRequest, CompletionData, CompletionSource, Task, TaskKind,
    TerminalData,
};

impl Task {
    /// Pins an allowlisted Project source exactly once, before approving execution.
    pub fn bind_git_repository(
        &mut self,
        repository: &crate::ProjectRepository,
        initial_base: crate::git::GitObjectId,
        surface_id: uuid::Uuid,
        changed_at: Timestamp,
    ) -> Result<(), DomainError> {
        self.require_lifecycle(LifecycleStatus::Draft)?;
        if repository.project_id != self.project_id
            || self.work_surface != super::TaskWorkSurface::None
        {
            return Err(DomainError::InvalidValue {
                field: "task.work_surface",
                reason: "requires an unbound draft and a repository in the same Project".into(),
            });
        }
        let binding = crate::TaskGitBinding::from_repository(repository, initial_base, surface_id)?;
        self.touch(changed_at)?;
        self.work_surface = super::TaskWorkSurface::Git(binding);
        Ok(())
    }

    /// Replaces a draft's intent without changing its identity or Pipeline binding.
    ///
    /// # Errors
    ///
    /// Returns a lifecycle refusal unless the Task is still draft.
    pub fn amend_draft(
        &mut self,
        spec: super::TaskSpec,
        changed_at: Timestamp,
    ) -> Result<(), DomainError> {
        self.require_lifecycle(LifecycleStatus::Draft)?;
        self.touch(changed_at)?;
        self.spec = spec;
        Ok(())
    }

    /// Validates and approves the draft contract for the Pipeline entry stage.
    ///
    /// # Errors
    ///
    /// Returns an error unless the Task is draft, has a Definition of Done, and
    /// all properties resolve against the Project schema.
    pub fn approve(
        &mut self,
        property_schema: &TaskPropertySchema,
        changed_at: Timestamp,
    ) -> Result<(), DomainError> {
        self.require_lifecycle(LifecycleStatus::Draft)?;
        let resolved_properties = self.spec.resolve_for_approval(property_schema)?;
        self.transition_to(LifecycleStatus::Ready, changed_at)?;
        self.spec.properties = resolved_properties;
        self.current_stage_id = Some(self.pipeline.entry_stage_id().clone());
        self.current_stage_visit = Some(crate::StageVisit::INITIAL);
        Ok(())
    }

    /// Marks the first Pipeline stage as started.
    ///
    /// # Errors
    ///
    /// Returns an error unless the Task is ready.
    pub(crate) fn start(&mut self, changed_at: Timestamp) -> Result<(), DomainError> {
        self.require_lifecycle(LifecycleStatus::Ready)?;
        self.transition_to(LifecycleStatus::InProgress, changed_at)
    }

    /// Records that Core issued one fenced Run attempt for the active employee
    /// stage. The attempt count itself is a scheduler projection, while this
    /// revision bump invalidates stale queue rows before later dispatch.
    ///
    /// # Errors
    ///
    /// Returns an error unless the Task is actively executing a Pipeline stage.
    pub fn record_run_attempt(&mut self, changed_at: Timestamp) -> Result<(), DomainError> {
        self.require_lifecycle(LifecycleStatus::InProgress)?;
        if self.current_stage_id.is_none() || self.current_stage_visit.is_none() {
            return Err(DomainError::InvalidStageOutcome {
                reason: "run attempt requires an active pipeline stage visit".to_owned(),
            });
        }
        self.touch(changed_at)
    }

    /// Assigns a new active priority level while the Task is non-terminal.
    ///
    /// # Errors
    ///
    /// Returns an error for terminal Tasks or unknown/retired Project levels.
    pub fn set_priority(
        &mut self,
        priority_scheme: &PriorityScheme,
        priority_level_id: PriorityLevelId,
        changed_at: Timestamp,
    ) -> Result<(), DomainError> {
        self.require_mutable()?;
        priority_scheme.validate_active_level(&priority_level_id)?;
        self.touch(changed_at)?;
        self.priority_level_id = priority_level_id;
        Ok(())
    }

    /// Adds an active wait condition and moves `ready`/`in_progress` to waiting.
    ///
    /// # Errors
    ///
    /// Returns an error for a terminal/draft Task, duplicated condition, or a
    /// condition whose resume target conflicts with existing waits.
    pub fn add_wait_condition(
        &mut self,
        condition: TaskWaitCondition,
        changed_at: Timestamp,
    ) -> Result<(), DomainError> {
        if condition.source_stage_id().is_some() {
            return Err(DomainError::InvalidWait {
                reason: "stage-owned waits may only be added by the pipeline engine".to_owned(),
            });
        }
        self.add_wait_condition_internal(condition, changed_at)
    }

    pub(crate) fn add_stage_wait_condition(
        &mut self,
        condition: TaskWaitCondition,
        changed_at: Timestamp,
    ) -> Result<(), DomainError> {
        self.add_wait_condition_internal(condition, changed_at)
    }

    fn add_wait_condition_internal(
        &mut self,
        condition: TaskWaitCondition,
        changed_at: Timestamp,
    ) -> Result<(), DomainError> {
        let resume_to = match self.lifecycle {
            LifecycleStatus::Ready => ResumeLifecycleStatus::Ready,
            LifecycleStatus::InProgress => ResumeLifecycleStatus::InProgress,
            LifecycleStatus::Waiting => self.resume_to.ok_or_else(|| DomainError::InvalidWait {
                reason: "waiting task is missing a resume target".to_owned(),
            })?,
            LifecycleStatus::Draft | LifecycleStatus::Done | LifecycleStatus::Cancelled => {
                return Err(DomainError::InvalidLifecycleTransition {
                    from: self.lifecycle.to_string(),
                    to: LifecycleStatus::Waiting.to_string(),
                });
            }
        };
        if self.wait_conditions.contains_key(&condition.id()) {
            return Err(DomainError::InvalidWait {
                reason: "condition is already active".to_owned(),
            });
        }
        self.touch(changed_at)?;
        self.wait_conditions.insert(condition.id(), condition);
        self.resume_to = Some(resume_to);
        self.lifecycle = LifecycleStatus::Waiting;
        Ok(())
    }

    /// Resolves one active wait. The Task resumes only when the last one clears.
    ///
    /// Returns `true` when the Task resumed, otherwise `false`.
    ///
    /// # Errors
    ///
    /// Returns an error unless the named wait is currently active on a waiting Task.
    pub fn resolve_wait_condition(
        &mut self,
        condition_id: WaitConditionId,
        changed_at: Timestamp,
    ) -> Result<bool, DomainError> {
        if let Some(condition) = self.wait_conditions.get(&condition_id) {
            if condition.source_stage_id().is_some() {
                return Err(DomainError::InvalidWait {
                    reason: "stage-owned waits may only resolve through a pipeline outcome"
                        .to_owned(),
                });
            }
            if matches!(condition.kind(), TaskWaitKind::RetryExhausted) {
                return Err(DomainError::InvalidWait {
                    reason: "retry-exhausted waits require cancellation or an explicit future pipeline migration"
                        .to_owned(),
                });
            }
        }
        self.resolve_wait_condition_internal(condition_id, changed_at)
    }

    pub(crate) fn resolve_stage_wait_condition(
        &mut self,
        condition_id: WaitConditionId,
        changed_at: Timestamp,
    ) -> Result<bool, DomainError> {
        self.resolve_wait_condition_internal(condition_id, changed_at)
    }

    fn resolve_wait_condition_internal(
        &mut self,
        condition_id: WaitConditionId,
        changed_at: Timestamp,
    ) -> Result<bool, DomainError> {
        self.require_lifecycle(LifecycleStatus::Waiting)?;
        if !self.wait_conditions.contains_key(&condition_id) {
            return Err(DomainError::InvalidWait {
                reason: "condition is not active".to_owned(),
            });
        }
        let resume_to = if self.wait_conditions.len() == 1 {
            Some(self.resume_to.ok_or_else(|| DomainError::InvalidWait {
                reason: "waiting task is missing a resume target".to_owned(),
            })?)
        } else {
            None
        };
        self.touch(changed_at)?;
        let _ = self.wait_conditions.remove(&condition_id);
        match resume_to {
            Some(resume_to) => {
                self.resume_to = None;
                self.lifecycle = resume_to.into();
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Attaches immutable Artifact evidence without interpreting its contents.
    ///
    /// # Errors
    ///
    /// Returns an error for terminal Tasks, cross-Project evidence, or a
    /// duplicate link. The current stage is captured internally, so an
    /// application caller cannot forge evidence for a different stage.
    pub fn attach_artifact(
        &mut self,
        artifact: &Artifact,
        producer: ArtifactProducer,
        submitted_by: Actor,
        employee_id: Option<EmployeeId>,
        changed_at: Timestamp,
    ) -> Result<(), DomainError> {
        self.require_mutable()?;
        if artifact.project_id() != self.project_id {
            return Err(DomainError::CrossProjectArtifact);
        }
        if self
            .artifact_links
            .iter()
            .any(|existing| existing.artifact_id() == artifact.id())
        {
            return Err(DomainError::DuplicateArtifactLink);
        }
        let link = ArtifactLink::from_artifact(
            artifact,
            producer,
            self.current_stage_id.clone(),
            self.current_stage_visit,
            submitted_by,
            changed_at,
            employee_id,
        );
        self.touch(changed_at)?;
        self.artifact_links.push(link);
        Ok(())
    }

    /// Applies an accepted terminal success outcome.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid lifecycle/source pair, unlinked outcome
    /// evidence, or an analysis Task without `analysis_result` evidence.
    pub(crate) fn complete(&mut self, completion: CompletionData) -> Result<(), DomainError> {
        let allowed = matches!(
            (self.lifecycle, completion.source()),
            (LifecycleStatus::InProgress, CompletionSource::StageOutcome)
                | (LifecycleStatus::Waiting, CompletionSource::ExternalOutcome)
        );
        if !allowed {
            return Err(DomainError::InvalidLifecycleTransition {
                from: self.lifecycle.to_string(),
                to: LifecycleStatus::Done.to_string(),
            });
        }
        self.ensure_outcome_artifacts_are_linked(&completion)?;
        if self.kind == TaskKind::Analysis && !self.has_analysis_result(&completion) {
            return Err(DomainError::MissingAnalysisResult);
        }
        self.transition_to(LifecycleStatus::Done, completion.completed_at())?;
        self.wait_conditions.clear();
        self.resume_to = None;
        self.terminal_data = Some(TerminalData::Completed(completion));
        Ok(())
    }

    /// Applies a Pipeline-validated move to a non-terminal next stage.
    ///
    /// This is intentionally crate-private: only the immutable Pipeline graph
    /// engine may select a stage. Transport, storage, and executors must submit
    /// outcomes rather than mutating a Task's stage directly.
    pub(crate) fn apply_validated_stage_transition(
        &mut self,
        next_stage_id: StageId,
        changed_at: Timestamp,
    ) -> Result<(), DomainError> {
        if self.lifecycle != LifecycleStatus::InProgress {
            return Err(DomainError::InvalidStageOutcome {
                reason: format!(
                    "pipeline stage transition requires in_progress, found {}",
                    self.lifecycle
                ),
            });
        }
        let next_visit = self
            .current_stage_visit
            .ok_or_else(|| DomainError::InvalidStageOutcome {
                reason: "task current stage is missing its visit identity".to_owned(),
            })?
            .next()?;
        self.touch(changed_at)?;
        self.current_stage_id = Some(next_stage_id);
        self.current_stage_visit = Some(next_visit);
        Ok(())
    }

    /// Cancels any non-terminal Task only through an active Project reason catalog.
    ///
    /// # Errors
    ///
    /// Returns an error when no reason is supplied, the reason is inactive, or
    /// the Task is already terminal.
    pub fn cancel(
        &mut self,
        catalog: &CancellationReasonCatalog,
        request: CancellationRequest,
    ) -> Result<(), DomainError> {
        self.require_mutable()?;
        let reason_id = request
            .reason_id
            .ok_or(DomainError::CancellationReasonRequired)?;
        if catalog.active(&reason_id).is_none() {
            return Err(DomainError::UnknownCancellationReason {
                reason_id: reason_id.as_str().to_owned(),
            });
        }
        if request
            .note
            .as_deref()
            .is_some_and(|note| note.trim().is_empty())
        {
            return Err(DomainError::InvalidValue {
                field: "task.cancellation_note",
                reason: "must not be blank when present".to_owned(),
            });
        }
        self.transition_to(LifecycleStatus::Cancelled, request.cancelled_at)?;
        self.wait_conditions.clear();
        self.resume_to = None;
        self.terminal_data = Some(TerminalData::Cancelled(CancellationData {
            reason_id,
            note: request.note,
            cancelled_by: request.cancelled_by,
            cancelled_at: request.cancelled_at,
        }));
        Ok(())
    }

    fn require_lifecycle(&self, expected: LifecycleStatus) -> Result<(), DomainError> {
        if self.lifecycle.is_terminal() {
            return Err(DomainError::TerminalTask {
                status: self.lifecycle.to_string(),
            });
        }
        if self.lifecycle != expected {
            return Err(DomainError::InvalidLifecycleTransition {
                from: self.lifecycle.to_string(),
                to: expected.to_string(),
            });
        }
        Ok(())
    }

    fn require_mutable(&self) -> Result<(), DomainError> {
        if self.lifecycle.is_terminal() {
            Err(DomainError::TerminalTask {
                status: self.lifecycle.to_string(),
            })
        } else {
            Ok(())
        }
    }

    fn transition_to(
        &mut self,
        target: LifecycleStatus,
        changed_at: Timestamp,
    ) -> Result<(), DomainError> {
        if !self.lifecycle.can_transition_to(target) {
            return Err(DomainError::InvalidLifecycleTransition {
                from: self.lifecycle.to_string(),
                to: target.to_string(),
            });
        }
        self.touch(changed_at)?;
        self.lifecycle = target;
        Ok(())
    }

    fn touch(&mut self, changed_at: Timestamp) -> Result<(), DomainError> {
        if changed_at < self.updated_at {
            return Err(DomainError::NonMonotonicTimestamp);
        }
        self.revision = self.revision.next()?;
        self.updated_at = changed_at;
        Ok(())
    }

    pub(super) fn ensure_outcome_artifacts_are_linked(
        &self,
        completion: &CompletionData,
    ) -> Result<(), DomainError> {
        if completion.outcome_artifact_ids().all(|id| {
            self.artifact_links
                .iter()
                .any(|link| link.artifact_id() == id)
        }) {
            Ok(())
        } else {
            Err(DomainError::UnknownOutcomeArtifact)
        }
    }

    pub(super) fn has_analysis_result(&self, completion: &CompletionData) -> bool {
        completion.outcome_artifact_ids().any(|artifact_id| {
            self.artifact_links
                .iter()
                .any(|link| link.artifact_id() == artifact_id && link.kind().is_analysis_result())
        })
    }
}
