use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

use rustix::{fs::OFlags, process::geteuid};
use zeroize::Zeroizing;

use crate::{SecretBytes, SecretStoreError};

/// An owner-only file outside TaskWorkSurface. Explicit cleanup reports failures.
pub struct PrivateMaterialization {
    path: PathBuf,
}

impl fmt::Debug for PrivateMaterialization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PrivateMaterialization([REDACTED PATH])")
    }
}

impl PrivateMaterialization {
    /// Removes only one exact private file beneath a pinned root. Never follows
    /// a symlink or recurses; callers first retain needed data durably.
    pub fn cleanup_beneath(root: &Path, relative: &Path) -> Result<bool, SecretStoreError> {
        verify_directory(root)?;
        if relative.is_absolute()
            || relative
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err(SecretStoreError::UnsafePermissions);
        }
        let parent = relative
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or(SecretStoreError::UnsafePermissions)?;
        let name = relative
            .file_name()
            .ok_or(SecretStoreError::UnsafePermissions)?;
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags((OFlags::DIRECTORY | OFlags::NOFOLLOW).bits() as i32)
            .open(root)?;
        let parent = match rustix::fs::openat2(
            &directory,
            parent,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::BENEATH | rustix::fs::ResolveFlags::NO_SYMLINKS,
        ) {
            Ok(fd) => File::from(fd),
            Err(rustix::io::Errno::NOENT) => return Ok(false),
            Err(_) => return Err(SecretStoreError::UnsafePermissions),
        };
        let file = match rustix::fs::openat(
            &parent,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            rustix::fs::Mode::empty(),
        ) {
            Ok(fd) => File::from(fd),
            Err(rustix::io::Errno::NOENT) => return Ok(false),
            Err(_) => return Err(SecretStoreError::UnsafePermissions),
        };
        validate_private_file(&file)?;
        rustix::fs::unlinkat(&parent, name, rustix::fs::AtFlags::empty())
            .map_err(|_| SecretStoreError::UnsafePermissions)?;
        parent.sync_all()?;
        Ok(true)
    }
    /// Reads a runtime-controlled descendant without following any symlink
    /// component. Linux openat2 pins resolution beneath the trusted root fd.
    pub fn read_beneath(
        root: &Path,
        relative: &Path,
        maximum_bytes: u64,
    ) -> Result<SecretBytes, SecretStoreError> {
        verify_directory(root)?;
        if relative.is_absolute()
            || relative
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err(SecretStoreError::UnsafePermissions);
        }
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags((OFlags::DIRECTORY | OFlags::NOFOLLOW).bits() as i32)
            .open(root)?;
        let fd = rustix::fs::openat2(
            &directory,
            relative,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::BENEATH | rustix::fs::ResolveFlags::NO_SYMLINKS,
        )
        .map_err(|_| SecretStoreError::UnsafePermissions)?;
        let mut file = File::from(fd);
        validate_private_file(&file)?;
        let mut bytes = Zeroizing::new(Vec::new());
        (&mut file)
            .take(maximum_bytes.saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > maximum_bytes {
            return Err(SecretStoreError::AuthTooLarge);
        }
        Ok(SecretBytes::new(std::mem::take(&mut *bytes)))
    }
    /// Opens an explicitly selected existing private file without changing it.
    pub fn open(path: &Path) -> Result<Self, SecretStoreError> {
        let _file = open_private(path)?;
        Ok(Self {
            path: path.to_owned(),
        })
    }
    pub fn create(path: &Path, secret: &SecretBytes) -> Result<Self, SecretStoreError> {
        let parent = path.parent().ok_or(SecretStoreError::UnsafePermissions)?;
        verify_directory(parent)?;
        write_new_private(path, secret.expose())?;
        Ok(Self {
            path: path.to_owned(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn read(&self, maximum_bytes: u64) -> Result<SecretBytes, SecretStoreError> {
        let mut file = open_private(&self.path)?;
        let mut bytes = Zeroizing::new(Vec::new());
        (&mut file)
            .take(maximum_bytes.saturating_add(1))
            .read_to_end(&mut bytes)?;
        let secret = SecretBytes::new(std::mem::take(&mut bytes));
        if secret.expose().len() as u64 > maximum_bytes {
            return Err(SecretStoreError::AuthTooLarge);
        }
        Ok(secret)
    }

    pub fn cleanup(self) -> Result<(), SecretStoreError> {
        fs::remove_file(&self.path)?;
        Ok(())
    }
}

pub(crate) fn create_private_directory(path: &Path) -> Result<(), SecretStoreError> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => verify_directory(path),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn verify_directory(path: &Path) -> Result<(), SecretStoreError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != geteuid().as_raw()
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(SecretStoreError::UnsafePermissions);
    }
    Ok(())
}

pub(crate) fn open_private(path: &Path) -> Result<File, SecretStoreError> {
    let file = OpenOptions::new()
        .read(true)
        // The runtime can replace its auth file with a FIFO. Do not block before
        // fstat has established that this is a bounded regular-file read.
        .custom_flags((OFlags::NOFOLLOW | OFlags::NONBLOCK).bits() as i32)
        .open(path)?;
    validate_private_file(&file)?;
    Ok(file)
}

fn validate_private_file(file: &File) -> Result<(), SecretStoreError> {
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != geteuid().as_raw()
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(SecretStoreError::UnsafePermissions);
    }
    Ok(())
}

pub(crate) fn write_new_private(path: &Path, bytes: &[u8]) -> Result<(), SecretStoreError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(OFlags::NOFOLLOW.bits() as i32)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}
