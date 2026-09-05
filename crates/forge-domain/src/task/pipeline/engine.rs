use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    Actor, ArtifactId, CancellationReasonCatalog, CancellationReasonId, CancellationRequest,
    CompletionData, CompletionSource, DomainError, LifecycleStatus, StageId, Task,
    TaskWaitCondition, Timestamp, WaitConditionId,
};

use super::{
    ArtifactRequirementScope, ExecutorKind, OutcomeKey, PipelineTransition,
    PipelineTransitionTarget, PipelineVersion,
};

/// Typed outcome supplied by a stage executor; Core selects no arbitrary route.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StageOutcomeSubmission {
    /// Declared current-stage outcome key.
    pub outcome: OutcomeKey,
    /// Attached Artifact identities cited by this outcome.
    pub outcome_artifact_ids: BTreeSet<ArtifactId>,
    /// Actor submitting the outcome.
    pub submitted_by: Actor,
    /// Authoritative submission time.
    pub submitted_at: Timestamp,
    /// Required only when the declared transition targets cancellation.
    pub cancellation_reason_id: Option<CancellationReasonId>,
    /// Optional cancellation context.
    pub cancellation_note: Option<String>,
}

/// Result of applying a valid Pipeline graph transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StageTransitionEffect {
    /// A named next stage became current.
    EnteredStage {
        /// Next stage identity.
        stage_id: StageId,
        /// Its execution category.
        executor_kind: ExecutorKind,
    },
    /// Task terminal-success contract was satisfied.
    Completed,
    /// Task terminal-cancellation contract was satisfied.
    Cancelled,
}

impl PipelineVersion {
    /// Enters a manual entry stage or returns its executor for later scheduling.
    ///
    /// The Task must already be approved and pinned to this exact version.
    pub fn enter_entry_stage(
        &self,
        task: &mut Task,
        wait_condition: Option<TaskWaitCondition>,
        changed_at: Timestamp,
    ) -> Result<ExecutorKind, DomainError> {
        self.ensure_pinned_task(task)?;
        if task.lifecycle() != LifecycleStatus::Ready {
            return Err(DomainError::InvalidStageOutcome {
                reason: "entry stage can only be entered from ready".to_owned(),
            });
        }
        let stage = self.entry_stage()?;
        if stage.executor_kind().requires_wait() {
            let condition = wait_condition.ok_or_else(|| DomainError::InvalidStageOutcome {
                reason: "human or external stage requires a wait condition".to_owned(),
            })?;
            let stage_visit =
                task.current_stage_visit()
                    .ok_or_else(|| DomainError::InvalidStageOutcome {
                        reason: "entry stage is missing its visit identity".to_owned(),
                    })?;
            validate_stage_wait_condition(&condition, stage.id(), stage_visit)?;
            task.start(changed_at)?;
            task.add_stage_wait_condition(condition, changed_at)?;
        } else if wait_condition.is_some() {
            return Err(DomainError::InvalidStageOutcome {
                reason: "automatic stage must not create a wait condition".to_owned(),
            });
        } else if stage.executor_kind() == ExecutorKind::System {
            task.start(changed_at)?;
        }
        Ok(stage.executor_kind())
    }

    /// Starts the initial Employee stage when Core has atomically issued its
    /// Lease and Run scope. Approval only makes that entry stage queueable;
    /// keeping the Task `ready` until this point prevents an unleased queue
    /// item from claiming active work in the board lifecycle.
    pub fn start_entry_employee_stage(
        &self,
        task: &mut Task,
        changed_at: Timestamp,
    ) -> Result<(), DomainError> {
        self.ensure_pinned_task(task)?;
        let entry = self.entry_stage()?;
        if entry.executor_kind() != ExecutorKind::Employee {
            return Err(DomainError::InvalidStageOutcome {
                reason: "only an employee entry stage starts with a Run".to_owned(),
            });
        }
        task.start(changed_at)
    }

