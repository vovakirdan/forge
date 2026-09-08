//! Local deployment concurrency policy, independent of per-Run resource budgets.
use serde::{Deserialize, Serialize};

/// Shared limits for one canonical database and its single local Supervisor.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionLimits {
    pub host_max_runs: u16,
    pub project_max_runs: u16,
    pub credential_account_max_runs: u16,
}

impl Default for AdmissionLimits {
    fn default() -> Self {
        Self {
            host_max_runs: 16,
            project_max_runs: 8,
            credential_account_max_runs: 4,
        }
    }
}

impl AdmissionLimits {
    /// Zero is not an implicit unlimited setting; Project stop is a separate control.
    pub const fn is_valid(self) -> bool {
        self.host_max_runs > 0 && self.project_max_runs > 0 && self.credential_account_max_runs > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_allow_the_three_run_employee_acceptance() {
        let limits = AdmissionLimits::default();
        assert!(limits.is_valid());
        assert!(limits.credential_account_max_runs >= 3);
    }

    #[test]
    fn zero_cannot_disable_a_safety_limit() {
        assert!(
            !AdmissionLimits {
                host_max_runs: 0,
                ..AdmissionLimits::default()
            }
            .is_valid()
        );
    }
}
