//! Owner-only bootstrap listener. This module never exposes an HTTP mint route.

use std::{
    fs::{self, File, OpenOptions},
    io::ErrorKind,
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use forge_protocol::{
    owner_socket::{ensure_owner_directory, validate_owner_peer, validate_owner_socket},
    ui_control::{
        CONTROL_EXCHANGE_TIMEOUT, ControlErrorResponse, ControlFailure, ControlRequest,
        ControlResponse, LoginCodeResponse, OwnerSocketError, read_frame, write_frame,
    },
};
use nix::{fcntl::OFlag, unistd::getuid};
use thiserror::Error;
use tokio::{
    net::{UnixListener, UnixStream},
    sync::watch,
    task::JoinSet,
    time::timeout,
};

const CONNECTION_LIMIT: usize = 4;
type Issuer = Arc<dyn Fn() -> Result<LoginCodeResponse, ControlError> + Send + Sync>;

/// Safe diagnostics never include codes, serialized frames or callback details.
#[derive(Debug, Error)]
pub enum ControlError {
    #[error(transparent)]
    UnsafeSocket(#[from] OwnerSocketError),
    #[error("UI owner control socket is already in use")]
    AlreadyRunning,
    #[error("UI owner control lock is unsafe or unavailable")]
    LockUnavailable,
    #[error("UI owner control socket could not be bound")]
    BindFailed,
    #[error("UI owner control listener failed")]
    ListenerFailed,
    #[error("UI login code issuance failed")]
    IssuanceFailed,
}

struct SocketOwner {
    path: PathBuf,
    identity: (u64, u64),
    // Keep the file and its inode for the process lifetime; unlinking a lock file
    // would let a new process acquire a different lock while this one is alive.
    _lock: File,
}

impl Drop for SocketOwner {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
            metadata.file_type().is_socket()
                && metadata.uid() == getuid().as_raw()
                && (metadata.dev(), metadata.ino()) == self.identity
        }) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Bound listener owns its socket inode and lock until shutdown or failed startup.
pub struct ControlServer {
    listener: UnixListener,
    owner: SocketOwner,
    origin: String,
    issue: Issuer,
}

/// Bind before starting HTTP service. Reject live listeners; reclaim only an owned
/// stale socket under the lifetime file lock. Existing files are never overwritten.
pub async fn bind_control(
    path: &Path,
    origin: &str,
    issue: Issuer,
) -> Result<ControlServer, ControlError> {
    let parent = path.parent().ok_or(OwnerSocketError::UnsafePath)?;
    ensure_owner_directory(parent)?;
    if path.file_name().is_none() || !path.is_absolute() {
        return Err(OwnerSocketError::UnsafePath.into());
    }
    let mut lock_name = path
        .file_name()
        .ok_or(OwnerSocketError::UnsafePath)?
        .to_os_string();
    lock_name.push(".lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(OFlag::O_NOFOLLOW.bits())
        .open(parent.join(lock_name))
        .map_err(|_| ControlError::LockUnavailable)?;
    let metadata = lock.metadata().map_err(|_| ControlError::LockUnavailable)?;
    if !metadata.is_file()
        || metadata.uid() != getuid().as_raw()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(ControlError::LockUnavailable);
    }
    lock.try_lock().map_err(|_| ControlError::AlreadyRunning)?;
    match fs::symlink_metadata(path) {
        Ok(_) => {
            let stale = validate_owner_socket(path)?;
            match timeout(Duration::from_secs(1), UnixStream::connect(path)).await {
                Ok(Err(error)) if error.kind() == ErrorKind::ConnectionRefused => {}
                _ => return Err(ControlError::AlreadyRunning),
            }
            let current = validate_owner_socket(path)?;
            if (current.dev(), current.ino()) != (stale.dev(), stale.ino()) {
                return Err(ControlError::AlreadyRunning);
            }
            fs::remove_file(path).map_err(|_| ControlError::BindFailed)?;
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(_) => return Err(ControlError::BindFailed),
    }
    let listener = UnixListener::bind(path).map_err(|_| ControlError::BindFailed)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| ControlError::BindFailed)?;
    let owner = SocketOwner {
        path: path.to_path_buf(),
        identity: (metadata.dev(), metadata.ino()),
        _lock: lock,
    };
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| ControlError::BindFailed)?;
    validate_owner_socket(path)?;
    Ok(ControlServer {
        listener,
        owner,
        origin: origin.to_owned(),
        issue,
    })
}

