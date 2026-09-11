//! Independent inspection receipts never reopen or advance the completed Run stream.
use forge_domain::{
    ProjectId,
    runtime::{RuntimeLaunchSpec, SurfaceAccess, SurfaceSpec},
};
use forge_protocol::supervisor::v1::{
    EnvironmentPresence, GitCandidateInspectionCode as Code, GitCandidateInspectionResult,
    InspectGitCandidate, ProvisionRun, SupervisorToCore, supervisor_to_core,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{Journal, RunRecord, scope_key};
use crate::SupervisorError;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct GitSourceScope {
    pub project_id: ProjectId,
    pub surface_id: Uuid,
    pub source: SurfaceSpec,
}

impl GitSourceScope {
    pub(super) fn from_provision(provision: &ProvisionRun) -> Option<Self> {
        if !matches!(provision.run_spec_version, 2 | 6)
            || Uuid::parse_str(&provision.task_id).ok()?.get_version_num() != 7
        {
            return None;
        }
        forge_domain::StageId::new(provision.stage_id.clone()).ok()?;
        if let Some(assignment) = &provision.assignment {
            match assignment {
                forge_protocol::supervisor::v1::provision_run::Assignment::TaskStage(task)
                    if task.task_id == provision.task_id
                        && task.stage_id == provision.stage_id
                        && Uuid::parse_str(&task.queue_entry_id)
                            .is_ok_and(|id| id.get_version_num() == 7) => {}
                _ => return None,
            }
        }
        let spec: RuntimeLaunchSpec = serde_json::from_str(&provision.run_spec_json).ok()?;
        spec.validate().ok()?;
        if spec.binding.access != SurfaceAccess::ReadWrite
            || !matches!(
                spec.binding.surface,
                SurfaceSpec::GitWorktree { .. } | SurfaceSpec::GitUnborn { .. }
            )
        {
            return None;
        }
        Some(Self {
            project_id: spec.project_id,
            surface_id: spec.surface_id,
            source: spec.binding.surface,
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct InspectionReceipt {
    request_hash: String,
    pub(super) message: SupervisorToCore,
    pub(super) pending: bool,
}

impl Journal {
    /// A duplicate replays the exact durable response, including after an ACK/restart.
    pub fn replay_inspection(
        &mut self,
        request: &InspectGitCandidate,
    ) -> Result<Option<GitCandidateInspectionResult>, SupervisorError> {
        let key = receipt_key(request)?;
        let Some(receipt) = self.snapshot.git_inspections.get(&key) else {
            return Ok(None);
        };
        let Some(supervisor_to_core::Message::GitCandidateInspection(result)) =
            &receipt.message.message
        else {
            return Err(SupervisorError::InvalidJournalMessage);
        };
        let result = result.clone();
        self.change(|snapshot| {
            snapshot
                .git_inspections
                .get_mut(&key)
                .ok_or(SupervisorError::InvalidJournalMessage)?
                .pending = true;
            Ok(())
        })?;
        Ok(Some(result))
    }

    pub fn inspection_request_conflicts(
        &self,
        request: &InspectGitCandidate,
    ) -> Result<bool, SupervisorError> {
        let hash = request_hash(request)?;
        Ok(self.snapshot.git_inspections.values().any(|receipt| {
            matches!(&receipt.message.message, Some(supervisor_to_core::Message::GitCandidateInspection(result))
                if result.request_command_id == request.command_id && receipt.request_hash != hash)
        }))
    }

    /// Completed inspection results are durable before transport publication.
    pub fn record_inspection(
        &mut self,
        request: &InspectGitCandidate,
        result: GitCandidateInspectionResult,
    ) -> Result<(), SupervisorError> {
        let key = receipt_key(request)?;
        let hash = request_hash(request)?;
        self.change(|snapshot| {
            snapshot
                .git_inspections
                .entry(key)
                .or_insert(InspectionReceipt {
                    request_hash: hash,
                    message: SupervisorToCore {
                        message: Some(supervisor_to_core::Message::GitCandidateInspection(result)),
                    },
                    pending: true,
                });
            Ok(())
        })
    }

    /// Latest Task writer identity, not merely any historical Run that once used this path.
    pub fn git_inspection_record(&self, request: &InspectGitCandidate) -> Result<RunRecord, Code> {
        let key = format!(
            "{}:{}:{}",
            request.run_id, request.lease_fencing_token, request.environment_epoch
        );
        let record = self.snapshot.runs.get(&key).ok_or(Code::StaleScope)?;
        let source = record.git_source.as_ref().ok_or(Code::StaleScope)?;
        if source.project_id.to_string() != request.project_id
            || source.surface_id.to_string() != request.surface_id
            || self.snapshot.git_surface_owners.get(&request.surface_id)
                != Some(&scope_key(&record.provision))
        {
            return Err(Code::StaleScope);
        }
        match record.presence {
            EnvironmentPresence::Active => Err(Code::Busy),
            EnvironmentPresence::Unknown | EnvironmentPresence::Unspecified => {
                Err(Code::QuiescenceUnknown)
            }
            EnvironmentPresence::Quiescent => Ok(record.clone()),
        }
    }
}

fn request_hash(request: &InspectGitCandidate) -> Result<String, SupervisorError> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(request)?)
    ))
}

fn receipt_key(request: &InspectGitCandidate) -> Result<String, SupervisorError> {
    Ok(format!("{}:{}", request.command_id, request_hash(request)?))
}
