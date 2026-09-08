//! Operator view excludes host paths and internal control payloads.
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
use forge_domain::{ProjectId, TaskId};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

pub(super) fn routes() -> Router<CoreService> {
    Router::new().route(
        "/v1/projects/{project_id}/tasks/{task_id}/integrations",
        get(integrations),
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    after: Option<String>,
    limit: Option<u32>,
}
async fn integrations(
    State(core): State<CoreService>,
    Path((project, task)): Path<(String, String)>,
    query: Result<Query<Page>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let task = TaskId::from(uuid_id("task_id", &task, &request)?);
    let query = query
        .map_err(|_| HttpError::invalid_request(request.clone(), "invalid integration query"))?
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
        .read_git_integrations(project, task, after, limit + 1)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let has_next = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_cursor = if has_next {
        items
            .last()
            .and_then(|item| item["id"].as_str())
            .map(str::to_owned)
    } else {
        None
    };
    Ok(json_response(
        StatusCode::OK,
        ListView { items, next_cursor },
        &request,
    ))
}
impl CoreService {
    async fn read_git_integrations(
        &self,
        project: ProjectId,
        task: TaskId,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<Value>, CoreError> {
        self.store
            .load_task(task)
            .await?
            .filter(|task| task.task.project_id() == project)
            .ok_or(CoreError::NotFound { aggregate: "Task" })?;
        self.store.git_integration_page(project,task,after,limit).await?.into_iter().map(|stored| {
            let op=stored.operation;
            Ok(json!({"id":op.id,"task_id":op.task_id,"fence":op.fence,"stage_id":op.stage_id,"stage_visit":op.stage_visit,
                "candidate_proposal_id":op.candidate_proposal_id,"candidate":op.candidate,"state":stored.state,
                "expected_task_revision":stored.expected_task_revision,"wait_condition_id":stored.wait_condition_id,"resolution":stored.resolution,
                "prepared_merge":stored.intent.map(|intent|json!({"expected_target":intent.expected_target,"merge_commit":intent.merge_commit})),
                "result_code":stored.result.map(|result|result.code),"created_at":super::views::timestamp(op.created_at)?}))
        }).collect()
    }
}
