//! Explicit, bounded management requests; provider output cannot enter this contract.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::SystemJobPolicy;
use crate::{
    DomainError, EmployeeId, ProjectId, TaskId,
    evidence::{invalid, validate_uuid},
    runtime::{RuntimeBinding, SurfaceSpec},
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum SystemJobManagement {
    Configure {
        expected_settings_revision: u64,
        policy: SystemJobPolicy,
        binding: Box<RuntimeBinding>,
    },
    RequestSummary {
        task_id: TaskId,
    },
    RequestOnboarding {
        employee_id: EmployeeId,
    },
    Retry {
        job_id: Uuid,
    },
    SkipOnboarding {
        employee_id: EmployeeId,
        reason: String,
    },
}

impl SystemJobManagement {
    pub fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::Configure {
                expected_settings_revision,
                policy,
                binding,
            } => {
                if *expected_settings_revision >= i64::MAX as u64 {
                    return Err(invalid(
                        "system_job.settings_revision",
                        "outside the revision range",
                    ));
                }
                policy.validate()?;
                binding
                    .validate()
                    .map_err(|_| invalid("system_job.binding", "invalid runtime binding"))?;
                if binding.surface != SurfaceSpec::None {
                    return Err(invalid(
                        "system_job.binding.surface",
                        "must use private scratch with surface none",
                    ));
                }
            }
            Self::RequestSummary { task_id } => task_id.validate_v7("system_job.task_id")?,
            Self::RequestOnboarding { employee_id } => {
                employee_id.validate_v7("system_job.employee_id")?
            }
            Self::Retry { job_id } => validate_uuid(*job_id, "system_job.job_id")?,
            Self::SkipOnboarding {
                employee_id,
                reason,
            } => {
                employee_id.validate_v7("system_job.employee_id")?;
                if reason.trim().is_empty() || reason.len() > 2000 {
                    return Err(invalid(
                        "system_job.skip_reason",
                        "must be nonblank and at most 2000 bytes",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Profile scope is known without loading a credential or starting external work.
    pub fn validate_project(&self, project: ProjectId) -> Result<(), DomainError> {
        self.validate()?;
        if let Self::Configure { binding, .. } = self
            && binding.execution_profile.project_id() != project
        {
            return Err(invalid(
                "system_job.binding.project_id",
                "must match the command Project",
            ));
        }
        Ok(())
    }
}
