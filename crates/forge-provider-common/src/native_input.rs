//! Sandbox-local transport mechanics. No Task, provider or canonical-store logic.
use crate::{PrivateMaterialization, SecretBytes};
use forge_protocol::{
    runtime::RuntimeInputConfig,
    supervisor::v1::{
        DeliverRuntimeInput, RuntimeInputReceipt, RuntimeInputStatus, deliver_runtime_input::Action,
    },
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs, io,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub const MAX_INPUT_BYTES: u64 = 128 * 1024;
pub const MAX_PENDING_INPUTS: usize = 129;
const MAX_RUN_INPUTS: usize = 16_384;

/// Fixed, private container layout. Never loads the host home or user config.
pub fn managed_scope(adapter: &str) -> io::Result<Option<RuntimeInputConfig>> {
    let bytes = PrivateMaterialization::open(Path::new("/run/forge-input/invocation.json"))
        .and_then(|file| file.read(1024 * 1024))
        .map_err(|_| invalid())?;
    let invocation: forge_protocol::runtime::RunnerInvocation =
        serde_json::from_slice(bytes.expose()).map_err(|_| invalid())?;
    if invocation.adapter_id != adapter {
        return Err(invalid());
    }
    if let Some(scope) = &invocation.runtime_input {
        validate_scope(scope)?;
    }
    Ok(invocation.runtime_input)
}

pub fn managed_mailbox(scope: RuntimeInputConfig) -> io::Result<NativeMailbox> {
    NativeMailbox::open(
        scope,
        Path::new("/run/forge"),
        PathBuf::from("/run/forge-input/inputs"),
        PathBuf::from("/run/forge-evidence/input-receipts"),
    )
}

pub struct NativeMailbox {
    scope: RuntimeInputConfig,
    inputs: PathBuf,
    receipts: PathBuf,
    sent: BTreeMap<String, [u8; 32]>,
}

impl NativeMailbox {
    /// Called once per driver process. A retained marker refuses uncertain resume.
    pub fn open(
        scope: RuntimeInputConfig,
        private: &Path,
        inputs: PathBuf,
        receipts: PathBuf,
    ) -> io::Result<Self> {
        validate_scope(&scope)?;
        PrivateMaterialization::create(
            &private.join("native-input-started"),
            &SecretBytes::new(b"1\n".to_vec()),
        )
        .map_err(|_| invalid())?;
        private_directory(&receipts)?;
        Ok(Self {
            scope,
            inputs,
            receipts,
            sent: BTreeMap::new(),
        })
    }
    pub fn scope(&self) -> &RuntimeInputConfig {
        &self.scope
    }

    /// Close controls dominate queued text. Busy drivers call this only at turn boundaries.
    pub fn next(&self) -> io::Result<Option<DeliverRuntimeInput>> {
        let inputs = read_inputs(&self.inputs, &self.scope)?;
        if let Some(close) = inputs
            .iter()
            .find(|input| matches!(input.action, Some(Action::CloseAfterTurn(_))))
        {
            return Ok(Some(close.clone()));
        }
        for input in inputs {
            if let Some(prior) = self.sent.get(&input.command_id) {
                if *prior != fingerprint(&input)? {
                    return Err(invalid());
                }
            } else {
                return Ok(Some(input));
            }
        }
        Ok(None)
    }
    pub fn mark_sent(&mut self, input: &DeliverRuntimeInput) -> io::Result<()> {
        if self.sent.len() >= MAX_RUN_INPUTS {
            return Err(invalid());
        }
        let digest = fingerprint(input)?;
        if let Some(prior) = self.sent.insert(input.command_id.clone(), digest)
            && prior != digest
        {
            return Err(invalid());
        }
        Ok(())
    }
    /// Only the provider-correlated success path calls this; write success is not proof.
    pub fn accepted(&self, input: &DeliverRuntimeInput) -> io::Result<()> {
        if !matches!(input.action, Some(Action::Message(_))) {
            return Err(invalid());
        }
        persist_receipt(&self.receipts, input, RuntimeInputStatus::RuntimeAccepted)
    }
    /// Driver has ended input after the current turn, without sending another prompt.
    pub fn closed(&self, close: &DeliverRuntimeInput) -> io::Result<()> {
        if !matches!(close.action, Some(Action::CloseAfterTurn(_))) {
            return Err(invalid());
        }
        persist_receipt(&self.receipts, close, RuntimeInputStatus::InputClosed)?;
        for input in read_inputs(&self.inputs, &self.scope)? {
            if matches!(input.action, Some(Action::Message(_)))
                && !self.sent.contains_key(&input.command_id)
            {
                persist_receipt(&self.receipts, &input, RuntimeInputStatus::DeliveryUnknown)?;
            }
        }
        Ok(())
    }
}

pub fn addressed_prompt(input: &DeliverRuntimeInput) -> io::Result<SecretBytes> {
    let Some(Action::Message(message)) = &input.action else {
        return Err(invalid());
    };
    if message.source_message_json.is_empty()
        || message.source_message_json.len() > MAX_INPUT_BYTES as usize
    {
        return Err(invalid());
    }
    Ok(SecretBytes::new(format!("Forge addressed instruction {}. Read the canonical source below, then acknowledge or reply through the Forge Inbox tools.\n{}",input.command_id,message.source_message_json).into_bytes()))
}

pub fn validate_scope(scope: &RuntimeInputConfig) -> io::Result<()> {
    if Uuid::parse_str(&scope.run_id)
        .ok()
        .is_none_or(|id| id.get_version_num() != 7)
        || scope.fencing_token == 0
        || scope.environment_epoch == 0
    {
        return Err(invalid());
    }
    Ok(())
}

pub fn read_inputs(
    path: &Path,
    scope: &RuntimeInputConfig,
) -> io::Result<Vec<DeliverRuntimeInput>> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut inputs = Vec::new();
    let mut scanned = 0;
    for entry in entries {
        scanned += 1;
        if scanned > MAX_PENDING_INPUTS + 16 {
            return Err(invalid());
        }
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(".pending-") {
            continue;
        }
        let bytes = match PrivateMaterialization::open(&entry.path())
            .and_then(|file| file.read(MAX_INPUT_BYTES))
        {
            Ok(bytes) => bytes,
            Err(crate::SecretStoreError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                continue;
            }
            Err(_) => return Err(invalid()),
        };
        let input: DeliverRuntimeInput =
            serde_json::from_slice(bytes.expose()).map_err(|_| invalid())?;
        let expected = format!("{:020}-{}.json", input.sequence, input.command_id);
        if inputs.len() >= MAX_PENDING_INPUTS
            || entry.file_name() != std::ffi::OsStr::new(&expected)
            || input.run_id != scope.run_id
            || input.lease_fencing_token != scope.fencing_token
            || input.environment_epoch != scope.environment_epoch
            || input.sequence == 0
            || Uuid::parse_str(&input.command_id)
                .ok()
                .is_none_or(|id| id.get_version_num() != 7)
        {
            return Err(invalid());
        }
        match &input.action {
            Some(Action::Message(message)) if !message.source_message_json.is_empty() => {}
            Some(Action::CloseAfterTurn(close))
                if matches!(
                    close.reason_code.as_str(),
                    "assignment_completed" | "authority_lost"
                ) => {}
            _ => return Err(invalid()),
        }
        inputs.push(input);
    }
    inputs.sort_by_key(|input| input.sequence);
    if inputs
        .windows(2)
        .any(|pair| pair[0].sequence == pair[1].sequence)
    {
        return Err(invalid());
    }
    Ok(inputs)
}