    /// Validates and applies a submitted outcome from the Task's current stage.
    ///
    /// Required Artifact kinds are checked structurally by linked identity and
    /// source stage. This deliberately does not judge the truth of evidence.
    pub fn apply_outcome(
        &self,
        task: &mut Task,
        submission: StageOutcomeSubmission,
        cancellation_catalog: &CancellationReasonCatalog,
        resolved_wait_condition_id: Option<WaitConditionId>,
        next_wait_condition: Option<TaskWaitCondition>,
    ) -> Result<StageTransitionEffect, DomainError> {
        self.ensure_pinned_task(task)?;
        let current_stage_id =
            task.current_stage_id()
                .cloned()
                .ok_or_else(|| DomainError::InvalidStageOutcome {
                    reason: "task has no current pipeline stage".to_owned(),
                })?;
        let stage =
            self.stage(&current_stage_id)
                .ok_or_else(|| DomainError::InvalidStageOutcome {
                    reason: "task current stage is not defined by its pinned pipeline".to_owned(),
                })?;
        let current_stage_visit =
            task.current_stage_visit()
                .ok_or_else(|| DomainError::InvalidStageOutcome {
                    reason: "task current stage is missing its visit identity".to_owned(),
                })?;
        self.ensure_stage_lifecycle(task, stage.executor_kind())?;
        validate_wait_control(
            task,
            &current_stage_id,
            current_stage_visit,
            stage.executor_kind(),
            resolved_wait_condition_id,
        )?;
        let transition = stage.transition(&submission.outcome).ok_or_else(|| {
            DomainError::InvalidStageOutcome {
                reason: format!(
                    "outcome {} is not declared by stage {}",
                    submission.outcome.as_str(),
                    current_stage_id
                ),
            }
        })?;
        validate_outcome_artifacts(task, &current_stage_id, transition, &submission)?;

        match transition.target() {
            PipelineTransitionTarget::Done => {
                if next_wait_condition.is_some() {
                    return Err(DomainError::InvalidStageOutcome {
                        reason: "terminal automatic outcome must not carry wait-condition controls"
                            .to_owned(),
                    });
                }
                let source = completion_source(stage.executor_kind());
                task.complete(CompletionData::new(
                    source,
                    submission.outcome_artifact_ids,
                    submission.submitted_by,
                    submission.submitted_at,
                ))?;
                Ok(StageTransitionEffect::Completed)
            }
            PipelineTransitionTarget::Cancelled => {
                if next_wait_condition.is_some() {
                    return Err(DomainError::InvalidStageOutcome {
                        reason: "terminal automatic outcome must not carry wait-condition controls"
                            .to_owned(),
                    });
                }
                let request = CancellationRequest {
                    reason_id: submission.cancellation_reason_id,
                    note: submission.cancellation_note,
                    cancelled_by: submission.submitted_by,
                    cancelled_at: submission.submitted_at,
                };
                task.cancel(cancellation_catalog, request)?;
                Ok(StageTransitionEffect::Cancelled)
            }
            PipelineTransitionTarget::Stage(next_stage_id) => {
                self.ensure_next_stage_visit_is_available(current_stage_visit)?;
                let next_stage =
                    self.stage(next_stage_id)
                        .ok_or_else(|| DomainError::InvalidPipeline {
                            reason: format!("missing target stage {next_stage_id}"),
                        })?;
                if next_stage.executor_kind().requires_wait() {
                    let condition = next_wait_condition.as_ref().ok_or_else(|| {
                        DomainError::InvalidStageOutcome {
                            reason: "next human or external stage requires a wait condition"
                                .to_owned(),
                        }
                    })?;
                    validate_stage_wait_condition(
                        condition,
                        next_stage_id,
                        current_stage_visit.next()?,
                    )?;
                } else if next_wait_condition.is_some() {
                    return Err(DomainError::InvalidStageOutcome {
                        reason: "next automatic stage must not create a wait condition".to_owned(),
                    });
                }
                if stage.executor_kind().requires_wait() {
                    let condition_id = resolved_wait_condition_id.ok_or_else(|| {
                        DomainError::InvalidStageOutcome {
                            reason: "manual stage outcome must name the active wait condition"
                                .to_owned(),
                        }
                    })?;
                    let _ =
                        task.resolve_stage_wait_condition(condition_id, submission.submitted_at)?;
                }
                task.apply_validated_stage_transition(
                    next_stage_id.clone(),
                    submission.submitted_at,
                )?;
                if next_stage.executor_kind().requires_wait() {
                    let condition =
                        next_wait_condition.ok_or_else(|| DomainError::InvalidStageOutcome {
                            reason: "validated next human or external stage is missing a wait"
                                .to_owned(),
                        })?;
                    task.add_stage_wait_condition(condition, submission.submitted_at)?;
                }
                Ok(StageTransitionEffect::EnteredStage {
                    stage_id: next_stage_id.clone(),
                    executor_kind: next_stage.executor_kind(),
                })
            }
        }
    }

    fn entry_stage(&self) -> Result<&super::PipelineStage, DomainError> {
        self.stage(self.entry_stage_id())
            .ok_or_else(|| DomainError::InvalidPipeline {
                reason: "validated pipeline is missing its entry stage".to_owned(),
            })
    }

