//! Owner-local SystemJob status contains policy and receipts, never credential bindings.

use axum::{
    Router,
    extract::{Path, Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::Response,
    routing::get,
};
use forge_domain::ProjectId;
use serde::Deserialize;
use serde_json::Value;

use super::{
    error::HttpError,
    handlers::{json_response, project_id, request_id, uuid_id},
    views::ListView,
};
use crate::{CoreError, CoreService};

pub(super) fn routes() -> Router<CoreService> {
    Router::new()
        .route("/v1/projects/{project_id}/system-jobs", get(status))
        .route(
            "/v1/projects/{project_id}/system-jobs/{job_id}/attempts",
            get(attempts),
        )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AttemptPage {
    cursor: Option<String>,
    limit: Option<u32>,
}

async fn attempts(
    State(core): State<CoreService>,
    Path((project, job)): Path<(String, String)>,
    query: Result<Query<AttemptPage>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let job = uuid_id("job_id", &job, &request)?;
    let query = query
        .map_err(|_| HttpError::invalid_request(request.clone(), "invalid attempt query"))?
        .0;
    let limit = query.limit.unwrap_or(20);
    if !(1..=50).contains(&limit) {
        return Err(HttpError::invalid_request(request, "limit must be 1..50"));
    }
    let cursor = query
        .cursor
        .as_deref()
        .map(|value| uuid_id("cursor", value, &request))
        .transpose()?;
    core.read_project(project)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    if !core
        .store()
        .system_job_exists(project, job)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error.into()))?
    {
        return Err(HttpError::from_core(
            request,
            CoreError::NotFound {
                aggregate: "system_job",
            },
        ));
    }
    if let Some(cursor) = cursor
        && !core
            .store()
            .system_job_attempt_cursor_exists(project, job, cursor)
            .await
            .map_err(|error| HttpError::from_core(request.clone(), error.into()))?
    {
        return Err(HttpError::cursor_invalid(
            request,
            "attempt cursor does not belong to this job",
        ));
    }
    let mut items = core
        .store()
        .system_job_attempt_page(project, job, cursor, limit + 1)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error.into()))?;
    let more = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_cursor = more
        .then(|| {
            items
                .last()
                .and_then(|item| item["id"].as_str())
                .map(str::to_owned)
        })
        .flatten();
    Ok(json_response(
        StatusCode::OK,
        ListView { items, next_cursor },
        &request,
    ))
}

async fn status(
    State(core): State<CoreService>,
    Path(project): Path<String>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let status = core
        .read_system_job_status(project)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    Ok(json_response(StatusCode::OK, status, &request))
}

impl CoreService {
    pub async fn read_system_job_status(&self, project: ProjectId) -> Result<Value, CoreError> {
        self.read_project(project).await?;
        Ok(self.store.system_job_status(project).await?)
    }
}
