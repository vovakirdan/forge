use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{DeliveryRequirement, EmployeeMessage, MessageTarget, TaskMessageContext};
use crate::evidence::{invalid, validate_uuid};
use crate::{DomainError, EmployeeId, ProjectId, Timestamp};

/// Supplied from authenticated execution state, never from an Employee tool body.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryScope {
    pub project_id: ProjectId,
    pub employee_id: EmployeeId,
    pub thread_id: Uuid,
    pub run_id: Uuid,
    pub fencing_token: u64,
    pub environment_epoch: u64,
    pub task_context: Option<TaskMessageContext>,
}

impl DeliveryScope {
    pub fn validate_message(&self, message: &EmployeeMessage) -> Result<(), DomainError> {
        validate_uuid(self.run_id, "delivery.run_id")?;
        if self.fencing_token == 0 || self.environment_epoch == 0 {
            return Err(invalid("delivery", "requires positive fence and epoch"));
        }
        let data = message.data();
        if matches!(data.target, MessageTarget::Inbox) && self.task_context.is_some() {
            return Err(invalid(
                "delivery.assignment",
                "Inbox requires a Communication assignment",
            ));
        }
        if self.project_id != data.project_id
            || self.employee_id != data.employee_id
            || self.thread_id != data.thread_id
        {
            return Err(invalid(
                "delivery.scope",
                "does not match message recipient",
            ));
        }
        if let Some(context) = data.target.task_context()
            && self.task_context.as_ref() != Some(context)
        {
            return Err(invalid(
                "delivery.task_context",
                "does not match target visit",
            ));
        }
        if let MessageTarget::ExactRun {
            run_id,
            fencing_token,
            environment_epoch,
            ..
        } = data.target
            && (run_id, fencing_token, environment_epoch)
                != (self.run_id, self.fencing_token, self.environment_epoch)
        {
            return Err(invalid(
                "delivery.run_scope",
                "does not match exact Run target",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryReceiptKind {
    RuntimeAccepted,
    Acknowledged,
    Answered,
}

/// An attributed observation, not proof the model understood or obeyed an instruction.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeliveryReceipt {
    message_id: Uuid,
    scope: DeliveryScope,
    kind: DeliveryReceiptKind,
    reply_id: Option<Uuid>,
    created_at: Timestamp,
}

impl DeliveryReceipt {
    /// Caller must additionally prove this scope is still live at transactional apply time.
    pub fn new(
        message: &EmployeeMessage,
        scope: DeliveryScope,
        kind: DeliveryReceiptKind,
        reply_id: Option<Uuid>,
        created_at: Timestamp,
    ) -> Result<Self, DomainError> {
        scope.validate_message(message)?;
        if created_at < message.data().created_at {
            return Err(invalid("delivery.created_at", "precedes message creation"));
        }
        if (kind == DeliveryReceiptKind::Answered) != reply_id.is_some() {
            return Err(invalid("delivery.reply_id", "required only for an answer"));
        }
        if let Some(id) = reply_id {
            validate_uuid(id, "delivery.reply_id")?;
        }
        Ok(Self {
            message_id: message.data().id,
            scope,
            kind,
            reply_id,
            created_at,
        })
    }

    pub fn message_id(&self) -> Uuid {
        self.message_id
    }
    pub fn scope(&self) -> &DeliveryScope {
        &self.scope
    }
    pub fn kind(&self) -> DeliveryReceiptKind {
        self.kind
    }
    pub fn reply_id(&self) -> Option<Uuid> {
        self.reply_id
    }
    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub fn satisfies(&self, message: &EmployeeMessage) -> bool {
        if self.message_id != message.data().id || self.scope.validate_message(message).is_err() {
            return false;
        }
        match message.data().requirement {
            DeliveryRequirement::Informational => true,
            DeliveryRequirement::Acknowledged => self.kind != DeliveryReceiptKind::RuntimeAccepted,
            DeliveryRequirement::Answered => self.kind == DeliveryReceiptKind::Answered,
        }
    }
}