impl ControlServer {
    /// Serve bounded peer-authenticated exchanges; shutdown joins or aborts children
    /// before releasing the socket inode and lifetime lock.
    pub async fn serve(self, mut shutdown: watch::Receiver<bool>) -> Result<(), ControlError> {
        let Self {
            listener,
            owner,
            origin,
            issue,
        } = self;
        let mut connections = JoinSet::new();
        let result = loop {
            if *shutdown.borrow() {
                break Ok(());
            }
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break Ok(());
                    }
                },
                completed = connections.join_next(), if !connections.is_empty() => {
                    if completed.is_some_and(|result| result.is_err()) {
                        break Err(ControlError::ListenerFailed);
                    }
                }
                accepted = listener.accept() => {
                    let (stream, _) = match accepted {
                        Ok(accepted) => accepted,
                        Err(_) => break Err(ControlError::ListenerFailed),
                    };
                    if connections.len() >= CONNECTION_LIMIT || validate_owner_peer(&stream).is_err() {
                        continue;
                    }
                    let issue = Arc::clone(&issue);
                    let origin = origin.clone();
                    connections.spawn(async move {
                        let _ = timeout(CONTROL_EXCHANGE_TIMEOUT, exchange(stream, &origin, issue)).await;
                    });
                }
            }
        };
        connections.abort_all();
        while connections.join_next().await.is_some() {}
        drop(listener);
        drop(owner);
        result
    }
}

