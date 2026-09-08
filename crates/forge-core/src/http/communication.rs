//! Scoped, bounded Inbox reads. Persistence never implies runtime delivery.
use axum::{
    Router,
    extract::{Path, Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::Response,
    routing::get,
};
use forge_domain::EmployeeId;
use serde::Deserialize;

use super::{
    error::HttpError,
    handlers::{json_response, project_id, request_id, uuid_id},
    views::ListView,
};
use crate::CoreService;

pub(super) fn routes() -> Router<CoreService> {
    Router::new()
        .route(
            "/v1/projects/{project_id}/employees/{employee_id}/threads",
            get(threads),
        )
        .route(
            "/v1/projects/{project_id}/threads/{thread_id}/messages",
            get(messages),
        )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InboxQuery {
    after: Option<String>,
    limit: Option<u32>,
}

fn query(
    value: Result<Query<InboxQuery>, QueryRejection>,
    request: &str,
) -> Result<InboxQuery, HttpError> {
    let value = value
        .map_err(|error| HttpError::invalid_request(request.to_owned(), error.to_string()))?
        .0;
    if !(1..=100).contains(&value.limit.unwrap_or(50)) {
        return Err(HttpError::invalid_request(
            request.to_owned(),
            "limit must be between 1 and 100",
        ));
    }
    Ok(value)
}

async fn threads(
    State(core): State<CoreService>,
    Path((project, employee)): Path<(String, String)>,
    value: Result<Query<InboxQuery>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let employee = EmployeeId::from(uuid_id("employee_id", &employee, &request)?);
    let query = query(value, &request)?;
    let after = query
        .after
        .as_deref()
        .map(|v| uuid_id("after", v, &request))
        .transpose()?;
    let limit = query.limit.unwrap_or(50);
    let mut items = core
        .read_employee_threads(project, employee, after, limit + 1)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let has_next = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_cursor = has_next
        .then(|| items.last().map(|v| v.data().id.to_string()))
        .flatten();
    Ok(json_response(
        StatusCode::OK,
        ListView { items, next_cursor },
        &request,
    ))
}

async fn messages(
    State(core): State<CoreService>,
    Path((project, thread)): Path<(String, String)>,
    value: Result<Query<InboxQuery>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let thread = uuid_id("thread_id", &thread, &request)?;
    let query = query(value, &request)?;
    let after = query
        .after
        .as_deref()
        .unwrap_or("0")
        .parse::<u64>()
        .ok()
        .filter(|value| *value <= i64::MAX as u64)
        .ok_or_else(|| {
            HttpError::invalid_request(
                request.clone(),
                "after must be a nonnegative database sequence",
            )
        })?;
    let limit = query.limit.unwrap_or(50);
    let mut items = core
        .read_employee_messages(project, thread, after, limit + 1)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let has_next = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_cursor = has_next
        .then(|| items.last().map(|v| v.data().sequence.to_string()))
        .flatten();
    Ok(json_response(
        StatusCode::OK,
        ListView { items, next_cursor },
        &request,
    ))
}
