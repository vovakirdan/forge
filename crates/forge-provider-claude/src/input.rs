use forge_provider_common::{SecretBytes, adapter::AdapterError};
use serde::Serialize;
use uuid::Uuid;

use crate::MAX_EVENT_BYTES;

/// Encodes initial or additional input as one stream-json line. The UUID must be
/// retained by the caller and matched against replay receipts before accepting
/// delivery. A replay is not an Employee acknowledgment or a stage outcome.
pub fn encode_user_message(
    session_id: Uuid,
    message_id: Uuid,
    text: &SecretBytes,
) -> Result<SecretBytes, AdapterError> {
    let text = std::str::from_utf8(text.expose()).map_err(|_| AdapterError::InvalidEvent)?;
    if session_id.is_nil()
        || message_id.is_nil()
        || text.trim().is_empty()
        || text.len() > MAX_EVENT_BYTES
    {
        return Err(AdapterError::InvalidEvent);
    }
    #[derive(Serialize)]
    struct Message<'a> {
        role: &'static str,
        content: &'a str,
    }
    #[derive(Serialize)]
    struct Input<'a> {
        r#type: &'static str,
        session_id: Uuid,
        uuid: Uuid,
        parent_tool_use_id: Option<&'static str>,
        message: Message<'a>,
    }
    let mut bytes = serde_json::to_vec(&Input {
        r#type: "user",
        session_id,
        uuid: message_id,
        parent_tool_use_id: None,
        message: Message {
            role: "user",
            content: text,
        },
    })
    .map_err(|_| AdapterError::InvalidEvent)?;
    bytes.push(b'\n');
    let encoded = SecretBytes::new(bytes);
    if encoded.expose().len() > MAX_EVENT_BYTES + 1 {
        return Err(AdapterError::InvalidEvent);
    }
    Ok(encoded)
}

/// Validates an enrolled `claude setup-token` value without logging it or
/// accepting an API key. Trailing line delimiters from a private file are allowed.
/// This checks shape only; validity/expiry require an explicit live auth check.
pub fn validate_setup_token(token: &SecretBytes) -> Result<&str, AdapterError> {
    let text = std::str::from_utf8(token.expose()).map_err(|_| AdapterError::ProfileMismatch)?;
    let text = text.trim_end_matches(['\r', '\n']);
    if text.len() > 16 * 1024
        || !text.starts_with("sk-ant-oat01-")
        || text.len() <= "sk-ant-oat01-".len()
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(AdapterError::ProfileMismatch);
    }
    Ok(text)
}
