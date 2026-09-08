//! Axum handlers that translate local HTTP into existing Core operations.

use std::{convert::Infallible, str::FromStr};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State, rejection::JsonRejection},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::{get, post},
};
use forge_application::CommandEnvelope;
use forge_domain::{PipelineVersionId, ProjectId, TaskId};
use forge_protocol::wire::{CommandName, CommandRequest};
use futures_util::stream;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use super::{
    error::HttpError,
    views::{
        ListView, health_view, pipeline_version_view, project_view, run_view, task_detail_view,
        task_summary_view,
    },
};
use crate::{CoreService, event_envelope_from_stored_event};

const MAX_COMMAND_BODY_BYTES: usize = 512 * 1024;
const DEFAULT_PAGE_LIMIT: usize = 50;
const MAX_PAGE_LIMIT: usize = 100;

/// Builds the complete local M0 API without giving routes direct storage access.
pub fn router(core: CoreService) -> Router {
    Router::new()
        .route("/healthz", get(crate::observability::http::healthz))
        .route("/readyz", get(crate::observability::http::readyz))
        .route("/metrics", get(crate::observability::http::metrics))
        .route("/v1/health", get(health))
        .route("/v1/commands/{name}", post(execute_command))
        .route("/v1/projects/{project_id}", get(get_project))
        .route("/v1/projects/{project_id}/tasks", get(list_tasks))
        .route("/v1/projects/{project_id}/tasks/{task_id}", get(get_task))
        .route("/v1/projects/{project_id}/pipelines", get(list_pipelines))
        .route(
            "/v1/projects/{project_id}/pipelines/{pipeline_version_id}",
            get(get_pipeline),
        )
        .route("/v1/projects/{project_id}/runs", get(list_runs))
        .route("/v1/projects/{project_id}/runs/{run_id}", get(get_run))
        .route("/v1/projects/{project_id}/events", get(stream_events))
        .merge(super::communication::routes())
        .merge(super::candidate_review::routes())
        .merge(super::git_integration::routes())
        .merge(super::project_hooks::routes())
        .merge(super::finding::routes())
        .layer(DefaultBodyLimit::max(MAX_COMMAND_BODY_BYTES))
        .layer(axum::middleware::from_fn_with_state(
            core.clone(),
            crate::observability::middleware,
        ))
        .with_state(core)
}

async fn health(State(_core): State<CoreService>) -> Response {
    json_response(StatusCode::OK, health_view(), &request_id())
}

async fn execute_command(
    State(core): State<CoreService>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Result<Json<CommandRequest>, JsonRejection>,
) -> Result<Response, HttpError> {
    let request_id = request_id();
    let name = command_name(&name, &request_id)?;
    let idempotency_key = idempotency_key(&headers, &request_id)?;
    let request = body
        .map_err(|error| HttpError::invalid_request(request_id.clone(), error.to_string()))?
        .0;
    let envelope = CommandEnvelope::parse(name, request, idempotency_key)
        .map_err(|error| HttpError::from_core(request_id.clone(), error.into()))?;
    let receipt = core
        .execute_command(envelope)
        .await
        .map_err(|error| HttpError::from_core(request_id.clone(), error))?;
    Ok(json_response(StatusCode::OK, receipt, &request_id))
}

async fn get_project(
    State(core): State<CoreService>,
    Path(path): Path<ProjectPath>,
) -> Result<Response, HttpError> {
    let request_id = request_id();
    let project_id = project_id(&path.project_id, &request_id)?;
    let project = core
        .read_project(project_id)
        .await
        .map_err(|error| HttpError::from_core(request_id.clone(), error))?;
    Ok(json_response(
        StatusCode::OK,
        project_view(&project),
        &request_id,
    ))
}

