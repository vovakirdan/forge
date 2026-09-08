//! Read-only candidate inspection is independent from Run observations and stop controls.
use forge_domain::git::{GitCandidate, GitObjectId};
use forge_protocol::supervisor::v1::{
    GitCandidateInspectionCode as Code, GitCandidateInspectionResult, InspectGitCandidate,
};
use std::time::Duration;
use uuid::Uuid;

use super::{PodmanBackend, valid_environment_id};
use crate::{
    SupervisorError,
    git::{GitBackend, GitBackendError},
    journal::RunRecord,
    registry::RunRegistry,
    surface,
};

impl PodmanBackend {
    pub(crate) async fn inspect_git_candidate(
        &self,
        request: InspectGitCandidate,
        registry: RunRegistry,
    ) -> Result<(), SupervisorError> {
        // Invalid command identities cannot allocate unbounded or ambiguous receipt keys.
        if !valid_uuid(&request.command_id) {
            return Err(SupervisorError::InvalidJournalMessage);
        }
        let conflict = {
            let mut journal = registry.journal.lock().await;
            if journal.replay_inspection(&request)?.is_some() {
                registry.changed.notify_one();
                return Ok(());
            }
            journal.inspection_request_conflicts(&request)?
        };
        let mut result = self.inspection_reply(&request);
        let inspection = if conflict {
            Err(Code::ConflictingRequest)
        } else if !self.valid_inspection_request(&request) {
            Err(Code::InvalidRequest)
        } else {
            tokio::time::timeout(
                Duration::from_secs(30),
                self.verify_retained_candidate(&request, &registry),
            )
            .await
            .unwrap_or(Err(Code::Unavailable))
        };
        match inspection {
            Ok(candidate) => {
                result.result_code = Code::Verified as i32;
                result.commit = candidate.commit.as_str().to_owned();
                result.tree = candidate.tree.as_str().to_owned();
            }
            Err(code) => result.result_code = code as i32,
        }
        let mut journal = registry.journal.lock().await;
        if journal.inspection_request_conflicts(&request)? {
            result.result_code = Code::ConflictingRequest as i32;
            result.commit.clear();
            result.tree.clear();
        }
        if result.result_code == Code::Verified as i32
            && let Err(code) = journal.git_inspection_record(&request)
        {
            result.result_code = code as i32;
            result.commit.clear();
            result.tree.clear();
        }
        journal.record_inspection(&request, result)?;
        registry.changed.notify_one();
        Ok(())
    }

    async fn verify_retained_candidate(
        &self,
        request: &InspectGitCandidate,
        registry: &RunRegistry,
    ) -> Result<GitCandidate, Code> {
        // Fail busy rather than queue behind provisioning. This lock never blocks the
        // session loop, Run monitors, or physical stop requests.
        let _reservation = self.provisioning.try_lock().map_err(|_| Code::Busy)?;
        let record = registry
            .journal
            .lock()
            .await
            .git_inspection_record(request)?;
        self.inspection_quiescence(&record).await?;
        let source = record.git_source.as_ref().ok_or(Code::StaleScope)?;
        let worktree = surface::inspection_worktree(
            &self.config.state_directory,
            &record.provision.task_id,
            source,
        )
        .map_err(|_| Code::UnsafeSurface)?;
        let expected =
            GitObjectId::new(&request.expected_commit).map_err(|_| Code::InvalidRequest)?;
        let candidate = GitBackend::default()
            .verify_candidate(&worktree, &expected, true)
            .await
            .map_err(git_code)?;
        self.inspection_quiescence(&record).await?;
        registry
            .journal
            .lock()
            .await
            .git_inspection_record(request)?;
        Ok(candidate)
    }

    async fn inspection_quiescence(&self, record: &RunRecord) -> Result<(), Code> {
        if !valid_environment_id(&record.environment_id) {
            return Err(Code::QuiescenceUnknown);
        }
        let source = record.git_source.as_ref().ok_or(Code::StaleScope)?;
        match self
            .inspect(&record.provision)
            .await
            .map_err(|_| Code::QuiescenceUnknown)?
        {
            Some(environment) => {
                if environment.id != record.environment_id
                    || environment.config.labels.get("forge.surface")
                        != Some(&source.surface_id.to_string())
                    || environment.config.labels.get("forge.boot") != Some(&record.boot_id)
                {
                    return Err(Code::StaleScope);
                }
                if environment.state.running {
                    return Err(Code::Busy);
                }
                if !matches!(environment.state.status.as_str(), "exited" | "stopped") {
                    return Err(Code::QuiescenceUnknown);
                }
            }
            // Containers are normally retained. If one was subsequently removed,
            // accepted durable stop evidence for this exact identity is required.
            None if record.quiescence_confirmed => {}
            None => return Err(Code::QuiescenceUnknown),
        }
        Ok(())
    }

    fn valid_inspection_request(&self, request: &InspectGitCandidate) -> bool {
        [&request.run_id, &request.project_id, &request.surface_id]
            .into_iter()
            .all(|id| valid_uuid(id))
            && request.lease_fencing_token > 0
            && request.environment_epoch > 0
            && request.host_id == self.config.host_id
            && request.boot_id == self.config.boot_id
            && GitObjectId::new(&request.expected_commit).is_ok()
    }

    fn inspection_reply(&self, request: &InspectGitCandidate) -> GitCandidateInspectionResult {
        GitCandidateInspectionResult {
            message_id: crate::new_id(),
            request_command_id: request.command_id.clone(),
            run_id: if valid_uuid(&request.run_id) {
                request.run_id.clone()
            } else {
                String::new()
            },
            lease_fencing_token: request.lease_fencing_token,
            environment_epoch: request.environment_epoch,
            commit: String::new(),
            tree: String::new(),
            result_code: Code::Unspecified as i32,
            host_id: self.config.host_id.clone(),
            boot_id: self.config.boot_id.clone(),
        }
    }
}

fn valid_uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| id.get_version_num() == 7 && id.to_string() == value)
}

fn git_code(error: GitBackendError) -> Code {
    match error {
        GitBackendError::HeadChanged => Code::HeadChanged,
        GitBackendError::DirtyCandidate => Code::DirtyCandidate,
        GitBackendError::UnsafePath | GitBackendError::InvalidValue(_) => Code::UnsafeSurface,
        GitBackendError::ExternalFilter | GitBackendError::SubmodulesUnsupported => {
            Code::UnsupportedRepository
        }
        _ => Code::Unavailable,
    }
}

#[cfg(test)]
pub(super) mod tests;
