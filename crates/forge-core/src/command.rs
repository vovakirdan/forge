//! Core-owned transaction and post-commit runtime composition.

use forge_application::{
    CommandContext, CommandEnvelope, CommandPayload, PreparedCommandState,
    execute_prepared_in_transaction, prepare_command,
};
use forge_domain::{AggregateRef, CommandId, DomainEventKind, Project, Timestamp};
use forge_protocol::wire::CommandReceipt;
use forge_storage::StorageTransaction;
use serde_json::json;
use tracing::warn;

use crate::{
    CoreError, CoreService,
    event::{event, event_payload},
};

impl CoreService {
    /// Applies a command through the shared engine inside one Core-owned transaction.
    /// Runtime work is delivered only after a successful commit, including replay.
    pub async fn execute_command(
        &self,
        envelope: CommandEnvelope,
    ) -> Result<CommandReceipt, CoreError> {
        let context = self.command_context(envelope.project_id);
        let mut transaction = self.store.begin().await?;
        let prepared = prepare_command(
            &mut transaction,
            &context,
            &envelope,
            self.command_clock.as_ref(),
        )
        .await?;
        let receipt = if is_runtime_command(&envelope.payload) {
            match prepared.into_state(&context, &envelope)? {
                PreparedCommandState::Replay(receipt) => receipt,
                PreparedCommandState::Ready {
                    project,
                    request_hash,
                    command_id,
                    now,
                } => {
                    let project = project.ok_or(CoreError::NotFound {
                        aggregate: "project",
                    })?;
                    self.apply_runtime_command(
                        &mut transaction,
                        project,
                        &envelope,
                        &request_hash,
                        command_id,
                        now,
                    )
                    .await?
                }
            }
        } else {
            execute_prepared_in_transaction(
                &mut transaction,
                &context,
                &envelope,
                prepared,
                self.command_clock.as_ref(),
            )
            .await?
        };
        transaction.commit().await?;
        self.dispatch_after_command(envelope.project_id).await;
        Ok(receipt)
    }

    pub(crate) fn command_context(&self, project_id: forge_domain::ProjectId) -> CommandContext {
        CommandContext::local_human(project_id, self.actors.human, self.actors.core)
    }

