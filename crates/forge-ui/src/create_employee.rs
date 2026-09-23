//! Narrow browser contract for Core-owned Employee creation.

use std::collections::BTreeSet;

use forge_protocol::wire::CommandReceipt;
use serde::Deserialize;

use crate::{command::uuid_v7, http::ApiError};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateEmployee {
    project_id: String,
    expected_revision: u64,
    payload: Payload,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    name: String,
    role: String,
    stage_eligibility: StageEligibility,
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

impl CreateEmployee {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, ApiError> {
        let shape: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !shape.is_object() || !shape.get("payload").is_some_and(|value| value.is_object()) {
            return Err(ApiError::BadRequest);
        }
        let eligibility = shape
            .pointer("/payload/stage_eligibility")
            .and_then(serde_json::Value::as_object)
            .ok_or(ApiError::BadRequest)?;
        let exact_eligibility = match eligibility.get("mode").and_then(serde_json::Value::as_str) {
            Some("any") => eligibility.len() == 1,
            Some("only") => eligibility.len() == 2 && eligibility.contains_key("stages"),
            _ => false,
        };
        if !exact_eligibility {
            return Err(ApiError::BadRequest);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        let valid_text =
            |text: &str, max: usize| !text.trim().is_empty() && text.chars().count() <= max;
        if !uuid_v7(&value.project_id)
            || !(1..9_007_199_254_740_991_u64).contains(&value.expected_revision)
            || !valid_text(&value.payload.name, 200)
            || !valid_text(&value.payload.role, 128)
        {
            return Err(ApiError::BadRequest);
        }
        if let StageEligibility::Only { stages } = &value.payload.stage_eligibility {
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
            || !receipt
                .resource
                .as_ref()
                .is_some_and(|resource| resource.kind == "employee" && uuid_v7(&resource.id))
        {
            return Err(ApiError::BadGateway);
        }
        Ok(())
    }
}
