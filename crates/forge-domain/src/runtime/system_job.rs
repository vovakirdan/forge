//! V7 executes one semantic job against frozen inputs in private scratch.
use super::{RuntimeBinding, RuntimeSpecError, SurfaceSpec};
use crate::{
    ProjectId,
    system_job::{SystemJobAssignmentRef, SystemJobInput},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const SYSTEM_JOB_RUN_SPEC_VERSION: u16 = 7;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SystemJobRunSpec {
    pub schema_version: u16,
    pub project_id: ProjectId,
    pub run_id: Uuid,
    pub assignment: SystemJobAssignmentRef,
    pub input: SystemJobInput,
    pub max_result_bytes: u32,
    pub binding: RuntimeBinding,
    pub instruction: String,
}
impl SystemJobRunSpec {
    pub fn validate(&self) -> Result<(), RuntimeSpecError> {
        if self.schema_version != SYSTEM_JOB_RUN_SPEC_VERSION
            || self
                .project_id
                .validate_v7("system_job.project_id")
                .is_err()
            || self.run_id.get_version_num() != 7
            || self.binding.execution_profile.project_id() != self.project_id
            || self.assignment.validate().is_err()
            || self
                .input
                .validate(self.assignment.kind, 256 * 1024)
                .is_err()
            || !(1024..=64 * 1024).contains(&self.max_result_bytes)
            || self.binding.surface != SurfaceSpec::None
            || self.instruction.trim().is_empty()
            || self.instruction.len() > 512 * 1024
        {
            return Err(RuntimeSpecError::InvalidProfile);
        }
        self.binding.validate()
    }
}