    async fn apply_runtime_command(
        &self,
        transaction: &mut StorageTransaction<'_>,
        mut project: Project,
        envelope: &CommandEnvelope,
        request_hash: &str,
        command_id: CommandId,
        now: Timestamp,
    ) -> Result<CommandReceipt, CoreError> {
        match &envelope.payload {
            CommandPayload::ImportTaskFileSnapshot(_)
            | CommandPayload::CaptureTaskFileSnapshot(_)
            | CommandPayload::AttachTaskFileInput(_) => {
                self.manage_file_snapshot(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                )
                .await
            }
            CommandPayload::ConfigureProjectHook(_) => {
                self.configure_project_hook(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                )
                .await
            }
            CommandPayload::RetryGitIntegration { .. }
            | CommandPayload::AcceptGitIntegrationResult { .. } => {
                self.manage_git_integration(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                )
                .await
            }
            CommandPayload::ConfigureBootRecoveryPolicy { .. }
            | CommandPayload::AcceptRunRecoveryAssessment { .. }
            | CommandPayload::RetryCommunication { .. } => {
                self.apply_recovery_command(
                    transaction,
                    project,
                    envelope,
                    request_hash,
                    command_id,
                    now,
                )
                .await
            }
            CommandPayload::EnrollCredential {
                secret_id,
                binding_id,
                kind,
                source_file,
            } => {
                self.enroll_credential(
                    transaction,
                    project.id(),
                    *secret_id,
                    *binding_id,
                    kind,
                    source_file,
                )
                .await?;
                let previous_revision = project.revision();
                project.record_child_mutation(now)?;
                transaction
                    .update_project(&project, previous_revision)
                    .await?;
                let audit = event(
                    project.id(),
                    AggregateRef::Project(project.id()),
                    project.revision(),
                    DomainEventKind::CredentialUpdated,
                    self.actors.human,
                    command_id,
                    None,
                    event_payload([
                        ("secret_id", json!(secret_id)),
                        ("version", json!(1)),
                        ("kind", json!(kind)),
                    ]),
                    now,
                )?;
                finish_command(
                    transaction,
                    &project,
                    envelope,
                    request_hash,
                    command_id,
                    self.actors.human,
                    vec![audit],
                    Some(resource("credential", *secret_id)),
                )
                .await
            }
            CommandPayload::ConfigureEmployeeRuntime {
                employee_id,
                binding,
            } => {
                let profile = &binding.execution_profile;
                match profile.adapter_id() {
                    "codex_cli" => forge_provider_codex::CodexAdapter::validate_profile(profile),
                    "opencode_runtime" => {
                        forge_provider_opencode::OpenCodeAdapter::validate_profile(profile)
                    }
                    "claude_code_cli" => {
                        forge_provider_claude::ClaudeAdapter::validate_profile(profile)
                    }
                    _ => return Err(crate::credentials::credential_error()),
                }
                .map_err(|_| crate::credentials::credential_error())?;
                if binding.execution_profile.adapter_id() == "claude_code_cli" {
                    let credential = transaction
                        .load_credential(
                            project.id(),
                            binding.execution_profile.credential_binding().secret_id,
                        )
                        .await?
                        .ok_or_else(crate::credentials::credential_error)?;
                    self.open_runtime_credential(
                        &credential,
                        binding.execution_profile.credential_binding(),
                        binding.execution_profile.adapter_id(),
                    )?;
                }
                transaction
                    .bind_employee_runtime(project.id(), *employee_id, binding)
                    .await?;
                let previous_revision = project.revision();
                project.record_child_mutation(now)?;
                transaction
                    .update_project(&project, previous_revision)
                    .await?;
                let audit = event(
                    project.id(),
                    AggregateRef::Project(project.id()),
                    project.revision(),
                    DomainEventKind::EmployeeRuntimeConfigured,
                    self.actors.human,
                    command_id,
                    None,
                    event_payload([
                        ("employee_id", json!(employee_id)),
                        ("profile_id", json!(binding.execution_profile.id())),
                        (
                            "profile_revision",
                            json!(binding.execution_profile.revision()),
                        ),
                    ]),
                    now,
                )?;
                finish_command(
                    transaction,
                    &project,
                    envelope,
                    request_hash,
                    command_id,
                    self.actors.human,
                    vec![audit],
                    Some(resource("employee", employee_id.as_uuid())),
                )
                .await
            }
            _ => Err(CoreError::InvalidTransport {
                field: "command",
                reason: "is not a runtime extension command".to_owned(),
            }),
        }
    }
}

fn is_runtime_command(payload: &CommandPayload) -> bool {
    matches!(
        payload,
        CommandPayload::ConfigureBootRecoveryPolicy { .. }
            | CommandPayload::AcceptRunRecoveryAssessment { .. }
            | CommandPayload::RetryCommunication { .. }
            | CommandPayload::RetryGitIntegration { .. }
            | CommandPayload::AcceptGitIntegrationResult { .. }
            | CommandPayload::EnrollCredential { .. }
            | CommandPayload::ConfigureEmployeeRuntime { .. }
            | CommandPayload::ConfigureProjectHook(_)
            | CommandPayload::ImportTaskFileSnapshot(_)
            | CommandPayload::CaptureTaskFileSnapshot(_)
            | CommandPayload::AttachTaskFileInput(_)
    )
}

impl CoreService {
    pub(crate) async fn dispatch_after_command(&self, project_id: forge_domain::ProjectId) {
        if let Err(error) = self.deliver_pending_stop_requests(project_id).await {
            warn!(project_id = %project_id, error = %error, "deferred Supervisor stop delivery failed");
        }
        if let Err(error) = self.dispatch_available(project_id).await {
            // Command state, audit Event, and outbox have already committed.
            // Dispatch is recoverable desired-state delivery, so returning this
            // error here would falsely report that the accepted command failed.
            warn!(project_id = %project_id, error = %error, "deferred scheduler dispatch failed");
        }
        if self
            .reconcile_runtime_inputs(Some(project_id))
            .await
            .is_err()
        {
            warn!(project_id = %project_id, "deferred runtime input delivery failed");
        }
    }
}

#[path = "command/receipt.rs"]
mod receipt;
pub(crate) use receipt::{finish_command, resource};