    fn ensure_next_stage_visit_is_available(
        &self,
        current_stage_visit: crate::StageVisit,
    ) -> Result<(), DomainError> {
        let next_stage_visit = current_stage_visit.next()?;
        if let Some(maximum) = self.max_stage_visits()
            && next_stage_visit.get() > u64::from(maximum)
        {
            return Err(DomainError::StageVisitLimitExceeded { maximum });
        }
        Ok(())
    }
}

fn completion_source(executor_kind: ExecutorKind) -> CompletionSource {
    if executor_kind.requires_wait() {
        CompletionSource::ExternalOutcome
    } else {
        CompletionSource::StageOutcome
    }
}

fn validate_wait_control(
    task: &Task,
    current_stage_id: &StageId,
    current_stage_visit: crate::StageVisit,
    executor_kind: ExecutorKind,
    resolved_wait_condition_id: Option<WaitConditionId>,
) -> Result<(), DomainError> {
    if !executor_kind.requires_wait() {
        if resolved_wait_condition_id.is_some() {
            return Err(DomainError::InvalidStageOutcome {
                reason: "automatic stage outcome must not name a wait condition".to_owned(),
            });
        }
        return Ok(());
    }
    let wait_condition_id =
        resolved_wait_condition_id.ok_or_else(|| DomainError::InvalidStageOutcome {
            reason: "manual stage outcome must name the active wait condition".to_owned(),
        })?;
    let conditions = task.wait_conditions().collect::<Vec<_>>();
    if conditions.len() != 1 {
        return Err(DomainError::InvalidStageOutcome {
            reason: "manual stage outcome cannot advance while another wait is active".to_owned(),
        });
    }
    let Some(condition) = conditions
        .into_iter()
        .find(|condition| condition.id() == wait_condition_id)
    else {
        return Err(DomainError::InvalidStageOutcome {
            reason: "manual stage outcome names a non-active wait condition".to_owned(),
        });
    };
    validate_stage_wait_condition(condition, current_stage_id, current_stage_visit)?;
    Ok(())
}

fn validate_stage_wait_condition(
    condition: &TaskWaitCondition,
    expected_stage_id: &StageId,
    expected_stage_visit: crate::StageVisit,
) -> Result<(), DomainError> {
    if !matches!(condition.kind(), crate::TaskWaitKind::DecisionRequired)
        || condition.source_stage_id() != Some(expected_stage_id)
        || condition.source_stage_visit() != Some(expected_stage_visit)
    {
        return Err(DomainError::InvalidStageOutcome {
            reason: "manual stage requires its stage-owned decision wait condition".to_owned(),
        });
    }
    Ok(())
}

fn validate_outcome_artifacts(
    task: &Task,
    current_stage_id: &StageId,
    transition: &PipelineTransition,
    submission: &StageOutcomeSubmission,
) -> Result<(), DomainError> {
    let current_stage_visit =
        task.current_stage_visit()
            .ok_or_else(|| DomainError::InvalidStageOutcome {
                reason: "task current stage is missing its visit identity".to_owned(),
            })?;
    for artifact_id in &submission.outcome_artifact_ids {
        let linked = task
            .artifact_links()
            .iter()
            .any(|link| link.artifact_id() == *artifact_id);
        if !linked {
            return Err(DomainError::InvalidStageOutcome {
                reason: format!(
                    "outcome references artifact {artifact_id} not linked to this task"
                ),
            });
        }
    }
    for requirement in transition.artifact_requirements() {
        let count = task
            .artifact_links()
            .iter()
            .filter(|link| {
                link.kind() == requirement.kind()
                    && artifact_matches_requirement_scope(
                        link.source_stage_id(),
                        link.source_stage_visit(),
                        current_stage_id,
                        current_stage_visit,
                        requirement.scope(),
                    )
                    && submission
                        .outcome_artifact_ids
                        .contains(&link.artifact_id())
            })
            .count();
        if count < usize::from(requirement.minimum_count()) {
            return Err(DomainError::InvalidStageOutcome {
                reason: format!(
                    "outcome {} requires {} artifact(s) of kind {}",
                    transition.outcome().as_str(),
                    requirement.minimum_count(),
                    requirement.kind().as_str()
                ),
            });
        }
    }
    Ok(())
}

fn artifact_matches_requirement_scope(
    source_stage_id: Option<&StageId>,
    source_stage_visit: Option<crate::StageVisit>,
    current_stage_id: &StageId,
    current_stage_visit: crate::StageVisit,
    scope: ArtifactRequirementScope,
) -> bool {
    match scope {
        ArtifactRequirementScope::CurrentStage => {
            source_stage_id == Some(current_stage_id)
                && source_stage_visit == Some(current_stage_visit)
        }
        ArtifactRequirementScope::TaskHistory => true,
    }
}
