use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{Actor, DomainError, EmployeeId, PipelineVersionId, ProjectId, StageId, Timestamp};

/// Stable role key that communicates an Employee's Project specialization.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EmployeeRole(String);

impl EmployeeRole {
    /// Builds a role key from non-blank display-safe text.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidValue`] for blank or overly long values.
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        validate_non_blank("employee.role", &value, 128)?;
        Ok(Self(value))
    }

    /// Returns the role key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(&self) -> Result<(), DomainError> {
        Self::new(self.0.clone()).map(|_| ())
    }
}

/// Scheduling state of an Employee identity.
///
/// Operational projections such as `busy` and `unreachable` intentionally do
/// not appear here: they belong to Runs and Supervisor observations.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmployeeState {
    /// Eligible for new Run assignment subject to stage eligibility.
    Enabled,
    /// Temporarily ineligible for new Run assignment.
    Disabled,
    /// Permanently ineligible while history remains retained.
    Retired,
}

/// One Pipeline-version-scoped stage selector used by Employee eligibility.
///
/// A bare [`StageId`] is only stable within a Pipeline version; pairing it with
/// its immutable version prevents an allow-list entry for one `review` stage
/// from accidentally applying to another version's different `review` stage.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct StageEligibilityTarget {
    pipeline_version_id: PipelineVersionId,
    stage_id: StageId,
}

impl StageEligibilityTarget {
    /// Creates one immutable Pipeline-version-scoped stage selector.
    #[must_use]
    pub const fn new(pipeline_version_id: PipelineVersionId, stage_id: StageId) -> Self {
        Self {
            pipeline_version_id,
            stage_id,
        }
    }

    /// Returns the selected Pipeline version.
    #[must_use]
    pub const fn pipeline_version_id(&self) -> PipelineVersionId {
        self.pipeline_version_id
    }

    /// Returns the selected stage inside that Pipeline version.
    #[must_use]
    pub fn stage_id(&self) -> &StageId {
        &self.stage_id
    }

    fn validate_snapshot(&self) -> Result<(), DomainError> {
        self.pipeline_version_id
            .validate_v7("employee.stage_eligibility.pipeline_version_id")?;
        self.stage_id.validate()
    }
}

/// Stage subset in which an enabled Employee may be scheduled.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "stages", rename_all = "snake_case")]
pub enum StageEligibility {
    /// The Employee may work in every stage permitted by later policy checks.
    Any,
    /// The Employee may work only in the listed Pipeline-version stages.
    Only(BTreeSet<StageEligibilityTarget>),
}

impl StageEligibility {
    /// Builds a non-empty allow-list of Pipeline-version stage selectors.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidValue`] for an empty list, because it is
    /// indistinguishable from a disabled Employee at scheduling time.
    pub fn only(
        stages: impl IntoIterator<Item = StageEligibilityTarget>,
    ) -> Result<Self, DomainError> {
        let stages = stages.into_iter().collect::<BTreeSet<_>>();
        if stages.is_empty() {
            return Err(DomainError::InvalidValue {
                field: "employee.stage_eligibility",
                reason: "only mode must contain at least one stage".to_owned(),
            });
        }
        Ok(Self::Only(stages))
    }

    /// Returns whether the stage is allowed by this eligibility rule.
    #[must_use]
    pub fn allows(&self, pipeline_version_id: PipelineVersionId, stage_id: &StageId) -> bool {
        match self {
            Self::Any => true,
            Self::Only(stages) => stages.iter().any(|target| {
                target.pipeline_version_id() == pipeline_version_id && target.stage_id() == stage_id
            }),
        }
    }

    fn validate_snapshot(&self) -> Result<(), DomainError> {
        match self {
            Self::Any => Ok(()),
            Self::Only(stages) => {
                for stage in stages {
                    stage.validate_snapshot()?;
                }
                Self::only(stages.clone()).map(|_| ())
            }
        }
    }
}