async fn list_tasks(
    State(core): State<CoreService>,
    Path(path): Path<ProjectPath>,
    query: Result<Query<PageQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, HttpError> {
    let request_id = request_id();
    let project_id = project_id(&path.project_id, &request_id)?;
    let query = page_query(query, &request_id)?;
    let tasks = core
        .read_tasks(project_id)
        .await
        .map_err(|error| HttpError::from_core(request_id.clone(), error))?;
    let page = paginate(tasks, &query, &request_id, |task| {
        task.task.id().to_string()
    })?;
    let items = page
        .items
        .iter()
        .map(|task| task_summary_view(&task.task))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| HttpError::from_core(request_id.clone(), error))?;
    Ok(json_response(
        StatusCode::OK,
        ListView {
            items,
            next_cursor: page.next_cursor,
        },
        &request_id,
    ))
}

async fn get_task(
    State(core): State<CoreService>,
    Path(path): Path<TaskPath>,
) -> Result<Response, HttpError> {
    let request_id = request_id();
    let project_id = project_id(&path.project_id, &request_id)?;
    let task_id = task_id(&path.task_id, &request_id)?;
    let task = core
        .read_task(project_id, task_id)
        .await
        .map_err(|error| HttpError::from_core(request_id.clone(), error))?;
    let view =
        task_detail_view(task).map_err(|error| HttpError::from_core(request_id.clone(), error))?;
    Ok(json_response(StatusCode::OK, view, &request_id))
}

async fn list_pipelines(
    State(core): State<CoreService>,
    Path(path): Path<ProjectPath>,
    query: Result<Query<PageQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, HttpError> {
    let request_id = request_id();
    let project_id = project_id(&path.project_id, &request_id)?;
    let query = page_query(query, &request_id)?;
    let versions = core
        .read_pipeline_versions(project_id)
        .await
        .map_err(|error| HttpError::from_core(request_id.clone(), error))?;
    let page = paginate(versions, &query, &request_id, |version| {
        version.version.id().to_string()
    })?;
    let items = page
        .items
        .into_iter()
        .map(pipeline_version_view)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| HttpError::from_core(request_id.clone(), error))?;
    Ok(json_response(
        StatusCode::OK,
        ListView {
            items,
            next_cursor: page.next_cursor,
        },
        &request_id,
    ))
}

async fn get_pipeline(
    State(core): State<CoreService>,
    Path(path): Path<PipelinePath>,
) -> Result<Response, HttpError> {
    let request_id = request_id();
    let project_id = project_id(&path.project_id, &request_id)?;
    let pipeline_version_id = pipeline_version_id(&path.pipeline_version_id, &request_id)?;
    let pipeline = core
        .read_pipeline_version(project_id, pipeline_version_id)
        .await
        .map_err(|error| HttpError::from_core(request_id.clone(), error))?;
    let view = pipeline_version_view(pipeline)
        .map_err(|error| HttpError::from_core(request_id.clone(), error))?;
    Ok(json_response(StatusCode::OK, view, &request_id))
}

