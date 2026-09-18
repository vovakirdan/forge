//! Approval intent only; Core owns the resulting lifecycle, stage and Task revision.
use crate::{
    command::{uuid_v7, validate_receipt},
    http::ApiError,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApproveTask {
    project_id: String,
    expected_revision: u64,
    payload: Payload,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    task_id: String,
    expected_task_revision: u64,
}

impl ApproveTask {
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
        if !uuid_v7(&value.project_id)
            || !uuid_v7(&value.payload.task_id)
            || !(1..9_007_199_254_740_991_u64).contains(&value.expected_revision)
            || !(1..9_007_199_254_740_991_u64).contains(&value.payload.expected_task_revision)
        {
            return Err(ApiError::BadRequest);
        }
        Ok(value)
    }

    pub(crate) fn validate_receipt(&self, bytes: &[u8]) -> Result<(), ApiError> {
        validate_receipt(bytes, self.expected_revision, Some(&self.payload.task_id))
    }
}
