//! Management-owned question routing is separate from Task stage execution.
mod assignment;
mod context;
mod escalation;
mod source;
#[cfg(test)]
mod tests;

pub use assignment::*;
pub use context::*;
pub use escalation::*;
pub use source::*;

use crate::{DomainError, EmployeeId, ProjectId, Timestamp};
use serde::{Deserialize, Serialize};

/// A mutable catalog entry; every escalation freezes its selected revision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolverRoute {
    pub project_id: ProjectId,
    pub key: String,
    pub revision: u64,
    pub employee_ids: Vec<EmployeeId>,
    pub assignment_timeout_seconds: u32,
    pub updated_at: Timestamp,
}

impl ResolverRoute {
    pub fn validate_snapshot(&self) -> Result<(), DomainError> {
        self.project_id.validate_v7("resolver_route.project_id")?;
        crate::ids::validate_stable_key("resolver_route.key", &self.key)?;
        if self.revision == 0
            || self.employee_ids.len() > 32
            || !(30..=86_400).contains(&self.assignment_timeout_seconds)
        {
            return Err(invalid("invalid route revision, size, or timeout"));
        }
        let mut unique = std::collections::BTreeSet::new();
        for employee in &self.employee_ids {
            employee.validate_v7("resolver_route.employee_id")?;
            if !unique.insert(*employee) {
                return Err(invalid("route candidates must be unique"));
            }
        }
        Ok(())
    }
}

pub(crate) fn text(value: &str, max: usize) -> Result<(), DomainError> {
    if value.trim().is_empty() || value.contains('\0') || value.chars().count() > max {
        return Err(invalid(
            "text must be bounded, nonblank, and contain no NUL",
        ));
    }
    Ok(())
}

pub(crate) fn invalid(reason: &str) -> DomainError {
    DomainError::InvalidValue {
        field: "resolution",
        reason: reason.into(),
    }
}
