//! A durable alarm authorizes one specific wait resolution, never an unrestricted retry.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    Actor, CommandId, DomainError, PipelineVersionId, ProjectId, StageId, StageVisit, TaskId,
    Timestamp, WaitConditionId,
};

/// A due alarm records why its original intent could not safely be applied.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeRejection {
    TaskChanged,
    WaitAbsent,
    ResolutionRequired,
    ProjectStopped,
    RecoveryHold,
    DependencyUnsatisfied,
    ExecutionRetained,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScheduledResumeState {
    Pending,
    Applied {
        command_id: CommandId,
        at: Timestamp,
    },
    Rejected {
        reason: ResumeRejection,
        at: Timestamp,
    },
    Cancelled {
        at: Timestamp,
    },
}

impl ScheduledResumeState {
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Applied { .. } => "applied",
            Self::Rejected { .. } => "rejected",
            Self::Cancelled { .. } => "cancelled",
        }
    }
}

/// Immutable scope and source authority; state advances exactly once from pending.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskResumeSchedule {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub expected_task_revision: u64,
    pub pipeline_version_id: PipelineVersionId,
    pub stage_id: StageId,
    pub stage_visit: StageVisit,
    pub wait_condition_id: WaitConditionId,
    pub not_before: Timestamp,
    pub reason: String,
    pub created_by: Actor,
    pub created_at: Timestamp,
    pub state: ScheduledResumeState,
}

impl TaskResumeSchedule {
    pub fn validate_snapshot(&self) -> Result<(), DomainError> {
        if self.id.get_version_num() != 7
            || self.expected_task_revision == 0
            || self.reason.trim().is_empty()
            || self.reason.contains('\0')
            || self.reason.chars().count() > 10_000
        {
            return Err(invalid());
        }
        self.project_id.validate_v7("resume.project_id")?;
        self.task_id.validate_v7("resume.task_id")?;
        self.pipeline_version_id
            .validate_v7("resume.pipeline_version_id")?;
        self.wait_condition_id
            .validate_v7("resume.wait_condition_id")?;
        self.stage_id.validate()?;
        self.created_by.validate_snapshot()?;
        if let ScheduledResumeState::Applied { command_id, .. } = self.state {
            command_id.validate_v7("resume.command_id")?;
        }
        let resolved_at = match self.state {
            ScheduledResumeState::Pending => None,
            ScheduledResumeState::Applied { at, .. }
            | ScheduledResumeState::Rejected { at, .. }
            | ScheduledResumeState::Cancelled { at } => Some(at),
        };
        if resolved_at.is_some_and(|at| at < self.created_at) {
            return Err(invalid());
        }
        Ok(())
    }

    pub fn resolve(&mut self, state: ScheduledResumeState) -> Result<(), DomainError> {
        if self.state != ScheduledResumeState::Pending || state == ScheduledResumeState::Pending {
            return Err(invalid());
        }
        let mut updated = self.clone();
        updated.state = state;
        updated.validate_snapshot()?;
        *self = updated;
        Ok(())
    }
}

fn invalid() -> DomainError {
    DomainError::InvalidValue {
        field: "task.resume_schedule",
        reason: "invalid scheduled resume or terminal result".into(),
    }
}
