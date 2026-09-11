//! Host-owned input delivery; mailbox contents never establish native acceptance.
use crate::{
    SupervisorConfig, SupervisorError, new_id, podman::scoped_directory, registry::RunRegistry,
};
use forge_domain::{
    ActorKind, RuntimeCapability,
    communication::{EmployeeMessage, MessageTarget},
    runtime::RuntimeLaunchSpec,
};
use forge_protocol::supervisor::v1::{
    DeliverRuntimeInput, EnvironmentPresence, ProvisionRun, RuntimeInputReceipt,
    RuntimeInputStatus as Status, deliver_runtime_input::Action,
};
use forge_provider_common::{PrivateMaterialization, SecretBytes};
use std::{io, path::Path};
use uuid::Uuid;

pub(crate) const MAX_INPUT_BYTES: u64 = 128 * 1024;

pub(crate) fn validate(
    input: &DeliverRuntimeInput,
    provision: &ProvisionRun,
) -> Result<(), SupervisorError> {
    crate::execution_assignment::validate(provision)?;
    if Uuid::parse_str(&input.command_id)
        .ok()
        .is_none_or(|id| id.get_version_num() != 7)
        || input.sequence == 0
        || input.run_id != provision.run_id
        || input.lease_fencing_token != provision.lease_fencing_token
        || input.environment_epoch != provision.environment_epoch
    {
        return Err(SupervisorError::InvalidRunSpec);
    }
    let spec: RuntimeLaunchSpec = serde_json::from_str(&provision.run_spec_json)?;
    if provision.run_spec_version != u32::from(spec.schema_version)
        || !matches!(spec.schema_version, 2 | 3 | 6)
        || !matches!(
            spec.binding.execution_profile.adapter_id(),
            "claude_code_cli" | "codex_cli" | "opencode_runtime"
        )
        || !spec
            .binding
            .execution_profile
            .capability_profile()
            .capabilities
            .contains(&RuntimeCapability::LiveInput)
    {
        return Err(SupervisorError::InvalidRunSpec);
    }
    match &input.action {
        Some(Action::Message(message)) => {
            if message.source_message_json.len() > MAX_INPUT_BYTES as usize {
                return Err(SupervisorError::InvalidRunSpec);
            }
            let source: EmployeeMessage = serde_json::from_str(&message.source_message_json)?;
            let source = source.data();
            if source.project_id != spec.project_id
                || source.employee_id.to_string() != provision.employee_id
                || (source.sender.kind() == ActorKind::Employee
                    && source.sender.id().to_string() == provision.employee_id)
            {
                return Err(SupervisorError::InvalidRunSpec);
            }
            let Some(context) = source.target.task_context() else {
                return Err(SupervisorError::InvalidRunSpec);
            };
            if context.task_id.to_string() != provision.task_id
                || context.stage_id.as_str() != provision.stage_id
            {
                return Err(SupervisorError::InvalidRunSpec);
            }
            if let MessageTarget::ExactRun {
                run_id,
                fencing_token,
                environment_epoch,
                ..
            } = source.target
                && (run_id.to_string() != input.run_id
                    || fencing_token != input.lease_fencing_token
                    || environment_epoch != input.environment_epoch)
            {
                return Err(SupervisorError::InvalidRunSpec);
            }
        }
        Some(Action::CloseAfterTurn(close))
            if matches!(
                close.reason_code.as_str(),
                "assignment_completed" | "authority_lost"
            ) => {}
        _ => return Err(SupervisorError::InvalidRunSpec),
    }
    Ok(())
}

pub(crate) async fn deliver(
    config: &SupervisorConfig,
    registry: &RunRegistry,
    input: DeliverRuntimeInput,
) -> Result<(), SupervisorError> {
    let provision = {
        let mut journal = registry.journal.lock().await;
        // Register replays completed native receipts without requiring a live Run.
        let record = journal.records().into_iter().find(|record| {
            record.provision.run_id == input.run_id
                && record.provision.lease_fencing_token == input.lease_fencing_token
                && record.provision.environment_epoch == input.environment_epoch
        });
        let Some(record) = record else {
            return Err(SupervisorError::InvalidJournalMessage);
        };
        if !journal.register_input(&input)? {
            registry.changed.notify_one();
            return Ok(());
        }
        if record.presence == EnvironmentPresence::Quiescent {
            journal.record_input_receipt(receipt(&input, Status::StaleScope))?;
            registry.changed.notify_one();
            return Ok(());
        }
        if validate(&input, &record.provision).is_err() {
            journal.record_input_receipt(receipt(&input, Status::Unsupported))?;
            registry.changed.notify_one();
            return Ok(());
        }
        record.provision
    };
    let grants = scoped_directory(&config.grants_directory, &provision);
    let dir = grants.join("inputs");
    crate::surface::private_directory(&grants)?;
    crate::surface::private_directory(&dir)?;
    let bytes = SecretBytes::new(serde_json::to_vec(&input)?);
    let path = dir.join(format!("{:020}-{}.json", input.sequence, input.command_id));
    immutable(&path, &bytes)?;
    Ok(())
}

