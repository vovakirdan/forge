use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use forge_domain::{
    EvidenceLocation, EvidenceObject, EvidenceObjectInput, EvidenceScope, EvidenceStream, Timestamp,
};
use uuid::Uuid;

use super::{EvidenceCollector, EvidenceError, EvidenceRedaction, digest};

/// Maximum local disk usage, including body and receipt bytes.
#[derive(Clone, Copy, Debug)]
pub struct SpoolLimits {
    pub per_run_bytes: u64,
    pub global_bytes: u64,
    pub chunk_bytes: usize,
}

impl Default for SpoolLimits {
    fn default() -> Self {
        Self {
            per_run_bytes: 256 * 1024 * 1024,
            global_bytes: 2 * 1024 * 1024 * 1024,
            chunk_bytes: 64 * 1024,
        }
    }
}

impl SpoolLimits {
    fn validate(self) -> Result<(), EvidenceError> {
        if self.per_run_bytes == 0
            || self.global_bytes < self.per_run_bytes
            || self.chunk_bytes == 0
            || self.chunk_bytes > 1024 * 1024
        {
            return Err(EvidenceError::InvalidConfiguration);
        }
        Ok(())
    }
}

#[derive(Default)]
struct Usage {
    total: u64,
    by_run: BTreeMap<Uuid, u64>,
}

struct SpoolInner {
    root: PathBuf,
    limits: SpoolLimits,
    usage: Mutex<Usage>,
    // A process cannot create a second accounting instance for this directory.
    _lock: File,
}

/// One shared spool per Core process. Existing bytes count against limits after
/// restart; upload failure and a dropped collector never delete evidence.
#[derive(Clone)]
pub struct EvidenceSpool(Arc<SpoolInner>);

/// A durable local body whose path is constructed only by the spool.
#[derive(Clone, Debug)]
pub struct PendingEvidence {
    receipt: EvidenceObject,
    body_path: PathBuf,
}

impl PendingEvidence {
    pub fn receipt(&self) -> &EvidenceObject {
        &self.receipt
    }

    /// Reads a bounded chunk and checks the digest before upload.
    pub fn read_verified(&self) -> Result<Vec<u8>, EvidenceError> {
        checked_metadata(&self.body_path, false)?;
        let mut bytes = Vec::new();
        File::open(&self.body_path)
            .map_err(|_| EvidenceError::SpoolIo)?
            .take(self.receipt.data().size_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| EvidenceError::SpoolIo)?;
        if bytes.len() as u64 != self.receipt.data().size_bytes
            || digest(&bytes) != self.receipt.data().sha256
        {
            return Err(EvidenceError::IntegrityMismatch);
        }
        Ok(bytes)
    }
}

