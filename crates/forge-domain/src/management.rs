//! Explicit execution control is independent of Task cancellation or runtime observation.
use serde::{Deserialize, Serialize};

mod dispatch;
mod resume;
pub use resume::{ResumeRejection, ScheduledResumeState, TaskResumeSchedule};
#[cfg(test)]
mod tests;
pub use dispatch::{
    ConstraintBlockReason, ConstraintEndReason, NextRunConstraintState, NextRunEmployeeConstraint,
};

/// Requested stop policy, never evidence that an executor has actually exited.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStopMode {
    /// Request bounded graceful shutdown while retaining the execution reservation.
    Graceful,
    /// Revoke logical authority and request forced termination, retaining the reservation.
    Force,
}

impl ExecutionStopMode {
    /// Whether logical authority must be revoked immediately.
    #[must_use]
    pub const fn is_force(self) -> bool {
        matches!(self, Self::Force)
    }
}