/// Project-scoped executor identity.
///
/// An Employee is not a provider process or a Run. Its state controls only new
/// scheduling eligibility; Core remains the only actor that changes Task state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Employee {
    id: EmployeeId,
    project_id: ProjectId,
    name: String,
    role: EmployeeRole,
    state: EmployeeState,
    stage_eligibility: StageEligibility,
    created_by: Actor,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl Employee {
    /// First and only aggregate revision for the immutable M0 Employee
    /// catalog entry. Later employee-management commands must introduce a
    /// durable aggregate revision before they can mutate this snapshot.
    pub const INITIAL_REVISION: u64 = 1;

    /// Creates an enabled Employee identity.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidValue`] for a blank or overly long name.
    pub fn new(
        id: EmployeeId,
        project_id: ProjectId,
        name: impl Into<String>,
        role: EmployeeRole,
        stage_eligibility: StageEligibility,
        created_by: Actor,
        created_at: Timestamp,
    ) -> Result<Self, DomainError> {
        let name = name.into();
        validate_non_blank("employee.name", &name, 200)?;

        Ok(Self {
            id,
            project_id,
            name,
            role,
            state: EmployeeState::Enabled,
            stage_eligibility,
            created_by,
            created_at,
            updated_at: created_at,
        })
    }

    /// Returns the immutable Employee identity.
    #[must_use]
    pub const fn id(&self) -> EmployeeId {
        self.id
    }

    /// Returns the owning Project.
    #[must_use]
    pub const fn project_id(&self) -> ProjectId {
        self.project_id
    }

    /// Returns the current display name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the specialization role.
    #[must_use]
    pub fn role(&self) -> &EmployeeRole {
        &self.role
    }

    /// Returns the scheduling state.
    #[must_use]
    pub const fn state(&self) -> EmployeeState {
        self.state
    }

    /// Returns the stage eligibility rule.
    #[must_use]
    pub fn stage_eligibility(&self) -> &StageEligibility {
        &self.stage_eligibility
    }

    /// Returns whether this Employee can receive a new Run in the exact
    /// Pipeline-version-scoped stage.
    #[must_use]
    pub fn is_eligible_for(
        &self,
        pipeline_version_id: PipelineVersionId,
        stage_id: &StageId,
    ) -> bool {
        self.state == EmployeeState::Enabled
            && self.stage_eligibility.allows(pipeline_version_id, stage_id)
    }

    /// Temporarily prevents new assignment while retaining identity and history.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidValue`] if a caller supplies a timestamp
    /// before the most recently recorded Employee change.
    pub fn disable(&mut self, changed_at: Timestamp) -> Result<(), DomainError> {
        self.ensure_monotonic_timestamp(changed_at)?;
        if self.state != EmployeeState::Retired {
            self.state = EmployeeState::Disabled;
            self.updated_at = changed_at;
        }
        Ok(())
    }

    /// Re-enables a previously disabled Employee.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidValue`] if the Employee is retired or the
    /// timestamp predates the current state.
    pub fn enable(&mut self, changed_at: Timestamp) -> Result<(), DomainError> {
        self.ensure_monotonic_timestamp(changed_at)?;
        if self.state == EmployeeState::Retired {
            return Err(DomainError::InvalidValue {
                field: "employee.state",
                reason: "a retired employee cannot be re-enabled".to_owned(),
            });
        }
        self.state = EmployeeState::Enabled;
        self.updated_at = changed_at;
        Ok(())
    }

    /// Retires the Employee permanently while preserving all history.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::NonMonotonicTimestamp`] if the timestamp predates
    /// the current state.
    pub fn retire(&mut self, changed_at: Timestamp) -> Result<(), DomainError> {
        self.ensure_monotonic_timestamp(changed_at)?;
        self.state = EmployeeState::Retired;
        self.updated_at = changed_at;
        Ok(())
    }

    /// Returns the actor that created this identity.
    #[must_use]
    pub const fn created_by(&self) -> Actor {
        self.created_by
    }

    /// Returns creation time.
    #[must_use]
    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    /// Returns the latest mutation time.
    #[must_use]
    pub const fn updated_at(&self) -> Timestamp {
        self.updated_at
    }

    /// Revalidates a deserialized Employee snapshot before storage exposes it.
    pub fn validate_snapshot(&self) -> Result<(), DomainError> {
        self.id.validate_v7("employee.id")?;
        self.project_id.validate_v7("employee.project_id")?;
        self.role.validate()?;
        self.stage_eligibility.validate_snapshot()?;
        self.created_by.validate_snapshot()?;
        if self.updated_at < self.created_at {
            return Err(DomainError::NonMonotonicTimestamp);
        }
        let _ = Self::new(
            self.id,
            self.project_id,
            self.name.clone(),
            self.role.clone(),
            self.stage_eligibility.clone(),
            self.created_by,
            self.created_at,
        )?;
        Ok(())
    }

    fn ensure_monotonic_timestamp(&self, changed_at: Timestamp) -> Result<(), DomainError> {
        if changed_at < self.updated_at {
            return Err(DomainError::NonMonotonicTimestamp);
        }
        Ok(())
    }
}

