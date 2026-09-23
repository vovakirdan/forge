//! Task-scoped handoff provenance without body or provider output.

use axum::{
    Router,
    extract::{Path, Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::Response,
    routing::get,
};
use forge_domain::TaskId;
use serde::Deserialize;

use super::{
    error::HttpError,
    handlers::{json_response, project_id, request_id, uuid_id},
    views::ListView,
};
use crate::CoreService;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    cursor: Option<String>,
    limit: Option<u32>,
}

pub(super) fn routes() -> Router<CoreService> {
    Router::new().route(
        "/v1/projects/{project_id}/tasks/{task_id}/handoffs",
        get(handoffs),
    )
}

async fn handoffs(
    State(core): State<CoreService>,
    Path((project, task)): Path<(String, String)>,
    query: Result<Query<Page>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let task = TaskId::from(uuid_id("task_id", &task, &request)?);
    let query = query
        .map_err(|error| HttpError::invalid_request(request.clone(), error.to_string()))?
        .0;
    let limit = query.limit.unwrap_or(20);
    if !(1..=20).contains(&limit) {
        return Err(HttpError::invalid_request(
            request,
            "limit must be between 1 and 20",
        ));
    }
    let after = query
        .cursor
        .as_deref()
        .map(|cursor| uuid_id("cursor", cursor, &request))
        .transpose()?;
    core.read_task(project, task)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let mut rows = core
        .store()
        .task_handoff_page(project, task, after, limit + 1)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error.into()))?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = more
        .then(|| {
            rows.last()
                .and_then(|row| row["id"].as_str())
                .map(str::to_owned)
        })
        .flatten();
    Ok(json_response(
        StatusCode::OK,
        ListView {
            items: rows,
            next_cursor,
        },
        &request,
    ))
}
