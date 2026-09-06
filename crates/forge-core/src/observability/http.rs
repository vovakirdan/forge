//! Read-only exporter routes and payload-free request correlation.

use super::Operation;
use crate::CoreService;
use axum::{
    Json, Router,
    extract::{MatchedPath, Request, State},
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::get,
};
use forge_protocol::trace_context::TraceContext;
use std::time::Instant;
use tracing::Instrument;
use uuid::Uuid;

tokio::task_local! { static TRACE_CONTEXT: TraceContext; }

fn fresh_context(parent: Option<TraceContext>) -> TraceContext {
    loop {
        let trace_id = Uuid::now_v7().as_u128();
        let span_id = ((Uuid::now_v7().as_u128() & u128::from(u64::MAX)) as u64).max(1);
        if let Some(context) = parent
            .and_then(|parent| parent.child(span_id))
            .or_else(|| TraceContext::new(trace_id, span_id, false))
        {
            return context;
        }
    }
}

/// Validated context to propagate at an explicit local transport boundary.
#[must_use]
pub fn current_traceparent() -> String {
    TRACE_CONTEXT
        .try_with(|context| context.header())
        .unwrap_or_else(|_| fresh_context(None).header())
}

/// Starts a payload-free child span and carries its validated context through async work.
pub(crate) async fn trace_operation<T>(
    parent: Option<&str>,
    operation: Operation,
    future: impl std::future::Future<Output = T>,
) -> T {
    let parent = parent.and_then(TraceContext::parse);
    let context = fresh_context(parent);
    let span = tracing::info_span!("forge.operation", component="core", operation=?operation,
        trace_id=%context.trace_id(), span_id=%context.span_id(),
        parent_span_id=%parent.map(|parent| parent.span_id()).unwrap_or_default());
    TRACE_CONTEXT.scope(context, future).instrument(span).await
}

/// Safe HTTP diagnostics. Paths, queries, headers and payloads are not recorded.
pub async fn middleware(State(core): State<CoreService>, request: Request, next: Next) -> Response {
    let parent = request
        .headers()
        .get("traceparent")
        .and_then(|header| header.to_str().ok())
        .and_then(TraceContext::parse);
    let context = fresh_context(parent);
    let operation = match request
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str)
    {
        Some("/v1/commands/{name}") => Operation::HttpCommand,
        Some("/tools" | "/mcp" | "/mcp/{*rest}") => Operation::Gateway,
        _ => Operation::HttpRead,
    };
    let span = tracing::info_span!("forge.operation", component="core_http", operation=?operation,
        trace_id=%context.trace_id(), span_id=%context.span_id(),
        parent_span_id=%parent.map(|parent| parent.span_id()).unwrap_or_default(), result=tracing::field::Empty);
    let start = Instant::now();
    let mut response = TRACE_CONTEXT
        .scope(context, next.run(request))
        .instrument(span.clone())
        .await;
    let success = !response.status().is_client_error() && !response.status().is_server_error();
    span.record("result", if success { "success" } else { "failure" });
    core.record_operation(operation, success, start.elapsed());
    if let Ok(header) = HeaderValue::from_str(&context.header()) {
        response.headers_mut().insert("traceparent", header);
    }
    tracing::info!(parent: &span, status=response.status().as_u16(), elapsed_ms=start.elapsed().as_millis(), "request completed");
    response
}

/// Exposes only health/readiness/metrics, never commands or Run tool access.
pub fn router(core: CoreService) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        .with_state(core)
}

pub(crate) async fn healthz() -> Response {
    Json(serde_json::json!({"status":"ok"})).into_response()
}
pub(crate) async fn readyz(State(core): State<CoreService>) -> Response {
    let Ok(_permit) = core.observability.probes.try_acquire() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let readiness = core.readiness().await;
    (
        if readiness.ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(readiness),
    )
        .into_response()
}
pub(crate) async fn metrics(State(core): State<CoreService>) -> Response {
    let Ok(_permit) = core.observability.probes.try_acquire() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match core.metrics().await {
        Ok(metrics) => (
            [(
                "content-type",
                "application/openmetrics-text; version=1.0.0; charset=utf-8",
            )],
            metrics,
        )
            .into_response(),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}
