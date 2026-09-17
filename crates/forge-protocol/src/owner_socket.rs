//! Strict native owner-socket checks. Explicit paths have the same policy as defaults.

use std::{
    fs::{self, DirBuilder, Metadata},
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt},
    path::{Component, Path, PathBuf},
    time::Duration,
};

use nix::unistd::getuid;
use thiserror::Error;
use tokio::{net::UnixStream, time::timeout};

/// Safe diagnostics deliberately omit paths and raw operating-system errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum OwnerSocketError {
    #[error("owner socket path must be absolute and contain no symlinks")]
    UnsafePath,
    #[error("owner socket directory must be owned by this user with mode 0700")]
    UnsafeDirectory,
    #[error("owner socket must be owned by this user with mode 0600")]
    UnsafeSocket,
    #[error("owner socket peer has a different user identity")]
    WrongPeer,
    #[error("owner socket is unavailable")]
    Unavailable,
    #[error("owner socket connection timed out")]
    Timeout,
}

fn components(path: &Path) -> Result<Vec<PathBuf>, OwnerSocketError> {
    if !path.is_absolute() {
        return Err(OwnerSocketError::UnsafePath);
    }
    let mut current = PathBuf::new();
    let mut paths = Vec::new();
    for part in path.components() {
        match part {
            Component::RootDir | Component::Normal(_) => current.push(part.as_os_str()),
            _ => return Err(OwnerSocketError::UnsafePath),
        }
        paths.push(current.clone());
    }
    Ok(paths)
}

/// Reject symlinks in every existing component, including public ancestors.
pub fn validate_directory_chain(path: &Path) -> Result<(), OwnerSocketError> {
    for part in components(path)? {
        let metadata = fs::symlink_metadata(part).map_err(|_| OwnerSocketError::UnsafePath)?;
        if !metadata.is_dir() || metadata.is_symlink() {
            return Err(OwnerSocketError::UnsafePath);
        }
    }
    Ok(())
}

/// Check the immediately enclosing owner directory without changing permissions.
pub fn validate_owner_directory(path: &Path) -> Result<(), OwnerSocketError> {
    validate_directory_chain(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| OwnerSocketError::UnsafeDirectory)?;
    if metadata.uid() != getuid().as_raw() || metadata.mode() & 0o7777 != 0o700 {
        return Err(OwnerSocketError::UnsafeDirectory);
    }
    Ok(())
}

/// Create missing owner directories; never repair or traverse an unsafe existing path.
pub fn ensure_owner_directory(path: &Path) -> Result<(), OwnerSocketError> {
    for part in components(path)? {
        match fs::symlink_metadata(&part) {
            Ok(metadata) if metadata.is_dir() && !metadata.is_symlink() => {}
            Ok(_) => return Err(OwnerSocketError::UnsafePath),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                DirBuilder::new()
                    .mode(0o700)
                    .create(&part)
                    .map_err(|_| OwnerSocketError::UnsafeDirectory)?;
            }
            Err(_) => return Err(OwnerSocketError::UnsafeDirectory),
        }
    }
    validate_owner_directory(path)
}

/// Validate the socket type, permissions and ownership, including its entire path.
pub fn validate_owner_socket(path: &Path) -> Result<Metadata, OwnerSocketError> {
    components(path)?;
    let parent = path.parent().ok_or(OwnerSocketError::UnsafePath)?;
    validate_owner_directory(parent)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| OwnerSocketError::UnsafeSocket)?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != getuid().as_raw()
        || metadata.mode() & 0o7777 != 0o600
    {
        return Err(OwnerSocketError::UnsafeSocket);
    }
    Ok(metadata)
}

/// Authenticate a connected Unix peer before sending or accepting secret material.
pub fn validate_owner_peer(stream: &UnixStream) -> Result<(), OwnerSocketError> {
    validate_peer_uid(stream, getuid().as_raw())
}

fn validate_peer_uid(stream: &UnixStream, uid: u32) -> Result<(), OwnerSocketError> {
    if stream
        .peer_cred()
        .map_err(|_| OwnerSocketError::WrongPeer)?
        .uid()
        != uid
    {
        return Err(OwnerSocketError::WrongPeer);
    }
    Ok(())
}

/// Connect within one second to a validated owner socket, with no TCP fallback.
pub async fn connect_owner_socket(path: &Path) -> Result<UnixStream, OwnerSocketError> {
    let before = validate_owner_socket(path)?;
    let stream = timeout(Duration::from_secs(1), UnixStream::connect(path))
        .await
        .map_err(|_| OwnerSocketError::Timeout)?
        .map_err(|_| OwnerSocketError::Unavailable)?;
    validate_owner_peer(&stream)?;
    let after = validate_owner_socket(path)?;
    if (before.dev(), before.ino()) != (after.dev(), after.ino()) {
        return Err(OwnerSocketError::UnsafeSocket);
    }
    Ok(stream)
}

#[cfg(test)]
#[path = "owner_socket_tests.rs"]
mod tests;
