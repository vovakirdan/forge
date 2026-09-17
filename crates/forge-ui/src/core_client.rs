//! Fixed reads over a checked owner socket. Browser input never becomes a URL.

use std::{path::PathBuf, time::Duration};

use bytes::{Bytes, BytesMut};
use http_body_util::{BodyExt, Empty};
use hyper::{
    Request, StatusCode,
    header::{CONTENT_LENGTH, CONTENT_TYPE},
};
use hyper_util::rt::TokioIo;

use crate::{http::ApiError, read_target::ReadTarget};

pub(crate) struct CoreClient {
    pub socket: PathBuf,
}

impl CoreClient {
    pub async fn read(&self, target: &ReadTarget) -> Result<Bytes, ApiError> {
        tokio::time::timeout(Duration::from_secs(5), self.read_inner(target))
            .await
            .map_err(|_| ApiError::Timeout)?
    }

    async fn read_inner(&self, target: &ReadTarget) -> Result<Bytes, ApiError> {
        let stream = forge_protocol::ui_control::connect_owner_socket(&self.socket)
            .await
            .map_err(|_| ApiError::Unavailable)?;
        let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
            .max_buf_size(16 * 1024)
            .handshake(TokioIo::new(stream))
            .await
            .map_err(|_| ApiError::BadGateway)?;
        let request = Request::builder()
            .method("GET")
            .uri(&target.path)
            .header("host", "localhost")
            .header("accept", "application/json")
            .body(Empty::<Bytes>::new())
            .map_err(|_| ApiError::BadGateway)?;
        // Poll the driver in this future rather than detach it; cancellation closes the UDS.
        let response = async {
            let response = sender
                .send_request(request)
                .await
                .map_err(|_| ApiError::BadGateway)?;
            let status = response.status();
            if response
                .headers()
                .get(CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .is_some_and(|length| length > target.body_limit as u64)
            {
                return Err(ApiError::ResponseTooLarge);
            }
            let json_content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.split(';').next() == Some("application/json"));
            let mut body = response.into_body();
            let mut bytes = BytesMut::new();
            while let Some(frame) = body.frame().await {
                let frame = frame.map_err(|_| ApiError::BadGateway)?;
                if let Some(data) = frame.data_ref() {
                    if bytes.len().saturating_add(data.len()) > target.body_limit {
                        return Err(ApiError::ResponseTooLarge);
                    }
                    bytes.extend_from_slice(data);
                }
            }
            match status {
                StatusCode::OK if json_content_type => {
                    let value: serde_json::Value =
                        serde_json::from_slice(&bytes).map_err(|_| ApiError::BadGateway)?;
                    if !value.is_object() {
                        return Err(ApiError::BadGateway);
                    }
                    Ok(bytes.freeze())
                }
                StatusCode::NOT_FOUND => Err(ApiError::NotFound),
                StatusCode::CONFLICT if target.cursor_conflict && json_content_type => {
                    let value: serde_json::Value =
                        serde_json::from_slice(&bytes).map_err(|_| ApiError::BadGateway)?;
                    if value
                        .pointer("/error/code")
                        .and_then(serde_json::Value::as_str)
                        == Some("cursor_invalid")
                    {
                        Err(ApiError::CursorInvalid)
                    } else {
                        Err(ApiError::BadGateway)
                    }
                }
                StatusCode::SERVICE_UNAVAILABLE => Err(ApiError::Unavailable),
                _ => Err(ApiError::BadGateway),
            }
        };
        tokio::pin!(connection, response);
        tokio::select! {
            biased;
            result = &mut response => result,
            result = &mut connection => {
                result.map_err(|_| ApiError::BadGateway)?;
                response.await
            }
        }
    }
}
