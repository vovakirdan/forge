//! Admission accounting for retained raw evidence. No automated deletion.

use crate::{SupervisorConfig, SupervisorError, registry::RunRegistry};
use forge_domain::runtime::SandboxLaunchSpec;
use forge_protocol::supervisor::v1::EnvironmentPresence;
use std::{fs, path::Path};

pub(super) async fn check(
    config: &SupervisorConfig,
    registry: &RunRegistry,
) -> Result<(), SupervisorError> {
    let mut bytes = retained_bytes(
        &config.state_directory.join("evidence"),
        config.evidence_max_bytes,
    )?;
    for record in registry.journal.lock().await.records() {
        if !matches!(record.provision.run_spec_version, 2..=5)
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
    if !root.try_exists()? {
        return Ok(0);
    }
    let mut bytes = 0u64;
    let mut entries = 0usize;
    // Only host-owned Run/epoch directories are traversed. Nested directories
    // inside the employee-writable epoch mount are rejected, not followed.
    for run in fs::read_dir(root)? {
        let run = run?;
        checked_entry(&run, true, &mut entries)?;
        for epoch in fs::read_dir(run.path())? {
            let epoch = epoch?;
            checked_entry(&epoch, true, &mut entries)?;
            for file in fs::read_dir(epoch.path())? {
                let file = file?;
                checked_entry(&file, false, &mut entries)?;
                bytes = bytes
                    .checked_add(file.metadata()?.len())
                    .ok_or(SupervisorError::EvidenceCapacity)?;
                if bytes > maximum {
                    return Err(SupervisorError::EvidenceCapacity);
                }
            }
        }
    }
    Ok(bytes)
}

fn checked_entry(
    entry: &fs::DirEntry,
    directory: bool,
    entries: &mut usize,
) -> Result<(), SupervisorError> {
    *entries += 1;
    if *entries > 100_000 {
        return Err(SupervisorError::EvidenceCapacity);
    }
    let kind = entry.file_type()?;
    if (directory && !kind.is_dir()) || (!directory && !kind.is_file()) {
        return Err(SupervisorError::UnsafeSurface);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_files_exhaust_capacity_without_being_deleted() {
        let root =
            std::env::temp_dir().join(format!("forge-evidence-admission-{}", crate::new_id()));
        let epoch = root.join("run/1");
        fs::create_dir_all(&epoch).unwrap();
        fs::write(epoch.join("stdout.jsonl"), b"retained evidence").unwrap();
        assert!(matches!(
            retained_bytes(&root, 1),
            Err(SupervisorError::EvidenceCapacity)
        ));
        assert_eq!(
            fs::read(epoch.join("stdout.jsonl")).unwrap(),
            b"retained evidence"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn evidence_admission_does_not_follow_employee_symlinks_or_directories() {
        let root =
            std::env::temp_dir().join(format!("forge-evidence-admission-{}", crate::new_id()));
        let epoch = root.join("run/1");
        fs::create_dir_all(&epoch).unwrap();
        std::os::unix::fs::symlink("/unmounted-host-data", epoch.join("link")).unwrap();
        assert!(matches!(
            retained_bytes(&root, 1024),
            Err(SupervisorError::UnsafeSurface)
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
