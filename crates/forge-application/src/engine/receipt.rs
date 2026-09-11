//! Canonical command fingerprints, audit receipts and idempotent replay.
use super::{CommandError, CommandTransaction, IdempotencyRecord};
use crate::CommandEnvelope;
use forge_domain::{CommandId, DomainEvent, Project};
use forge_protocol::wire::{CommandReceipt, CommandStatus, ResourceReference};
use serde_json::json;
use sha2::{Digest, Sha256};
pub async fn replay_if_present(
    transaction: &mut impl CommandTransaction,
    envelope: &CommandEnvelope,
    request_hash: &str,
) -> Result<Option<CommandReceipt>, CommandError> {
    let Some(record) = transaction
        .lookup_idempotency(envelope.project_id, envelope.idempotency_key.as_str())
        .await?
    else {
        return Ok(None);
    };
    if record.request_hash != request_hash || record.command_name != command_name(envelope) {
        return Err(CommandError::IdempotencyConflict);
    }
    let mut receipt: CommandReceipt =
        serde_json::from_value(record.receipt).map_err(|error| CommandError::InvalidTransport {
            field: "idempotency.receipt",
            reason: error.to_string(),
        })?;
    receipt.status = CommandStatus::Replayed;
    Ok(Some(receipt))
}

#[expect(
    clippy::too_many_arguments,
    reason = "the canonical transaction boundary keeps command audit inputs explicit"
)]
pub async fn finish_command(
    transaction: &mut impl CommandTransaction,
    project: &Project,
    envelope: &CommandEnvelope,
    request_hash: &str,
    command_id: CommandId,
    actor: forge_domain::Actor,
    events: Vec<DomainEvent>,
    resource: Option<ResourceReference>,
) -> Result<CommandReceipt, CommandError> {
    let mut event_ids = Vec::with_capacity(events.len());
    for audit in &events {
        event_ids.push(transaction.append_event_and_outbox(audit).await?);
    }
    let receipt = CommandReceipt {
        command_id: command_id.as_uuid().to_string(),
        status: CommandStatus::Applied,
        project_revision: project.revision(),
        event_ids: event_ids.iter().map(ToString::to_string).collect(),
        resource,
    };
    let record = IdempotencyRecord {
        project_id: project.id(),
        key: envelope.idempotency_key.as_str().to_owned(),
        command_id,
        command_name: command_name(envelope),
        expected_revision: Some(envelope.expected_project_revision),
        request_hash: request_hash.to_owned(),
        actor: serde_json::to_value(actor).map_err(|error| CommandError::InvalidTransport {
            field: "idempotency.actor",
            reason: error.to_string(),
        })?,
        receipt: serde_json::to_value(&receipt).map_err(|error| {
            CommandError::InvalidTransport {
                field: "idempotency.receipt",
                reason: error.to_string(),
            }
        })?,
        event_id: event_ids.first().copied(),
        response_revision: Some(project.revision()),
    };
    transaction.insert_idempotency(&record).await?;
    Ok(receipt)
}

