//! Bounded, descriptor-relative reads of explicitly selected runtime input files.
//! Unlike credential materialization, ordinary 0644/0755 files are allowed.

use rustix::{
    fs::{Mode, OFlags, ResolveFlags},
    process::geteuid,
};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

mod capture;
pub use capture::{CapturedFile, capture_selected_files, read_capture};

/// Bytes intentionally have no Debug implementation.
pub struct SelectedFile {
    pub bytes: Vec<u8>,
    pub executable: bool,
}

/// Reads beneath a canonical owned directory without symlinks, hardlinks or devices.
/// Metadata before/after the read catches concurrent edits; callers reserve the
/// surface as well when a snapshot spans multiple files.
pub fn read_selected_file(
    root: &Path,
    relative: &Path,
    maximum_bytes: u64,
) -> io::Result<SelectedFile> {
    if !root.is_absolute()
        || root.canonicalize()? != root
        || relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err(unsafe_file());
    }
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags((OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC).bits() as i32)
        .open(root)?;
    let root_meta = directory.metadata()?;
    if root_meta.uid() != geteuid().as_raw() || root_meta.mode() & 0o022 != 0 {
        return Err(unsafe_file());
    }
    let fd = rustix::fs::openat2(
        &directory,
        relative,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV,
    )
    .map_err(|_| unsafe_file())?;
    let mut file = File::from(fd);
    let before = file.metadata()?;
    if !before.is_file()
        || before.nlink() != 1
        || before.uid() != geteuid().as_raw()
        || before.mode() & 0o022 != 0
        || before.len() > maximum_bytes
    {
        return Err(unsafe_file());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(maximum_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    if bytes.len() as u64 != before.len()
        || after.len() != before.len()
        || after.nlink() != 1
        || after.mode() != before.mode()
        || after.mtime() != before.mtime()
        || after.mtime_nsec() != before.mtime_nsec()
        || after.ctime() != before.ctime()
        || after.ctime_nsec() != before.ctime_nsec()
    {
        return Err(unsafe_file());
    }
    Ok(SelectedFile {
        bytes,
        executable: before.mode() & 0o111 != 0,
    })
}

fn unsafe_file() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "selected input file is unsafe or changed while reading",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn selected_files_preserve_binary_empty_and_executable_but_reject_links() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        std::fs::write(root.join("binary"), [0, 255, 3]).unwrap();
        std::fs::set_permissions(root.join("binary"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        let file = read_selected_file(&root, Path::new("binary"), 3).unwrap();
        assert_eq!(file.bytes, [0, 255, 3]);
        assert!(file.executable);
        assert!(read_selected_file(&root, Path::new("binary"), 2).is_err());
        std::fs::write(root.join("empty"), []).unwrap();
        assert!(
            read_selected_file(&root, Path::new("empty"), 0)
                .unwrap()
                .bytes
                .is_empty()
        );
        symlink("empty", root.join("symlink")).unwrap();
        assert!(read_selected_file(&root, Path::new("symlink"), 9).is_err());
        std::fs::hard_link(root.join("empty"), root.join("hardlink")).unwrap();
        assert!(read_selected_file(&root, Path::new("hardlink"), 9).is_err());
        assert!(read_selected_file(&root, Path::new("../escape"), 9).is_err());
        assert!(read_selected_file(&root, Path::new("."), 9).is_err());
    }
}
