//! Admission accounting for retained raw evidence. No automated deletion.

use crate::{SupervisorConfig, SupervisorError, registry::RunRegistry};
use forge_domain::runtime::SandboxLaunchSpec;
use forge_protocol::supervisor::v1::EnvironmentPresence;
use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io,
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Path, PathBuf},
};

pub(super) async fn check(
    config: &SupervisorConfig,
    registry: &RunRegistry,
) -> Result<(), SupervisorError> {
    let mut bytes = retained_bytes(
        &config.state_directory.join("evidence"),
        config.evidence_max_bytes,
    )?;
    for record in registry.journal.lock().await.records() {
        if !matches!(record.provision.run_spec_version, 2..=7)
            || record.presence == EnvironmentPresence::Quiescent
        {
            continue;
        }
        let spec: SandboxLaunchSpec = serde_json::from_str(&record.provision.run_spec_json)?;
        // Conservative: retain the full reservation even after partial capture.
        bytes = bytes
            .checked_add(spec.max_output_bytes())
            .and_then(|bytes| bytes.checked_add(16 * 1024))
            .ok_or(SupervisorError::EvidenceCapacity)?;
        if bytes > config.evidence_max_bytes {
            return Err(SupervisorError::EvidenceCapacity);
        }
    }
    Ok(())
}

fn retained_bytes(root: &Path, maximum: u64) -> Result<u64, SupervisorError> {
    let root = match open_directory(root) {
        Ok(root) => root,
        Err(SupervisorError::JournalIo(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(0);
        }
        Err(error) => return Err(error),
    };
    let mut usage = EvidenceUsage {
        bytes: 0,
        entries: 0,
        maximum,
    };
    for run in fs::read_dir(directory_path(&root))? {
        let run = run?;
        usage.entry()?;
        let run = open_directory(&run.path())?;
        for epoch in fs::read_dir(directory_path(&run))? {
            let epoch = epoch?;
            usage.entry()?;
            let epoch = open_directory(&epoch.path())?;
            for file in fs::read_dir(directory_path(&epoch))? {
                let file = file?;
                usage.entry()?;
                if file.file_name() == "input-receipts" {
                    let receipts = open_directory(&file.path())?;
                    account_receipts(&receipts, &mut usage)?;
                } else {
                    account_file(&epoch, &file.file_name(), &mut usage)?;
                }
            }
        }
    }
    Ok(usage.bytes)
}

struct EvidenceUsage {
    bytes: u64,
    entries: usize,
    maximum: u64,
}

impl EvidenceUsage {
    fn entry(&mut self) -> Result<(), SupervisorError> {
        self.entries += 1;
        if self.entries > 100_000 {
            return Err(SupervisorError::EvidenceCapacity);
        }
        Ok(())
    }
}

fn account_receipts(directory: &File, usage: &mut EvidenceUsage) -> Result<(), SupervisorError> {
    // NativeMailbox owns this sole allowed subtree. Count atomic .pending-*
    // files too; neither nested directories nor other special types are legal.
    for entry in fs::read_dir(directory_path(directory))? {
        let entry = entry?;
        usage.entry()?;
        account_file(directory, &entry.file_name(), usage)?;
    }
    Ok(())
}

fn account_file(
    directory: &File,
    name: &OsStr,
    usage: &mut EvidenceUsage,
) -> Result<(), SupervisorError> {
    // O_PATH pins metadata without opening a FIFO/device; NOFOLLOW also pins
    // a symlink itself, which the regular-file check rejects without following.
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_PATH | nix::libc::O_NOFOLLOW)
        .open(directory_path(directory).join(name))
    {
        Ok(file) => file,
        // Core ACK cleanup or atomic rename can remove a leaf during admission.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(SupervisorError::UnsafeSurface);
    }
    usage.bytes = usage
        .bytes
        .checked_add(metadata.len())
        .ok_or(SupervisorError::EvidenceCapacity)?;
    if usage.bytes > usage.maximum {
        return Err(SupervisorError::EvidenceCapacity);
    }
    Ok(())
}

fn open_directory(path: &Path) -> Result<File, SupervisorError> {
    OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_DIRECTORY | nix::libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| match error.raw_os_error() {
            Some(nix::libc::ELOOP | nix::libc::ENOTDIR) => SupervisorError::UnsafeSurface,
            _ => error.into(),
        })
}

fn directory_path(directory: &File) -> PathBuf {
    // Linux's own fd link is trusted, unlike a worker path. Keep the descriptor
    // alive across enumeration and leaf opens so renames cannot retarget lookup.
    PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()))
}

#[cfg(test)]
mod tests;
