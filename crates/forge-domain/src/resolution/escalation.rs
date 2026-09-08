use crate::{Actor, ArtifactId, DomainError, ProjectId, Timestamp, evidence::validate_uuid};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{EscalationSource, ResolverRoute, invalid, text};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscalationCategory {
    ActionApproval,
    Clarification,
    ScopeOrPolicyConflict,
    StaleOrInvalidTask,
    TechnicalDecision,
    Blocked,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum EscalationState {
    Queued,
    Assigned { assignment_id: Uuid },
    Resolved { artifact_id: ArtifactId },
    NeedsManagementChange { artifact_id: ArtifactId },
    Superseded { reason: String },
}

impl EscalationState {
    pub fn key(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Assigned { .. } => "assigned",
            Self::Resolved { .. } => "resolved",
            Self::NeedsManagementChange { .. } => "needs_management_change",
            Self::Superseded { .. } => "superseded",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Escalation {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub source: EscalationSource,
    pub category: EscalationCategory,
    pub question: String,
    pub route: Option<ResolverRoute>,
    pub allowed_outcomes: Vec<String>,
    pub created_by: Actor,
    pub created_at: Timestamp,
    pub revision: u64,
    pub generation: u64,
    pub next_candidate: u32,
    pub state: EscalationState,
}

impl Escalation {
    pub fn validate_snapshot(&self) -> Result<(), DomainError> {
        validate_uuid(self.id, "escalation.id")?;
        self.project_id.validate_v7("escalation.project_id")?;
        self.source.validate()?;
        if self.source.communication().is_some() && !self.allowed_outcomes.is_empty() {
            return Err(invalid(
                "Communication context grants no Pipeline outcome authority",
            ));
        }
        self.created_by.validate_snapshot()?;
        text(&self.question, 20_000)?;
        if self.revision == 0 || self.allowed_outcomes.len() > 128 || self.next_candidate > 32 {
            return Err(invalid("invalid escalation revision or route cursor"));
        }
        if let Some(route) = &self.route {
            route.validate_snapshot()?;
            if route.project_id != self.project_id {
                return Err(invalid("route belongs to another Project"));
            }
        }
        for key in &self.allowed_outcomes {
            crate::ids::validate_stable_key("escalation.outcome", key)?;
        }
        match self.state {
            EscalationState::Queued => {}
            EscalationState::Assigned { assignment_id } => {
                validate_uuid(assignment_id, "escalation.assignment_id")?;
                if self.generation == 0 {
                    return Err(invalid("assigned escalation requires a generation"));
                }
            }
            EscalationState::Resolved { artifact_id }
            | EscalationState::NeedsManagementChange { artifact_id } => {
                artifact_id.validate_v7("escalation.artifact_id")?
            }
            EscalationState::Superseded { ref reason } => text(reason, 10_000)?,
        }
        Ok(())
    }

    pub fn requires_human(&self) -> bool {
        self.category == EscalationCategory::ActionApproval
            || self
                .route
                .as_ref()
                .is_none_or(|route| self.next_candidate as usize >= route.employee_ids.len())
    }

    pub fn advance(&mut self, state: EscalationState) -> Result<(), DomainError> {
        let allowed = matches!(
            (&self.state, &state),
            (
                EscalationState::Queued,
                EscalationState::Queued
                    | EscalationState::Assigned { .. }
                    | EscalationState::Superseded { .. }
            ) | (EscalationState::Assigned { .. }, _)
                | (
                    EscalationState::NeedsManagementChange { .. },
                    EscalationState::Queued
                        | EscalationState::Assigned { .. }
                        | EscalationState::Superseded { .. }
                )
        );
        if !allowed {
            return Err(invalid("invalid escalation transition"));
        }
        let mut updated = self.clone();
        updated.revision = updated
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("revision exhausted"))?;
        updated.state = state;
        updated.validate_snapshot()?;
        *self = updated;
        Ok(())
    }
}
