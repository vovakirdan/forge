//! Bounded owner command for the Project's Task property schema.

use std::collections::BTreeMap;

use forge_protocol::wire::CommandReceipt;
use serde::{Deserialize, Serialize};

use crate::{command::uuid_v7, http::ApiError};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    project_id: String,
    expected_revision: u64,
    payload: Payload,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    schema: Schema,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Schema {
    definitions: BTreeMap<String, Definition>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    key: String,
    display_name: String,
    property_type: String,
    #[serde(rename = "required")]
    _required: bool,
    default_value: Option<serde_json::Value>,
    allowed_choices: Option<Vec<String>>,
}

fn stable_key(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes.first().is_some_and(u8::is_ascii_lowercase)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
}

pub(crate) struct PropertySchemaCommand {
    project_id: String,
    expected_revision: u64,
}

impl PropertySchemaCommand {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, ApiError> {
        let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !uuid_v7(&envelope.project_id)
            || !(1..MAX_SAFE_INTEGER).contains(&envelope.expected_revision)
            || envelope.payload.schema.definitions.len() > 64
            || serde_json::to_vec(&envelope.payload.schema)
                .map_err(|_| ApiError::BadRequest)?
                .len()
                > 65_536
            || envelope
                .payload
                .schema
                .definitions
                .iter()
                .any(|(key, definition)| {
                    !stable_key(key)
                        || key != &definition.key
                        || definition.display_name.trim().is_empty()
                        || definition.display_name.chars().count() > 200
                        || !matches!(
                            definition.property_type.as_str(),
                            "boolean"
                                | "text"
                                | "number"
                                | "date"
                                | "enum"
                                | "multi_enum"
                                | "reference"
                        )
                        || definition
                            .default_value
                            .as_ref()
                            .is_some_and(|value| !value.is_object())
                        || definition.allowed_choices.as_ref().is_some_and(|choices| {
                            choices.iter().any(|choice| {
                                choice.trim().is_empty() || choice.chars().count() > 128
                            })
                        })
                })
        {
            return Err(ApiError::BadRequest);
        }
        Ok(Self {
            project_id: envelope.project_id,
            expected_revision: envelope.expected_revision,
        })
    }

    pub(crate) fn validate_receipt(&self, bytes: &[u8]) -> Result<(), ApiError> {
        let receipt: CommandReceipt =
            serde_json::from_slice(bytes).map_err(|_| ApiError::BadGateway)?;
        if !uuid_v7(&receipt.command_id)
            || receipt.project_revision != self.expected_revision + 1
            || receipt.event_ids.is_empty()
            || !receipt.event_ids.iter().all(|id| uuid_v7(id))
            || !receipt.resource.as_ref().is_some_and(|resource| {
                resource.kind == "project" && resource.id.eq_ignore_ascii_case(&self.project_id)
            })
        {
            return Err(ApiError::BadGateway);
        }
        Ok(())
    }
}