async fn list_runs(
    State(core): State<CoreService>,
    Path(path): Path<ProjectPath>,
    query: Result<Query<PageQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, HttpError> {
    let request_id = request_id();
    let project_id = project_id(&path.project_id, &request_id)?;
    let query = page_query(query, &request_id)?;
    let runs = core
        .read_runs(project_id)
        .await
        .map_err(|error| HttpError::from_core(request_id.clone(), error))?;
    let page = paginate(runs, &query, &request_id, |run| run.id.to_string())?;
    let items = page.items.into_iter().map(run_view).collect();
    Ok(json_response(
        StatusCode::OK,
        ListView {
            items,
            next_cursor: page.next_cursor,
        },
        &request_id,
    ))
}

async fn get_run(
    State(core): State<CoreService>,
    Path(path): Path<RunPath>,
) -> Result<Response, HttpError> {
    let request_id = request_id();
    let project_id = project_id(&path.project_id, &request_id)?;
    let run_id = uuid_id("run_id", &path.run_id, &request_id)?;
    let run = core
        .read_run(project_id, run_id)
        .await
        .map_err(|error| HttpError::from_core(request_id.clone(), error))?;
    let diagnostics = core
        .read_run_diagnostics(project_id, run_id)
        .await
        .map_err(|error| HttpError::from_core(request_id.clone(), error))?;
    Ok(json_response(
        StatusCode::OK,
        super::views::RunDetailView {
            run: run_view(run),
            diagnostics,
        },
        &request_id,
    ))
}

async fn stream_events(
    State(core): State<CoreService>,
    Path(path): Path<ProjectPath>,
    headers: HeaderMap,
    query: Result<Query<EventQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, HttpError> {
    let request_id = request_id();
    let project_id = project_id(&path.project_id, &request_id)?;
    let query = query
        .map_err(|error| HttpError::invalid_request(request_id.clone(), error.to_string()))?
        .0;
    let cursor = event_cursor(&headers, query.after, &request_id)?;
    let events = core
        .read_events(project_id, cursor)
        .await
        .map_err(|error| HttpError::from_core(request_id.clone(), error))?;
    let records = events
        .iter()
        .map(event_record)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| HttpError::from_core(request_id.clone(), error))?;
    let response = Sse::new(stream::iter(
        records.into_iter().map(Ok::<Event, Infallible>),
    ))
    .into_response();
    Ok(with_request_id(response, &request_id))
}

fn command_name(value: &str, request_id: &str) -> Result<CommandName, HttpError> {
    serde_json::from_value(Value::String(value.to_owned())).map_err(|_| {
        HttpError::invalid_request(request_id.to_owned(), "unknown named command path")
    })
}

fn idempotency_key(headers: &HeaderMap, request_id: &str) -> Result<String, HttpError> {
    headers
        .get("idempotency-key")
        .ok_or_else(|| {
            HttpError::invalid_request(request_id.to_owned(), "Idempotency-Key is required")
        })?
        .to_str()
        .map(ToOwned::to_owned)
        .map_err(|_| {
            HttpError::invalid_request(request_id.to_owned(), "Idempotency-Key is invalid")
        })
}

pub(super) fn project_id(value: &str, request_id: &str) -> Result<ProjectId, HttpError> {
    Ok(ProjectId::from(uuid_id("project_id", value, request_id)?))
}

fn task_id(value: &str, request_id: &str) -> Result<TaskId, HttpError> {
    Ok(TaskId::from(uuid_id("task_id", value, request_id)?))
}

fn pipeline_version_id(value: &str, request_id: &str) -> Result<PipelineVersionId, HttpError> {
    Ok(PipelineVersionId::from(uuid_id(
        "pipeline_version_id",
        value,
        request_id,
    )?))
}

pub(super) fn uuid_id(
    field: &'static str,
    value: &str,
    request_id: &str,
) -> Result<Uuid, HttpError> {
    let parsed = Uuid::from_str(value).map_err(|_| {
        HttpError::invalid_request(request_id.to_owned(), format!("{field} must be a UUIDv7"))
    })?;
    if parsed.get_version_num() != 7 {
        return Err(HttpError::invalid_request(
            request_id.to_owned(),
            format!("{field} must be a UUIDv7"),
        ));
    }
    Ok(parsed)
}

fn page_query(
    query: Result<Query<PageQuery>, axum::extract::rejection::QueryRejection>,
    request_id: &str,
) -> Result<PageQuery, HttpError> {
    query
        .map_err(|error| HttpError::invalid_request(request_id.to_owned(), error.to_string()))
        .map(|query| query.0)
}

fn paginate<T>(
    values: Vec<T>,
    query: &PageQuery,
    request_id: &str,
    key: impl Fn(&T) -> String,
) -> Result<ListView<T>, HttpError> {
    let limit = query.limit.unwrap_or(DEFAULT_PAGE_LIMIT);
    if !(1..=MAX_PAGE_LIMIT).contains(&limit) {
        return Err(HttpError::invalid_request(
            request_id.to_owned(),
            format!("limit must be between 1 and {MAX_PAGE_LIMIT}"),
        ));
    }
    let start = match query.cursor.as_deref() {
        Some(cursor) => values
            .iter()
            .position(|value| key(value) == cursor)
            .map(|index| index + 1)
            .ok_or_else(|| {
                HttpError::cursor_invalid(request_id.to_owned(), "cursor is not retained")
            })?,
        None => 0,
    };
    let mut items = values.into_iter().skip(start).collect::<Vec<_>>();
    let has_next = items.len() > limit;
    items.truncate(limit);
    let next_cursor = if has_next {
        items.last().map(&key)
    } else {
        None
    };
    Ok(ListView { items, next_cursor })
}

fn event_cursor(
    headers: &HeaderMap,
    after: Option<u64>,
    request_id: &str,
) -> Result<Option<u64>, HttpError> {
    let Some(value) = headers.get("last-event-id") else {
        return Ok(after);
    };
    let value = value.to_str().map_err(|_| {
        HttpError::cursor_invalid(request_id.to_owned(), "Last-Event-ID is not valid text")
    })?;
    value.parse::<u64>().map(Some).map_err(|_| {
        HttpError::cursor_invalid(
            request_id.to_owned(),
            "Last-Event-ID must be a non-negative integer",
        )
    })
}

fn event_record(event: &forge_storage::StoredEvent) -> Result<Event, crate::CoreError> {
    let envelope = event_envelope_from_stored_event(event)?;
    let data =
        serde_json::to_string(&envelope).map_err(|error| crate::CoreError::InvalidTransport {
            field: "event.envelope",
            reason: error.to_string(),
        })?;
    Ok(Event::default()
        .id(event.project_sequence.to_string())
        .event("forge.event")
        .data(data))
}

pub(super) fn json_response<T: Serialize>(
    status: StatusCode,
    body: T,
    request_id: &str,
) -> Response {
    with_request_id((status, Json(body)).into_response(), request_id)
}

fn with_request_id(mut response: Response, request_id: &str) -> Response {
    if let Ok(value) = HeaderValue::from_str(request_id) {
        response
            .headers_mut()
            .insert(HeaderName::from_static("x-request-id"), value);
    }
    response
}

pub(super) fn request_id() -> String {
    Uuid::now_v7().to_string()
}

#[derive(Deserialize)]
struct ProjectPath {
    project_id: String,
}

#[derive(Deserialize)]
struct TaskPath {
    project_id: String,
    task_id: String,
}

#[derive(Deserialize)]
struct PipelinePath {
    project_id: String,
    pipeline_version_id: String,
}

#[derive(Deserialize)]
struct RunPath {
    project_id: String,
    run_id: String,
}

#[derive(Default, Deserialize)]
struct PageQuery {
    cursor: Option<String>,
    limit: Option<usize>,
}

#[derive(Default, Deserialize)]
struct EventQuery {
    after: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::{PageQuery, event_cursor, paginate};
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn page_cursor_resumes_strictly_after_the_last_item() {
        let page = paginate(
            vec!["first", "second", "third"],
            &PageQuery {
                cursor: Some("first".to_owned()),
                limit: Some(1),
            },
            "01900000-0000-7000-8000-000000000000",
            |value| (*value).to_owned(),
        )
        .expect("valid page");

        assert_eq!(page.items, vec!["second"]);
    }

    #[test]
    fn last_event_id_precedes_the_query_cursor() {
        let mut headers = HeaderMap::new();
        headers.insert("last-event-id", HeaderValue::from_static("42"));
        let cursor = event_cursor(&headers, Some(7), "01900000-0000-7000-8000-000000000000")
            .expect("valid cursor");

        assert_eq!(cursor, Some(42));
    }
}
