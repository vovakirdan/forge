//! Versioned taskless execution inputs and a transport-neutral decoded view.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{RuntimeBinding, RuntimeSpecError, SandboxRunSpec, SurfaceSpec, TaskRunSpecV6};
use crate::{CommunicationAssignmentRef, ExecutionAssignment, ProjectId, ResolutionAssignmentRef};

pub const COMMUNICATION_RUN_SPEC_VERSION: u16 = 3;
pub const RESOLUTION_RUN_SPEC_VERSION: u16 = 4;

/// A resolver receives a private scratch environment, never a contextual Task surface.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionRunSpec {
    pub schema_version: u16,
    pub project_id: ProjectId,
    pub run_id: Uuid,
    pub assignment: ResolutionAssignmentRef,
    pub binding: RuntimeBinding,
    pub instruction: String,
}
impl ResolutionRunSpec {
    pub fn validate(&self) -> Result<(), RuntimeSpecError> {
        if self.schema_version != RESOLUTION_RUN_SPEC_VERSION
            || self.project_id.validate_v7("run.project_id").is_err()
            || self.binding.execution_profile.project_id() != self.project_id
            || self.run_id.get_version_num() != 7
            || ExecutionAssignment::Resolution(self.assignment.clone())
                .validate()
                .is_err()
            || self.binding.surface != SurfaceSpec::None
            || self.instruction.trim().is_empty()
            || self.instruction.len() > 512 * 1024
        {
            return Err(RuntimeSpecError::InvalidProfile);
        }
        self.binding.validate()
    }
}

/// V3 deliberately has no Task, Pipeline, stage, or persistent work surface.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommunicationRunSpec {
    pub schema_version: u16,
    pub project_id: ProjectId,
    pub run_id: Uuid,
    pub assignment: CommunicationAssignmentRef,
    pub binding: RuntimeBinding,
    pub instruction: String,
}

impl CommunicationRunSpec {
    pub fn validate(&self) -> Result<(), RuntimeSpecError> {
        if self.schema_version != COMMUNICATION_RUN_SPEC_VERSION
            || self.project_id.validate_v7("run.project_id").is_err()
            || self.binding.execution_profile.project_id() != self.project_id
            || self.run_id.get_version_num() != 7
            || ExecutionAssignment::Communication(self.assignment.clone())
                .validate()
                .is_err()
            || self.binding.surface != SurfaceSpec::None
            || self.instruction.trim().is_empty()
            || self.instruction.len() > 512 * 1024
        {
            return Err(RuntimeSpecError::InvalidProfile);
        }
        self.binding.validate()
    }
}

/// Internal normalized launch view, never serialized as a different RunSpec.
/// V2 surface_id is Task-owned; V3 uses its Run identity only for private scratch.
#[derive(Clone, Debug)]
pub struct RuntimeLaunchSpec {
    pub schema_version: u16,
    pub project_id: ProjectId,
    pub surface_id: Uuid,
    pub binding: RuntimeBinding,
    pub instruction: String,
    pub communication: Option<CommunicationAssignmentRef>,
    pub resolution: Option<ResolutionAssignmentRef>,
    pub system_job: Option<(
        crate::system_job::SystemJobAssignmentRef,
        crate::system_job::SystemJobInput,
        u32,
    )>,
    pub source_request: Option<crate::git::GitSourceRequest>,
    pub file_inputs: Vec<crate::file_snapshot::TaskFileInput>,
}

