//! One bounded, length-prefixed JSON command per private owner socket connection.

use std::{fmt, path::Path, time::Duration};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub use crate::owner_socket::{OwnerSocketError, connect_owner_socket};

/// Maximum JSON frame body in either direction, excluding the four-byte length.
pub const CONTROL_FRAME_LIMIT: usize = 1024;
/// Whole control exchange deadline, not a separate allowance for every byte.
pub const CONTROL_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

/// Only the trusted owner CLI can submit this private command.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlRequest {
    IssueLoginCode {},
}

/// Secret-bearing response. Debug output never contains the code.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginCodeResponse {
    pub origin: String,
    pub code: String,
    pub expires_at: String,
}

impl fmt::Debug for LoginCodeResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LoginCodeResponse { code: [REDACTED], .. }")
    }
}

impl LoginCodeResponse {
    /// Reject malformed fields before any untrusted text reaches an owner terminal.
    pub fn validate(&self) -> Result<(), ControlWireError> {
        let port = self
            .origin
            .strip_prefix("http://127.0.0.1:")
            .and_then(|port| port.parse::<u16>().ok())
            .filter(|port| *port != 0)
            .ok_or(ControlWireError::InvalidFrame)?;
        if self.origin != format!("http://127.0.0.1:{port}")
            || self.code.len() != 64
            || !self
                .code
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || OffsetDateTime::parse(&self.expires_at, &Rfc3339).is_err()
        {
            return Err(ControlWireError::InvalidFrame);
        }
        Ok(())
    }
}

/// Finite wire errors cannot echo a malformed frame or an issuance implementation error.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlFailure {
    InvalidRequest,
    IssuanceUnavailable,
}

/// Safe error envelope, separate from the successful login response.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlErrorResponse {
    pub error: ControlFailure,
}

/// The success fields remain top-level as required by the private wire contract.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ControlResponse {
    Login(LoginCodeResponse),
    Error(ControlErrorResponse),
}

/// Safe control transport diagnostics; never retain JSON decoder input or secrets.
#[derive(Debug, Error)]
pub enum ControlWireError {
    #[error("invalid UI control frame")]
    InvalidFrame,
    #[error("UI control transport failed")]
    Transport,
    #[error("UI control exchange timed out")]
    Timeout,
    #[error("UI login code issuance is unavailable")]
    IssuanceUnavailable,
    #[error(transparent)]
    OwnerSocket(#[from] OwnerSocketError),
}

/// Read a big-endian length and exactly one bounded JSON frame.
pub async fn read_frame<T: DeserializeOwned>(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<T, ControlWireError> {
    let length = stream
        .read_u32()
        .await
        .map_err(|_| ControlWireError::Transport)? as usize;
    if length == 0 || length > CONTROL_FRAME_LIMIT {
        return Err(ControlWireError::InvalidFrame);
    }
    let mut bytes = vec![0; length];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|_| ControlWireError::Transport)?;
    serde_json::from_slice(&bytes).map_err(|_| ControlWireError::InvalidFrame)
}

/// Write one bounded JSON frame, without logging serialized material.
pub async fn write_frame<T: Serialize>(
    stream: &mut (impl AsyncWrite + Unpin),
    value: &T,
) -> Result<(), ControlWireError> {
    let bytes = serde_json::to_vec(value).map_err(|_| ControlWireError::InvalidFrame)?;
    if bytes.is_empty() || bytes.len() > CONTROL_FRAME_LIMIT {
        return Err(ControlWireError::InvalidFrame);
    }
    let length = u32::try_from(bytes.len()).map_err(|_| ControlWireError::InvalidFrame)?;
    stream
        .write_u32(length)
        .await
        .map_err(|_| ControlWireError::Transport)?;
    stream
        .write_all(&bytes)
        .await
        .map_err(|_| ControlWireError::Transport)?;
    stream
        .flush()
        .await
        .map_err(|_| ControlWireError::Transport)
}

/// Issue one code over a peer-authenticated owner socket within the total deadline.
pub async fn request_login_code(path: &Path) -> Result<LoginCodeResponse, ControlWireError> {
    tokio::time::timeout(CONTROL_EXCHANGE_TIMEOUT, async {
        let mut stream = connect_owner_socket(path).await?;
        write_frame(&mut stream, &ControlRequest::IssueLoginCode {}).await?;
        match read_frame(&mut stream).await? {
            ControlResponse::Login(response) => {
                response.validate()?;
                Ok(response)
            }
            ControlResponse::Error(_) => Err(ControlWireError::IssuanceUnavailable),
        }
    })
    .await
    .map_err(|_| ControlWireError::Timeout)?
}

#[cfg(test)]
#[path = "ui_control_tests.rs"]
mod tests;