pub fn command_name(envelope: &CommandEnvelope) -> String {
    match envelope.name {
        forge_protocol::wire::CommandName::RegisterProjectRepository => {
            "register_project_repository"
        }
        forge_protocol::wire::CommandName::BindTaskGitRepository => "bind_task_git_repository",
        forge_protocol::wire::CommandName::SetTaskGitSourcePolicy => "set_task_git_source_policy",
        forge_protocol::wire::CommandName::CreateProject => "create_project",
        forge_protocol::wire::CommandName::CreatePipeline => "create_pipeline",
        forge_protocol::wire::CommandName::PublishPipelineVersion => "publish_pipeline_version",
        forge_protocol::wire::CommandName::SetPipelineDefaultVersion => {
            "set_pipeline_default_version"
        }
        forge_protocol::wire::CommandName::DeletePipeline => "delete_pipeline",
        forge_protocol::wire::CommandName::CreateEmployee => "create_employee",
        forge_protocol::wire::CommandName::AmendEmployee => "amend_employee",
        forge_protocol::wire::CommandName::EnableEmployee => "enable_employee",
        forge_protocol::wire::CommandName::DisableEmployee => "disable_employee",
        forge_protocol::wire::CommandName::RetireEmployee => "retire_employee",
        forge_protocol::wire::CommandName::StopEmployee => "stop_employee",
        forge_protocol::wire::CommandName::SetNextRunEmployee => "set_next_run_employee",
        forge_protocol::wire::CommandName::ClearNextRunEmployee => "clear_next_run_employee",
        forge_protocol::wire::CommandName::ScheduleTaskResume => "schedule_task_resume",
        forge_protocol::wire::CommandName::ConfigureResolverRoute => "configure_resolver_route",
        forge_protocol::wire::CommandName::RaiseEscalation => "raise_escalation",
        forge_protocol::wire::CommandName::SubmitHumanResolution => "submit_human_resolution",
        forge_protocol::wire::CommandName::RerouteEscalation => "reroute_escalation",
        forge_protocol::wire::CommandName::CancelTaskResume => "cancel_task_resume",
        forge_protocol::wire::CommandName::PauseTask => "pause_task",
        forge_protocol::wire::CommandName::OpenEmployeeThread => "open_employee_thread",
        forge_protocol::wire::CommandName::SendEmployeeMessage => "send_employee_message",
        forge_protocol::wire::CommandName::WaiveMessageRequirement => "waive_message_requirement",
        forge_protocol::wire::CommandName::CreateTask => "create_task",
        forge_protocol::wire::CommandName::AmendDraft => "amend_draft",
        forge_protocol::wire::CommandName::ApproveTask => "approve_task",
        forge_protocol::wire::CommandName::CancelTask => "cancel_task",
        forge_protocol::wire::CommandName::ResumeTask => "resume_task",
        forge_protocol::wire::CommandName::SetTaskPriority => "set_task_priority",
        forge_protocol::wire::CommandName::CreateDependency => "create_dependency",
        forge_protocol::wire::CommandName::RemoveDependency => "remove_dependency",
        forge_protocol::wire::CommandName::StartProjectExecution => "start_project_execution",
        forge_protocol::wire::CommandName::StopProjectExecution => "stop_project_execution",
        forge_protocol::wire::CommandName::SubmitExternalStageOutcome => {
            "submit_external_stage_outcome"
        }
        forge_protocol::wire::CommandName::ConfigureEmployeeRuntime => "configure_employee_runtime",
        forge_protocol::wire::CommandName::ConfigureProjectHook => "configure_project_hook",
        forge_protocol::wire::CommandName::ImportTaskFileSnapshot => "import_task_file_snapshot",
        forge_protocol::wire::CommandName::CaptureTaskFileSnapshot => "capture_task_file_snapshot",
        forge_protocol::wire::CommandName::AttachTaskFileInput => "attach_task_file_input",
        forge_protocol::wire::CommandName::EnrollCredential => "enroll_credential",
        forge_protocol::wire::CommandName::ConfigureBootRecoveryPolicy => {
            "configure_boot_recovery_policy"
        }
        forge_protocol::wire::CommandName::AcceptRunRecoveryAssessment => {
            "accept_run_recovery_assessment"
        }
        forge_protocol::wire::CommandName::RetryCommunication => "retry_communication",
        forge_protocol::wire::CommandName::ReportFinding => "report_finding",
        forge_protocol::wire::CommandName::TriageFinding => "triage_finding",
        forge_protocol::wire::CommandName::PromoteFinding => "promote_finding",
        forge_protocol::wire::CommandName::RetryGitIntegration => "retry_git_integration",
        forge_protocol::wire::CommandName::AcceptGitIntegrationResult => {
            "accept_git_integration_result"
        }
    }
    .to_owned()
}

pub fn resource(kind: &str, id: uuid::Uuid) -> ResourceReference {
    ResourceReference {
        kind: kind.to_owned(),
        id: id.to_string(),
    }
}

pub fn command_fingerprint(
    envelope: &CommandEnvelope,
    actor: forge_domain::Actor,
) -> Result<String, CommandError> {
    let canonical = json!({
        "command_name": command_name(envelope),
        "project_id": envelope.project_id.as_uuid(),
        "expected_project_revision": envelope.expected_project_revision,
        "payload": envelope.canonical_payload(),
        "actor": actor,
    });
    let bytes = serde_json::to_vec(&canonical).map_err(|error| CommandError::InvalidTransport {
        field: "idempotency.request",
        reason: error.to_string(),
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
