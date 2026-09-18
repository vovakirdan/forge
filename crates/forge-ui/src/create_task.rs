//! Narrow draft creation; pipeline selection and lifecycle policy remain in Core.

use serde::Deserialize;

use crate::{
    command::{uuid_v7, validate_receipt},
    http::ApiError,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateTask {
    project_id: String,
    expected_revision: u64,
    payload: Payload,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    title: String,
    description: String,
    definition_of_done: Option<String>,
    #[serde(rename = "kind")]
    _kind: Kind,
    pipeline_version_id: String,
    priority: String,
    properties: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Delivery,
    Analysis,
}

impl CreateTask {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, ApiError> {
        // Serde struct visitors can also accept positional arrays. The browser
        // contract is object-only; still parse original bytes below to reject duplicates.
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
        let payload = &value.payload;
        if !uuid_v7(&value.project_id)
            || !payload.properties.is_empty()
            || !(1..9_007_199_254_740_991_u64).contains(&value.expected_revision)
            || !uuid_v7(&payload.pipeline_version_id)
            || payload.title.trim().is_empty()
            || payload.title.chars().count() > 240
            || payload.description.chars().count() > 50_000
            || payload
                .definition_of_done
                .as_ref()
                .is_some_and(|text| text.trim().is_empty() || text.chars().count() > 20_000)
            || !(1..=64).contains(&payload.priority.len())
            || !payload
                .priority
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_lowercase)
            || !payload
                .priority
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(ApiError::BadRequest);
        }
        Ok(value)
    }

    pub(crate) fn validate_receipt(&self, bytes: &[u8]) -> Result<(), ApiError> {
        validate_receipt(bytes, self.expected_revision, None)
    }
}
