//! Cancellation intent only; Core controls terminal state and physical Run shutdown.
use crate::{
    command::{uuid_v7, validate_receipt_revision},
    http::ApiError,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CancelTask {
    project_id: String,
    expected_revision: u64,
    payload: Payload,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    task_id: String,
    expected_task_revision: u64,
    cancellation_reason_key: String,
    note: Option<String>,
}

impl CancelTask {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, ApiError> {
        let shape: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !shape.is_object()
            || !shape
                .get("payload")
                .is_some_and(serde_json::Value::is_object)
        {
            return Err(ApiError::BadRequest);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        let reason = &value.payload.cancellation_reason_key;
        if !uuid_v7(&value.project_id)
            || !uuid_v7(&value.payload.task_id)
            || !(1..9_007_199_254_740_991_u64).contains(&value.expected_revision)
            || !(1..9_007_199_254_740_991_u64).contains(&value.payload.expected_task_revision)
            || !(1..=64).contains(&reason.len())
            || !reason
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_lowercase)
            || !reason
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
            || value
                .payload
                .note
                .as_ref()
                .is_some_and(|note| note.trim().is_empty() || note.chars().count() > 20_000)
        {
            return Err(ApiError::BadRequest);
        }
        Ok(value)
    }

    pub(crate) fn validate_receipt(&self, bytes: &[u8]) -> Result<(), ApiError> {
        // Cancellation can also reconcile dependent Tasks in the same transaction.
        validate_receipt_revision(
            bytes,
            self.expected_revision,
            Some(&self.payload.task_id),
            true,
        )
    }
}
