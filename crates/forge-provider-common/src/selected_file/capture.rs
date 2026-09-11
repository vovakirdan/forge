//! Host-owned staging is replayable by its exact request, not by reading the
//! source again after a complete capture. No worktree is modified.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

use super::{read_selected_file, unsafe_file};
use crate::{PrivateMaterialization, SecretBytes, private_file::create_private_directory};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturedFile {
    pub path: String,
    pub size_bytes: u64,
    pub executable: bool,
    pub sha256: String,
}

/// Returns only a complete, byte-verified capture bound to the supplied request.
pub fn read_capture(
    target: &Path,
    request: &[u8],
    max_bytes: u64,
) -> io::Result<Option<Vec<CapturedFile>>> {
    if !target.try_exists()? {
        return Ok(None);
    }
    verify_request(target, request)?;
    let receipt = target.join("capture.json");
    if !receipt.try_exists()? {
        return Ok(None);
    }
    let bytes = PrivateMaterialization::open(&receipt)
        .and_then(|file| file.read(128 * 1024))
        .map_err(|_| unsafe_file())?;
    let files: Vec<CapturedFile> =
        serde_json::from_slice(bytes.expose()).map_err(|_| unsafe_file())?;
    if files.is_empty() || files.len() > 64 {
        return Err(unsafe_file());
    }
    let mut total = 0_u64;
    for file in &files {
        if file.sha256.len() != 64
            || !file
                .sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(unsafe_file());
        }
        total = total.checked_add(file.size_bytes).ok_or_else(unsafe_file)?;
        if total > max_bytes {
            return Err(unsafe_file());
        }
        let blob = PrivateMaterialization::read_beneath(
            target,
            &PathBuf::from("blobs").join(&file.sha256),
            file.size_bytes,
        )
        .map_err(|_| unsafe_file())?;
        if blob.expose().len() as u64 != file.size_bytes || digest(blob.expose()) != file.sha256 {
            return Err(unsafe_file());
        }
    }
    Ok(Some(files))
}

/// The caller validates the path policy and reserves the source against writers.
/// Existing incomplete staging fails closed: it is retained, never silently
/// replaced with bytes from a later source revision.
pub fn capture_selected_files(
    root: &Path,
    paths: &[String],
    target: &Path,
    request: &[u8],
    max_bytes: u64,
) -> io::Result<Vec<CapturedFile>> {
    if let Some(files) = read_capture(target, request, max_bytes)? {
        return Ok(files);
    }
    if target.try_exists()? {
        return Err(unsafe_file());
    }
    fs::create_dir(target)?;
    // The parent is owner-only; tighten before writing request or file bodies.
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(target, fs::Permissions::from_mode(0o700))?;
    write_private(&target.join("request.json"), request)?;
    create_private_directory(&target.join("blobs")).map_err(|_| unsafe_file())?;
    if paths.is_empty() || paths.len() > 64 {
        return Err(unsafe_file());
    }
    let mut files = Vec::with_capacity(paths.len());
    let mut remaining = max_bytes;
    for path in paths {
        let file = read_selected_file(root, Path::new(path), remaining)?;
        let size_bytes = file.bytes.len() as u64;
        remaining = remaining.checked_sub(size_bytes).ok_or_else(unsafe_file)?;
        let sha256 = digest(&file.bytes);
        let blob = target.join("blobs").join(&sha256);
        if !blob.try_exists()? {
            write_private(&blob, &file.bytes)?;
        }
        files.push(CapturedFile {
            path: path.clone(),
            size_bytes,
            executable: file.executable,
            sha256,
        });
    }
    write_private(
        &target.join("capture.json"),
        &serde_json::to_vec(&files).map_err(|_| unsafe_file())?,
    )?;
    Ok(files)
}

fn verify_request(target: &Path, request: &[u8]) -> io::Result<()> {
    create_private_directory(target).map_err(|_| unsafe_file())?;
    let bytes = PrivateMaterialization::open(&target.join("request.json"))
        .and_then(|file| file.read(128 * 1024))
        .map_err(|_| unsafe_file())?;
    if bytes.expose() != request {
        return Err(unsafe_file());
    }
    Ok(())
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    PrivateMaterialization::create(path, &SecretBytes::new(bytes.to_vec()))
        .map_err(|_| unsafe_file())?;
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_uses_captured_bytes_after_source_changes() {
        let source = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        fs::write(source.path().join("a"), b"original").unwrap();
        let output = staging.path().join("capture");
        let first = capture_selected_files(source.path(), &["a".into()], &output, b"request", 1024)
            .unwrap();
        fs::write(source.path().join("a"), b"changed").unwrap();
        assert_eq!(
            capture_selected_files(source.path(), &["a".into()], &output, b"request", 1024)
                .unwrap(),
            first
        );
        assert!(read_capture(&output, b"different request", 1024).is_err());
    }
}
