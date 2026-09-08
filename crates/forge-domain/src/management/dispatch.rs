//! A one-shot admission constraint never transfers an existing Run or Task ownership.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    Actor, DomainError, EmployeeId, PipelineVersionId, ProjectId, StageId, StageVisit, Task,
    TaskId, Timestamp,
};

/// A blocked assignment is visible and requires explicit replacement or clearing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintBlockReason {
    EmployeeDisabled,
    StageIneligible,
}

/// Why an unused constraint no longer applies.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintEndReason {
    Cleared,
    Superseded,
    StageLeft,
    TaskTerminal,
}

/// Only a successfully committed Lease consumes a constraint.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum NextRunConstraintState {
    Pending,
    Blocked { reason: ConstraintBlockReason },
    Consumed { run_id: Uuid },
    Cancelled { reason: ConstraintEndReason },
}

impl NextRunConstraintState {
    /// Includes held constraints: a disabled assignee never becomes an implicit fallback.
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Pending | Self::Blocked { .. })
    }

    /// Stable normalized storage discriminator.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Blocked { .. } => "blocked",
            Self::Consumed { .. } => "consumed",
            Self::Cancelled { .. } => "cancelled",
        }
    }
}

/// Immutable instruction scope with an auditable, monotonic dispatch result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NextRunEmployeeConstraint {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub pipeline_version_id: PipelineVersionId,
    pub stage_id: StageId,
    pub stage_visit: StageVisit,
    pub employee_id: EmployeeId,
    pub created_by: Actor,
    pub created_at: Timestamp,
    pub state: NextRunConstraintState,
}

impl NextRunEmployeeConstraint {
    pub fn validate_snapshot(&self) -> Result<(), DomainError> {
        if self.id.get_version_num() != 7 {
            return Err(invalid("constraint id must be UUIDv7"));
        }
        self.project_id.validate_v7("constraint.project_id")?;
        self.task_id.validate_v7("constraint.task_id")?;
        self.employee_id.validate_v7("constraint.employee_id")?;
        self.pipeline_version_id
            .validate_v7("constraint.pipeline_version_id")?;
        self.stage_id.validate()?;
        self.created_by.validate_snapshot()?;
        if let NextRunConstraintState::Consumed { run_id } = self.state
            && run_id.get_version_num() != 7
        {
            return Err(invalid("consuming Run must be UUIDv7"));
        }
        Ok(())
    }

    /// Historical constraints cannot apply to a later visit of the same stage name.
    #[must_use]
    pub fn applies_to(&self, task: &Task) -> bool {
        self.state.is_active()
            && !task.lifecycle().is_terminal()
            && self.project_id == task.project_id()
            && self.task_id == task.id()
            && self.pipeline_version_id == task.pipeline().pipeline_version_id()
            && Some(&self.stage_id) == task.current_stage_id()
            && Some(self.stage_visit) == task.current_stage_visit()
    }

    /// A held instruction cannot be resumed implicitly by becoming available again.
    pub fn transition(&mut self, target: NextRunConstraintState) -> Result<(), DomainError> {
        let allowed = matches!(
            (&self.state, &target),
            (
                NextRunConstraintState::Pending,
                NextRunConstraintState::Blocked { .. }
                    | NextRunConstraintState::Consumed { .. }
                    | NextRunConstraintState::Cancelled { .. }
            ) | (
                NextRunConstraintState::Blocked { .. },
                NextRunConstraintState::Cancelled { .. }
            )
        );
        if !allowed {
            return Err(invalid(
                "constraint result is terminal or transition is forbidden",
            ));
        }
        let mut updated = self.clone();
        updated.state = target;
        updated.validate_snapshot()?;
        *self = updated;
        Ok(())
    }
}

fn invalid(reason: &str) -> DomainError {
    DomainError::InvalidValue {
        field: "task.next_run_employee",
        reason: reason.into(),
    }
}
