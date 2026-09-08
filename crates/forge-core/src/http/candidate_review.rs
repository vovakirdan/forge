//! Bounded operator read of immutable revision assessments, never a write path.
use super::{
    error::HttpError,
    handlers::{json_response, project_id, request_id, uuid_id},
    views::ListView,
};
use crate::{CoreError, CoreService};
use axum::{
    Router,
    extract::{Path, Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::Response,
    routing::get,
};
use forge_domain::{ProjectId, TaskId, candidate_review::CandidateReviewRecord};
use serde::Deserialize;
use uuid::Uuid;

pub(super) fn routes() -> Router<CoreService> {
    Router::new().route(
        "/v1/projects/{project_id}/tasks/{task_id}/reviews",
        get(reviews),
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewQuery {
    after: Option<String>,
    limit: Option<u32>,
}
async fn reviews(
    State(core): State<CoreService>,
    Path((project, task)): Path<(String, String)>,
    query: Result<Query<ReviewQuery>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let task = TaskId::from(uuid_id("task_id", &task, &request)?);
    let query = query
        .map_err(|_| HttpError::invalid_request(request.clone(), "invalid review query"))?
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
    let mut items = core
        .read_candidate_reviews(project, task, after, limit + 1)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let has_next = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_cursor = if has_next {
        items.last().map(|review| review.id.to_string())
    } else {
        None
    };
    let items = items
        .into_iter()
        .map(|review| {
            let mut value =
                serde_json::to_value(&review).map_err(|_| CoreError::InvalidTransport {
                    field: "review",
                    reason: "cannot serialize review view".into(),
                })?;
            value["recorded_at"] = serde_json::json!(super::views::timestamp(review.recorded_at)?);
            Ok::<_, CoreError>(value)
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    Ok(json_response(
        StatusCode::OK,
        ListView { items, next_cursor },
        &request,
    ))
}
impl CoreService {
    async fn read_candidate_reviews(
        &self,
        project: ProjectId,
        task: TaskId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<CandidateReviewRecord>, CoreError> {
        self.store
            .load_task(task)
            .await?
            .filter(|stored| stored.task.project_id() == project)
            .ok_or(CoreError::NotFound { aggregate: "Task" })?;
        Ok(self
            .store
            .candidate_review_page(project, task, after, limit)
            .await?)
    }
}
