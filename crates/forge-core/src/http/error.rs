//! Stable local API errors without storage or runtime diagnostic leakage.

use axum::{
    Json,
    http::{HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use forge_application::RepositoryError;
use forge_domain::DomainError;
use forge_protocol::wire::{ApiError, ApiErrorCode, ErrorResponse};
use forge_storage::StorageError;

use crate::CoreError;

#[derive(Debug)]
pub(crate) struct HttpError {
    status: StatusCode,
    body: ErrorResponse,
}

impl HttpError {
    pub(crate) fn invalid_request(request_id: String, message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::InvalidRequest,
            request_id,
            message,
        )
    }

    pub(crate) fn cursor_invalid(request_id: String, message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ApiErrorCode::CursorInvalid,
            request_id,
            message,
        )
    }

    pub(crate) fn from_core(request_id: String, error: CoreError) -> Self {
        let message = error.to_string();
        match error {
            CoreError::Application(_) => Self::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                ApiErrorCode::ValidationFailed,
                request_id,
                message,
            ),
            CoreError::Domain(DomainError::StaleProjectRevision { .. })
            | CoreError::Repository(RepositoryError::StaleRevision { .. })
            | CoreError::Storage(StorageError::StaleRevision { .. }) => Self::new(
                StatusCode::CONFLICT,
                ApiErrorCode::StaleRevision,
                request_id,
                message,
            ),
            CoreError::Domain(_) => Self::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                ApiErrorCode::ValidationFailed,
                request_id,
                message,
            ),
            CoreError::NotFound { .. }
            | CoreError::Repository(RepositoryError::NotFound { .. })
            | CoreError::Storage(StorageError::NotFound { .. }) => Self::new(
                StatusCode::NOT_FOUND,
                ApiErrorCode::NotFound,
                request_id,
                message,
            ),
            CoreError::IdempotencyConflict => Self::new(
                StatusCode::CONFLICT,
                ApiErrorCode::IdempotencyConflict,
                request_id,
                message,
            ),
            CoreError::SupervisorUnavailable => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                ApiErrorCode::Unavailable,
                request_id,
                message,
            ),
            CoreError::InvalidTransport { .. } => Self::new(
                StatusCode::BAD_REQUEST,
                ApiErrorCode::InvalidRequest,
                request_id,
                message,
            ),
            CoreError::Forbidden => Self::new(
                StatusCode::FORBIDDEN,
                ApiErrorCode::Forbidden,
                request_id,
                message,
            ),
            CoreError::Storage(_) | CoreError::Repository(_) => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                ApiErrorCode::Unavailable,
                request_id,
                "canonical storage is temporarily unavailable",
            ),
        }
    }

    fn new(
        status: StatusCode,
        code: ApiErrorCode,
        request_id: String,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status,
            body: ErrorResponse {
                error: ApiError {
                    code,
                    message: message.into(),
                    details: None,
                    request_id: Some(request_id),
                },
            },
        }
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let request_id = self
            .body
            .error
            .request_id
            .as_deref()
            .unwrap_or_default()
            .to_owned();
        let mut response = (self.status, Json(self.body)).into_response();
        if let Ok(value) = HeaderValue::from_str(&request_id) {
            response
                .headers_mut()
                .insert(HeaderName::from_static("x-request-id"), value);
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use forge_domain::DomainError;
    use forge_protocol::wire::ApiErrorCode;

    use super::HttpError;
    use crate::CoreError;

    #[test]
    fn stale_project_revision_maps_to_a_machine_readable_conflict() {
        let error = HttpError::from_core(
            "01900000-0000-7000-8000-000000000000".to_owned(),
            CoreError::Domain(DomainError::StaleProjectRevision {
                expected: 1,
                actual: 2,
            }),
        );

        assert_eq!(error.status, StatusCode::CONFLICT);
        assert_eq!(error.body.error.code, ApiErrorCode::StaleRevision);
    }

    #[test]
    fn stale_task_revision_keeps_the_existing_invalid_request_contract() {
        let error = HttpError::from_core(
            "request".to_owned(),
            CoreError::InvalidTransport {
                field: "expected_task_revision",
                reason: "does not match the current task revision".to_owned(),
            },
        );
        assert_eq!(
            (error.status, error.body.error.code),
            (StatusCode::BAD_REQUEST, ApiErrorCode::InvalidRequest)
        );
    }

    #[test]
    fn reused_idempotency_key_is_a_conflict() {
        let error = HttpError::from_core("request".to_owned(), CoreError::IdempotencyConflict);
        assert_eq!(
            (error.status, error.body.error.code),
            (StatusCode::CONFLICT, ApiErrorCode::IdempotencyConflict)
        );
    }

    #[test]
    fn storage_unavailability_never_exposes_adapter_diagnostics() {
        let error = HttpError::from_core(
            "request".to_owned(),
            forge_storage::StorageError::InvalidInput {
                reason: "synthetic-private-database-diagnostic".to_owned(),
            }
            .into(),
        );
        assert_eq!(
            (
                error.status,
                error.body.error.code,
                error.body.error.message.as_str()
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiErrorCode::Unavailable,
                "canonical storage is temporarily unavailable",
            )
        );
    }

    #[test]
    fn application_authorization_refusal_maps_to_forbidden() {
        let error = HttpError::from_core(
            "request".to_owned(),
            forge_application::CommandError::Forbidden.into(),
        );
        assert_eq!(
            (error.status, error.body.error.code),
            (StatusCode::FORBIDDEN, ApiErrorCode::Forbidden)
        );
    }

    #[test]
    fn command_repository_failures_keep_the_transport_classification() {
        use forge_application::{CommandError, RepositoryError};
        let cases = [
            (
                RepositoryError::NotFound { aggregate: "task" },
                StatusCode::NOT_FOUND,
                ApiErrorCode::NotFound,
            ),
            (
                RepositoryError::StaleRevision {
                    aggregate: "project",
                },
                StatusCode::CONFLICT,
                ApiErrorCode::StaleRevision,
            ),
            (
                RepositoryError::Unavailable,
                StatusCode::SERVICE_UNAVAILABLE,
                ApiErrorCode::Unavailable,
            ),
            (
                RepositoryError::InvalidInput {
                    reason: "private adapter diagnostic".into(),
                },
                StatusCode::SERVICE_UNAVAILABLE,
                ApiErrorCode::Unavailable,
            ),
        ];
        for (repository, status, code) in cases {
            let error = HttpError::from_core(
                "request".to_owned(),
                CommandError::Repository(repository).into(),
            );
            assert_eq!((error.status, error.body.error.code), (status, code));
            assert!(
                !error
                    .body
                    .error
                    .message
                    .contains("private adapter diagnostic")
            );
        }
    }
}