impl EvidenceSpool {
    /// Opens an absolute owner-only spool and accounts for retained files.
    /// This is blocking filesystem work; async callers use their blocking lane.
    pub fn open(root: impl Into<PathBuf>, limits: SpoolLimits) -> Result<Self, EvidenceError> {
        limits.validate()?;
        let root = root.into();
        if !root.is_absolute() {
            return Err(EvidenceError::UnsafeSpool);
        }
        create_directory(&root)?;
        let lock_path = root.join(".lock");
        if lock_path.exists() {
            checked_metadata(&lock_path, false)?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(&lock_path)
            .map_err(|_| EvidenceError::UnsafeSpool)?;
        lock.try_lock().map_err(|_| EvidenceError::SpoolBusy)?;
        let usage = scan_usage(&root)?;
        Ok(Self(Arc::new(SpoolInner {
            root,
            limits,
            usage: Mutex::new(usage),
            _lock: lock,
        })))
    }

    /// Starts one independently redacted byte stream. Stdout and stderr must
    /// have separate collectors so their byte boundaries cannot be interleaved.
    pub fn start_stream(
        &self,
        scope: EvidenceScope,
        stream: EvidenceStream,
        redaction: EvidenceRedaction,
    ) -> Result<EvidenceCollector, EvidenceError> {
        scope
            .validate()
            .map_err(|_| EvidenceError::InvalidMetadata)?;
        Ok(EvidenceCollector::new(
            self.clone(),
            scope,
            stream,
            redaction,
        ))
    }

    pub fn used_bytes(&self) -> Result<u64, EvidenceError> {
        Ok(self
            .0
            .usage
            .lock()
            .map_err(|_| EvidenceError::SpoolIo)?
            .total)
    }

    /// Releases only a confirmed uploaded local duplicate after Core commits
    /// its Stored receipt. Remote evidence is retained; upload never calls this
    /// implicitly. Repeated post-commit acknowledgements are idempotent.
    pub fn release_uploaded(
        &self,
        pending: &PendingEvidence,
        stored: &EvidenceObject,
    ) -> Result<(), EvidenceError> {
        let expected = pending
            .receipt
            .stored()
            .map_err(|_| EvidenceError::InvalidMetadata)?;
        let expected_path = self
            .0
            .root
            .join(pending.receipt.data().scope.run_id.to_string())
            .join(format!("{}.chunk", pending.receipt.data().id));
        if stored != &expected || pending.body_path != expected_path {
            return Err(EvidenceError::InvalidMetadata);
        }
        if fs::symlink_metadata(&pending.body_path).is_ok() {
            pending.read_verified()?;
        }
        let mut usage = self.0.usage.lock().map_err(|_| EvidenceError::SpoolIo)?;
        for path in [
            pending.body_path.with_extension("json"),
            pending.body_path.clone(),
        ] {
            let metadata = match fs::symlink_metadata(&path) {
                Ok(_) => checked_metadata(&path, false)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return Err(EvidenceError::SpoolIo),
            };
            fs::remove_file(&path).map_err(|_| EvidenceError::SpoolIo)?;
            usage.total = usage.total.saturating_sub(metadata.len());
            if let Some(bytes) = usage.by_run.get_mut(&pending.receipt.data().scope.run_id) {
                *bytes = bytes.saturating_sub(metadata.len());
            }
        }
        if let Some(parent) = pending.body_path.parent() {
            File::open(parent)
                .and_then(|file| file.sync_all())
                .map_err(|_| EvidenceError::SpoolIo)?;
        }
        Ok(())
    }

    /// Returns committed local receipts after restart. Orphan partial writes
    /// still count against capacity but are never falsely declared complete.
    pub fn pending(&self) -> Result<Vec<PendingEvidence>, EvidenceError> {
        let mut pending = Vec::new();
        for directory in fs::read_dir(&self.0.root).map_err(|_| EvidenceError::SpoolIo)? {
            let directory = directory.map_err(|_| EvidenceError::SpoolIo)?;
            if directory.file_name() == ".lock" {
                continue;
            }
            checked_metadata(&directory.path(), true)?;
            for file in fs::read_dir(directory.path()).map_err(|_| EvidenceError::SpoolIo)? {
                let path = file.map_err(|_| EvidenceError::SpoolIo)?.path();
                if path.extension().is_none_or(|extension| extension != "json") {
                    continue;
                }
                checked_metadata(&path, false)?;
                let mut json = Vec::new();
                File::open(&path)
                    .map_err(|_| EvidenceError::SpoolIo)?
                    .take(64 * 1024)
                    .read_to_end(&mut json)
                    .map_err(|_| EvidenceError::SpoolIo)?;
                let receipt: EvidenceObject =
                    serde_json::from_slice(&json).map_err(|_| EvidenceError::InvalidMetadata)?;
                if receipt.data().location != EvidenceLocation::PendingUpload
                    || receipt.data().size_bytes > 1024 * 1024
                    || directory.file_name() != receipt.data().scope.run_id.to_string().as_str()
                    || path
                        .file_stem()
                        .is_none_or(|stem| stem != receipt.data().id.to_string().as_str())
                {
                    return Err(EvidenceError::InvalidMetadata);
                }
                let entry = PendingEvidence {
                    receipt,
                    body_path: path.with_extension("chunk"),
                };
                entry.read_verified()?;
                pending.push(entry);
            }
        }
        pending.sort_by_key(|entry| (entry.receipt.data().created_at, entry.receipt.data().id));
        Ok(pending)
    }

    pub(super) fn chunk_bytes(&self) -> usize {
        self.0.limits.chunk_bytes
    }

    pub(super) fn write_chunk(
        &self,
        scope: EvidenceScope,
        stream: EvidenceStream,
        sequence: u64,
        policy: &str,
        bytes: &[u8],
    ) -> Result<PendingEvidence, EvidenceError> {
        let receipt = EvidenceObject::new(EvidenceObjectInput {
            id: Uuid::now_v7(),
            scope,
            stream,
            sequence,
            sha256: digest(bytes),
            size_bytes: bytes.len() as u64,
            redaction_policy_reference: policy.to_owned(),
            location: EvidenceLocation::PendingUpload,
            created_at: Timestamp::now_utc(),
        })
        .map_err(|_| EvidenceError::InvalidMetadata)?;
        let json = serde_json::to_vec(&receipt).map_err(|_| EvidenceError::InvalidMetadata)?;
        self.reserve(scope.run_id, (bytes.len() + json.len()) as u64)?;
        let directory = self.0.root.join(scope.run_id.to_string());
        create_directory(&directory)?;
        let body_path = directory.join(format!("{}.chunk", receipt.data().id));
        // A failed write leaves accounted partial evidence for inspection. The
        // receipt is created last, only after the redacted body is durable.
        write_new(&body_path, bytes)?;
        write_new(&body_path.with_extension("json"), &json)?;
        File::open(&directory)
            .and_then(|file| file.sync_all())
            .map_err(|_| EvidenceError::SpoolIo)?;
        Ok(PendingEvidence { receipt, body_path })
    }

    fn reserve(&self, run_id: Uuid, bytes: u64) -> Result<(), EvidenceError> {
        let mut usage = self.0.usage.lock().map_err(|_| EvidenceError::SpoolIo)?;
        let run = usage.by_run.get(&run_id).copied().unwrap_or(0);
        if bytes > self.0.limits.global_bytes.saturating_sub(usage.total)
            || bytes > self.0.limits.per_run_bytes.saturating_sub(run)
        {
            return Err(EvidenceError::SpoolCapacity);
        }
        usage.total += bytes;
        usage.by_run.insert(run_id, run + bytes);
        Ok(())
    }
}

fn create_directory(path: &Path) -> Result<(), EvidenceError> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|_| EvidenceError::SpoolIo)?;
    checked_metadata(path, true).map(|_| ())
}