async fn exchange(mut stream: UnixStream, origin: &str, issue: Issuer) {
    let response = match read_frame::<ControlRequest>(&mut stream).await {
        Ok(ControlRequest::IssueLoginCode {}) => match issue() {
            Ok(response) if response.origin == origin && response.validate().is_ok() => {
                ControlResponse::Login(response)
            }
            _ => ControlResponse::Error(ControlErrorResponse {
                error: ControlFailure::IssuanceUnavailable,
            }),
        },
        Err(_) => ControlResponse::Error(ControlErrorResponse {
            error: ControlFailure::InvalidRequest,
        }),
    };
    let _ = write_frame(&mut stream, &response).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_protocol::ui_control::{connect_owner_socket, request_login_code};
    use std::os::unix::fs::{DirBuilderExt, symlink};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const ORIGIN: &str = "http://127.0.0.1:45000";

    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "forge-control-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }

        fn socket(&self) -> PathBuf {
            self.0.join("control.sock")
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn issuer(count: Arc<AtomicUsize>) -> Issuer {
        Arc::new(move || {
            count.fetch_add(1, Ordering::Relaxed);
            Ok(LoginCodeResponse {
                origin: ORIGIN.into(),
                code: "a".repeat(64),
                expires_at: "2026-09-17T12:00:00Z".into(),
            })
        })
    }

    #[tokio::test]
    async fn authenticated_control_issues_one_code_and_cleans_up() {
        let directory = Directory::new();
        let path = directory.socket();
        let count = Arc::new(AtomicUsize::new(0));
        let server = bind_control(&path, ORIGIN, issuer(Arc::clone(&count)))
            .await
            .unwrap();
        assert!(matches!(
            bind_control(&path, ORIGIN, issuer(Arc::clone(&count))).await,
            Err(ControlError::AlreadyRunning)
        ));
        let (shutdown, receiver) = watch::channel(false);
        let task = tokio::spawn(server.serve(receiver));
        let response = request_login_code(&path).await.unwrap();
        assert_eq!(response.origin, ORIGIN);
        assert_eq!(count.load(Ordering::Relaxed), 1);
        shutdown.send(true).unwrap();
        task.await.unwrap().unwrap();
        assert!(!path.exists());
        assert!(directory.0.join("control.sock.lock").exists());
    }

    #[tokio::test]
    async fn one_connection_cannot_issue_two_codes() {
        let directory = Directory::new();
        let path = directory.socket();
        let count = Arc::new(AtomicUsize::new(0));
        let server = bind_control(&path, ORIGIN, issuer(Arc::clone(&count)))
            .await
            .unwrap();
        let (shutdown, receiver) = watch::channel(false);
        let task = tokio::spawn(server.serve(receiver));
        let mut stream = connect_owner_socket(&path).await.unwrap();
        let request = serde_json::to_vec(&ControlRequest::IssueLoginCode {}).unwrap();
        let mut pipelined = Vec::new();
        for _ in 0..2 {
            pipelined.extend_from_slice(&(request.len() as u32).to_be_bytes());
            pipelined.extend_from_slice(&request);
        }
        stream.write_all(&pipelined).await.unwrap();
        assert!(matches!(
            read_frame::<ControlResponse>(&mut stream).await.unwrap(),
            ControlResponse::Login(_)
        ));
        assert!(read_frame::<ControlResponse>(&mut stream).await.is_err());
        assert_eq!(count.load(Ordering::Relaxed), 1);
        shutdown.send(true).unwrap();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn issuer_failure_returns_only_a_finite_safe_error() {
        let directory = Directory::new();
        let path = directory.socket();
        let server = bind_control(
            &path,
            ORIGIN,
            Arc::new(|| Err(ControlError::IssuanceFailed)),
        )
        .await
        .unwrap();
        let (shutdown, receiver) = watch::channel(false);
        let task = tokio::spawn(server.serve(receiver));
        assert!(matches!(
            request_login_code(&path).await,
            Err(forge_protocol::ui_control::ControlWireError::IssuanceUnavailable)
        ));
        shutdown.send(true).unwrap();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn malformed_command_never_calls_issuer_and_shutdown_aborts_idle_peers() {
        let directory = Directory::new();
        let path = directory.socket();
        let count = Arc::new(AtomicUsize::new(0));
        let server = bind_control(&path, ORIGIN, issuer(Arc::clone(&count)))
            .await
            .unwrap();
        let (shutdown, receiver) = watch::channel(false);
        let task = tokio::spawn(server.serve(receiver));
        let mut stream = connect_owner_socket(&path).await.unwrap();
        write_frame(
            &mut stream,
            &serde_json::json!({"command":"execute","code":"synthetic-private"}),
        )
        .await
        .unwrap();
        assert!(matches!(
            read_frame::<ControlResponse>(&mut stream).await.unwrap(),
            ControlResponse::Error(ControlErrorResponse {
                error: ControlFailure::InvalidRequest
            })
        ));
        assert_eq!(count.load(Ordering::Relaxed), 0);
        let mut idle = connect_owner_socket(&path).await.unwrap();
        idle.write_u32(100).await.unwrap();
        shutdown.send(true).unwrap();
        timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn slow_partial_frame_has_one_total_deadline() {
        let directory = Directory::new();
        let path = directory.socket();
        let count = Arc::new(AtomicUsize::new(0));
        let server = bind_control(&path, ORIGIN, issuer(Arc::clone(&count)))
            .await
            .unwrap();
        let (shutdown, receiver) = watch::channel(false);
        let task = tokio::spawn(server.serve(receiver));
        let mut stream = connect_owner_socket(&path).await.unwrap();
        stream.write_u32(20).await.unwrap();
        let mut byte = [0];
        assert_eq!(
            timeout(Duration::from_secs(6), stream.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert_eq!(count.load(Ordering::Relaxed), 0);
        shutdown.send(true).unwrap();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn stale_owned_socket_is_reclaimed_but_live_listener_is_preserved() {
        let directory = Directory::new();
        let path = directory.socket();
        let listener = UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(
            bind_control(&path, ORIGIN, issuer(Arc::default())).await,
            Err(ControlError::AlreadyRunning)
        ));
        assert!(UnixStream::connect(&path).await.is_ok());
        drop(listener);
        let server = bind_control(&path, ORIGIN, issuer(Arc::default()))
            .await
            .unwrap();
        assert!(validate_owner_socket(&path).is_ok());
        drop(server);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn unsafe_socket_and_lock_files_are_not_replaced_or_repaired() {
        let directory = Directory::new();
        let path = directory.socket();
        fs::write(&path, b"retain-me").unwrap();
        assert!(
            bind_control(&path, ORIGIN, issuer(Arc::default()))
                .await
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"retain-me");
        fs::remove_file(&path).unwrap();
        let lock_path = directory.0.join("control.sock.lock");
        fs::remove_file(&lock_path).unwrap();
        let target = directory.0.join("retain-me");
        fs::write(&target, b"not-a-lock").unwrap();
        symlink(&target, &lock_path).unwrap();
        assert!(matches!(
            bind_control(&path, ORIGIN, issuer(Arc::default())).await,
            Err(ControlError::LockUnavailable)
        ));
        assert_eq!(fs::read(target).unwrap(), b"not-a-lock");
    }

    #[tokio::test]
    async fn cleanup_does_not_remove_a_replacement_socket_inode() {
        let directory = Directory::new();
        let path = directory.socket();
        let server = bind_control(&path, ORIGIN, issuer(Arc::default()))
            .await
            .unwrap();
        fs::remove_file(&path).unwrap();
        let replacement = UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        drop(server);
        assert!(path.exists());
        assert!(UnixStream::connect(&path).await.is_ok());
        drop(replacement);
    }
}
