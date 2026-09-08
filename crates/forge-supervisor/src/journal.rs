//! Bounded operational replay state. This is not Forge canonical storage.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

use forge_protocol::supervisor::v1::{
    AcknowledgementDisposition, CoreAcknowledgement, EnvironmentPresence, ProvisionRun,
    RunInventoryEntry, SupervisorToCore, supervisor_to_core,
};
use nix::unistd::Uid;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{SupervisorError, new_id};
mod inspection;
mod integration;
mod runtime_inputs;
pub(crate) use inspection::GitSourceScope;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct RunRecord {
    /// Minimal host-owned source identity survives prompt/spec tombstone compaction.
    #[serde(default)]
    pub git_source: Option<GitSourceScope>,
    pub provision: ProvisionRun,
    #[serde(default)]
    pub payload_hash: String,
    #[serde(default)]
    pub environment_id: String,
    pub boot_id: String,
    pub presence: EnvironmentPresence,
    pub last_sequence: u64,
    pub pending: Vec<SupervisorToCore>,
    /// Only a durable Accepted Stopped observation ACK proves Core learned that
    /// this physical reservation is quiescent. Legacy records remain visible.
    #[serde(default)]
    pub quiescence_confirmed: bool,
    /// Retained even after the corresponding Stopping event is acknowledged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

impl RunRecord {
    fn inventory_required(&self) -> bool {
        self.presence != EnvironmentPresence::Quiescent
            || !self.quiescence_confirmed
            || !self.pending.is_empty()
    }

    fn compact(&mut self) {
        if self.presence == EnvironmentPresence::Quiescent && self.pending.is_empty() {
            // Original prompts belong in canonical evidence, not lifetime-long
            // operational tombstones. The hash retains immutable request equality.
            self.provision.command_id.clear();
            self.provision.employee_id.clear();
            self.provision.stage_id.clear();
            self.provision.context_snapshot_id.clear();
            self.provision.run_spec_json.clear();
            self.provision.run_spec_version = 0;
            self.provision.attempt = 0;
        }
    }

    pub fn inventory(&self) -> RunInventoryEntry {
        RunInventoryEntry {
            run_id: self.provision.run_id.clone(),
            lease_fencing_token: self.provision.lease_fencing_token,
            environment_epoch: self.provision.environment_epoch,
            last_sequence: self.last_sequence,
            presence: self.presence as i32,
            environment_id: self.environment_id.clone(),
            provision_boot_id: self.boot_id.clone(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    host_id: String,
    runs: BTreeMap<String, RunRecord>,
    #[serde(default)]
    runtime_inputs: BTreeMap<String, runtime_inputs::InputRecord>,
    #[serde(default)]
    git_surface_owners: BTreeMap<String, String>,
    #[serde(default)]
    git_inspections: BTreeMap<String, inspection::InspectionReceipt>,
    #[serde(default)]
    integrations: BTreeMap<uuid::Uuid, integration::IntegrationRecord>,
    #[serde(default)]
    integration_receipts: BTreeMap<uuid::Uuid, integration::IntegrationReceipt>,
}

pub(crate) struct Journal {
    directory: PathBuf,
    snapshot: Snapshot,
    max_bytes: usize,
    last_write_healthy: std::sync::atomic::AtomicBool,
    // An OS lock, unlike a PID file, is released when the process dies.
    _lock: File,
}

impl Drop for Journal {
    fn drop(&mut self) {
        // A concurrently forked child briefly inherits the same open-file
        // description until CLOEXEC. Closing only our descriptor would keep
        // flock held during that window, despite the Journal owner being gone.
        let _ = self._lock.unlock();
    }
}

impl Journal {
    pub fn open(
        directory: &Path,
        host_id: &str,
        max_bytes: usize,
    ) -> Result<Self, SupervisorError> {
        prepare_directory(directory)?;
        let lock_path = directory.join("lock");
        let lock = open_private(&lock_path)?;
        lock.try_lock().map_err(|_| SupervisorError::JournalInUse)?;
        let path = directory.join("journal.json");
        let mut snapshot = match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                validate_file(&metadata)?;
                if metadata.len() > max_bytes as u64 {
                    return Err(SupervisorError::JournalFull);
                }
                serde_json::from_slice::<Snapshot>(&fs::read(&path)?)?
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Snapshot {
                version: 1,
                host_id: host_id.to_owned(),
                runs: BTreeMap::new(),
                runtime_inputs: BTreeMap::new(),
                git_surface_owners: BTreeMap::new(),
                git_inspections: BTreeMap::new(),
                integrations: BTreeMap::new(),
                integration_receipts: BTreeMap::new(),
            },
            Err(error) => return Err(error.into()),
        };
        if snapshot.version != 1 || snapshot.host_id != host_id {
            return Err(SupervisorError::JournalIdentity);
        }
        // A persisted active flag is not proof that a process survived. TASK-12
        // reconciles actual Podman identities; the fake executor cannot adopt.
        for record in snapshot.runs.values_mut() {
            if record.payload_hash.is_empty() {
                record.payload_hash = provision_hash(&record.provision)?;
            }
            if record.presence == EnvironmentPresence::Active {
                record.presence = EnvironmentPresence::Unknown;
            }
            record.compact();
        }
        let journal = Self {
            directory: directory.to_owned(),
            snapshot,
            max_bytes,
            last_write_healthy: std::sync::atomic::AtomicBool::new(true),
            _lock: lock,
        };
        journal.persist(&journal.snapshot)?;
        Ok(journal)
    }

    /// Admits exactly once. A tombstone prevents replaying already finished work.
    pub fn register(
        &mut self,
        provision: &ProvisionRun,
        boot_id: &str,
    ) -> Result<bool, SupervisorError> {
        crate::execution_assignment::validate(provision)?;
        if [
            &provision.command_id,
            &provision.run_id,
            &provision.context_snapshot_id,
        ]
        .iter()
        .any(|id| uuid::Uuid::parse_str(id).is_err())
            || provision.lease_fencing_token == 0
            || provision.environment_epoch == 0
            || provision.run_spec_version == 0
        {
            return Err(SupervisorError::InvalidJournalMessage);
        }
        let key = scope_key(provision);
        let payload_hash = provision_hash(provision)?;
        if let Some(record) = self.snapshot.runs.get(&key) {
            if payload_hash == record.payload_hash {
                return Ok(false);
            }
            return Err(SupervisorError::ConflictingProvision);
        }
        if self.snapshot.runs.values().any(|record| {
            record.provision.run_id == provision.run_id
                && (record.presence != EnvironmentPresence::Quiescent
                    || record.provision.environment_epoch >= provision.environment_epoch
                    || record.provision.lease_fencing_token > provision.lease_fencing_token)
        }) {
            return Err(SupervisorError::ConflictingProvision);
        }
        if provision.run_spec_version == 2 {
            let surface_id = serde_json::from_str::<serde_json::Value>(&provision.run_spec_json)?
                .get("surface_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or(SupervisorError::InvalidJournalMessage)?;
            if self.snapshot.runs.values().any(|record| {
                record.provision.run_spec_version == 2
                    && record.presence != EnvironmentPresence::Quiescent
                    && serde_json::from_str::<serde_json::Value>(&record.provision.run_spec_json)
                        .ok()
                        .and_then(|value| {
                            value
                                .get("surface_id")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_owned)
                        })
                        .is_some_and(|existing| existing == surface_id)
            }) {
                return Err(SupervisorError::ConflictingProvision);
            }
        }
        if self
            .snapshot
            .runs
            .values()
            .filter(|record| record.inventory_required())
            .count()
            >= forge_protocol::SUPERVISOR_INVENTORY_LIMIT
        {
            return Err(SupervisorError::InventoryCapacity);
        }
        self.change(|snapshot| {
            let git_source = GitSourceScope::from_provision(provision);
            if let Some(source) = &git_source {
                snapshot
                    .git_surface_owners
                    .insert(source.surface_id.to_string(), key.clone());
            }
            snapshot.runs.insert(
                key,
                RunRecord {
                    git_source,
                    provision: provision.clone(),
                    payload_hash,
                    environment_id: String::new(),
                    boot_id: boot_id.to_owned(),
                    presence: EnvironmentPresence::Active,
                    last_sequence: 0,
                    pending: Vec::new(),
                    quiescence_confirmed: false,
                    stop_reason: None,
                },
            );
            Ok(())
        })?;
        Ok(true)
    }

    pub fn append(&mut self, message: SupervisorToCore) -> Result<(), SupervisorError> {
        let (run_id, fence, epoch, sequence) =
            message_scope(&message).ok_or(SupervisorError::InvalidJournalMessage)?;
        let key = format!("{run_id}:{fence}:{epoch}");
        self.change(|snapshot| {
            let record = snapshot
                .runs
                .get_mut(&key)
                .ok_or(SupervisorError::InvalidJournalMessage)?;
            if sequence
                != record
                    .last_sequence
                    .checked_add(1)
                    .ok_or(SupervisorError::SequenceOverflow)?
            {
                return Err(SupervisorError::InvalidJournalMessage);
            }
            record.last_sequence = sequence;
            if let Some(supervisor_to_core::Message::ObservedRunEvent(event)) = &message.message
                && event.kind == forge_protocol::supervisor::v1::RunEventKind::Stopping as i32
                && let Ok(details) = serde_json::from_str::<serde_json::Value>(&event.details_json)
                && let Some(reason @ ("wall_limit" | "core_stop_requested")) = details
                    .get("reason_code")
                    .and_then(serde_json::Value::as_str)
                && record.stop_reason.is_none()
            {
                record.stop_reason = Some(reason.into());
            }
            record.pending.push(message);
            Ok(())
        })
    }

    pub fn finish(
        &mut self,
        provision: &ProvisionRun,
        quiescent: bool,
    ) -> Result<(), SupervisorError> {
        self.change(|snapshot| {
            let record = snapshot
                .runs
                .get_mut(&scope_key(provision))
                .ok_or(SupervisorError::InvalidJournalMessage)?;
            record.presence = if quiescent {
                EnvironmentPresence::Quiescent
            } else {
                record.quiescence_confirmed = false;
                EnvironmentPresence::Unknown
            };
            record.compact();
            Ok(())
        })
    }

    pub fn acknowledge(&mut self, ack: &CoreAcknowledgement) -> Result<(), SupervisorError> {
        // Retryable internal failures are not durable decisions by Core.
        let disposition = AcknowledgementDisposition::try_from(ack.disposition).ok();
        if !matches!(
            disposition,
            Some(
                AcknowledgementDisposition::Accepted
                    | AcknowledgementDisposition::IgnoredStale
                    | AcknowledgementDisposition::Rejected
            )
        ) || retryable_ack(ack)
        {
            return Ok(());
        }
        if !self
            .pending()
            .any(|message| message_id(message) == Some(&ack.acknowledged_message_id))
        {
            return Ok(());
        }
        self.change(|snapshot| {
            for record in snapshot.runs.values_mut() {
                if disposition == Some(AcknowledgementDisposition::Accepted)
                    && record.pending.iter().any(|message| {
                        matches!(&message.message,
                            Some(supervisor_to_core::Message::ObservedRunEvent(event))
                            if event.message_id == ack.acknowledged_message_id
                                && event.kind == forge_protocol::supervisor::v1::RunEventKind::Stopped as i32)
                    })
                {
                    record.quiescence_confirmed = true;
                }
                record
                    .pending
                    .retain(|message| message_id(message) != Some(&ack.acknowledged_message_id));
                record.compact();
            }
            for input in snapshot.runtime_inputs.values_mut() {
                if input.receipt.as_ref().and_then(message_id)==Some(&ack.acknowledged_message_id) {input.pending=false;}
            }
            for receipt in snapshot.git_inspections.values_mut() {
                if message_id(&receipt.message) == Some(&ack.acknowledged_message_id) {
                    receipt.pending = false;
                }
            }
            for receipt in snapshot.integration_receipts.values_mut() {
                if receipt.message.as_ref().and_then(message_id)==Some(&ack.acknowledged_message_id) {receipt.pending=false;}
            }
            Ok(())
        })
    }

    pub fn pending(&self) -> impl Iterator<Item = &SupervisorToCore> {
        self.snapshot
            .runs
            .values()
            .flat_map(|record| record.pending.iter())
            .chain(
                self.snapshot
                    .runtime_inputs
                    .values()
                    .filter(|record| record.pending)
                    .filter_map(|record| record.receipt.as_ref()),
            )
            .chain(
                self.snapshot
                    .git_inspections
                    .values()
                    .filter(|receipt| receipt.pending)
                    .map(|receipt| &receipt.message),
            )
            .chain(
                self.snapshot
                    .integration_receipts
                    .values()
                    .filter(|receipt| receipt.pending)
                    .filter_map(|receipt| receipt.message.as_ref()),
            )
    }

    // Per-scope stop-and-wait prevents a transient failure of N from being
    // overtaken by N+1 and subsequently classified as an old sequence by Core.
    pub fn pending_heads(&self) -> impl Iterator<Item = &SupervisorToCore> {
        self.snapshot
            .runs
            .values()
            .filter_map(|record| record.pending.first())
            .chain(
                self.snapshot
                    .runtime_inputs
                    .values()
                    .filter(|record| record.pending)
                    .filter_map(|record| record.receipt.as_ref()),
            )
            .chain(
                self.snapshot
                    .git_inspections
                    .values()
                    .filter(|receipt| receipt.pending)
                    .map(|receipt| &receipt.message),
            )
            .chain(
                self.snapshot
                    .integration_receipts
                    .values()
                    .filter(|receipt| receipt.pending)
                    .filter_map(|receipt| receipt.message.as_ref()),
            )
    }

    pub fn inventory(&self) -> Vec<RunInventoryEntry> {
        self.snapshot
            .runs
            .values()
            .filter(|record| record.inventory_required())
            .map(RunRecord::inventory)
            .collect()
    }

    pub fn records(&self) -> Vec<RunRecord> {
        self.snapshot.runs.values().cloned().collect()
    }

    pub fn is_healthy(&self) -> bool {
        self.last_write_healthy
            .load(std::sync::atomic::Ordering::Acquire)
    }

    #[cfg(test)]
    pub fn set_test_capacity(&mut self, max_bytes: usize) {
        self.max_bytes = max_bytes;
    }

    pub fn attach_environment(
        &mut self,
        provision: &ProvisionRun,
        id: &str,
        active: bool,
    ) -> Result<(), SupervisorError> {
        self.change(|snapshot| {
            let record = snapshot
                .runs
                .get_mut(&scope_key(provision))
                .ok_or(SupervisorError::InvalidJournalMessage)?;
            record.environment_id = id.to_owned();
            record.quiescence_confirmed = false;
            record.presence = if active {
                EnvironmentPresence::Active
            } else {
                EnvironmentPresence::Unknown
            };
            Ok(())
        })
    }

    pub fn next_sequence(&self, provision: &ProvisionRun) -> Result<u64, SupervisorError> {
        self.snapshot
            .runs
            .get(&scope_key(provision))
            .ok_or(SupervisorError::InvalidJournalMessage)?
            .last_sequence
            .checked_add(1)
            .ok_or(SupervisorError::SequenceOverflow)
    }

    fn change(
        &mut self,
        apply: impl FnOnce(&mut Snapshot) -> Result<(), SupervisorError>,
    ) -> Result<(), SupervisorError> {
        let mut next = self.snapshot.clone();
        apply(&mut next)?;
        self.persist(&next)?;
        self.snapshot = next;
        Ok(())
    }

    fn persist(&self, snapshot: &Snapshot) -> Result<(), SupervisorError> {
        self.last_write_healthy
            .store(false, std::sync::atomic::Ordering::Release);
        let bytes = serde_json::to_vec(snapshot)?;
        if bytes.len() > self.max_bytes {
            return Err(SupervisorError::JournalFull);
        }
        let temp = self.directory.join(format!("journal-{}.tmp", new_id()));
        let result = (|| -> Result<(), SupervisorError> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temp, self.directory.join("journal.json"))?;
            File::open(&self.directory)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        self.last_write_healthy
            .store(result.is_ok(), std::sync::atomic::Ordering::Release);
        result
    }
}

pub(crate) fn retryable_ack(ack: &CoreAcknowledgement) -> bool {
    matches!(
        ack.reason_code.as_str(),
        "internal_error" | "canonical_storage_unavailable" | "supervisor_channel_unavailable"
    )
}

fn provision_hash(provision: &ProvisionRun) -> Result<String, SupervisorError> {
    let mut canonical = provision.clone();
    canonical.command_id.clear();
    canonical.traceparent.clear();
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&canonical)?)
    ))
}