pub(crate) fn receipt(input: &DeliverRuntimeInput, status: Status) -> RuntimeInputReceipt {
    RuntimeInputReceipt {
        message_id: new_id(),
        input_command_id: input.command_id.clone(),
        run_id: input.run_id.clone(),
        lease_fencing_token: input.lease_fencing_token,
        environment_epoch: input.environment_epoch,
        status: status as i32,
    }
}

/// Removes only a canonical-acknowledged transport copy; original message remains.
pub(crate) fn cleanup_mailbox(
    config: &SupervisorConfig,
    input: &DeliverRuntimeInput,
) -> Result<(), SupervisorError> {
    if Uuid::parse_str(&input.run_id).is_err() || Uuid::parse_str(&input.command_id).is_err() {
        return Err(SupervisorError::InvalidJournalMessage);
    }
    let path = config
        .grants_directory
        .join(&input.run_id)
        .join(input.environment_epoch.to_string())
        .join("inputs")
        .join(format!("{:020}-{}.json", input.sequence, input.command_id));
    let receipt = config
        .state_directory
        .join("evidence")
        .join(&input.run_id)
        .join(input.environment_epoch.to_string())
        .join("input-receipts")
        .join(format!("{}.json", input.command_id));
    for path in [path, receipt] {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(crate) fn immutable(path: &Path, bytes: &SecretBytes) -> Result<(), SupervisorError> {
    if path.try_exists()? {
        let existing = PrivateMaterialization::open(path)
            .and_then(|file| file.read(MAX_INPUT_BYTES))
            .map_err(|_| SupervisorError::UnsafeSurface)?;
        return if existing.expose() == bytes.expose() {
            Ok(())
        } else {
            Err(SupervisorError::ConflictingProvision)
        };
    }
    let parent = path.parent().ok_or(SupervisorError::UnsafeSurface)?;
    let temporary = parent.join(format!(".pending-{}", new_id()));
    PrivateMaterialization::create(&temporary, bytes)
        .map_err(|_| SupervisorError::UnsafeSurface)?;
    std::fs::rename(&temporary, path)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

/// Only native-correlated driver receipts are mirrored into durable Supervisor replay.
pub(crate) async fn collect(
    config: &SupervisorConfig,
    registry: &RunRegistry,
    provision: &ProvisionRun,
) -> Result<(), SupervisorError> {
    let root = scoped_directory(&config.state_directory.join("evidence"), provision);
    let path = root.join("input-receipts");
    let entries = match std::fs::read_dir(&path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries.take(130) {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_str().ok_or(SupervisorError::UnsafeSurface)?;
        if name.starts_with(".pending-") {
            continue;
        }
        let Some(id) = name
            .strip_suffix(".json")
            .and_then(|id| Uuid::parse_str(id).ok())
        else {
            return Err(SupervisorError::InvalidJournalMessage);
        };
        let bytes = PrivateMaterialization::read_beneath(
            &root,
            Path::new("input-receipts").join(name).as_path(),
            4096,
        )
        .map_err(|_| SupervisorError::UnsafeSurface)?;
        let receipt: RuntimeInputReceipt = serde_json::from_slice(bytes.expose())?;
        if receipt.input_command_id != id.to_string()
            || receipt.run_id != provision.run_id
            || receipt.lease_fencing_token != provision.lease_fencing_token
            || receipt.environment_epoch != provision.environment_epoch
        {
            return Err(SupervisorError::InvalidJournalMessage);
        }
        let mut journal = registry.journal.lock().await;
        let request = journal
            .input_request(&receipt.input_command_id)
            .ok_or(SupervisorError::InvalidJournalMessage)?;
        let valid = matches!(
            (&request.action, Status::try_from(receipt.status)),
            (
                Some(Action::Message(_)),
                Ok(Status::RuntimeAccepted | Status::DeliveryUnknown)
            ) | (Some(Action::CloseAfterTurn(_)), Ok(Status::InputClosed))
        );
        if !valid {
            return Err(SupervisorError::InvalidJournalMessage);
        }
        journal.record_input_receipt(receipt)?;
        registry.changed.notify_one();
    }
    Ok(())
}

#[cfg(test)]
mod tests;
