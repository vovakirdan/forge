use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    Actor, DomainError, StageId, StageVisit, Timestamp, WaitConditionId, ids::validate_stable_key,
};

/// Canonical Task lifecycle, deliberately independent from Pipeline board stages.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStatus {
    /// The Task is being described and cannot be dispatched.
    Draft,
    /// The Task has an approved contract and can enter its Pipeline entry stage.
    Ready,
    /// At least one Pipeline stage has begun.
    InProgress,
    /// Work is paused by one or more typed conditions.
    Waiting,
    /// The Pipeline's terminal success contract has been satisfied.
    Done,
    /// Work was intentionally stopped with a Project catalog reason.
    Cancelled,
}

impl LifecycleStatus {
    /// Returns whether no further Task mutation is allowed.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Cancelled)
    }

    /// Returns whether the core lifecycle permits a direct transition.
    ///
    /// Pipeline rules still decide stage transitions and outcome contracts.
    #[must_use]
    pub const fn can_transition_to(self, target: Self) -> bool {
        matches!(
            (self, target),
            (Self::Draft, Self::Ready | Self::Cancelled)
                | (
                    Self::Ready,
                    Self::InProgress | Self::Waiting | Self::Cancelled
                )
                | (
                    Self::InProgress,
                    Self::Waiting | Self::Done | Self::Cancelled
                )
                | (
                    Self::Waiting,
                    Self::Ready | Self::InProgress | Self::Done | Self::Cancelled
                )
        )
    }
}

impl fmt::Display for LifecycleStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Draft => "draft",
            Self::Ready => "ready",
            Self::InProgress => "in_progress",
            Self::Waiting => "waiting",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        };
        formatter.write_str(value)
    }
}

/// Non-terminal lifecycle status restored after every active wait resolves.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeLifecycleStatus {
    /// Resume an approved Task that has not started its first stage.
    Ready,
    /// Resume an already-started Task in its current Pipeline stage.
    InProgress,
}

impl From<ResumeLifecycleStatus> for LifecycleStatus {
    fn from(value: ResumeLifecycleStatus) -> Self {
        match value {
            ResumeLifecycleStatus::Ready => Self::Ready,
            ResumeLifecycleStatus::InProgress => Self::InProgress,
        }
    }
}

/// Stable Project-defined name for an additional wait-condition category.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskWaitKindId(String);

impl TaskWaitKindId {
    /// Validates a custom wait-condition category.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidIdentifier`] for malformed values.
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        validate_stable_key("task_wait_kind", &value)?;
        Ok(Self(value))
    }

    /// Returns the stable category key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Typed reason that prevents a Task from moving through its Pipeline.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskWaitKind {
    /// A Task dependency has not satisfied its declared gate.
    Dependency,
    /// A human or external decision is required.
    DecisionRequired,
    /// A manager requested a manual pause.
    ManualPause,
    /// An Employee or manager escalation awaits a resolver.
    EscalationPending,
    /// Retry policy requires an explicit management decision.
    RetryExhausted,
    /// A Run was interrupted and cannot restart automatically.
    Interrupted,
    /// Project-specific wait reason that retains a stable identifier.
    Other(TaskWaitKindId),
}

/// One active, auditable condition that holds a Task in `waiting`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TaskWaitCondition {
    id: WaitConditionId,
    kind: TaskWaitKind,
    source_stage_id: Option<StageId>,
    source_stage_visit: Option<StageVisit>,
    detail: Option<String>,
    created_by: Actor,
    created_at: Timestamp,
}

impl TaskWaitCondition {
    /// Creates a typed wait condition.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidValue`] when a supplied detail contains
    /// only whitespace.
    pub fn new(
        id: WaitConditionId,
        kind: TaskWaitKind,
        detail: Option<String>,
        created_by: Actor,
        created_at: Timestamp,
    ) -> Result<Self, DomainError> {
        if detail
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(DomainError::InvalidValue {
                field: "task_wait_condition.detail",
                reason: "must not be blank when present".to_owned(),
            });
        }

        Ok(Self {
            id,
            kind,
            source_stage_id: None,
            source_stage_visit: None,
            detail,
            created_by,
            created_at,
        })
    }

    /// Creates the single stage-owned decision wait required by a human or
    /// external Pipeline executor. The stage binding prevents another active
    /// wait, such as a manual pause, from authorizing an outcome transition.
    pub fn for_stage(
        id: WaitConditionId,
        stage_id: StageId,
        stage_visit: StageVisit,
        detail: Option<String>,
        created_by: Actor,
        created_at: Timestamp,
    ) -> Result<Self, DomainError> {
        let mut condition = Self::new(
            id,
            TaskWaitKind::DecisionRequired,
            detail,
            created_by,
            created_at,
        )?;
        condition.source_stage_id = Some(stage_id);
        condition.source_stage_visit = Some(stage_visit);
        Ok(condition)
    }

    /// Returns the immutable condition identity.
    #[must_use]
    pub const fn id(&self) -> WaitConditionId {
        self.id
    }

    /// Returns the typed wait reason.
    #[must_use]
    pub fn kind(&self) -> &TaskWaitKind {
        &self.kind
    }

    /// Returns the Pipeline stage that created this decision wait, if any.
    #[must_use]
    pub fn source_stage_id(&self) -> Option<&StageId> {
        self.source_stage_id.as_ref()
    }

    /// Returns the Task-local stage visit that owns this decision wait, if any.
    #[must_use]
    pub const fn source_stage_visit(&self) -> Option<StageVisit> {
        self.source_stage_visit
    }

    /// Returns optional context for a human or resolver.
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// Returns the actor that created the condition.
    #[must_use]
    pub const fn created_by(&self) -> Actor {
        self.created_by
    }

    /// Returns when the condition became active.
    #[must_use]
    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub(crate) fn validate_snapshot(&self) -> Result<(), DomainError> {
        self.id.validate_v7("task_wait_condition.id")?;
        self.created_by.validate_snapshot()?;
        if self
            .detail
            .as_deref()
            .is_some_and(|detail| detail.trim().is_empty())
        {
            return Err(DomainError::InvalidValue {
                field: "task_wait_condition.detail",
                reason: "must not be blank when present".to_owned(),
            });
        }
        match (&self.kind, &self.source_stage_id, self.source_stage_visit) {
            (TaskWaitKind::DecisionRequired, Some(stage_id), Some(_)) => stage_id.validate(),
            (TaskWaitKind::Other(kind), None, None) => {
                validate_stable_key("task_wait_kind", kind.as_str())
            }
            (
                TaskWaitKind::Dependency
                | TaskWaitKind::ManualPause
                | TaskWaitKind::EscalationPending
                | TaskWaitKind::RetryExhausted
                | TaskWaitKind::Interrupted,
                None,
                None,
            ) => Ok(()),
            _ => Err(DomainError::InvalidWait {
                reason: "only a decision-required wait may be stage-owned".to_owned(),
            }),
        }
    }
}