fn prepare_directory(path: &Path) -> Result<(), SupervisorError> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != Uid::effective().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(SupervisorError::UnsafeJournal);
    }
    Ok(())
}

fn open_private(path: &Path) -> Result<File, SupervisorError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_file(&metadata)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    validate_file(&file.metadata()?)?;
    Ok(file)
}

fn validate_file(metadata: &fs::Metadata) -> Result<(), SupervisorError> {
    if !metadata.is_file()
        || metadata.uid() != Uid::effective().as_raw()
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err(SupervisorError::UnsafeJournal);
    }
    Ok(())
}

pub(crate) fn scope_key(provision: &ProvisionRun) -> String {
    format!(
        "{}:{}:{}",
        provision.run_id, provision.lease_fencing_token, provision.environment_epoch
    )
}

pub(crate) fn message_id(message: &SupervisorToCore) -> Option<&String> {
    match message.message.as_ref()? {
        supervisor_to_core::Message::ObservedRunEvent(event) => Some(&event.message_id),
        supervisor_to_core::Message::ExecutorSubmission(submission) => Some(&submission.message_id),
        supervisor_to_core::Message::GitCandidateInspection(result) => Some(&result.message_id),
        supervisor_to_core::Message::RuntimeInputReceipt(result) => Some(&result.message_id),
        supervisor_to_core::Message::GitIntegration(result) => Some(&result.message_id),
        _ => None,
    }
}

fn message_scope(message: &SupervisorToCore) -> Option<(&str, u64, u64, u64)> {
    match message.message.as_ref()? {
        supervisor_to_core::Message::ObservedRunEvent(event) => Some((
            &event.run_id,
            event.lease_fencing_token,
            event.environment_epoch,
            event.sequence,
        )),
        supervisor_to_core::Message::ExecutorSubmission(event) => Some((
            &event.run_id,
            event.lease_fencing_token,
            event.environment_epoch,
            event.sequence,
        )),
        _ => None,
    }
}

#[cfg(test)]
mod inventory_tests;
