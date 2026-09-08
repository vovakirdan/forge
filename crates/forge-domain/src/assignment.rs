//! Execution ownership is independent of optional context and runtime transport.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{DomainError, PipelineVersionId, StageId, TaskId, evidence::validate_uuid};

/// Task-stage authority belongs only to this exact queue admission.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskStageAssignment {
    pub task_id: TaskId,
    pub queue_entry_id: Uuid,
    pub stage_id: StageId,
}

/// A bounded conversation turn; neither its thread nor its context grants Task writes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommunicationAssignmentRef {
    pub assignment_id: Uuid,
    pub thread_id: Uuid,
    pub source_message_id: Uuid,
}

/// One fenced resolver lease, independent of any contextual Task or conversation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionAssignmentRef {
    pub assignment_id: Uuid,
    pub escalation_id: Uuid,
    pub lease_generation: u64,
}

/// A System command's contextual Task does not confer a Task writer lease.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookAssignmentRef {
    pub invocation_id: Uuid,
    pub task_id: TaskId,
    pub pipeline_version_id: PipelineVersionId,
    pub stage_id: StageId,
    pub stage_visit: u64,
    pub candidate_proposal_id: Uuid,
}

/// Closed purpose discriminator. A contextual Task never changes execution ownership.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "purpose",
    content = "owner",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ExecutionAssignment {
    TaskStage(TaskStageAssignment),
    Communication(CommunicationAssignmentRef),
    Resolution(ResolutionAssignmentRef),
    Hook(HookAssignmentRef),
}

impl ExecutionAssignment {
    pub fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::TaskStage(owner) => {
                owner.task_id.validate_v7("assignment.task_id")?;
                validate_uuid(owner.queue_entry_id, "assignment.queue_entry_id")?;
                owner.stage_id.validate()
            }
            Self::Communication(owner) => {
                validate_uuid(owner.assignment_id, "assignment.id")?;
                validate_uuid(owner.thread_id, "assignment.thread_id")?;
                validate_uuid(owner.source_message_id, "assignment.source_message_id")
            }
            Self::Resolution(owner) => {
                validate_uuid(owner.assignment_id, "assignment.id")?;
                validate_uuid(owner.escalation_id, "assignment.escalation_id")?;
                if owner.lease_generation == 0 {
                    return Err(crate::evidence::invalid(
                        "assignment.lease_generation",
                        "must be positive",
                    ));
                }
                Ok(())
            }
            Self::Hook(owner) => {
                validate_uuid(owner.invocation_id, "assignment.invocation_id")?;
                validate_uuid(
                    owner.candidate_proposal_id,
                    "assignment.candidate_proposal_id",
                )?;
                owner.task_id.validate_v7("assignment.task_id")?;
                owner
                    .pipeline_version_id
                    .validate_v7("assignment.pipeline_version_id")?;
                owner.stage_id.validate()?;
                if owner.stage_visit == 0 {
                    return Err(crate::evidence::invalid(
                        "assignment.stage_visit",
                        "must be positive",
                    ));
                }
                Ok(())
            }
        }
    }

    pub fn task_stage(&self) -> Option<&TaskStageAssignment> {
        match self {
            Self::TaskStage(owner) => Some(owner),
            _ => None,
        }
    }

    pub fn communication(&self) -> Option<&CommunicationAssignmentRef> {
        match self {
            Self::Communication(owner) => Some(owner),
            _ => None,
        }
    }

    pub fn purpose(&self) -> &'static str {
        match self {
            Self::TaskStage(_) => "task_stage",
            Self::Communication(_) => "communication",
            Self::Resolution(_) => "resolution",
            Self::Hook(_) => "hook",
        }
    }

    pub fn resolution(&self) -> Option<&ResolutionAssignmentRef> {
        match self {
            Self::Resolution(owner) => Some(owner),
            _ => None,
        }
    }
    pub fn hook(&self) -> Option<&HookAssignmentRef> {
        match self {
            Self::Hook(owner) => Some(owner),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn communication_has_no_task_authority() {
        let value = ExecutionAssignment::Communication(CommunicationAssignmentRef {
            assignment_id: Uuid::now_v7(),
            thread_id: Uuid::now_v7(),
            source_message_id: Uuid::now_v7(),
        });
        assert!(value.validate().is_ok());
        assert!(value.task_stage().is_none());
        assert_eq!(
            serde_json::from_value::<ExecutionAssignment>(serde_json::to_value(&value).unwrap())
                .unwrap(),
            value
        );
    }

    #[test]
    fn unknown_purpose_and_cross_purpose_fields_are_rejected() {
        let id = Uuid::now_v7();
        assert!(
            serde_json::from_value::<ExecutionAssignment>(
                serde_json::json!({"purpose":"other","owner":{}})
            )
            .is_err()
        );
        assert!(serde_json::from_value::<ExecutionAssignment>(serde_json::json!({
            "purpose":"communication", "owner":{"assignment_id":id,"thread_id":id,"source_message_id":id,"task_id":id}
        })).is_err());
    }
}
