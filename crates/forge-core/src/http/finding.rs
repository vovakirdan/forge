//! Bounded operator projection. Reading an observation never triages it.
use super::{
    error::HttpError,
    handlers::{json_response, project_id, request_id, uuid_id},
    views::{ListView, timestamp},
};
use crate::{CoreError, CoreService};
use axum::{
    Router,
    extract::{Path, Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::Response,
    routing::get,
};
use forge_domain::{
    TaskId,
    finding::{Finding, FindingState},
};
use serde::Deserialize;
use serde_json::{Value, json};

pub(super) fn routes() -> Router<CoreService> {
    Router::new().route("/v1/projects/{project_id}/findings", get(findings))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FindingQuery {
    after: Option<String>,
    task_id: Option<String>,
    limit: Option<u32>,
}
async fn findings(
    State(core): State<CoreService>,
    Path(project): Path<String>,
    query: Result<Query<FindingQuery>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let query = query
        .map_err(|_| HttpError::invalid_request(request.clone(), "invalid Finding query"))?
        .0;
    let limit = query.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(HttpError::invalid_request(request, "limit must be 1..100"));
    }
    let after = query
        .after
        .as_deref()
        .map(|value| uuid_id("after", value, &request))
        .transpose()?;
    let task = query
        .task_id
        .as_deref()
        .map(|value| uuid_id("task_id", value, &request).map(TaskId::from))
        .transpose()?;
    core.store
        .load_project(project)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error.into()))?
        .ok_or_else(|| {
            HttpError::from_core(
                request.clone(),
                CoreError::NotFound {
                    aggregate: "project",
                },
            )
        })?;
    if let Some(task) = task {
        core.store
            .load_task(task)
            .await
            .map_err(|error| HttpError::from_core(request.clone(), error.into()))?
            .filter(|stored| stored.task.project_id() == project)
            .ok_or_else(|| {
                HttpError::from_core(request.clone(), CoreError::NotFound { aggregate: "Task" })
            })?;
    }
    let mut rows = core
        .store
        .finding_page(project, task, after, limit + 1)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error.into()))?;
    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = if has_more {
        rows.last().map(|finding| finding.id.to_string())
    } else {
        None
    };
    let items = rows
        .into_iter()
        .map(view)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    Ok(json_response(
        StatusCode::OK,
        ListView { items, next_cursor },
        &request,
    ))
}
fn view(finding: Finding) -> Result<Value, CoreError> {
    let mut value = serde_json::to_value(&finding).map_err(|_| CoreError::InvalidTransport {
        field: "finding",
        reason: "cannot serialize Finding".into(),
    })?;
    value["reported_at"] = json!(timestamp(finding.reported_at)?);
    if let FindingState::Attached { at, .. }
    | FindingState::Promoted { at, .. }
    | FindingState::Ignored { at, .. } = finding.state
    {
        value["state"]["at"] = json!(timestamp(at)?);
    }
    Ok(value)
}
