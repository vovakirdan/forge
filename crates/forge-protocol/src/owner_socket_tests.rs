use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("forge-owner-socket-{}-{nonce}", std::process::id()));
        DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[tokio::test]
async fn owner_socket_accepts_only_private_real_socket_and_peer() {
    let directory = Directory::new();
    let path = directory.0.join("core.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let client = connect_owner_socket(&path).await.unwrap();
    let (server, _) = listener.accept().await.unwrap();
    assert!(validate_owner_peer(&client).is_ok());
    assert!(validate_owner_peer(&server).is_ok());
    assert_eq!(
        validate_peer_uid(&server, getuid().as_raw().wrapping_add(1)),
        Err(OwnerSocketError::WrongPeer)
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    assert!(matches!(
        connect_owner_socket(&path).await,
        Err(OwnerSocketError::UnsafeSocket)
    ));
}

#[tokio::test]
async fn explicit_paths_do_not_bypass_directory_or_symlink_checks() {
    let directory = Directory::new();
    let private = directory.0.join("private");
    ensure_owner_directory(&private).unwrap();
    let path = private.join("core.sock");
    let _listener = tokio::net::UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    symlink(&private, directory.0.join("alias")).unwrap();
    assert!(
        connect_owner_socket(&directory.0.join("alias/core.sock"))
            .await
            .is_err()
    );
    symlink(&path, private.join("alias.sock")).unwrap();
    assert!(
        connect_owner_socket(&private.join("alias.sock"))
            .await
            .is_err()
    );
    assert!(connect_owner_socket(Path::new("core.sock")).await.is_err());
    fs::set_permissions(&private, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        connect_owner_socket(&path).await,
        Err(OwnerSocketError::UnsafeDirectory)
    ));
}

#[tokio::test]
async fn ordinary_files_and_parent_traversal_are_not_socket_endpoints() {
    let directory = Directory::new();
    let path = directory.0.join("core.sock");
    fs::write(&path, b"synthetic").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(matches!(
        connect_owner_socket(&path).await,
        Err(OwnerSocketError::UnsafeSocket)
    ));
    assert!(validate_owner_directory(&directory.0.join("../")).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"synthetic");
}

#[test]
fn creating_owner_directory_does_not_repair_existing_insecure_permissions() {
    let directory = Directory::new();
    let child = directory.0.join("child");
    fs::create_dir(&child).unwrap();
    fs::set_permissions(&child, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        ensure_owner_directory(&child),
        Err(OwnerSocketError::UnsafeDirectory)
    );
    assert_eq!(fs::metadata(child).unwrap().mode() & 0o7777, 0o755);
}
