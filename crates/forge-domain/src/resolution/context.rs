//! Immutable resolver input grants question authority, never Task-stage execution.
use super::{Escalation, ResolutionAssignment, ResolutionAssignmentState, Resolver, invalid};
use crate::{DomainError, EmployeeId, ProjectId, Timestamp, evidence::validate_uuid};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionContextInput {
    pub schema_version: u16,
    pub context_snapshot_id: Uuid,
    pub project_id: ProjectId,
    pub employee_id: EmployeeId,
    pub run_id: Uuid,
    pub assignment: ResolutionAssignment,
    pub escalation: Escalation,
    pub capability_grants: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_context: Option<crate::knowledge::KnowledgeContextBundle>,
    pub created_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ResolutionContext(ResolutionContextInput);

impl ResolutionContext {
    pub fn new(input: ResolutionContextInput) -> Result<Self, DomainError> {
        validate_uuid(input.context_snapshot_id, "resolution.context_id")?;
        validate_uuid(input.run_id, "resolution.run_id")?;
        input.project_id.validate_v7("resolution.project_id")?;
        input.employee_id.validate_v7("resolution.employee_id")?;
        if let Some(knowledge) = &input.knowledge_context {
            knowledge.validate()?;
            if knowledge.project_id != input.project_id
                || knowledge.employee_id != input.employee_id
            {
                return Err(invalid("knowledge context scope mismatch"));
            }
        }
        input.escalation.validate_snapshot()?;
        input.assignment.validate_owner(&input.escalation)?;
        if input.schema_version != 4
            || input.project_id != input.escalation.project_id
            || input.assignment.state != ResolutionAssignmentState::Active
            || input.assignment.resolver
                != (Resolver::Employee {
                    employee_id: input.employee_id,
                })
            || input.created_at < input.assignment.issued_at
            || input.capability_grants.iter().any(|grant| {
                !matches!(
                    grant.as_str(),
                    "resolution.read"
                        | "resolution.submit"
                        | "resolution.decline"
                        | "board.list"
                        | "task.read"
                        | "memory.search"
                        | "memory.read"
                        | "memory.refresh"
                )
            })
        {
            return Err(invalid(
                "invalid resolver context, assignment, or capability",
            ));
        }
        Ok(Self(input))
    }
    pub fn data(&self) -> &ResolutionContextInput {
        &self.0
    }
}
impl<'de> Deserialize<'de> for ResolutionContext {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(ResolutionContextInput::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}
