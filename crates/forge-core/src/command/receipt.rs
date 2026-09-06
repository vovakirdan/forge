//! Canonical command fingerprints, audit receipts and idempotent replay.
use super::*;
pub(super) async fn replay_if_present(
    transaction: &mut StorageTransaction<'_>,
    envelope: &CommandEnvelope,
    request_hash: &str,
) -> Result<Option<CommandReceipt>, CoreError> {
    let Some(record) = transaction
        .lookup_idempotency(envelope.project_id, envelope.idempotency_key.as_str())
        .await?
    else {
        return Ok(None);
    };
    if record.request_hash != request_hash || record.command_name != command_name(envelope) {
        return Err(CoreError::IdempotencyConflict);
    }
    let mut receipt: CommandReceipt =
        serde_json::from_value(record.receipt).map_err(|error| CoreError::InvalidTransport {
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
pub(crate) async fn finish_command(
    transaction: &mut StorageTransaction<'_>,
    project: &Project,
    envelope: &CommandEnvelope,
    request_hash: &str,
    command_id: CommandId,
    actor: forge_domain::Actor,
    events: Vec<DomainEvent>,
    resource: Option<ResourceReference>,
) -> Result<CommandReceipt, CoreError> {
    let mut event_ids = Vec::with_capacity(events.len());
    for audit in &events {
        event_ids.push(transaction.append_event_and_outbox(audit).await?.id);
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
        actor: serde_json::to_value(actor).map_err(|error| CoreError::InvalidTransport {
            field: "idempotency.actor",
            reason: error.to_string(),
        })?,
        receipt: serde_json::to_value(&receipt).map_err(|error| CoreError::InvalidTransport {
            field: "idempotency.receipt",
            reason: error.to_string(),
        })?,
        event_id: event_ids.first().copied(),
        response_revision: Some(project.revision()),
    };
    transaction.insert_idempotency(&record).await?;
    Ok(receipt)
}

pub(crate) fn command_name(envelope: &CommandEnvelope) -> String {
    match envelope.name {
        forge_protocol::wire::CommandName::CreateProject => "create_project",
        forge_protocol::wire::CommandName::CreatePipeline => "create_pipeline",
        forge_protocol::wire::CommandName::CreateEmployee => "create_employee",
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
        forge_protocol::wire::CommandName::EnrollCredential => "enroll_credential",
        forge_protocol::wire::CommandName::ConfigureBootRecoveryPolicy => {
            "configure_boot_recovery_policy"
        }
        forge_protocol::wire::CommandName::AcceptRunRecoveryAssessment => {
            "accept_run_recovery_assessment"
        }
    }
    .to_owned()
}

pub(crate) fn resource(kind: &str, id: uuid::Uuid) -> ResourceReference {
    ResourceReference {
        kind: kind.to_owned(),
        id: id.to_string(),
    }
}

pub(super) fn command_fingerprint(
    envelope: &CommandEnvelope,
    actor: forge_domain::Actor,
) -> Result<String, CoreError> {
    let canonical = json!({
        "command_name": command_name(envelope),
        "project_id": envelope.project_id.as_uuid(),
        "expected_project_revision": envelope.expected_project_revision,
        "payload": envelope.canonical_payload(),
        "actor": actor,
    });
    let bytes = serde_json::to_vec(&canonical).map_err(|error| CoreError::InvalidTransport {
        field: "idempotency.request",
        reason: error.to_string(),
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
