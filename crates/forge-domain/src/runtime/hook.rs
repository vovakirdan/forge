//! Provider-free System execution over a pinned Task candidate.
use super::{ResourceLimits, RuntimeLaunchSpec, RuntimeSpecError};
use crate::{
    ExecutionAssignment, HookAssignmentRef, ProjectHookVersion, ProjectId, TaskGitBinding,
    git::GitCandidate,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const HOOK_RUN_SPEC_VERSION: u16 = 5;
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookRunSpec {
    pub schema_version: u16,
    pub project_id: ProjectId,
    pub run_id: Uuid,
    pub assignment: HookAssignmentRef,
    pub hook: ProjectHookVersion,
    pub binding: TaskGitBinding,
    pub candidate: GitCandidate,
}
impl HookRunSpec {
    pub fn validate(&self) -> Result<(), RuntimeSpecError> {
        if self.schema_version != HOOK_RUN_SPEC_VERSION
            || self.run_id.get_version_num() != 7
            || self.project_id.validate_v7("hook_run.project_id").is_err()
            || self.hook.project_id != self.project_id
            || self.hook.validate_snapshot().is_err()
            || self.binding.validate_snapshot().is_err()
            || ExecutionAssignment::Hook(self.assignment.clone())
                .validate()
                .is_err()
            || self.candidate.commit.as_str().len() != self.candidate.tree.as_str().len()
        {
            return Err(RuntimeSpecError::InvalidProfile);
        }
        Ok(())
    }
}

/// Internal launch normalization. Never serialize this as a different RunSpec.
#[derive(Clone, Debug)]
pub enum SandboxLaunchSpec {
    Provider(Box<RuntimeLaunchSpec>),
    Hook(Box<HookRunSpec>),
}
impl SandboxLaunchSpec {
    pub fn schema_version(&self) -> u16 {
        match self {
            Self::Provider(s) => s.schema_version,
            Self::Hook(s) => s.schema_version,
        }
    }
    pub fn validate(&self) -> Result<(), RuntimeSpecError> {
        match self {
            Self::Provider(s) => s.validate(),
            Self::Hook(s) => s.validate(),
        }
    }
    pub fn limits(&self) -> &ResourceLimits {
        match self {
            Self::Provider(s) => &s.binding.limits,
            Self::Hook(s) => &s.hook.limits,
        }
    }
    pub fn max_output_bytes(&self) -> u64 {
        match self {
            Self::Provider(s) => s.binding.budget.max_output_bytes,
            Self::Hook(s) => s.hook.max_output_bytes,
        }
    }
    pub fn image(&self) -> &str {
        match self {
            Self::Provider(s) => &s.binding.image,
            Self::Hook(s) => &s.hook.image,
        }
    }
    /// Hook scratch identity never denotes the Task's writable surface.
    pub fn surface_id(&self) -> Uuid {
        match self {
            Self::Provider(s) => s.surface_id,
            Self::Hook(s) => s.run_id,
        }
    }
}
impl<'de> Deserialize<'de> for SandboxLaunchSpec {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let decoded = match value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
        {
            Some(2..=4 | 6) => serde_json::from_value::<RuntimeLaunchSpec>(value)
                .map(|spec| Self::Provider(Box::new(spec))),
            Some(5) => {
                serde_json::from_value::<HookRunSpec>(value).map(|spec| Self::Hook(Box::new(spec)))
            }
            _ => return Err(serde::de::Error::custom("unsupported sandbox spec schema")),
        }
        .map_err(serde::de::Error::custom)?;
        decoded.validate().map_err(serde::de::Error::custom)?;
        Ok(decoded)
    }
}
