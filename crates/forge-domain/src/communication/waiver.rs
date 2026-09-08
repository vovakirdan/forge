use super::{DeliveryRequirement, EmployeeMessage};
use crate::{
    Actor, DomainError, ProjectId, Timestamp,
    evidence::{invalid, validate_uuid},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An operator decision removes a requirement; it is not an Employee receipt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageRequirementWaiver {
    pub message_id: Uuid,
    pub project_id: ProjectId,
    pub actor: Actor,
    pub reason: String,
    pub created_at: Timestamp,
}
impl MessageRequirementWaiver {
    pub fn new(
        message: &EmployeeMessage,
        actor: Actor,
        reason: String,
        created_at: Timestamp,
    ) -> Result<Self, DomainError> {
        if message.data().requirement == DeliveryRequirement::Informational
            || created_at < message.data().created_at
        {
            return Err(invalid(
                "message.waiver",
                "requires an existing acknowledgement or answer requirement",
            ));
        }
        let waiver = Self {
            message_id: message.data().id,
            project_id: message.data().project_id,
            actor,
            reason,
            created_at,
        };
        waiver.validate_snapshot()?;
        Ok(waiver)
    }
    pub fn validate_snapshot(&self) -> Result<(), DomainError> {
        validate_uuid(self.message_id, "waiver.message_id")?;
        self.project_id.validate_v7("waiver.project_id")?;
        self.actor.validate_snapshot()?;
        if self.reason.trim().is_empty() || self.reason.len() > 4096 {
            return Err(invalid(
                "waiver.reason",
                "requires nonblank text up to 4096 bytes",
            ));
        }
        Ok(())
    }
}
