//! Explicit browser mutation contracts, independent of the wider Core command catalog.

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

#[derive(Clone, Copy)]
pub(crate) enum CommandTarget {
    CreateTask,
    ApproveTask,
    AmendDraft,
    SetTaskPriority,
}

impl CommandTarget {
    pub(crate) fn from_browser_path(path: &str) -> Option<Self> {
        match path {
            "/api/commands/create_task" => Some(Self::CreateTask),
            "/api/commands/approve_task" => Some(Self::ApproveTask),
            "/api/commands/amend_draft" => Some(Self::AmendDraft),
            "/api/commands/set_task_priority" => Some(Self::SetTaskPriority),
            _ => None,
        }
    }

    fn core_path(self) -> &'static str {
        match self {
            Self::CreateTask => "/v1/commands/create_task",
            Self::ApproveTask => "/v1/commands/approve_task",
            Self::AmendDraft => "/v1/commands/amend_draft",
            Self::SetTaskPriority => "/v1/commands/set_task_priority",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetTaskPriority {
    project_id: String,
    expected_revision: u64,
    payload: PriorityPayload,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PriorityPayload {
    task_id: String,
    expected_task_revision: u64,
    priority: String,
}

impl SetTaskPriority {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, ApiError> {
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !uuid_v7(&value.project_id)
            || !uuid_v7(&value.payload.task_id)
            || !(1..MAX_SAFE_INTEGER).contains(&value.expected_revision)
            || !(1..MAX_SAFE_INTEGER).contains(&value.payload.expected_task_revision)
            || !(1..=64).contains(&value.payload.priority.len())
            || !value
                .payload
                .priority
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_lowercase)
            || !value
                .payload
                .priority
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(ApiError::BadRequest);
        }
        Ok(value)
    }

    pub(crate) fn validate_receipt(&self, bytes: &[u8]) -> Result<(), ApiError> {
        validate_receipt(bytes, self.expected_revision, Some(&self.payload.task_id))
    }
}

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
    #[serde(default, deserialize_with = "present_nullable_string")]
    definition_of_done: Option<Option<String>>,
    #[serde(default, deserialize_with = "present_string")]
    title: Option<String>,
    #[serde(default, deserialize_with = "present_string")]
    description: Option<String>,
}

fn present_nullable_string<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}

fn present_string<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    // Missing means unchanged; JSON null is not an instruction to erase a text field.
    String::deserialize(deserializer).map(Some)
}

impl AmendDraft {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, ApiError> {
        let shape: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !shape.is_object()
            || !shape
                .get("payload")
                .is_some_and(serde_json::Value::is_object)
            || !shape
                .pointer("/payload/patch")
                .is_some_and(serde_json::Value::is_object)
        {
            return Err(ApiError::BadRequest);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ApiError::BadRequest)?;
        if !uuid_v7(&value.project_id)
            || !uuid_v7(&value.payload.task_id)
            || !(1..MAX_SAFE_INTEGER).contains(&value.expected_revision)
            || !(1..MAX_SAFE_INTEGER).contains(&value.payload.expected_task_revision)
            || (value.payload.patch.title.is_none()
                && value.payload.patch.description.is_none()
                && value.payload.patch.definition_of_done.is_none())
            || value
                .payload
                .patch
                .definition_of_done
                .as_ref()
                .and_then(Option::as_ref)
                .is_some_and(|text| text.trim().is_empty() || text.chars().count() > 20_000)
        {
            return Err(ApiError::BadRequest);
        }
        Ok(value)
    }

    pub(crate) fn validate_receipt(&self, bytes: &[u8]) -> Result<(), ApiError> {
        validate_receipt(bytes, self.expected_revision, Some(&self.payload.task_id))
    }
}

pub(crate) fn validate_receipt(
    bytes: &[u8],
    expected_revision: u64,
    task_id: Option<&str>,
) -> Result<(), ApiError> {
    let receipt: CommandReceipt =
        serde_json::from_slice(bytes).map_err(|_| ApiError::BadGateway)?;
    if !uuid_v7(&receipt.command_id)
        || receipt.project_revision != expected_revision + 1
        || receipt.event_ids.is_empty()
        || !receipt.event_ids.iter().all(|id| uuid_v7(id))
        || !receipt.resource.as_ref().is_some_and(|resource| {
            resource.kind == "task"
                && uuid_v7(&resource.id)
                && task_id.is_none_or(|task_id| resource.id.eq_ignore_ascii_case(task_id))
        })
    {
        return Err(ApiError::BadGateway);
    }
    Ok(())
}

pub(crate) fn uuid_v7(value: &str) -> bool {
    value.len() == 36
        && uuid::Uuid::parse_str(value)
            .is_ok_and(|id| id.get_version_num() == 7 && id.get_variant() == uuid::Variant::RFC4122)
}

pub(crate) async fn execute(
    state: &HttpState,
    request: Request<Body>,
    target: CommandTarget,
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
    let response = match target {
        CommandTarget::ApproveTask => {
            let command = crate::approve_task::ApproveTask::parse(&body)?;
            let response = state
                .core
                .command(CommandTarget::ApproveTask, &key, body)
                .await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::CreateTask => {
            let command = crate::create_task::CreateTask::parse(&body)?;
            let response = state
                .core
                .command(CommandTarget::CreateTask, &key, body)
                .await?;
            command.validate_receipt(&response)?;
            response
        }
        CommandTarget::AmendDraft => {
            let command = AmendDraft::parse(&body)?;
            state.core.amend_draft(&command, &key, body).await?
        }
        CommandTarget::SetTaskPriority => {
            let command = SetTaskPriority::parse(&body)?;
            state.core.set_task_priority(&command, &key, body).await?
        }
    };
    Ok(([(header::CONTENT_TYPE, "application/json")], response).into_response())
}

impl crate::core_client::CoreClient {
    pub(crate) async fn amend_draft(
        &self,
        command: &AmendDraft,
        key: &str,
        body: Bytes,
    ) -> Result<Bytes, ApiError> {
        let bytes = self.command(CommandTarget::AmendDraft, key, body).await?;
        command.validate_receipt(&bytes)?;
        Ok(bytes)
    }

    pub(crate) async fn set_task_priority(
        &self,
        command: &SetTaskPriority,
        key: &str,
        body: Bytes,
    ) -> Result<Bytes, ApiError> {
        let bytes = self
            .command(CommandTarget::SetTaskPriority, key, body)
            .await?;
        command.validate_receipt(&bytes)?;
        Ok(bytes)
    }

    async fn command(
        &self,
        target: CommandTarget,
        key: &str,
        body: Bytes,
    ) -> Result<Bytes, ApiError> {
        let request = Request::builder()
            .method("POST")
            .uri(target.core_path())
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
