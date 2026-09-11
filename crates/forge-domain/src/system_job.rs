//! Restricted semantic work owns an attempt, never a contextual Task or Employee.

mod management;
pub use management::SystemJobManagement;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    DomainError, EmployeeId, TaskId,
    evidence::{invalid, validate_uuid},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemJobKind {
    Summarization,
    Onboarding,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemJobAssignmentRef {
    pub job_id: Uuid,
    pub attempt_id: Uuid,
    pub generation: u64,
    pub kind: SystemJobKind,
}

impl SystemJobAssignmentRef {
    pub fn validate(&self) -> Result<(), DomainError> {
        validate_uuid(self.job_id, "system_job.id")?;
        validate_uuid(self.attempt_id, "system_job.attempt_id")?;
        if self.generation == 0 || self.generation > i64::MAX as u64 {
            return Err(invalid(
                "system_job.generation",
                "must be positive and representable",
            ));
        }
        Ok(())
    }
}

/// A frozen input envelope. Core validates the sources before constructing it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemJobInput {
    pub source_task_id: Option<TaskId>,
    pub target_employee_id: Option<EmployeeId>,
    pub covered_sequence: u64,
    pub source_digest: String,
    pub context: serde_json::Value,
}

impl SystemJobInput {
    pub fn validate(&self, kind: SystemJobKind, max_bytes: u32) -> Result<(), DomainError> {
        if let Some(id) = self.source_task_id {
            id.validate_v7("system_job.source_task_id")?;
        }
        if let Some(id) = self.target_employee_id {
            id.validate_v7("system_job.target_employee_id")?;
        }
        if self.source_digest.len() != 64
            || !self.source_digest.bytes().all(|c| c.is_ascii_hexdigit())
            || !self.context.is_object()
            || self.covered_sequence > i64::MAX as u64
            || serde_json::to_vec(&self.context)
                .map_err(|_| invalid("system_job.context", "invalid JSON"))?
                .len()
                > max_bytes as usize
            || matches!(kind, SystemJobKind::Summarization) && self.source_task_id.is_none()
            || matches!(kind, SystemJobKind::Onboarding)
                && (self.target_employee_id.is_none() || self.source_task_id.is_some())
        {
            return Err(invalid(
                "system_job.input",
                "invalid bounded input or purpose scope",
            ));
        }
        Ok(())
    }
}

/// Persisted, explicit project allowance; omission never means unlimited inference.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemJobPolicy {
    pub enabled: bool,
    pub max_concurrent: u16,
    pub max_attempts_per_job: u16,
    pub max_attempts_per_day: u32,
    pub max_input_bytes: u32,
    pub max_result_bytes: u32,
    pub wall_seconds: u32,
    pub coalesce_seconds: u32,
}

impl Default for SystemJobPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            max_concurrent: 1,
            max_attempts_per_job: 2,
            max_attempts_per_day: 20,
            max_input_bytes: 64 * 1024,
            max_result_bytes: 16 * 1024,
            wall_seconds: 300,
            coalesce_seconds: 5,
        }
    }
}

impl SystemJobPolicy {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.max_concurrent == 0
            || self.max_concurrent > 16
            || self.max_attempts_per_job == 0
            || self.max_attempts_per_job > 10
            || self.max_attempts_per_day == 0
            || self.max_attempts_per_day > 100_000
            || !(1024..=256 * 1024).contains(&self.max_input_bytes)
            || !(1024..=64 * 1024).contains(&self.max_result_bytes)
            || !(10..=3600).contains(&self.wall_seconds)
            || self.coalesce_seconds > 300
        {
            return Err(invalid("system_job.policy", "invalid bounded allowance"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_require_explicit_enablement_and_bound_every_attempt() {
        let p = SystemJobPolicy::default();
        assert!(!p.enabled);
        assert_eq!(p.max_concurrent, 1);
        assert!(p.validate().is_ok());
    }
    #[test]
    fn zero_quota_is_not_unlimited() {
        let p = SystemJobPolicy {
            max_attempts_per_day: 0,
            ..Default::default()
        };
        assert!(p.validate().is_err());
    }
    #[test]
    fn onboarding_has_no_task_execution_owner() {
        let input = SystemJobInput {
            source_task_id: None,
            target_employee_id: Some(EmployeeId::new()),
            covered_sequence: 0,
            source_digest: "a".repeat(64),
            context: serde_json::json!({"role":"reader"}),
        };
        assert!(input.validate(SystemJobKind::Onboarding, 1024).is_ok());
        assert!(input.validate(SystemJobKind::Summarization, 1024).is_err());
    }
}
