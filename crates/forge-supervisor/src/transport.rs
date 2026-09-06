//! Authenticated local Unix-domain gRPC transport.

use crate::SupervisorError;
use hyper_util::rt::TokioIo;
use nix::unistd::Uid;
use std::{
    io,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::Path,
};
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint};
use tower::service_fn;

pub(crate) async fn connect(socket_path: &Path) -> Result<Channel, SupervisorError> {
    validate_socket_path(socket_path)?;
    let socket_path = socket_path.to_owned();
    let expected_uid = Uid::effective().as_raw();
    Endpoint::from_static("http://[::]:50051")
        .connect_with_connector(service_fn(move |_| {
            let socket_path = socket_path.clone();
            async move {
                let stream = UnixStream::connect(socket_path).await?;
                if stream.peer_cred()?.uid() != expected_uid {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "Core peer identity does not match local Supervisor user",
                    ));
                }
                Ok::<_, io::Error>(TokioIo::new(stream))
            }
        }))
        .await
        .map_err(SupervisorError::Transport)
}

fn validate_socket_path(socket_path: &Path) -> Result<(), SupervisorError> {
    let owner = Uid::effective().as_raw();
    let directory = socket_path.parent().ok_or(SupervisorError::UnsafeSocket)?;
    let directory_metadata =
        std::fs::symlink_metadata(directory).map_err(|_| SupervisorError::UnsafeSocket)?;
    let socket_metadata =
        std::fs::symlink_metadata(socket_path).map_err(|_| SupervisorError::UnsafeSocket)?;
    let directory_is_safe = directory_metadata.file_type().is_dir()
        && directory_metadata.uid() == owner
        && directory_metadata.mode() & 0o077 == 0;
    let socket_is_safe = socket_metadata.file_type().is_socket()
        && socket_metadata.uid() == owner
        && socket_metadata.mode() & 0o077 == 0;
    if directory_is_safe && socket_is_safe {
        Ok(())
    } else {
        Err(SupervisorError::UnsafeSocket)
    }
}
