//! Frozen attempt inputs: no mutable memory or prompt lookup during a Run.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    DomainError, EmployeeId, HandoffArtifactReference, PipelineVersionId, ProjectId, StageId,
    TaskHandoff, TaskId, TaskSpec, Timestamp,
    evidence::{invalid, validate_reference, validate_uuid},
};

/// Canonical, immutable inputs used to issue one Run.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ContextSnapshot(ContextSnapshotInput);

/// Owned inputs frozen by [`ContextSnapshot::new`]. Prompt and policy references
/// refer to pinned revisions; credentials and provider secrets are never inputs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextSnapshotInput {
    pub context_snapshot_id: Uuid,
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub run_id: Uuid,
    pub employee_id: EmployeeId,
    pub pipeline_version_id: PipelineVersionId,
    pub stage_id: StageId,
    /// Exact stage visit for M2 inputs. None is a historical, non-Inbox Run.
    #[serde(default)]
    pub stage_visit: Option<u64>,
    pub task_revision_before_dispatch: u64,
    pub task_spec: TaskSpec,
    pub system_policy_revision: String,
    pub employee_prompt_revision: String,
    pub capability_grants: Vec<String>,
    pub tool_catalog_revision: String,
    pub run_spec_id: Uuid,
    pub prior_handoff: Option<TaskHandoff>,
    pub artifacts: Vec<HandoffArtifactReference>,
    pub control_instruction: Option<String>,
    pub created_at: Timestamp,
}

impl ContextSnapshot {
    /// Validates shape and same-Task handoff scope, then freezes all owned inputs.
    pub fn new(input: ContextSnapshotInput) -> Result<Self, DomainError> {
        for (id, field) in [
            (input.context_snapshot_id, "context.id"),
            (input.run_id, "context.run_id"),
            (input.run_spec_id, "context.run_spec_id"),
        ] {
            validate_uuid(id, field)?;
        }
        input.project_id.validate_v7("context.project_id")?;
        input.task_id.validate_v7("context.task_id")?;
        input.employee_id.validate_v7("context.employee_id")?;
        input
            .pipeline_version_id
            .validate_v7("context.pipeline_version_id")?;
        input.stage_id.validate()?;
        if input.stage_visit == Some(0) {
            return Err(invalid(
                "context.stage_visit",
                "must be positive when present",
            ));
        }
        input.task_spec.validate_snapshot()?;
        if input.task_revision_before_dispatch == 0 {
            return Err(invalid("context.task_revision", "must be positive"));
        }
        for (value, field) in [
            (
                &input.system_policy_revision,
                "context.system_policy_revision",
            ),
            (
                &input.employee_prompt_revision,
                "context.employee_prompt_revision",
            ),
            (
                &input.tool_catalog_revision,
                "context.tool_catalog_revision",
            ),
        ] {
            validate_reference(value, field)?;
        }
        for capability in &input.capability_grants {
            validate_reference(capability, "context.capability_grants")?;
        }
        if let Some(handoff) = &input.prior_handoff {
            let data = handoff.data();
            if data.project_id != input.project_id
                || data.task_id != input.task_id
                || data.target_stage_id != input.stage_id
            {
                return Err(invalid(
                    "context.prior_handoff",
                    "must target this task and stage",
                ));
            }
        }
        for artifact in &input.artifacts {
            artifact.validate()?;
        }
        if input
            .control_instruction
            .as_ref()
            .is_some_and(|value| value.trim().is_empty() || value.len() > 16_384)
        {
            return Err(invalid(
                "context.control_instruction",
                "must be nonblank and at most 16384 bytes",
            ));
        }
        Ok(Self(input))
    }

    pub fn id(&self) -> Uuid {
        self.0.context_snapshot_id
    }

    pub fn data(&self) -> &ContextSnapshotInput {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ContextSnapshot {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(ContextSnapshotInput::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}
