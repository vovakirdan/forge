//! Durable command admission precedes Git effects; incomplete commands only reconcile.
use super::{GitSourceScope, Journal};
use crate::SupervisorError;
use forge_domain::{
    git::GitIntegrationIntent,
    git_integration::{GitIntegrationOperation, IntegrationRequest, IntegrationResult},
};
use forge_protocol::supervisor::v1::{
    EnvironmentPresence, GitIntegrationReply, SupervisorToCore, supervisor_to_core,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct IntegrationRecord {
    pub operation: GitIntegrationOperation,
    pub intent: Option<GitIntegrationIntent>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct IntegrationReceipt {
    request: IntegrationRequest,
    pub message: Option<SupervisorToCore>,
    pub pending: bool,
}
impl Journal {
    pub fn integration_source(
        &self,
        operation: &GitIntegrationOperation,
    ) -> Result<GitSourceScope, SupervisorError> {
        let key = format!(
            "{}:{}:{}",
            operation.writer.run_id,
            operation.writer.fencing_token,
            operation.writer.environment_epoch
        );
        let record = self
            .snapshot
            .runs
            .get(&key)
            .ok_or(SupervisorError::UnsafeSurface)?;
        let source = record
            .git_source
            .clone()
            .ok_or(SupervisorError::UnsafeSurface)?;
        if record.provision.task_id != operation.task_id.to_string()
            || source.project_id != operation.project_id
            || source.surface_id != operation.binding.surface_id
            || source.source
                != match &operation.binding.initial_base {
                    forge_domain::git::GitInitialRevision::Commit(commit) => {
                        forge_domain::runtime::SurfaceSpec::GitWorktree {
                            repository: operation
                                .binding
                                .source
                                .as_path()
                                .to_string_lossy()
                                .into_owned(),
                            base_ref: commit.as_str().into(),
                        }
                    }
                    forge_domain::git::GitInitialRevision::Unborn { object_format } => {
                        forge_domain::runtime::SurfaceSpec::GitUnborn {
                            repository: operation
                                .binding
                                .source
                                .as_path()
                                .to_string_lossy()
                                .into_owned(),
                            object_format: *object_format,
                        }
                    }
                }
            || self
                .snapshot
                .git_surface_owners
                .get(&source.surface_id.to_string())
                != Some(&key)
            || self.snapshot.runs.values().any(|run| {
                run.provision.task_id == operation.task_id.to_string()
                    && run.presence != EnvironmentPresence::Quiescent
            })
        {
            return Err(SupervisorError::UnsafeSurface);
        }
        Ok(source)
    }
    pub fn integration_replay(
        &mut self,
        request: &IntegrationRequest,
    ) -> Result<Option<bool>, SupervisorError> {
        let Some(receipt) = self.snapshot.integration_receipts.get(&request.command_id) else {
            return Ok(None);
        };
        if receipt.request != *request {
            return Err(SupervisorError::ConflictingProvision);
        }
        let complete = receipt.message.is_some();
        self.change(|snapshot| {
            snapshot
                .integration_receipts
                .get_mut(&request.command_id)
                .ok_or(SupervisorError::InvalidJournalMessage)?
                .pending = true;
            Ok(())
        })?;
        Ok(Some(complete))
    }
    pub fn begin_integration(
        &mut self,
        request: &IntegrationRequest,
    ) -> Result<(), SupervisorError> {
        request
            .validate()
            .map_err(|_| SupervisorError::InvalidRunSpec)?;
        self.change(|snapshot| {
            if request.phase != forge_domain::git_integration::IntegrationPhase::Reconcile
                && snapshot.integrations.values().any(|record| {
                    record.operation.task_id == request.operation.task_id
                        && record.operation.fence > request.operation.fence
                })
            {
                return Err(SupervisorError::ConflictingProvision);
            }
            if let Some(prior) = snapshot.integrations.get(&request.operation.id) {
                if prior.operation != request.operation
                    || (request.intent.is_some() && prior.intent != request.intent)
                {
                    return Err(SupervisorError::ConflictingProvision);
                }
            } else {
                if request.intent.is_some() {
                    return Err(SupervisorError::UnsafeSurface);
                }
                snapshot.integrations.insert(
                    request.operation.id,
                    IntegrationRecord {
                        operation: request.operation.clone(),
                        intent: None,
                    },
                );
            }
            snapshot.integration_receipts.insert(
                request.command_id,
                IntegrationReceipt {
                    request: request.clone(),
                    message: None,
                    pending: false,
                },
            );
            Ok(())
        })
    }
    pub fn prepared_integration(
        &mut self,
        operation: &GitIntegrationOperation,
        intent: &GitIntegrationIntent,
    ) -> Result<(), SupervisorError> {
        operation
            .validate_intent(intent)
            .map_err(|_| SupervisorError::InvalidRunSpec)?;
        self.change(|snapshot| {
            let record = snapshot
                .integrations
                .get_mut(&operation.id)
                .ok_or(SupervisorError::InvalidJournalMessage)?;
            if record.operation != *operation
                || record
                    .intent
                    .as_ref()
                    .is_some_and(|existing| existing != intent)
            {
                return Err(SupervisorError::ConflictingProvision);
            }
            record.intent = Some(intent.clone());
            Ok(())
        })
    }
    pub fn retained_integration_intent(&self, id: Uuid) -> Option<GitIntegrationIntent> {
        self.snapshot.integrations.get(&id)?.intent.clone()
    }
    pub fn integration_has_no_preparation(&self, operation: &GitIntegrationOperation) -> bool {
        self.snapshot
            .integrations
            .get(&operation.id)
            .is_some_and(|record| record.operation == *operation && record.intent.is_none())
            && self.integration_source(operation).is_ok()
            && !self.snapshot.integration_receipts.values().any(|receipt| {
                receipt.request.operation.id == operation.id
                    && receipt.request.phase
                        == forge_domain::git_integration::IntegrationPhase::Apply
            })
    }
    pub fn finish_integration(
        &mut self,
        result: &IntegrationResult,
    ) -> Result<(), SupervisorError> {
        let message = SupervisorToCore {
            message: Some(supervisor_to_core::Message::GitIntegration(
                GitIntegrationReply {
                    message_id: result.message_id.to_string(),
                    result_json: serde_json::to_string(result)?,
                },
            )),
        };
        self.change(|snapshot| {
            let receipt = snapshot
                .integration_receipts
                .get_mut(&result.command_id)
                .ok_or(SupervisorError::InvalidJournalMessage)?;
            if receipt.message.as_ref().is_some_and(|old| old != &message) {
                return Err(SupervisorError::ConflictingProvision);
            }
            receipt.message = Some(message);
            receipt.pending = true;
            Ok(())
        })
    }
}