fn validate_non_blank(field: &'static str, value: &str, max_len: usize) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        return Err(DomainError::InvalidValue {
            field,
            reason: "must not be blank".to_owned(),
        });
    }
    if value.chars().count() > max_len {
        return Err(DomainError::InvalidValue {
            field,
            reason: format!("must not exceed {max_len} characters"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Employee, EmployeeRole, EmployeeState, StageEligibility, StageEligibilityTarget};
    use crate::{Actor, ActorId, EmployeeId, PipelineVersionId, ProjectId, StageId, Timestamp};

    #[test]
    fn disabled_employee_is_not_eligible_for_a_permitted_stage() {
        let stage = StageId::new("review").expect("valid stage");
        let version = PipelineVersionId::new();
        let mut employee = Employee::new(
            EmployeeId::new(),
            ProjectId::new(),
            "Rita",
            EmployeeRole::new("reviewer").expect("valid role"),
            StageEligibility::only([StageEligibilityTarget::new(version, stage.clone())])
                .expect("non-empty stages"),
            Actor::human(ActorId::new()),
            Timestamp::now_utc(),
        )
        .expect("valid employee");

        employee
            .disable(Timestamp::now_utc())
            .expect("disable employee");

        assert!(!employee.is_eligible_for(version, &stage));
    }

    #[test]
    fn stage_allow_list_does_not_cross_pipeline_versions() {
        let stage = StageId::new("review").expect("valid stage");
        let allowed_version = PipelineVersionId::new();
        let other_version = PipelineVersionId::new();
        let employee = Employee::new(
            EmployeeId::new(),
            ProjectId::new(),
            "Rita",
            EmployeeRole::new("reviewer").expect("valid role"),
            StageEligibility::only([StageEligibilityTarget::new(allowed_version, stage.clone())])
                .expect("non-empty stages"),
            Actor::human(ActorId::new()),
            Timestamp::now_utc(),
        )
        .expect("valid employee");

        assert!(employee.is_eligible_for(allowed_version, &stage));
        assert!(!employee.is_eligible_for(other_version, &stage));
    }

    #[test]
    fn retired_employee_cannot_be_enabled_again() {
        let mut employee = Employee::new(
            EmployeeId::new(),
            ProjectId::new(),
            "Rita",
            EmployeeRole::new("reviewer").expect("valid role"),
            StageEligibility::Any,
            Actor::human(ActorId::new()),
            Timestamp::now_utc(),
        )
        .expect("valid employee");
        employee
            .retire(Timestamp::now_utc())
            .expect("retire employee");

        let result = employee.enable(Timestamp::now_utc());

        assert!(result.is_err());
        assert_eq!(employee.state(), EmployeeState::Retired);
    }
}
