//! Narrow browser contract for Core-owned Employee amendments.

use std::collections::BTreeSet;

use forge_protocol::wire::CommandReceipt;
use serde::Deserialize;
use serde_json::Value;

use crate::{command::uuid_v7, http::ApiError};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AmendEmployee {
    project_id: String,
    expected_revision: u64,
    payload: Payload,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    employee_id: String,
    expected_employee_revision: u64,
    patch: Patch,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    name: Option<String>,
    role: Option<String>,
    max_concurrent_runs: Option<u16>,
    stage_eligibility: Option<StageEligibility>,
}

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum StageEligibility {
    Any,
    Only { stages: Vec<Stage> },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stage {
    pipeline_version_id: String,
    stage_id: String,
}

impl AmendEmployee {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, ApiError> {
        let shape: Value = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        let patch = shape
            .pointer("/payload/patch")
            .and_then(Value::as_object)
            .filter(|patch| !patch.is_empty())
            .ok_or(ApiError::BadRequest)?;
        if patch.values().any(Value::is_null) {
            return Err(ApiError::BadRequest);
        }
        if let Some(eligibility) = patch.get("stage_eligibility") {
            let object = eligibility.as_object().ok_or(ApiError::BadRequest)?;
            let exact = match object.get("mode").and_then(Value::as_str) {
                Some("any") => object.len() == 1,
                Some("only") => object.len() == 2 && object.contains_key("stages"),
                _ => false,
            };
            if !exact {
                return Err(ApiError::BadRequest);
            }
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        let valid_text =
            |text: &str, max: usize| !text.trim().is_empty() && text.chars().count() <= max;
        if !uuid_v7(&value.project_id)
            || !uuid_v7(&value.payload.employee_id)
            || !(1..MAX_SAFE_INTEGER).contains(&value.expected_revision)
            || !(1..MAX_SAFE_INTEGER).contains(&value.payload.expected_employee_revision)
            || value
                .payload
                .patch
                .name
                .as_ref()
                .is_some_and(|name| !valid_text(name, 200))
            || value
                .payload
                .patch
                .role
                .as_ref()
                .is_some_and(|role| !valid_text(role, 128))
            || value.payload.patch.max_concurrent_runs == Some(0)
        {
            return Err(ApiError::BadRequest);
        }
        if let Some(StageEligibility::Only { stages }) = &value.payload.patch.stage_eligibility {
            if !(1..=128).contains(&stages.len()) {
                return Err(ApiError::BadRequest);
            }
            let mut seen = BTreeSet::new();
            for stage in stages {
                let key = &stage.stage_id;
                if !uuid_v7(&stage.pipeline_version_id)
                    || !(1..=64).contains(&key.len())
                    || !key.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
                    || !key.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                    })
                    || !seen.insert((stage.pipeline_version_id.to_ascii_lowercase(), key.clone()))
                {
                    return Err(ApiError::BadRequest);
                }
            }
        }
        Ok(value)
    }

    pub(crate) fn validate_receipt(&self, bytes: &[u8]) -> Result<(), ApiError> {
        let receipt: CommandReceipt =
            serde_json::from_slice(bytes).map_err(|_| ApiError::BadGateway)?;
        if !uuid_v7(&receipt.command_id)
            || receipt.project_revision != self.expected_revision + 1
            || receipt.event_ids.is_empty()
            || !receipt.event_ids.iter().all(|id| uuid_v7(id))
            || !receipt.resource.as_ref().is_some_and(|resource| {
                resource.kind == "employee"
                    && resource.id.eq_ignore_ascii_case(&self.payload.employee_id)
                    && uuid_v7(&resource.id)
            })
        {
            return Err(ApiError::BadGateway);
        }
        Ok(())
    }
}
