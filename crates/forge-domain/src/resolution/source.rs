//! Question context is an exact source, never an implicit Task execution grant.
use crate::{
    CommunicationAssignmentRef, DomainError, ExecutionAssignment, PipelineVersionId, StageId,
    StageVisit, TaskId, WaitConditionId, evidence::validate_uuid,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskEscalationSource {
    pub task_id: TaskId,
    pub pipeline_version_id: PipelineVersionId,
    pub stage_id: StageId,
    pub stage_visit: StageVisit,
    pub wait_condition_id: WaitConditionId,
}

/// The source conversation may mention a Task, but does not own its stage/wait.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommunicationEscalationSource {
    pub assignment: CommunicationAssignmentRef,
    pub run_id: Uuid,
    pub fencing_token: u64,
    pub environment_epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EscalationSource {
    Task(TaskEscalationSource),
    Communication(CommunicationEscalationSource),
}

impl EscalationSource {
    pub fn task(&self) -> Option<&TaskEscalationSource> {
        match self {
            Self::Task(source) => Some(source),
            _ => None,
        }
    }
    pub fn communication(&self) -> Option<&CommunicationEscalationSource> {
        match self {
            Self::Communication(source) => Some(source),
            _ => None,
        }
    }
    pub fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::Task(source) => {
                source.task_id.validate_v7("escalation.task_id")?;
                source
                    .pipeline_version_id
                    .validate_v7("escalation.pipeline_version_id")?;
                source
                    .wait_condition_id
                    .validate_v7("escalation.wait_condition_id")?;
                source.stage_id.validate()
            }
            Self::Communication(source) => {
                ExecutionAssignment::Communication(source.assignment.clone()).validate()?;
                validate_uuid(source.run_id, "escalation.source_run_id")?;
                if source.fencing_token == 0 || source.environment_epoch == 0 {
                    return Err(super::invalid("source fence and epoch must be positive"));
                }
                Ok(())
            }
        }
    }
}
