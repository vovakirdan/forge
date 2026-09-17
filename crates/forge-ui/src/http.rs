//! Browser guards precede route dispatch and every Core connection.

use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    extract::State,
    http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use tokio::sync::Semaphore;

use crate::{
    assets::AssetSnapshot,
    auth::{AuthError, SessionStore},
    core_client::CoreClient,
};

pub(crate) const CSP: &str = "default-src 'none'; script-src 'self'; script-src-attr 'none'; style-src 'self'; connect-src 'self'; img-src 'self'; font-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'self'";

pub(crate) struct HttpState {
    pub host: String,
    pub origin: String,
    pub sessions: Arc<SessionStore>,
    pub core: CoreClient,
    pub assets: AssetSnapshot,
    pub capacity: Semaphore,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ApiError {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    TooLarge,
    Capacity,
    BadGateway,
    Unavailable,
    Timeout,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::BadRequest => (StatusCode::BAD_REQUEST, "invalid request"),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "authentication required"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "request origin rejected"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not found"),
            Self::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "request too large"),
            Self::Capacity => (StatusCode::TOO_MANY_REQUESTS, "capacity reached"),
            Self::BadGateway => (StatusCode::BAD_GATEWAY, "invalid Core response"),
            Self::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "service unavailable"),
            Self::Timeout => (StatusCode::GATEWAY_TIMEOUT, "request timed out"),
        };
        (status, axum::Json(serde_json::json!({"error":message}))).into_response()
    }
}

impl From<AuthError> for ApiError {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::Unauthorized => Self::Unauthorized,
            AuthError::Capacity => Self::Capacity,
            AuthError::Unavailable => Self::Unavailable,
        }
    }
}

pub(crate) async fn handle(
    State(state): State<Arc<HttpState>>,
    request: Request<Body>,
) -> Response {
    // Reserve time for writing a safe timeout response inside the hard connection deadline.
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(4500),
        guarded_dispatch(&state, request),
    )
    .await
    .unwrap_or(Err(ApiError::Timeout));
    let response = match result {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    secure_headers(response)
}

async fn guarded_dispatch(state: &HttpState, request: Request<Body>) -> Result<Response, ApiError> {
    let headers = request.headers();
    if single_header(headers, header::HOST) != Some(state.host.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let origin = single_header(headers, header::ORIGIN);
    if headers.contains_key(header::ORIGIN) && origin != Some(state.origin.as_str()) {
        return Err(ApiError::Forbidden);
    }
    if request.uri().scheme().is_some()
        || request.uri().authority().is_some()
        || request.uri().query().is_some()
    {
        return Err(ApiError::NotFound);
    }
    let path = request.uri().path();
    if path.starts_with("/api/") {
        let _permit = state
            .capacity
            .try_acquire()
            .map_err(|_| ApiError::Capacity)?;
        if request.method() == Method::POST
            && matches!(path, "/api/auth/exchange" | "/api/auth/logout")
        {
            if origin != Some(state.origin.as_str()) {
                return Err(ApiError::Forbidden);
            }
            if path == "/api/auth/exchange" {
                return exchange(state, request).await;
            }
            let token = bearer(headers)?;
            state.sessions.authorize(token)?;
            // Logout has no payload. Refuse chunked/declared bodies before revocation.
            if headers.contains_key(header::TRANSFER_ENCODING)
                || single_header(headers, header::CONTENT_LENGTH)
                    .is_some_and(|length| length != "0")
            {
                return Err(ApiError::BadRequest);
            }
            state.sessions.logout(token)?;
            return Ok(StatusCode::NO_CONTENT.into_response());
        }
        if request.method() != Method::GET {
            return Err(ApiError::NotFound);
        }
        let core_path = core_path(path).ok_or(ApiError::NotFound)?;
        let token = bearer(headers)?;
        state.sessions.authorize(token)?;
        if headers.contains_key(header::TRANSFER_ENCODING)
            || single_header(headers, header::CONTENT_LENGTH).is_some_and(|length| length != "0")
        {
            return Err(ApiError::BadRequest);
        }
        let body = state.core.read(&core_path).await?;
        return Ok(([(header::CONTENT_TYPE, "application/json")], body).into_response());
    }
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return Err(ApiError::NotFound);
    }
    let asset = state.assets.get(path).ok_or(ApiError::NotFound)?;
    let body = if request.method() == Method::HEAD {
        Body::empty()
    } else {
        Body::from(asset.body.clone())
    };
    let mut response = body.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(asset.content_type),
    );
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from(asset.body.len()));
    Ok(response)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Exchange {
    code: String,
}

async fn exchange(state: &HttpState, request: Request<Body>) -> Result<Response, ApiError> {
    let content_type = single_header(request.headers(), header::CONTENT_TYPE);
    if !content_type.is_some_and(|value| {
        value.eq_ignore_ascii_case("application/json")
            || value.eq_ignore_ascii_case("application/json; charset=utf-8")
    }) {
        return Err(ApiError::BadRequest);
    }
    let body = to_bytes(request.into_body(), 1024)
        .await
        .map_err(|_| ApiError::TooLarge)?;
    let payload: Exchange = serde_json::from_slice(&body).map_err(|_| ApiError::BadRequest)?;
    // Malformed codes still consume a guess: the registry hashes all bounded input.
    let session = state.sessions.exchange(&payload.code)?;
    Ok(axum::Json(session).into_response())
}

fn core_path(path: &str) -> Option<String> {
    if path == "/api/health" {
        return Some("/v1/health".into());
    }
    let id = path.strip_prefix("/api/projects/")?;
    let id = uuid::Uuid::parse_str(id).ok()?;
    Some(format!("/v1/projects/{id}"))
}

fn bearer(headers: &HeaderMap) -> Result<&str, ApiError> {
    single_header(headers, header::AUTHORIZATION)
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| {
            token.len() == 64
                && token
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or(ApiError::Unauthorized)
}

fn single_header(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        None
    } else {
        Some(value)
    }
}

pub(crate) fn secure_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    for (name, value) in [
        (header::CONTENT_SECURITY_POLICY, CSP),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::CACHE_CONTROL, "no-store"),
        (header::CONNECTION, "close"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}