impl RuntimeLaunchSpec {
    pub fn validate(&self) -> Result<(), RuntimeSpecError> {
        if let Some((owner, input, max_result_bytes)) = &self.system_job {
            if self.communication.is_some()
                || self.resolution.is_some()
                || self.source_request.is_some()
                || !self.file_inputs.is_empty()
            {
                return Err(RuntimeSpecError::InvalidProfile);
            }
            return super::SystemJobRunSpec {
                schema_version: self.schema_version,
                project_id: self.project_id,
                run_id: self.surface_id,
                assignment: owner.clone(),
                input: input.clone(),
                max_result_bytes: *max_result_bytes,
                binding: self.binding.clone(),
                instruction: self.instruction.clone(),
            }
            .validate();
        }
        if self.schema_version != 6
            && (self.source_request.is_some() || !self.file_inputs.is_empty())
        {
            return Err(RuntimeSpecError::InvalidProfile);
        }
        match (self.schema_version, &self.communication, &self.resolution) {
            (6, None, None) => TaskRunSpecV6 {
                schema_version: 6,
                project_id: self.project_id,
                surface_id: self.surface_id,
                binding: self.binding.clone(),
                instruction: self.instruction.clone(),
                source_request: self.source_request.clone(),
                file_inputs: self.file_inputs.clone(),
            }
            .validate(),
            (2, None, None) => SandboxRunSpec {
                schema_version: 2,
                project_id: self.project_id,
                surface_id: self.surface_id,
                binding: self.binding.clone(),
                instruction: self.instruction.clone(),
            }
            .validate(),
            (3, Some(owner), None) => CommunicationRunSpec {
                schema_version: 3,
                project_id: self.project_id,
                run_id: self.surface_id,
                assignment: owner.clone(),
                binding: self.binding.clone(),
                instruction: self.instruction.clone(),
            }
            .validate(),
            (4, None, Some(owner)) => ResolutionRunSpec {
                schema_version: 4,
                project_id: self.project_id,
                run_id: self.surface_id,
                assignment: owner.clone(),
                binding: self.binding.clone(),
                instruction: self.instruction.clone(),
            }
            .validate(),
            _ => Err(RuntimeSpecError::InvalidProfile),
        }
    }
}

impl From<SandboxRunSpec> for RuntimeLaunchSpec {
    fn from(spec: SandboxRunSpec) -> Self {
        Self {
            schema_version: spec.schema_version,
            project_id: spec.project_id,
            surface_id: spec.surface_id,
            binding: spec.binding,
            instruction: spec.instruction,
            communication: None,
            resolution: None,
            system_job: None,
            source_request: None,
            file_inputs: vec![],
        }
    }
}

impl From<TaskRunSpecV6> for RuntimeLaunchSpec {
    fn from(spec: TaskRunSpecV6) -> Self {
        Self {
            schema_version: spec.schema_version,
            project_id: spec.project_id,
            surface_id: spec.surface_id,
            binding: spec.binding,
            instruction: spec.instruction,
            communication: None,
            resolution: None,
            system_job: None,
            source_request: spec.source_request,
            file_inputs: spec.file_inputs,
        }
    }
}

impl<'de> Deserialize<'de> for RuntimeLaunchSpec {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let decoded = match value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
        {
            Some(2) => serde_json::from_value::<SandboxRunSpec>(value).map(Into::into),
            Some(6) => serde_json::from_value::<TaskRunSpecV6>(value).map(Into::into),
            Some(3) => serde_json::from_value::<CommunicationRunSpec>(value).map(|spec| Self {
                schema_version: spec.schema_version,
                project_id: spec.project_id,
                surface_id: spec.run_id,
                binding: spec.binding,
                instruction: spec.instruction,
                communication: Some(spec.assignment),
                resolution: None,
                system_job: None,
                source_request: None,
                file_inputs: vec![],
            }),
            Some(4) => serde_json::from_value::<ResolutionRunSpec>(value).map(|spec| Self {
                schema_version: spec.schema_version,
                project_id: spec.project_id,
                surface_id: spec.run_id,
                binding: spec.binding,
                instruction: spec.instruction,
                communication: None,
                resolution: Some(spec.assignment),
                system_job: None,
                source_request: None,
                file_inputs: vec![],
            }),
            Some(7) => serde_json::from_value::<super::SystemJobRunSpec>(value).map(|spec| Self {
                schema_version: spec.schema_version,
                project_id: spec.project_id,
                surface_id: spec.run_id,
                binding: spec.binding,
                instruction: spec.instruction,
                communication: None,
                resolution: None,
                system_job: Some((spec.assignment, spec.input, spec.max_result_bytes)),
                source_request: None,
                file_inputs: vec![],
            }),
            _ => return Err(serde::de::Error::custom("unsupported runtime spec schema")),
        }
        .map_err(serde::de::Error::custom)?;
        Self::validate(&decoded).map_err(serde::de::Error::custom)?;
        Ok(decoded)
    }
}
