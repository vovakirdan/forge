use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{DeliveryRequirement, EmployeeMessage, MessageInput, MessageKind, MessageTarget};
use crate::evidence::{invalid, validate_uuid};
use crate::{Actor, DomainError, EmployeeId, ProjectId, TaskId, Timestamp};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmployeeThreadInput {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub employee_id: EmployeeId,
    pub task_id: Option<TaskId>,
    pub created_by: Actor,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub revision: u64,
    pub last_sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct EmployeeThread(EmployeeThreadInput);

/// The command boundary supplies authenticated sender and canonical time.
pub struct SendMessageInput {
    pub id: Uuid,
    pub expected_thread_revision: u64,
    pub sender: Actor,
    pub target: MessageTarget,
    pub kind: MessageKind,
    pub requirement: DeliveryRequirement,
    pub body: String,
    pub reply_to: Option<Uuid>,
    pub created_at: Timestamp,
}

impl EmployeeThread {
    pub fn new(input: EmployeeThreadInput) -> Result<Self, DomainError> {
        validate_uuid(input.id, "thread.id")?;
        input.project_id.validate_v7("thread.project_id")?;
        input.employee_id.validate_v7("thread.employee_id")?;
        input.created_by.validate_snapshot()?;
        if let Some(id) = input.task_id {
            id.validate_v7("thread.task_id")?;
        }
        if input.revision == 0
            || input.updated_at < input.created_at
            || input.last_sequence.checked_add(1) != Some(input.revision)
        {
            return Err(invalid("thread", "invalid revision, sequence or timestamp"));
        }
        Ok(Self(input))
    }

    pub fn data(&self) -> &EmployeeThreadInput {
        &self.0
    }

    /// Builds the next immutable message; rejection leaves the thread unchanged.
    pub fn send(&mut self, input: SendMessageInput) -> Result<EmployeeMessage, DomainError> {
        if self.0.revision != input.expected_thread_revision {
            return Err(invalid(
                "thread.revision",
                "does not match expected revision",
            ));
        }
        if input.created_at < self.0.updated_at {
            return Err(invalid("message.created_at", "precedes thread update"));
        }
        if let (Some(task), Some(context)) = (self.0.task_id, input.target.task_context())
            && task != context.task_id
        {
            return Err(invalid("message.target", "does not match the thread Task"));
        }
        let sequence = self
            .0
            .last_sequence
            .checked_add(1)
            .ok_or(DomainError::RevisionOverflow)?;
        let revision = self
            .0
            .revision
            .checked_add(1)
            .ok_or(DomainError::RevisionOverflow)?;
        let message = EmployeeMessage::new(MessageInput {
            id: input.id,
            thread_id: self.0.id,
            project_id: self.0.project_id,
            employee_id: self.0.employee_id,
            sequence,
            sender: input.sender,
            target: input.target,
            kind: input.kind,
            requirement: input.requirement,
            body: input.body,
            reply_to: input.reply_to,
            created_at: input.created_at,
        })?;
        self.0.last_sequence = sequence;
        self.0.revision = revision;
        self.0.updated_at = input.created_at;
        Ok(message)
    }
}

impl<'de> Deserialize<'de> for EmployeeThread {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(EmployeeThreadInput::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
