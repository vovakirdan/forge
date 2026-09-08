use crate::{
    Actor, ArtifactId, DomainError, EmployeeId, ProjectId, Timestamp, evidence::validate_uuid,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{invalid, text};

/// The coordinator owns this lease. It grants no Task-stage execution rights.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionLease {
    pub generation: u64,
    pub held_by: Actor,
    pub expires_at: Option<Timestamp>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Resolver {
    Human,
    Employee { employee_id: EmployeeId },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionDisposition {
    ContinueStage,
    NeedsManagementChange,
    ForwardToHuman,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionAnswer {
    pub disposition: ResolutionDisposition,
    pub summary: String,
    #[serde(default)]
    pub recommended_outcome_key: Option<String>,
}

impl ResolutionAnswer {
    pub fn validate(&self, allowed_outcomes: &[String]) -> Result<(), DomainError> {
        text(&self.summary, 20_000)?;
        if self
            .recommended_outcome_key
            .as_ref()
            .is_some_and(|key| !allowed_outcomes.contains(key))
        {
            return Err(invalid(
                "recommended outcome is absent from the pinned stage",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResolutionAssignmentState {
    Active,
    Answered { artifact_id: ArtifactId },
    Retired { reason: String },
}

impl ResolutionAssignmentState {
    pub fn key(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Answered { .. } => "answered",
            Self::Retired { .. } => "retired",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionAssignment {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub escalation_id: Uuid,
    pub resolver: Resolver,
    pub lease: ResolutionLease,
    pub allowed_outcomes: Vec<String>,
    pub issued_at: Timestamp,
    pub state: ResolutionAssignmentState,
}

impl ResolutionAssignment {
    /// Admission must agree with the current generation and immutable question scope.
    pub fn validate_owner(&self, escalation: &super::Escalation) -> Result<(), DomainError> {
        self.validate_snapshot()?;
        if self.project_id != escalation.project_id
            || self.escalation_id != escalation.id
            || self.lease.generation != escalation.generation
            || self.allowed_outcomes != escalation.allowed_outcomes
            || escalation.state
                != (super::EscalationState::Assigned {
                    assignment_id: self.id,
                })
            || (escalation.category == super::EscalationCategory::ActionApproval
                && self.resolver != Resolver::Human)
        {
            return Err(invalid(
                "assignment differs from escalation or requires Human authority",
            ));
        }
        Ok(())
    }
    pub fn validate_snapshot(&self) -> Result<(), DomainError> {
        validate_uuid(self.id, "resolution_assignment.id")?;
        validate_uuid(self.escalation_id, "resolution_assignment.escalation_id")?;
        self.project_id
            .validate_v7("resolution_assignment.project_id")?;
        self.lease.held_by.validate_snapshot()?;
        if self.lease.generation == 0
            || self.lease.held_by.kind() != crate::ActorKind::SystemManager
            || self.lease.expires_at.is_some_and(|at| at <= self.issued_at)
            || self.allowed_outcomes.len() > 128
        {
            return Err(invalid("invalid assignment lease or outcome scope"));
        }
        for key in &self.allowed_outcomes {
            crate::ids::validate_stable_key("resolution.outcome", key)?;
        }
        match self.resolver {
            Resolver::Employee { employee_id } if self.lease.expires_at.is_some() => {
                employee_id.validate_v7("resolver.employee_id")?
            }
            Resolver::Human if self.lease.expires_at.is_none() => {}
            _ => return Err(invalid("only Employee assignments require an expiry")),
        }
        match &self.state {
            ResolutionAssignmentState::Active => {}
            ResolutionAssignmentState::Answered { artifact_id } => {
                artifact_id.validate_v7("resolution.artifact_id")?
            }
            ResolutionAssignmentState::Retired { reason } => text(reason, 10_000)?,
        }
        Ok(())
    }

    pub fn finish(&mut self, state: ResolutionAssignmentState) -> Result<(), DomainError> {
        if self.state != ResolutionAssignmentState::Active
            || state == ResolutionAssignmentState::Active
        {
            return Err(invalid("assignment can finish only once"));
        }
        let mut updated = self.clone();
        updated.state = state;
        updated.validate_snapshot()?;
        *self = updated;
        Ok(())
    }
}
