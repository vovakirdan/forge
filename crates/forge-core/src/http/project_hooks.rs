//! Local operator read surface; no worker-facing hook configuration capability.
use super::{
    error::HttpError,
    handlers::{json_response, project_id, request_id, uuid_id},
    views::ListView,
};
use crate::CoreService;
use axum::{
    Router,
    extract::{Path, Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::Response,
    routing::get,
};
use serde::Deserialize;

pub(super) fn routes() -> Router<CoreService> {
    Router::new()
        .route("/v1/projects/{project_id}/hook-versions", get(versions))
        .route(
            "/v1/projects/{project_id}/hook-invocations",
            get(invocations),
        )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    after: Option<String>,
    limit: Option<u32>,
}
async fn versions(
    State(core): State<CoreService>,
    Path(project): Path<String>,
    query: Result<Query<Page>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let query = query
        .map_err(|_| HttpError::invalid_request(request.clone(), "invalid hook query"))?
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
    let mut hooks = core
        .read_project_hook_versions(project, after, limit + 1)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let has_next = hooks.len() > limit as usize;
    hooks.truncate(limit as usize);
    let next_cursor = has_next
        .then(|| hooks.last().map(|hook| hook.id.to_string()))
        .flatten();
    let mut items = Vec::with_capacity(hooks.len());
    for hook in hooks {
        let timestamp = super::views::timestamp(hook.created_at)
            .map_err(|error| HttpError::from_core(request.clone(), error))?;
        let mut value = serde_json::to_value(hook).map_err(|source| {
            HttpError::from_core(
                request.clone(),
                forge_storage::StorageError::Snapshot {
                    aggregate: "project_hook_version",
                    source,
                }
                .into(),
            )
        })?;
        value["created_at"] = timestamp.into();
        items.push(value);
    }
    Ok(json_response(
        StatusCode::OK,
        ListView { items, next_cursor },
        &request,
    ))
}

async fn invocations(
    State(core): State<CoreService>,
    Path(project): Path<String>,
    query: Result<Query<Page>, QueryRejection>,
) -> Result<Response, HttpError> {
    let request = request_id();
    let project = project_id(&project, &request)?;
    let query = query
        .map_err(|_| HttpError::invalid_request(request.clone(), "invalid invocation query"))?
        .0;
    let limit = query.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return Err(HttpError::invalid_request(request, "limit must be 1..100"));
    }
    let after = query
        .after
        .as_deref()
        .map(|value| uuid_id("after", value, &request))
        .transpose()?;
    let mut rows = core
        .read_hook_invocations(project, after, limit + 1)
        .await
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
    let has_next = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = has_next
        .then(|| rows.last().map(|row| row.id.to_string()))
        .flatten();
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let created_at = super::views::timestamp(forge_domain::Timestamp::from_offset_date_time(
            row.created_at,
        ))
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
        let updated_at = super::views::timestamp(forge_domain::Timestamp::from_offset_date_time(
            row.updated_at,
        ))
        .map_err(|error| HttpError::from_core(request.clone(), error))?;
        items.push(serde_json::json!({
            "id": row.id,
            "task_id": row.task_id,
            "pipeline_version_id": row.pipeline_version_id,
            "stage_id": row.stage_id,
            "stage_visit": row.stage_visit,
            "hook_version_id": row.hook_version_id,
            "candidate_proposal_id": row.candidate_proposal_id,
            "run_id": row.run_id,
            "state": row.state,
            "verdict": row.verdict,
            "mapped_outcome": row.mapped_outcome,
            "artifact_id": row.artifact_id,
            "created_at": created_at,
            "updated_at": updated_at,
        }));
    }
    Ok(json_response(
        StatusCode::OK,
        ListView { items, next_cursor },
        &request,
    ))
}
impl CoreService {
    pub async fn read_project_hook_versions(
        &self,
        project: forge_domain::ProjectId,
        after: Option<uuid::Uuid>,
        limit: u32,
    ) -> Result<Vec<forge_domain::ProjectHookVersion>, crate::CoreError> {
        self.read_project(project).await?;
        Ok(self
            .store
            .project_hook_versions_page(project, after, limit)
            .await?)
    }

    pub async fn read_hook_invocations(
        &self,
        project: forge_domain::ProjectId,
        after: Option<uuid::Uuid>,
        limit: u32,
    ) -> Result<Vec<forge_storage::HookInvocationView>, crate::CoreError> {
        self.read_project(project).await?;
        Ok(self
            .store
            .hook_invocations_page(project, after, limit)
            .await?)
    }
}
