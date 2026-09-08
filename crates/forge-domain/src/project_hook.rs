//! Immutable, explicitly configured project commands. No test discovery or provider.
use crate::{
    DomainError, OutcomeKey, ProjectId, TaskKind, Timestamp,
    evidence::{invalid, validate_uuid},
    runtime::ResourceLimits,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Component, Path},
};
use uuid::Uuid;

/// A new identity is a new version; existing Pipeline pins never follow a name.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectHookVersion {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub name: String,
    /// Pinned owner-selected image containing the Forge runner wrapper.
    pub image: String,
    /// Executable followed by arguments, all executed inside the sandbox.
    pub command: Vec<String>,
    /// Relative to the exact candidate checkout. `.` means its root.
    pub workdir: String,
    pub limits: ResourceLimits,
    pub max_output_bytes: u64,
    /// Empty means all Task kinds. A nonmatch is an explicit skipped result.
    #[serde(default)]
    pub applicable_task_kinds: BTreeSet<TaskKind>,
    pub required: bool,
    pub created_at: Timestamp,
}
impl ProjectHookVersion {
    pub fn validate_snapshot(&self) -> Result<(), DomainError> {
        validate_uuid(self.id, "hook.id")?;
        self.project_id.validate_v7("hook.project_id")?;
        self.limits
            .validate()
            .map_err(|_| invalid("hook.limits", "invalid resource limits"))?;
        let digest = self
            .image
            .rsplit_once("@sha256:")
            .filter(|(name, _)| !name.is_empty())
            .map(|(_, digest)| digest);
        if self.name.trim().is_empty()
            || self.name.len() > 128
            || self.name.chars().any(char::is_control)
            || self.image.starts_with('-')
            || self.image.len() > 4096
            || self
                .image
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
            || !digest.is_some_and(|d| d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit()))
            || self.command.is_empty()
            || self.command.len() > 128
            || self.command[0].trim().is_empty()
            || self.command[0].starts_with('-')
            || self
                .command
                .iter()
                .any(|arg| arg.contains('\0') || arg.len() > 32 * 1024)
            || self.command.iter().map(String::len).sum::<usize>() > 128 * 1024
            || self.max_output_bytes == 0
            || self.max_output_bytes > 256 * 1024 * 1024
            || self.workdir.is_empty()
            || self.workdir.len() > 4096
            || self.workdir.chars().any(char::is_control)
            || (self.workdir != "."
                && Path::new(&self.workdir)
                    .components()
                    .any(|part| !matches!(part, Component::Normal(_))))
        {
            return Err(invalid(
                "hook",
                "invalid pinned command, image, workdir or output bound",
            ));
        }
        Ok(())
    }
    pub fn applies_to(&self, kind: TaskKind) -> bool {
        self.applicable_task_kinds.is_empty() || self.applicable_task_kinds.contains(&kind)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookOutcomes {
    pub passed: OutcomeKey,
    pub failed: OutcomeKey,
    pub timed_out: OutcomeKey,
    pub skipped: OutcomeKey,
}

/// Reported process evidence, not a claim that arbitrary repository code is correct.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookVerdict {
    Passed,
    Failed,
    TimedOut,
    Interrupted,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookExecutionResult {
    pub invocation_id: Uuid,
    pub candidate_proposal_id: Uuid,
    pub candidate: crate::git::GitCandidate,
    pub verdict: HookVerdict,
    pub exit_code: Option<i32>,
    pub output_incomplete: bool,
}
impl HookExecutionResult {
    pub fn validate(&self) -> Result<(), DomainError> {
        validate_uuid(self.invocation_id, "hook_result.invocation_id")?;
        validate_uuid(
            self.candidate_proposal_id,
            "hook_result.candidate_proposal_id",
        )?;
        if self.candidate.commit.as_str().len() != self.candidate.tree.as_str().len()
            || (self.verdict == HookVerdict::Passed
                && (self.exit_code != Some(0) || self.output_incomplete))
        {
            return Err(invalid(
                "hook_result",
                "invalid candidate or contradictory successful result",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
