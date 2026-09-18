//! The single browser mutation contract, independent of the wider Core command catalog.

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use forge_protocol::wire::{ApiErrorCode, CommandReceipt, ErrorResponse};
use http_body_util::Full;
use serde::{Deserialize, Deserializer};

use crate::http::{ApiError, HttpState, single_header};

pub(crate) const BODY_LIMIT: usize = 512 * 1024;
pub(crate) const RECEIPT_LIMIT: usize = 64 * 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AmendDraft {
    project_id: String,
    expected_revision: u64,
    payload: DraftPayload,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftPayload {
    task_id: String,
    expected_task_revision: u64,
    patch: TextPatch,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextPatch {
    #[serde(default, deserialize_with = "present_string")]
    title: Option<String>,
    #[serde(default, deserialize_with = "present_string")]
    description: Option<String>,
}

fn present_string<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    // Missing means unchanged; JSON null is not an instruction to erase a text field.
    String::deserialize(deserializer).map(Some)
}

impl AmendDraft {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, ApiError> {
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !uuid_v7(&value.project_id)
            || !uuid_v7(&value.payload.task_id)
            || !(1..MAX_SAFE_INTEGER).contains(&value.expected_revision)
            || !(1..MAX_SAFE_INTEGER).contains(&value.payload.expected_task_revision)
            || (value.payload.patch.title.is_none() && value.payload.patch.description.is_none())
        {
            return Err(ApiError::BadRequest);
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
                resource.kind == "task" && resource.id.eq_ignore_ascii_case(&self.payload.task_id)
            })
        {
            return Err(ApiError::BadGateway);
        }
        Ok(())
    }
}

fn uuid_v7(value: &str) -> bool {
    value.len() == 36
        && uuid::Uuid::parse_str(value)
            .is_ok_and(|id| id.get_version_num() == 7 && id.get_variant() == uuid::Variant::RFC4122)
}

pub(crate) async fn amend_draft(
    state: &HttpState,
    request: Request<Body>,
) -> Result<Response, ApiError> {
    if !single_header(request.headers(), header::CONTENT_TYPE).is_some_and(|value| {
        value.eq_ignore_ascii_case("application/json")
            || value.eq_ignore_ascii_case("application/json; charset=utf-8")
    }) {
        return Err(ApiError::BadRequest);
    }
    let key = single_header(
        request.headers(),
        header::HeaderName::from_static("idempotency-key"),
    )
    .filter(|key| !key.trim().is_empty() && key.len() <= 128)
    .ok_or(ApiError::BadRequest)?
    .to_owned();
    if single_header(request.headers(), header::CONTENT_LENGTH)
        .and_then(|length| length.parse::<u64>().ok())
        .is_some_and(|length| length > BODY_LIMIT as u64)
    {
        return Err(ApiError::TooLarge);
    }
    let body = to_bytes(request.into_body(), BODY_LIMIT)
        .await
        .map_err(|_| ApiError::TooLarge)?;
    let command = AmendDraft::parse(&body)?;
    let response = state.core.amend_draft(&command, &key, body).await?;
    Ok(([(header::CONTENT_TYPE, "application/json")], response).into_response())
}

impl crate::core_client::CoreClient {
    pub(crate) async fn amend_draft(
        &self,
        command: &AmendDraft,
        key: &str,
        body: Bytes,
    ) -> Result<Bytes, ApiError> {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/commands/amend_draft")
            .header(header::HOST, "localhost")
            .header(header::ACCEPT, "application/json")
            .header(header::CONTENT_TYPE, "application/json")
            .header("idempotency-key", key)
            .body(Full::new(body))
            .map_err(|_| ApiError::BadRequest)?;
        let (status, json, bytes) = self.exchange(request, RECEIPT_LIMIT).await?;
        if !json {
            return Err(ApiError::BadGateway);
        }
        if status == StatusCode::OK {
            command.validate_receipt(&bytes)?;
            return Ok(bytes);
        }
        let error: ErrorResponse =
            serde_json::from_slice(&bytes).map_err(|_| ApiError::BadGateway)?;
        // Only explicit, typed Core refusals prove that the command was not applied.
        // Unknown statuses, malformed bodies and 5xx never become a definite refusal.
        Err(match (status, error.error.code) {
            (StatusCode::BAD_REQUEST, ApiErrorCode::InvalidRequest) => ApiError::BadRequest,
            (StatusCode::CONFLICT, ApiErrorCode::StaleRevision) => ApiError::StaleRevision,
            (StatusCode::CONFLICT, ApiErrorCode::IdempotencyConflict) => {
                ApiError::IdempotencyConflict
            }
            (StatusCode::UNPROCESSABLE_ENTITY, ApiErrorCode::ValidationFailed) => {
                ApiError::ValidationFailed
            }
            (StatusCode::NOT_FOUND, ApiErrorCode::NotFound) => ApiError::NotFound,
            (StatusCode::FORBIDDEN, ApiErrorCode::Forbidden) => ApiError::CommandForbidden,
            (StatusCode::SERVICE_UNAVAILABLE, ApiErrorCode::Unavailable) => ApiError::Unavailable,
            _ => ApiError::BadGateway,
        })
    }
}
