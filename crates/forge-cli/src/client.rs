//! Minimal HTTP/1.1 client for Forge's owner-local Unix socket.

use std::{
    fs,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::{Path, PathBuf},
};

use forge_protocol::{
    local_paths::default_runtime_directory,
    wire::{CommandReceipt, CommandRequest, ErrorResponse, EventEnvelope},
};
use nix::unistd::Uid;
use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

use crate::sse::parse_events;

/// Bounded local HTTP client; it has no direct Core or storage dependency.
#[derive(Clone, Debug)]
pub struct LocalClient {
    socket: PathBuf,
    verify_owner_only_socket: bool,
}

impl LocalClient {
    /// Creates a client for an explicit Forge API Unix-domain socket.
    #[must_use]
    pub fn new(socket: PathBuf) -> Self {
        Self {
            socket,
            verify_owner_only_socket: false,
        }
    }

    /// Uses the documented XDG location for the local Core API socket.
    #[must_use]
    pub fn from_environment() -> Self {
        Self {
            socket: default_socket_path(),
            verify_owner_only_socket: true,
        }
    }

    /// Returns the local socket path selected for this client.
    #[must_use]
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Sends one named command through the public Core command endpoint.
    pub async fn execute_command(
        &self,
        name: &str,
        request: &CommandRequest,
        idempotency_key: &str,
    ) -> Result<CommandReceipt, ClientError> {
        let body = serde_json::to_vec(request)?;
        let response = self
            .request(
                "POST",
                &format!("/v1/commands/{name}"),
                [
                    ("content-type", "application/json"),
                    ("idempotency-key", idempotency_key),
                ],
                Some(&body),
            )
            .await?;
        response.into_json()
    }

    /// Reads one public JSON view without bypassing the Core transport boundary.
    pub async fn get_json(&self, path: &str) -> Result<Value, ClientError> {
        if !path.starts_with("/v1/") {
            return Err(ClientError::InvalidPath);
        }
        self.request("GET", path, [], None).await?.into_json()
    }

    /// Replays the currently available bounded Project event segment over SSE.
    pub async fn watch_events(
        &self,
        project_id: &str,
        after: Option<u64>,
    ) -> Result<Vec<EventEnvelope>, ClientError> {
        let suffix = after.map_or_else(String::new, |value| format!("?after={value}"));
        let response = self
            .request(
                "GET",
                &format!("/v1/projects/{project_id}/events{suffix}"),
                [("accept", "text/event-stream")],
                None,
            )
            .await?;
        response.require_success()?;
        parse_events(&response.body)
    }

    /// Reconnects an SSE replay using the protocol's `Last-Event-ID` header.
    pub async fn watch_events_after_last_event_id(
        &self,
        project_id: &str,
        last_event_id: u64,
    ) -> Result<Vec<EventEnvelope>, ClientError> {
        let last_event_id = last_event_id.to_string();
        let response = self
            .request(
                "GET",
                &format!("/v1/projects/{project_id}/events"),
                [
                    ("accept", "text/event-stream"),
                    ("last-event-id", last_event_id.as_str()),
                ],
                None,
            )
            .await?;
        response.require_success()?;
        parse_events(&response.body)
    }

    async fn request<'a, const N: usize>(
        &self,
        method: &str,
        path: &str,
        headers: [(&'a str, &'a str); N],
        body: Option<&[u8]>,
    ) -> Result<HttpResponse, ClientError> {
        if self.verify_owner_only_socket {
            validate_owner_only_socket(&self.socket)?;
        }
        let mut stream = UnixStream::connect(&self.socket).await?;
        let content_length = body.map_or(0, |body| body.len());
        let mut request = format!(
            "{method} {path} HTTP/1.1\r\nHost: forge.local\r\nConnection: close\r\nContent-Length: {content_length}\r\n"
        );
        for (name, value) in headers {
            request.push_str(name);
            request.push_str(": ");
            request.push_str(value);
            request.push_str("\r\n");
        }
        request.push_str("\r\n");
        stream.write_all(request.as_bytes()).await?;
        if let Some(body) = body {
            stream.write_all(body).await?;
        }
        stream.flush().await?;
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await?;
        HttpResponse::parse(&bytes)
    }
}

/// Returns the documented per-user Core API socket path.
#[must_use]
pub fn default_socket_path() -> PathBuf {
    default_runtime_directory().join("api.sock")
}