pub fn persist_receipt(
    path: &Path,
    input: &DeliverRuntimeInput,
    status: RuntimeInputStatus,
) -> io::Result<()> {
    let receipt = RuntimeInputReceipt {
        message_id: Uuid::now_v7().to_string(),
        input_command_id: input.command_id.clone(),
        run_id: input.run_id.clone(),
        lease_fencing_token: input.lease_fencing_token,
        environment_epoch: input.environment_epoch,
        status: status as i32,
    };
    let path = path.join(format!("{}.json", input.command_id));
    if path.try_exists()? {
        let bytes = PrivateMaterialization::open(&path)
            .and_then(|file| file.read(4096))
            .map_err(|_| invalid())?;
        let prior: RuntimeInputReceipt =
            serde_json::from_slice(bytes.expose()).map_err(|_| invalid())?;
        if prior.input_command_id == receipt.input_command_id
            && prior.status == receipt.status
            && prior.run_id == receipt.run_id
            && prior.lease_fencing_token == receipt.lease_fencing_token
            && prior.environment_epoch == receipt.environment_epoch
        {
            return Ok(());
        }
        return Err(invalid());
    }
    atomic_write(
        &path,
        &SecretBytes::new(serde_json::to_vec(&receipt).map_err(|_| invalid())?),
    )
}

pub fn atomic_write(path: &Path, bytes: &SecretBytes) -> io::Result<()> {
    if path.try_exists()? {
        let existing = PrivateMaterialization::open(path)
            .and_then(|file| file.read(MAX_INPUT_BYTES))
            .map_err(|_| invalid())?;
        return if existing.expose() == bytes.expose() {
            Ok(())
        } else {
            Err(invalid())
        };
    }
    let parent = path.parent().ok_or_else(invalid)?;
    let temporary = parent.join(format!(".pending-{}", Uuid::now_v7()));
    PrivateMaterialization::create(&temporary, bytes).map_err(|_| invalid())?;
    fs::rename(&temporary, path)?;
    fs::File::open(parent)?.sync_all()
}

fn fingerprint(input: &DeliverRuntimeInput) -> io::Result<[u8; 32]> {
    Ok(Sha256::digest(serde_json::to_vec(input).map_err(|_| invalid())?).into())
}
fn private_directory(path: &Path) -> io::Result<()> {
    if !path.try_exists()? {
        fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir()
        || meta.mode() & 0o077 != 0
        || meta.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(invalid());
    }
    Ok(())
}
pub fn invalid() -> io::Error {
    io::Error::other("native runtime transport contract failed")
}
