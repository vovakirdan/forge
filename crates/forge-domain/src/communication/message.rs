use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::evidence::{invalid, validate_uuid};
use crate::{
    Actor, DomainError, EmployeeId, PipelineVersionId, ProjectId, StageId, TaskId, Timestamp,
};

/// Bounds canonical message bodies independently of provider context limits.
pub const MAX_MESSAGE_BYTES: usize = 32 * 1024;

/// The exact Pipeline visit addressed by an instruction, not a general Task grant.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskMessageContext {
    pub task_id: TaskId,
    pub pipeline_version_id: PipelineVersionId,
    pub stage_id: StageId,
    pub stage_visit: u64,
}

impl TaskMessageContext {
    pub fn validate(&self) -> Result<(), DomainError> {
        self.task_id.validate_v7("message.task_id")?;
        self.pipeline_version_id
            .validate_v7("message.pipeline_version_id")?;
        self.stage_id.validate()?;
        if self.stage_visit == 0 {
            return Err(invalid("message.stage_visit", "must be positive"));
        }
        Ok(())
    }
}

/// A recipient identity alone never selects an arbitrary active worker process.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MessageTarget {
    Inbox,
    TaskExecution {
        context: TaskMessageContext,
    },
    ExactRun {
        context: TaskMessageContext,
        run_id: Uuid,
        fencing_token: u64,
        environment_epoch: u64,
    },
}

impl MessageTarget {
    pub fn task_context(&self) -> Option<&TaskMessageContext> {
        match self {
            Self::Inbox => None,
            Self::TaskExecution { context } | Self::ExactRun { context, .. } => Some(context),
        }
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if let Some(context) = self.task_context() {
            context.validate()?;
        }
        if let Self::ExactRun {
            run_id,
            fencing_token,
            environment_epoch,
            ..
        } = self
        {
            validate_uuid(*run_id, "message.run_id")?;
            if *fencing_token == 0 || *environment_epoch == 0 {
                return Err(invalid(
                    "message.run_scope",
                    "requires positive fence and epoch",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Instruction,
    Question,
    Notification,
    Reply,
}

/// Runtime transport acceptance alone never satisfies an Employee requirement.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryRequirement {
    Informational,
    Acknowledged,
    Answered,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageInput {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub project_id: ProjectId,
    pub employee_id: EmployeeId,
    pub sequence: u64,
    pub sender: Actor,
    pub target: MessageTarget,
    pub kind: MessageKind,
    pub requirement: DeliveryRequirement,
    pub body: String,
    pub reply_to: Option<Uuid>,
    pub created_at: Timestamp,
}

impl std::fmt::Debug for MessageInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageInput")
            .field("id", &self.id)
            .field("thread_id", &self.thread_id)
            .field("sequence", &self.sequence)
            .field("kind", &self.kind)
            .field("body", &"[redacted]")
            .finish_non_exhaustive()
    }
}

/// Immutable, validated canonical input. Debug output excludes message text.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct EmployeeMessage(MessageInput);

impl EmployeeMessage {
    pub fn new(input: MessageInput) -> Result<Self, DomainError> {
        validate_uuid(input.id, "message.id")?;
        validate_uuid(input.thread_id, "message.thread_id")?;
        input.project_id.validate_v7("message.project_id")?;
        input.employee_id.validate_v7("message.employee_id")?;
        input.sender.validate_snapshot()?;
        input.target.validate()?;
        if input.sequence == 0
            || input.body.trim().is_empty()
            || input.body.len() > MAX_MESSAGE_BYTES
        {
            return Err(invalid(
                "message",
                "requires a positive sequence and bounded nonblank body",
            ));
        }
        if let Some(id) = input.reply_to {
            validate_uuid(id, "message.reply_to")?;
            if id == input.id {
                return Err(invalid("message.reply_to", "cannot reference itself"));
            }
        }
        if (input.kind == MessageKind::Reply) != input.reply_to.is_some() {
            return Err(invalid("message.reply_to", "required only for replies"));
        }
        if matches!(input.kind, MessageKind::Reply | MessageKind::Notification)
            && input.requirement != DeliveryRequirement::Informational
        {
            return Err(invalid(
                "message.requirement",
                "replies and notifications do not gate completion",
            ));
        }
        Ok(Self(input))
    }

    pub fn data(&self) -> &MessageInput {
        &self.0
    }

    pub fn gates_completion(&self, context: &TaskMessageContext) -> bool {
        self.0.requirement != DeliveryRequirement::Informational
            && self.0.target.task_context() == Some(context)
    }
}

impl<'de> Deserialize<'de> for EmployeeMessage {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(MessageInput::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