fn validate_owner_only_socket(socket_path: &Path) -> Result<(), ClientError> {
    let owner = Uid::effective().as_raw();
    let directory = socket_path
        .parent()
        .ok_or(ClientError::UnsafeDefaultSocket)?;
    let directory_metadata =
        fs::symlink_metadata(directory).map_err(|_| ClientError::UnsafeDefaultSocket)?;
    let socket_metadata =
        fs::symlink_metadata(socket_path).map_err(|_| ClientError::UnsafeDefaultSocket)?;
    let directory_is_safe = directory_metadata.file_type().is_dir()
        && directory_metadata.uid() == owner
        && directory_metadata.mode() & 0o077 == 0;
    let socket_is_safe = socket_metadata.file_type().is_socket()
        && socket_metadata.uid() == owner
        && socket_metadata.mode() & 0o077 == 0;
    if directory_is_safe && socket_is_safe {
        Ok(())
    } else {
        Err(ClientError::UnsafeDefaultSocket)
    }
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("cannot use a non-Forge API path")]
    InvalidPath,
    #[error("default local Core API socket is not owner-only for the current user")]
    UnsafeDefaultSocket,
    #[error("local Core API transport failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("local Core API JSON encoding failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("local Core API returned malformed HTTP: {0}")]
    MalformedHttp(&'static str),
    #[error("local Core API returned HTTP {status}: {message}")]
    Api { status: u16, message: String },
    #[error("local Core API returned malformed SSE: {0}")]
    Sse(&'static str),
}

struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

impl HttpResponse {
    fn parse(bytes: &[u8]) -> Result<Self, ClientError> {
        let separator = bytes
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .ok_or(ClientError::MalformedHttp(
                "response headers are incomplete",
            ))?;
        let headers = std::str::from_utf8(&bytes[..separator])
            .map_err(|_| ClientError::MalformedHttp("response headers are not UTF-8"))?;
        let mut lines = headers.split("\r\n");
        let status = lines
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .ok_or(ClientError::MalformedHttp("status line is missing"))?
            .parse()
            .map_err(|_| ClientError::MalformedHttp("status code is invalid"))?;
        let headers = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
            .collect::<Vec<_>>();
        let body = &bytes[(separator + 4)..];
        let chunked = headers
            .iter()
            .find(|(name, _)| name == "transfer-encoding")
            .is_some_and(|(_, value)| value.eq_ignore_ascii_case("chunked"));
        let body = if chunked {
            decode_chunked(body)?
        } else {
            body.to_vec()
        };
        Ok(Self { status, body })
    }

    fn into_json<T: DeserializeOwned>(self) -> Result<T, ClientError> {
        self.require_success()?;
        serde_json::from_slice(&self.body).map_err(ClientError::from)
    }

    fn require_success(&self) -> Result<(), ClientError> {
        if (200..300).contains(&self.status) {
            return Ok(());
        }
        let message = serde_json::from_slice::<ErrorResponse>(&self.body)
            .map(|response| response.error.message)
            .unwrap_or_else(|_| "local Core request failed".to_owned());
        Err(ClientError::Api {
            status: self.status,
            message,
        })
    }
}

fn decode_chunked(bytes: &[u8]) -> Result<Vec<u8>, ClientError> {
    let mut cursor = 0;
    let mut result = Vec::new();
    loop {
        let line_end = bytes[cursor..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .map(|index| cursor + index)
            .ok_or(ClientError::MalformedHttp("chunk length is incomplete"))?;
        let length = std::str::from_utf8(&bytes[cursor..line_end])
            .ok()
            .and_then(|line| line.split(';').next())
            .and_then(|length| usize::from_str_radix(length, 16).ok())
            .ok_or(ClientError::MalformedHttp("chunk length is invalid"))?;
        cursor = line_end + 2;
        if length == 0 {
            return Ok(result);
        }
        let end = cursor
            .checked_add(length)
            .ok_or(ClientError::MalformedHttp("chunk length overflow"))?;
        let suffix_end = end
            .checked_add(2)
            .ok_or(ClientError::MalformedHttp("chunk length overflow"))?;
        if bytes.get(end..suffix_end) != Some(b"\r\n") {
            return Err(ClientError::MalformedHttp("chunk payload is incomplete"));
        }
        result.extend_from_slice(
            bytes
                .get(cursor..end)
                .ok_or(ClientError::MalformedHttp("chunk payload is incomplete"))?,
        );
        cursor = end + 2;
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::{fs::PermissionsExt, net::UnixListener},
    };

    use super::{ClientError, HttpResponse, decode_chunked, validate_owner_only_socket};
    use uuid::Uuid;

    #[test]
    fn chunked_body_is_decoded_before_the_sse_parser_reads_it() {
        let decoded = decode_chunked(b"4\r\ntest\r\n0\r\n\r\n").expect("valid chunk fixture");

        assert_eq!(decoded, b"test");
    }

    #[test]
    fn response_parser_preserves_a_success_status() {
        let response = HttpResponse::parse(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
            .expect("valid response fixture");

        assert_eq!(response.status, 200);
    }

    #[test]
    fn default_socket_validation_requires_owner_only_directory_and_socket() {
        let directory = std::env::temp_dir().join(format!("forge-cli-{}", Uuid::now_v7()));
        fs::create_dir(&directory).expect("create test directory");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .expect("restrict test directory");
        let socket = directory.join("api.sock");
        let listener = UnixListener::bind(&socket).expect("bind test socket");
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
            .expect("restrict test socket");

        assert!(validate_owner_only_socket(&socket).is_ok());
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o666))
            .expect("make test socket unsafe");
        assert!(matches!(
            validate_owner_only_socket(&socket),
            Err(ClientError::UnsafeDefaultSocket)
        ));

        drop(listener);
        fs::remove_file(&socket).expect("remove test socket");
        fs::remove_dir(&directory).expect("remove test directory");
    }
}
