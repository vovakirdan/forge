//! Immutable, bounded conversation input. Context Task never grants Task authority.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    ActorKind, CommunicationAssignmentRef, DomainError, EmployeeId, ExecutionAssignment, ProjectId,
    TaskId, Timestamp,
    evidence::{invalid, validate_uuid},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommunicationContextInput {
    pub schema_version: u16,
    pub context_snapshot_id: Uuid,
    pub project_id: ProjectId,
    pub employee_id: EmployeeId,
    pub run_id: Uuid,
    pub assignment: CommunicationAssignmentRef,
    pub context_task_id: Option<TaskId>,
    pub capability_grants: Vec<String>,
    pub source_message: super::EmployeeMessage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_context: Option<crate::knowledge::KnowledgeContextBundle>,
    pub created_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(transparent)]
pub struct CommunicationContext(CommunicationContextInput);

impl CommunicationContext {
    pub fn new(input: CommunicationContextInput) -> Result<Self, DomainError> {
        validate_uuid(input.context_snapshot_id, "communication.context_id")?;
        validate_uuid(input.run_id, "communication.run_id")?;
        input.project_id.validate_v7("communication.project_id")?;
        input.employee_id.validate_v7("communication.employee_id")?;
        if let Some(knowledge) = &input.knowledge_context {
            knowledge.validate()?;
            if knowledge.project_id != input.project_id
                || knowledge.employee_id != input.employee_id
            {
                return Err(invalid("communication.knowledge", "scope mismatch"));
            }
        }
        ExecutionAssignment::Communication(input.assignment.clone()).validate()?;
        if let Some(task) = input.context_task_id {
            task.validate_v7("communication.context_task_id")?;
        }
        let source = input.source_message.data();
        if input.schema_version != 3
            || source.project_id != input.project_id
            || source.employee_id != input.employee_id
            || source.id != input.assignment.source_message_id
            || source.thread_id != input.assignment.thread_id
            || source.target != super::MessageTarget::Inbox
            || (source.sender.kind() == ActorKind::Employee
                && source.sender.id().as_uuid() == input.employee_id.as_uuid())
            || input.created_at < source.created_at
            || input.capability_grants.iter().any(|grant| {
                !matches!(
                    grant.as_str(),
                    "inbox.list"
                        | "inbox.acknowledge"
                        | "inbox.reply"
                        | "communication.complete"
                        | "escalation.raise"
                        | "board.list"
                        | "task.read"
                        | "memory.search"
                        | "memory.read"
                        | "memory.refresh"
                )
            })
        {
            return Err(invalid(
                "communication.context",
                "invalid source, schema, or purpose grant",
            ));
        }
        Ok(Self(input))
    }

    pub fn data(&self) -> &CommunicationContextInput {
        &self.0
    }
}

impl<'de> Deserialize<'de> for CommunicationContext {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(CommunicationContextInput::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}