fn checked_metadata(path: &Path, directory: bool) -> Result<fs::Metadata, EvidenceError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| EvidenceError::SpoolIo)?;
    if metadata.uid() != nix::unistd::Uid::effective().as_raw()
        || metadata.mode() & 0o777 != if directory { 0o700 } else { 0o600 }
        || if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file() || metadata.nlink() != 1
        }
    {
        return Err(EvidenceError::UnsafeSpool);
    }
    Ok(metadata)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), EvidenceError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| EvidenceError::SpoolIo)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| EvidenceError::SpoolIo)
}

fn scan_usage(root: &Path) -> Result<Usage, EvidenceError> {
    let mut usage = Usage::default();
    for entry in fs::read_dir(root).map_err(|_| EvidenceError::SpoolIo)? {
        let directory = entry.map_err(|_| EvidenceError::SpoolIo)?;
        if directory.file_name() == ".lock" {
            continue;
        }
        checked_metadata(&directory.path(), true)?;
        let run_id = directory
            .file_name()
            .to_str()
            .and_then(|name| Uuid::parse_str(name).ok())
            .filter(|id| id.get_version_num() == 7)
            .ok_or(EvidenceError::UnsafeSpool)?;
        let mut bytes = 0_u64;
        for file in fs::read_dir(directory.path()).map_err(|_| EvidenceError::SpoolIo)? {
            let path = file.map_err(|_| EvidenceError::SpoolIo)?.path();
            bytes = bytes
                .checked_add(checked_metadata(&path, false)?.len())
                .ok_or(EvidenceError::SpoolCapacity)?;
        }
        usage.total = usage
            .total
            .checked_add(bytes)
            .ok_or(EvidenceError::SpoolCapacity)?;
        usage.by_run.insert(run_id, bytes);
    }
    Ok(usage)
}
