use serde::{Deserialize, Serialize};

use crate::{
    CancellationReason, CancellationReasonCatalog, DomainError, PriorityScheme, ProjectId,
    ProjectLocalSequence, TaskKey, TaskPropertySchema, Timestamp,
};

/// Project-wide switch controlling whether Scheduler may issue new Runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectExecutionGate {
    /// Eligible queued work may receive a Lease and Run.
    Open,
    /// No new work may start; history and recovery remain readable.
    Stopped,
}

/// Canonical Project configuration required by M0 task creation and dispatch.
///
/// Pipelines, Employees, Tasks, and Artifacts are separate aggregates. This
/// type owns only Project-wide policy, revision, and local Task-key allocation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    id: ProjectId,
    name: String,
    revision: u64,
    execution_gate: ProjectExecutionGate,
    priority_scheme: PriorityScheme,
    cancellation_reasons: CancellationReasonCatalog,
    property_schema: TaskPropertySchema,
    next_task_sequence: u64,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl Project {
    /// Creates a Project with the conventional M0 priority and cancellation
    /// catalogs. The initial creation command produces revision one.
    pub fn new(
        id: ProjectId,
        name: impl Into<String>,
        created_at: Timestamp,
    ) -> Result<Self, DomainError> {
        let name = name.into();
        if name.trim().is_empty() || name.chars().count() > 240 {
            return Err(DomainError::InvalidValue {
                field: "project.name",
                reason: "must be non-blank and at most 240 characters".to_owned(),
            });
        }
        let priority_scheme = PriorityScheme::default_three_levels()?;
        let cancellation_reasons =
            CancellationReasonCatalog::new([CancellationReason::unspecified()?])?;
        Ok(Self {
            id,
            name,
            revision: 1,
            execution_gate: ProjectExecutionGate::Stopped,
            priority_scheme,
            cancellation_reasons,
            property_schema: TaskPropertySchema::default(),
            next_task_sequence: 0,
            created_at,
            updated_at: created_at,
        })
    }

    /// Returns the immutable Project identity.
    #[must_use]
    pub const fn id(&self) -> ProjectId {
        self.id
    }

    /// Returns the display-safe Project name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the current optimistic command revision.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns whether Scheduler may issue a new Lease.
    #[must_use]
    pub const fn execution_gate(&self) -> ProjectExecutionGate {
        self.execution_gate
    }

    /// Returns the Project priority policy.
    #[must_use]
    pub fn priority_scheme(&self) -> &PriorityScheme {
        &self.priority_scheme
    }

    /// Returns the catalog required by Task cancellation transitions.
    #[must_use]
    pub fn cancellation_reasons(&self) -> &CancellationReasonCatalog {
        &self.cancellation_reasons
    }

    /// Returns the typed Task-property schema used on approval.
    #[must_use]
    pub fn property_schema(&self) -> &TaskPropertySchema {
        &self.property_schema
    }

    /// Replaces the Project's future Task approval schema at a new revision.
    /// The command layer permits this only before the first Task exists.
    pub fn configure_property_schema(
        &mut self,
        schema: TaskPropertySchema,
        changed_at: Timestamp,
    ) -> Result<(), DomainError> {
        schema.validate_for_configuration()?;
        self.touch(changed_at)?;
        self.property_schema = schema;
        Ok(())
    }

    /// Returns Project creation time.
    #[must_use]
    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    /// Returns the latest Project mutation time.
    #[must_use]
    pub const fn updated_at(&self) -> Timestamp {
        self.updated_at
    }

    /// Returns the most recently allocated Project-local Task sequence.
    #[must_use]
    pub const fn task_sequence(&self) -> u64 {
        self.next_task_sequence
    }

    /// Checks a named command's optimistic Project revision.
    pub fn require_revision(&self, expected: u64) -> Result<(), DomainError> {
        if self.revision != expected {
            return Err(DomainError::StaleProjectRevision {
                expected,
                actual: self.revision,
            });
        }
        Ok(())
    }

    /// Allocates the next durable Project-local Task key and increments the
    /// Project revision as part of the containing create-task command.
    pub fn allocate_task_key(&mut self, changed_at: Timestamp) -> Result<TaskKey, DomainError> {
        let next =
            self.next_task_sequence
                .checked_add(1)
                .ok_or_else(|| DomainError::InvalidValue {
                    field: "project.task_sequence",
                    reason: "cannot exceed u64::MAX".to_owned(),
                })?;
        let sequence = ProjectLocalSequence::new(next)?;
        self.touch(changed_at)?;
        self.next_task_sequence = next;
        Ok(TaskKey::new(sequence))
    }

    /// Opens scheduling for this Project and records a Project revision.
    pub fn start_execution(&mut self, changed_at: Timestamp) -> Result<(), DomainError> {
        self.touch(changed_at)?;
        self.execution_gate = ProjectExecutionGate::Open;
        Ok(())
    }

    /// Stops new scheduling for this Project and records a Project revision.
    pub fn stop_execution(&mut self, changed_at: Timestamp) -> Result<(), DomainError> {
        self.touch(changed_at)?;
        self.execution_gate = ProjectExecutionGate::Stopped;
        Ok(())
    }

    /// Records a Project-level mutation to a child aggregate atomically with
    /// the enclosing command/event/outbox transaction.
    pub fn record_child_mutation(&mut self, changed_at: Timestamp) -> Result<(), DomainError> {
        self.touch(changed_at)
    }

    /// Revalidates a deserialized Project snapshot before storage exposes it.
    ///
    /// # Errors
    ///
    /// Returns a domain error when durable policy or revision state violates
    /// the same structural constraints used by the Project aggregate.
    pub fn validate_snapshot(&self) -> Result<(), DomainError> {
        self.id.validate_v7("project.id")?;
        if self.revision == 0 {
            return Err(DomainError::InvalidValue {
                field: "project.revision",
                reason: "must be greater than zero".to_owned(),
            });
        }
        if self.updated_at < self.created_at {
            return Err(DomainError::NonMonotonicTimestamp);
        }
        self.priority_scheme.validate_snapshot()?;
        self.cancellation_reasons.validate_snapshot()?;
        self.property_schema.validate_snapshot()?;
        let _ = Self::new(self.id, self.name.clone(), self.created_at)?;
        Ok(())
    }

    fn touch(&mut self, changed_at: Timestamp) -> Result<(), DomainError> {
        if changed_at < self.updated_at {
            return Err(DomainError::NonMonotonicTimestamp);
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| DomainError::InvalidValue {
                field: "project.revision",
                reason: "cannot exceed u64::MAX".to_owned(),
            })?;
        self.updated_at = changed_at;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Project, ProjectExecutionGate};
    use crate::{ProjectId, Timestamp};

    #[test]
    fn project_allocates_stable_task_keys_and_tracks_execution_gate() {
        let now = Timestamp::now_utc();
        let mut project = Project::new(ProjectId::new(), "M0", now).expect("project");
        assert_eq!(project.execution_gate(), ProjectExecutionGate::Stopped);
        assert_eq!(
            project.allocate_task_key(now).expect("key").to_string(),
            "TASK-001"
        );
        project.start_execution(now).expect("start");
        assert_eq!(project.execution_gate(), ProjectExecutionGate::Open);
    }
}
